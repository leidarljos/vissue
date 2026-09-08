//! In-memory query facade over a parsed issue catalog.

use chrono::Local;
use std::collections::{HashMap, HashSet};
use std::fs;

use crate::config::Layout;
use crate::error::{Error, Result};
use crate::graph::DependencyGraph;
use crate::model::{IssueHeading, READY_STATES};
use crate::related::related_hits_from;
use crate::report::parse_org_date;
use crate::store::{IssueDoc, list_projects, project_selected};
use crate::views::{
    AgendaRow, ClaimRow, Excerpt, IssueDetail, IssueRec, IssueRow, ListQuery, Recall, RecallInput,
    SearchHit, TreeNode, WalkHit,
};

pub(crate) const BODY_EXCERPT_MAX_LINES: usize = 40;
pub(crate) const BODY_EXCERPT_MAX_CHARS: usize = 4000;

/// Snapshot every heading across every project, with the path `detail` and
/// `excerpt` need.
///
/// # Errors
///
/// Returns an error if a project directory cannot be listed or an
/// `issues.org` cannot be read or parsed.
pub fn load_recs(layout: &Layout) -> Result<Vec<IssueRec>> {
    let mut recs = Vec::new();
    for project in list_projects(layout)? {
        let path = layout.project_issues_path(&project);
        let doc = IssueDoc::parse_file(&project, &path)?;
        for heading in doc.headings {
            recs.push(IssueRec {
                project: project.clone(),
                heading,
                path: path.clone(),
                tag_settings: doc.tag_settings.clone(),
            });
        }
    }
    Ok(recs)
}

/// Read-only queries over a cached `&[IssueRec]`.
#[derive(Debug)]
pub struct CatalogService<'a> {
    issues: &'a [IssueRec],
}

impl<'a> CatalogService<'a> {
    /// Query over an already-loaded catalog snapshot.
    pub fn from_recs(issues: &'a [IssueRec]) -> Self {
        Self { issues }
    }

    fn rec(&self, id: &str) -> Result<&IssueRec> {
        self.issues
            .iter()
            .find(|r| r.heading.id == id)
            .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })
    }

    /// List rows matching `q`, same filters and sort as [`issues_rows_from`].
    ///
    /// # Errors
    ///
    /// Does not fail for a parsed catalog.
    pub fn issues_rows(&self, q: ListQuery) -> Result<Vec<IssueRow>> {
        issues_rows_from(self.issues, q)
    }

    /// Actionable issues: TODO or STARTED with no open blocker.
    ///
    /// # Errors
    ///
    /// Does not fail for a parsed catalog.
    pub fn ready(&self, project: Option<&str>) -> Result<Vec<IssueRow>> {
        issues_rows_from(
            self.issues,
            ListQuery {
                project: project.map(str::to_string),
                ready: true,
                ..ListQuery::default()
            },
        )
    }

    /// One issue as a detail card, including body and logbook.
    ///
    /// # Errors
    ///
    /// Returns an error if `id` is not in the catalog.
    pub fn detail(&self, id: &str) -> Result<IssueDetail> {
        Ok(issue_detail(self.rec(id)?))
    }

    /// On-disk heading range, capped and screened for secrets.
    ///
    /// # Errors
    ///
    /// Returns an error if `id` is not in the catalog, or the heading's file
    /// cannot be read.
    pub fn excerpt(&self, id: &str) -> Result<Excerpt> {
        excerpt_from(self.rec(id)?)
    }

    /// Case-insensitive substring scan over id, title, properties, and body.
    ///
    /// # Errors
    ///
    /// Does not fail for a parsed catalog.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        search_hits_from(self.issues, query, limit)
    }

    /// Live claims, oldest first, optionally narrowed by holder or project.
    ///
    /// # Errors
    ///
    /// Does not fail for a parsed catalog.
    pub fn claims(&self, holder: Option<&str>, project: Option<&str>) -> Result<Vec<ClaimRow>> {
        claims_from(self.issues, holder, project)
    }

    /// Dated open work in the next `days` days, plus anything already overdue.
    ///
    /// # Errors
    ///
    /// Does not fail for a parsed catalog.
    pub fn agenda(&self, days: i64, project: Option<&str>) -> Result<Vec<AgendaRow>> {
        agenda_rows_from(self.issues, days, project)
    }

    /// Parent/child subtree rooted at `id`.
    ///
    /// # Errors
    ///
    /// Returns an error if `id` is not in the catalog.
    pub fn tree(&self, id: &str) -> Result<TreeNode> {
        tree_from(self.issues, id)
    }

    /// Ranked related issues for `id`.
    ///
    /// # Errors
    ///
    /// Returns an error if `id` is not in the catalog.
    pub fn related(
        &self,
        id: &str,
        depth: usize,
        limit: usize,
    ) -> Result<Vec<crate::views::RelatedHit>> {
        related_hits_from(self.issues, id, depth, limit)
    }

    /// Issues whose `:PARENT:` points at `id`.
    ///
    /// # Errors
    ///
    /// Returns an error if `id` is not in the catalog and no children exist.
    pub fn children(&self, id: &str) -> Result<Vec<WalkHit>> {
        children_from(self.issues, id)
    }

    /// Transitive blocker ancestors, limited to `depth` hops.
    ///
    /// # Errors
    ///
    /// Returns an error if `id` is not in the catalog, or the blocker graph
    /// cannot be built.
    pub fn ancestors(&self, id: &str, depth: usize) -> Result<Vec<WalkHit>> {
        walk_from(self.issues, id, depth, WalkKind::Ancestors)
    }

    /// Transitive issues waiting on `id`, limited to `depth` hops.
    ///
    /// # Errors
    ///
    /// Returns an error if `id` is not in the catalog, or the blocker graph
    /// cannot be built.
    pub fn impact(&self, id: &str, depth: usize) -> Result<Vec<WalkHit>> {
        walk_from(self.issues, id, depth, WalkKind::Impact)
    }

    /// The working set for `id`: plan, declared inputs and their deeds, and
    /// what `id` has produced.
    ///
    /// # Errors
    ///
    /// Returns an error if `id` is not in the catalog, or the blocker graph
    /// cannot be built.
    pub fn recall(&self, id: &str, depth: usize, excerpts: bool) -> Result<crate::views::Recall> {
        recall_from(self.issues, id, depth, excerpts)
    }

    /// Issues that refer to `id` through an edge, a parent, a discovered-from
    /// or pivoted-to property, or a body mention.
    ///
    /// # Errors
    ///
    /// Returns an error if `id` is not in the catalog and no backlinks exist.
    pub fn backlinks(&self, id: &str) -> Result<Vec<WalkHit>> {
        backlinks_from(self.issues, id)
    }
}

/// List/ready rows, same filters and sort as [`crate::agent::issues_json`].
///
/// # Errors
///
/// Does not fail for a parsed catalog.
pub fn issues_rows_from(issues: &[IssueRec], q: ListQuery) -> Result<Vec<IssueRow>> {
    let active_blockers: HashSet<&str> = if q.ready {
        issues
            .iter()
            .filter(|r| r.heading.state != "DONE" && r.heading.state != "CANCELLED")
            .map(|r| r.heading.id.as_str())
            .collect()
    } else {
        HashSet::new()
    };
    // Built once for the whole call rather than walked per issue. The ordered
    // check needs the parent of an issue and then that parent's other children,
    // and finding each by scanning made `ready` quadratic in the corpus on any
    // tracker whose issues have parents, which is every tracker with a plan in
    // it. `ready` is the verb an agent polls.
    let ordering = q.ready.then(|| OrderingIndex::new(issues));

    let mut rows: Vec<(char, String, String, IssueRow)> = Vec::new();
    for rec in issues {
        if !project_selected(&rec.project, q.project.as_deref()) {
            continue;
        }
        if let Some(state) = q.state.as_deref()
            && rec.heading.state != state
        {
            continue;
        }
        if q.ready {
            if !READY_STATES.contains(&rec.heading.state.as_str()) {
                continue;
            }
            if rec
                .heading
                .blocked_by()
                .iter()
                .any(|b| active_blockers.contains(b.as_str()))
            {
                continue;
            }
            if ordering
                .as_ref()
                .is_some_and(|index| ordered_sibling_holds(rec, index))
            {
                continue;
            }
        }
        if let Some(needle) = q.query.as_deref()
            && !list_query_matches(rec, needle)
        {
            continue;
        }
        rows.push((
            rec.heading.priority,
            rec.heading.state.clone(),
            rec.heading.id.clone(),
            issue_row(rec),
        ));
    }
    rows.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    let mut out: Vec<IssueRow> = rows.into_iter().map(|r| r.3).collect();
    let offset = q.offset.unwrap_or(0);
    if offset >= out.len() {
        out.clear();
    } else if offset > 0 {
        out = out.split_off(offset);
    }
    if let Some(limit) = q.limit {
        out.truncate(limit);
    }
    Ok(out)
}

/// Parents and their children, indexed once for a `ready` call.
struct OrderingIndex<'a> {
    by_id: HashMap<&'a str, &'a IssueRec>,
    children: HashMap<&'a str, Vec<&'a IssueRec>>,
}

impl<'a> OrderingIndex<'a> {
    fn new(issues: &'a [IssueRec]) -> Self {
        let mut by_id = HashMap::with_capacity(issues.len());
        let mut children: HashMap<&str, Vec<&IssueRec>> = HashMap::new();
        for rec in issues {
            by_id.insert(rec.heading.id.as_str(), rec);
            if let Some(parent) = rec.heading.parent() {
                children.entry(parent).or_default().push(rec);
            }
        }
        Self { by_id, children }
    }
}

fn ordered_sibling_holds(rec: &IssueRec, index: &OrderingIndex<'_>) -> bool {
    if crate::org::org_property_is_set(&rec.heading.properties, "NOBLOCKING") {
        return false;
    }
    let Some(parent_id) = rec.heading.parent() else {
        return false;
    };
    let Some(parent) = index.by_id.get(parent_id) else {
        return false;
    };
    if !crate::org::org_property_is_set(&parent.heading.properties, "ORDERED") {
        return false;
    }
    index
        .children
        .get(parent_id)
        .into_iter()
        .flatten()
        .any(|sib| {
            sib.heading.id != rec.heading.id
                && sib.heading.line_start < rec.heading.line_start
                && sib.heading.state != "DONE"
                && sib.heading.state != "CANCELLED"
        })
}

fn list_query_matches(rec: &IssueRec, needle: &str) -> bool {
    let h = &rec.heading;
    let needle = needle.to_lowercase();
    if h.id.to_lowercase().contains(&needle) || h.title.to_lowercase().contains(&needle) {
        return true;
    }
    if rec.tag_settings.matches_query(&h.tags(), &needle) {
        return true;
    }
    h.properties
        .iter()
        .any(|(k, v)| k.to_lowercase().contains(&needle) || v.to_lowercase().contains(&needle))
}

fn issue_row(rec: &IssueRec) -> IssueRow {
    IssueRow {
        id: rec.heading.id.clone(),
        state: rec.heading.state.clone(),
        priority: rec.heading.priority.to_string(),
        title: rec.heading.title.clone(),
        project: rec.project.clone(),
        blocked_by: rec.heading.blocked_by(),
        claimed_by: rec.heading.claimed_by().map(str::to_string),
        claimed_at: rec.heading.claimed_at().map(str::to_string),
        parent: rec.heading.parent().map(str::to_string),
    }
}

fn issue_detail(rec: &IssueRec) -> IssueDetail {
    IssueDetail {
        id: rec.heading.id.clone(),
        project: rec.project.clone(),
        title: rec.heading.title.clone(),
        state: rec.heading.state.clone(),
        priority: rec.heading.priority.to_string(),
        properties: rec.heading.properties.clone(),
        org_tags: rec.heading.org_tags.clone(),
        deeds: rec.heading.deeds(),
        tags: rec.tag_settings.all_tags(&rec.heading.tags()),
        blocked_by: rec.heading.blocked_by(),
        parent: rec.heading.parent().map(str::to_string),
        claimed_by: rec.heading.claimed_by().map(str::to_string),
        claimed_at: rec.heading.claimed_at().map(str::to_string),
        file: format!(
            "{}:{}-{}",
            rec.path.display(),
            rec.heading.line_start,
            rec.heading.line_end
        ),
        line_start: rec.heading.line_start,
        line_end: rec.heading.line_end,
        body: rec.heading.body.trim_end().to_string(),
        logbook: rec
            .heading
            .logbook
            .iter()
            .map(|e| crate::views::LogbookLine {
                timestamp: e.timestamp.clone(),
                from_state: e.from_state.clone(),
                to_state: e.to_state.clone(),
                note: e.note.clone(),
                raw: e.raw.clone(),
            })
            .collect(),
    }
}

/// On-disk heading range, capped and screened for secrets.
///
/// # Errors
///
/// Returns an error if the heading's file cannot be read.
pub fn excerpt_from(rec: &IssueRec) -> Result<Excerpt> {
    let content = fs::read_to_string(&rec.path)?;
    let lines: Vec<&str> = content.lines().collect();
    let from = rec.heading.line_start.saturating_sub(1).min(lines.len());
    let to = rec
        .heading
        .line_end
        .min(lines.len())
        .min(from + BODY_EXCERPT_MAX_LINES);
    let mut text = lines[from..to].join("\n");
    if text.len() > BODY_EXCERPT_MAX_CHARS {
        text.truncate(BODY_EXCERPT_MAX_CHARS);
        text.push_str("\n...");
    }
    let suppressed = match secret_marker(&text) {
        Some(marker) => {
            text = format!(
                "(excerpt suppressed: {marker} looks like secret material; open {} directly)\n",
                rec.path.display()
            );
            true
        }
        None => false,
    };
    Ok(Excerpt {
        id: rec.heading.id.clone(),
        file: rec.path.display().to_string(),
        line_start: rec.heading.line_start,
        line_end: rec.heading.line_end,
        text,
        suppressed,
    })
}

/// The heading's on-disk text in full, screened for secrets.
///
/// [`excerpt_from`] caps its output at the preview line cap, which is
/// right for a preview and wrong for handing the issue to someone as a
/// specification: an issue longer than the cap loses its tail silently. This
/// returns the whole range, so what comes back is what the file holds.
///
/// The secret screen stays: a heading that carries credential-shaped text is
/// refused here exactly as it is in a preview.
///
/// # Errors
///
/// Returns an error if the heading's file cannot be read, or the heading
/// looks like secret material.
pub fn org_text_from(rec: &IssueRec) -> Result<String> {
    let content = fs::read_to_string(&rec.path)?;
    let lines: Vec<&str> = content.lines().collect();
    let from = rec.heading.line_start.saturating_sub(1).min(lines.len());
    let to = rec.heading.line_end.min(lines.len()).max(from);
    let text = lines[from..to].join("\n");
    if let Some(marker) = secret_marker(&text) {
        return Err(Error::Other(anyhow::anyhow!(
            "{} looks like secret material; open {} directly",
            marker,
            rec.path.display()
        )));
    }
    Ok(text)
}

/// Text shape of [`crate::agent::body_excerpt`].
pub(crate) fn format_body_excerpt(excerpt: &Excerpt) -> String {
    if excerpt.suppressed {
        return excerpt.text.clone();
    }
    let from = excerpt.line_start.saturating_sub(1);
    let to = excerpt.line_end.min(from + BODY_EXCERPT_MAX_LINES);
    format!(
        "id: {}\nfile: {}:{}-{}\n--- excerpt (lines {}-{}) ---\n{}\n",
        excerpt.id,
        excerpt.file,
        excerpt.line_start,
        excerpt.line_end,
        from + 1,
        to,
        excerpt.text
    )
}

/// The marker that makes an excerpt look like it carries a credential.
///
/// A guard against handing an agent a secret by accident, not a redaction
/// guarantee: it screens the shapes credentials are usually written in, and
/// SECURITY.md says plainly that the answer is to keep them out of issue
/// bodies. Widening it is cheap; relying on it is not.
pub(crate) fn secret_marker(excerpt: &str) -> Option<&'static str> {
    let lower = excerpt.to_lowercase();
    // PEM and OpenSSH private key blocks, whatever the algorithm.
    if lower.contains("-----begin") && lower.contains("private key") {
        return Some("a private key block");
    }
    for token in [
        "private_key",
        "secret_key",
        "client_secret",
        "access_token",
        "refresh_token",
        "bearer ",
        "authorization:",
        "aws_secret_access_key",
        "begin rsa",
        "begin openssh",
        "begin pgp private",
    ] {
        if lower.contains(token) {
            return Some("a credential keyword");
        }
    }
    // `key = value` shapes: an assignment whose name reads like a credential
    // and whose value holds no space, which prose after a colon usually does.
    // Judged on the name, not on how random the value looks: a guard should
    // suppress a placeholder in an `api_key =` line rather than reason about
    // whether this particular one is live.
    for line in lower.lines() {
        let Some((name, value)) = line.split_once(['=', ':']) else {
            continue;
        };
        let name = name
            .trim()
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
        let value = value.trim().trim_matches(['"', '\'']);
        if value.len() < 12 || value.contains(char::is_whitespace) {
            continue;
        }
        if ["password", "passwd", "api_key", "apikey", "token", "secret"]
            .iter()
            .any(|needle| name.ends_with(needle))
        {
            return Some("an assignment to a credential name");
        }
    }
    // Token prefixes, matched on a whole word and in the case they are
    // issued in. A substring test here is what turns "making" into a cloud
    // key and "task-force" into an API one.
    for word in excerpt.split(|c: char| c.is_whitespace() || c == '"' || c == '\'') {
        let word = word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-');
        if word.len() < 12 {
            continue;
        }
        for prefix in [
            "ghp_",
            "gho_",
            "ghs_",
            "github_pat_",
            "xoxb-",
            "xoxp-",
            "xoxa-",
            "xoxs-",
            "sk-",
            "AKIA",
            "ASIA",
            "glpat-",
        ] {
            if word.starts_with(prefix) {
                return Some("a vendor token prefix");
            }
        }
    }
    None
}

/// Case-insensitive substring scan over id, title, properties, and body.
///
/// # Errors
///
/// Does not fail for a parsed catalog.
pub fn search_hits_from(issues: &[IssueRec], query: &str, limit: usize) -> Result<Vec<SearchHit>> {
    let needle = query.to_lowercase();
    let mut hits: Vec<(char, String, String, SearchHit)> = Vec::new();
    for rec in issues {
        let h = &rec.heading;
        if !search_haystack(rec).to_lowercase().contains(&needle)
            && !rec.tag_settings.matches_query(&h.tags(), &needle)
        {
            continue;
        }
        hits.push((
            h.priority,
            h.state.clone(),
            h.id.clone(),
            SearchHit {
                id: h.id.clone(),
                project: rec.project.clone(),
                state: h.state.clone(),
                priority: h.priority.to_string(),
                title: h.title.clone(),
                snippet: search_snippet(rec, &needle),
            },
        ));
    }
    hits.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    hits.truncate(limit);
    Ok(hits.into_iter().map(|h| h.3).collect())
}

fn search_haystack(rec: &IssueRec) -> String {
    let h = &rec.heading;
    let mut hay = String::new();
    hay.push_str(&h.id);
    hay.push(' ');
    hay.push_str(&h.title);
    hay.push(' ');
    for (k, v) in &h.properties {
        hay.push_str(k);
        hay.push(':');
        hay.push_str(v);
        hay.push(' ');
    }
    for tag in rec.tag_settings.all_tags(&h.tags()) {
        hay.push_str(&tag);
        hay.push(' ');
    }
    hay.push_str(&h.body);
    hay
}

fn search_snippet(rec: &IssueRec, needle: &str) -> String {
    let h = &rec.heading;
    let mut candidates = vec![h.id.clone(), h.title.clone()];
    for (k, v) in &h.properties {
        candidates.push(format!("{k}:{v}"));
    }
    candidates.extend(rec.tag_settings.all_tags(&h.tags()));
    candidates.extend(h.body.lines().map(str::to_string));
    let found = candidates
        .into_iter()
        .find(|line| line.to_lowercase().contains(needle))
        .unwrap_or_else(|| h.title.clone());
    const CAP: usize = 160;
    if found.chars().count() > CAP {
        let mut cut: String = found.chars().take(CAP).collect();
        cut.push_str("...");
        cut
    } else {
        found
    }
}

/// Live claims, oldest first, optionally narrowed by holder or project.
///
/// # Errors
///
/// Does not fail for a parsed catalog.
pub fn claims_from(
    issues: &[IssueRec],
    holder: Option<&str>,
    project: Option<&str>,
) -> Result<Vec<ClaimRow>> {
    let today = Local::now().date_naive();
    let mut rows: Vec<(String, ClaimRow)> = Vec::new();
    for rec in issues {
        if !project_selected(&rec.project, project) {
            continue;
        }
        let Some(who) = rec.heading.claimed_by() else {
            continue;
        };
        if let Some(filter) = holder
            && who != filter
        {
            continue;
        }
        let age = rec
            .heading
            .claimed_at()
            .and_then(parse_org_date)
            .map(|d| (today - d).num_days())
            .unwrap_or(-1);
        rows.push((
            rec.heading.claimed_at().unwrap_or("").to_string(),
            ClaimRow {
                id: rec.heading.id.clone(),
                project: rec.project.clone(),
                state: rec.heading.state.clone(),
                priority: rec.heading.priority.to_string(),
                holder: Some(who.to_string()),
                claimed_at: rec.heading.claimed_at().map(str::to_string),
                age_days: age,
                title: rec.heading.title.clone(),
            },
        ));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(rows.into_iter().map(|r| r.1).collect())
}

/// Dated open work in the next `days` days, plus anything already overdue.
///
/// # Errors
///
/// Does not fail for a parsed catalog.
pub fn agenda_rows_from(
    issues: &[IssueRec],
    days: i64,
    project: Option<&str>,
) -> Result<Vec<AgendaRow>> {
    let today = Local::now().date_naive();
    let horizon = today + chrono::Duration::days(days);
    let mut rows: Vec<(chrono::NaiveDate, char, AgendaRow)> = Vec::new();
    for rec in issues {
        if !project_selected(&rec.project, project) {
            continue;
        }
        let h = &rec.heading;
        if !READY_STATES.contains(&h.state.as_str()) && h.state != "BLOCKED" {
            continue;
        }
        for (kind_ch, kind, value) in [
            ('D', "deadline", h.deadline()),
            ('S', "scheduled", h.scheduled()),
        ] {
            let Some(parsed) = value.and_then(parse_org_date) else {
                continue;
            };
            if parsed > horizon {
                continue;
            }
            let delta = (parsed - today).num_days();
            rows.push((
                parsed,
                kind_ch,
                AgendaRow {
                    date: parsed.to_string(),
                    kind: kind.to_string(),
                    overdue_days: if delta < 0 { -delta } else { 0 },
                    id: h.id.clone(),
                    project: rec.project.clone(),
                    state: h.state.clone(),
                    priority: h.priority.to_string(),
                    title: h.title.clone(),
                },
            ));
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.id.cmp(&b.2.id)));
    Ok(rows.into_iter().map(|r| r.2).collect())
}

/// Parent/child subtree rooted at `id`.
///
/// # Errors
///
/// Returns an error if `id` is not in the catalog.
pub fn tree_from(issues: &[IssueRec], id: &str) -> Result<TreeNode> {
    if !issues.iter().any(|r| r.heading.id == id) {
        return Err(Error::IssueNotFound { id: id.to_string() });
    }
    let mut by_id: HashMap<&str, &IssueHeading> = HashMap::new();
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    for rec in issues {
        by_id.insert(rec.heading.id.as_str(), &rec.heading);
        if let Some(parent) = rec.heading.parent() {
            children
                .entry(parent)
                .or_default()
                .push(rec.heading.id.as_str());
        }
    }
    for kids in children.values_mut() {
        kids.sort_unstable();
    }
    Ok(build_tree(id, &by_id, &children, &mut HashSet::new()))
}

fn build_tree<'a>(
    id: &'a str,
    by_id: &HashMap<&'a str, &'a IssueHeading>,
    children: &HashMap<&'a str, Vec<&'a str>>,
    seen: &mut HashSet<&'a str>,
) -> TreeNode {
    if !seen.insert(id) {
        return TreeNode {
            id: id.to_string(),
            state: String::new(),
            title: String::new(),
            children: Vec::new(),
            blocked_by: Vec::new(),
        };
    }
    let Some(h) = by_id.get(id) else {
        return TreeNode {
            id: id.to_string(),
            state: String::new(),
            title: String::new(),
            children: Vec::new(),
            blocked_by: Vec::new(),
        };
    };
    let kids = children
        .get(id)
        .into_iter()
        .flatten()
        .map(|kid| build_tree(kid, by_id, children, seen))
        .collect();
    TreeNode {
        id: h.id.clone(),
        state: h.state.clone(),
        title: h.title.clone(),
        children: kids,
        blocked_by: h.blocked_by(),
    }
}

/// Issues whose `:PARENT:` points at `parent_id`.
///
/// # Errors
///
/// Returns an error if `parent_id` is not in the catalog and no children exist.
pub fn children_from(issues: &[IssueRec], parent_id: &str) -> Result<Vec<WalkHit>> {
    let mut rows: Vec<(char, String, String, WalkHit)> = Vec::new();
    for rec in issues {
        if rec.heading.parent() == Some(parent_id) {
            rows.push((
                rec.heading.priority,
                rec.heading.state.clone(),
                rec.heading.id.clone(),
                walk_hit(rec, "child"),
            ));
        }
    }
    if rows.is_empty() && !known_issue_id(issues, parent_id) {
        return Err(Error::IssueNotFound {
            id: parent_id.to_string(),
        });
    }
    rows.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    Ok(rows.into_iter().map(|r| r.3).collect())
}

enum WalkKind {
    Ancestors,
    Impact,
}

fn walk_from(issues: &[IssueRec], id: &str, depth: usize, kind: WalkKind) -> Result<Vec<WalkHit>> {
    let graph = DependencyGraph::from_headings(issues.iter().map(|r| &r.heading))?;
    let walked = match kind {
        WalkKind::Ancestors => graph.ancestors(id, depth)?,
        WalkKind::Impact => graph.descendants(id, depth)?,
    };
    let relation = match kind {
        WalkKind::Ancestors => "ancestor",
        WalkKind::Impact => "descendant",
    };
    Ok(walked
        .into_iter()
        .filter_map(|(_distance, other)| {
            issues
                .iter()
                .find(|r| r.heading.id == other)
                .map(|r| walk_hit(r, relation))
        })
        .collect())
}

/// The working set for `id`: its plan, its declared inputs and their products,
/// and what it has produced so far.
///
/// Retrieval is the wrong shape for this question. An agent about to work a node
/// does not need what a scorer thinks resembles it; it needs what the plan says
/// the node stands on, which the corpus already states as `:PARENT:`,
/// `:BLOCKED_BY:`, and `:DISCOVERED_FROM:`. Walking those edges answers exactly,
/// with no index to build, no embedding to drift, and no threshold to tune.
///
/// `depth` bounds the blocker walk and defaults to one hop at every caller,
/// because a deed carries its own `sources` and `deedar trail` walks them. Going
/// deeper here would re-derive, less well, a graph the deed store already holds.
///
/// # Errors
///
/// Returns an error if `id` is not in the catalog, or the blocker graph cannot
/// be built.
pub fn recall_from(issues: &[IssueRec], id: &str, depth: usize, excerpts: bool) -> Result<Recall> {
    let rec = issues
        .iter()
        .find(|r| r.heading.id == id)
        .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;

    let mut plan: Vec<WalkHit> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::from([id]);
    let mut at = rec.heading.parent();
    // A hand-edited `:PARENT:` can point back down at a descendant, and `check`
    // reports that rather than preventing it. Following it here would hang the
    // command that was supposed to explain the issue.
    while let Some(parent) = at {
        if !seen.insert(parent) {
            break;
        }
        match issues.iter().find(|r| r.heading.id == parent) {
            Some(prec) => {
                plan.push(walk_hit(prec, "plan"));
                at = prec.heading.parent();
            }
            None => {
                // A `:PARENT:` may name any Org heading with an `:ID:` under
                // the prefix, so a design document can head a work hierarchy.
                // This catalog holds issues and not that document, and the
                // document is exactly what a reader should open, so it is
                // named rather than dropped.
                plan.push(WalkHit {
                    id: parent.to_string(),
                    project: String::new(),
                    state: String::new(),
                    title: "(a heading outside the tracker)".to_string(),
                    relation: "plan".to_string(),
                });
                break;
            }
        }
    }
    plan.reverse();

    let graph = DependencyGraph::from_headings(issues.iter().map(|r| &r.heading))?;
    let mut walked = graph.ancestors(id, depth)?;
    // Furthest first: that is the order the work happened, so the products read
    // in the order they were made.
    walked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let mut inputs: Vec<RecallInput> = Vec::new();
    for (distance, other) in walked {
        let Some(orec) = issues.iter().find(|r| r.heading.id == other) else {
            continue;
        };
        let relation = if distance == 1 {
            "blocked-by".to_string()
        } else {
            format!("blocked-by:{distance}")
        };
        inputs.push(recall_input(orec, &relation, excerpts));
    }
    // Where the work came from is an input a blocker edge does not carry: a
    // bounce names its origin and nothing else points back at it.
    if let Some(origin) = crate::props::get(&rec.heading.properties, crate::props::DISCOVERED_FROM)
        && !inputs.iter().any(|i| i.id == origin)
        && let Some(orec) = issues.iter().find(|r| r.heading.id == origin)
    {
        inputs.push(recall_input(orec, "discovered-from", excerpts));
    }

    Ok(Recall {
        id: rec.heading.id.clone(),
        project: rec.project.clone(),
        state: rec.heading.state.clone(),
        title: rec.heading.title.clone(),
        plan,
        inputs,
        produced: rec.heading.deeds(),
        body: rec.heading.body.trim_end().to_string(),
    })
}

fn recall_input(rec: &IssueRec, relation: &str, excerpts: bool) -> RecallInput {
    RecallInput {
        id: rec.heading.id.clone(),
        project: rec.project.clone(),
        state: rec.heading.state.clone(),
        title: rec.heading.title.clone(),
        relation: relation.to_string(),
        deeds: rec.heading.deeds(),
        // Through the excerpt path rather than the raw body, so the cap and the
        // credential screening that `body-excerpt` applies are applied here too.
        // A failure to read the file is not worth failing the whole working set
        // over: the rest of the answer is still correct.
        excerpt: excerpts
            .then(|| excerpt_from(rec).ok().map(|e| e.text))
            .flatten(),
        // Newest first, so the first note in the drawer is the last thing that
        // was said about the issue. The tracker's own claim-release line is not
        // one of those, and it is the newest note on almost every closed issue.
        last_note: rec
            .heading
            .logbook
            .iter()
            .filter(|entry| !entry.is_bookkeeping())
            .find_map(|entry| entry.note.clone()),
    }
}

/// Issues that refer to `target_id` through an edge, a parent, a
/// discovered-from or pivoted-to property, or a body mention.
///
/// A deed accession is answered too, and there the relation is `cites`: the
/// issues carrying it in `:DEEDS:`, plus any that name it only in prose. The
/// corpus decides which of the two namespaces the target is in, so an issue id
/// that happens to look like an accession keeps its own meaning.
///
/// # Errors
///
/// Returns an error if `target_id` is neither a known issue id nor an
/// accession, and no backlinks exist.
pub fn backlinks_from(issues: &[IssueRec], target_id: &str) -> Result<Vec<WalkHit>> {
    let mut out = Vec::new();
    if !known_issue_id(issues, target_id) && crate::ops::is_deed_accession(target_id) {
        for rec in issues {
            if rec.heading.deeds().iter().any(|cited| cited == target_id) {
                out.push(walk_hit(rec, "cites"));
            } else if rec.heading.body.contains(target_id) {
                out.push(walk_hit(rec, "body mention"));
            }
        }
        // A deed nobody cited is an empty answer rather than an error. The
        // product may be real and simply unused, which is a fact about the
        // tracker and not a bad argument.
        return Ok(out);
    }
    for rec in issues {
        if rec.heading.id == target_id {
            continue;
        }
        let mut hit = false;
        if rec.heading.blocked_by().iter().any(|b| b == target_id) {
            out.push(walk_hit(rec, "blocked-by"));
            hit = true;
        }
        if rec.heading.parent() == Some(target_id) {
            out.push(walk_hit(rec, "parent"));
            hit = true;
        }
        if rec
            .heading
            .properties
            .get("DISCOVERED_FROM")
            .map(String::as_str)
            == Some(target_id)
        {
            out.push(walk_hit(rec, "discovered-from"));
            hit = true;
        }
        if rec.heading.properties.get("PIVOTED_TO").map(String::as_str) == Some(target_id) {
            out.push(walk_hit(rec, "pivoted-to"));
            hit = true;
        }
        if !hit && rec.heading.body.contains(target_id) {
            out.push(walk_hit(rec, "body mention"));
        }
    }
    if out.is_empty() && !known_issue_id(issues, target_id) {
        return Err(Error::IssueNotFound {
            id: target_id.to_string(),
        });
    }
    Ok(out)
}

/// Children and blockers below `id` as indented text or Graphviz DOT.
///
/// # Errors
///
/// Returns an error if `id` is not in the catalog, or `format` is not
/// `ascii`, `text`, or `dot`.
pub fn tree_text_from(issues: &[IssueRec], id: &str, format: &str) -> Result<String> {
    if !issues.iter().any(|r| r.heading.id == id) {
        return Err(Error::IssueNotFound { id: id.to_string() });
    }
    let mut by_id: HashMap<&str, &IssueHeading> = HashMap::new();
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut blockers: HashMap<&str, Vec<String>> = HashMap::new();
    for rec in issues {
        by_id.insert(rec.heading.id.as_str(), &rec.heading);
        if let Some(parent) = rec.heading.parent() {
            children
                .entry(parent)
                .or_default()
                .push(rec.heading.id.as_str());
        }
        let blocked = rec.heading.blocked_by();
        if !blocked.is_empty() {
            blockers.insert(rec.heading.id.as_str(), blocked);
        }
    }
    for kids in children.values_mut() {
        kids.sort_unstable();
    }
    let mut out = String::new();
    match format {
        "ascii" | "text" => tree_ascii_from(
            id,
            0,
            &by_id,
            &children,
            &blockers,
            &mut HashSet::new(),
            &mut out,
        ),
        "dot" => tree_dot_from(id, &by_id, &children, &blockers, &mut out),
        other => {
            return Err(Error::Other(anyhow::anyhow!(
                "unknown format {other:?}; allowed: ascii, dot"
            )));
        }
    }
    Ok(out)
}

fn tree_ascii_from<'a>(
    id: &'a str,
    depth: usize,
    by_id: &HashMap<&str, &IssueHeading>,
    children: &HashMap<&str, Vec<&'a str>>,
    blockers: &'a HashMap<&str, Vec<String>>,
    seen: &mut HashSet<&'a str>,
    out: &mut String,
) {
    use std::fmt::Write as _;
    if !seen.insert(id) {
        let _ = writeln!(out, "{}{id} (cycle, stopping)", "  ".repeat(depth));
        return;
    }
    let Some(h) = by_id.get(id) else {
        let _ = writeln!(out, "{}{id} (missing)", "  ".repeat(depth));
        return;
    };
    let _ = writeln!(
        out,
        "{}{id} {:<9} [#{}]  {}",
        "  ".repeat(depth),
        h.state,
        h.priority,
        h.title
    );
    if let Some(blocked) = blockers.get(id) {
        for blocker in blocked {
            let _ = writeln!(out, "{}* blocked-by {blocker}", "  ".repeat(depth + 1));
        }
    }
    if let Some(kids) = children.get(id) {
        for kid in kids {
            tree_ascii_from(kid, depth + 1, by_id, children, blockers, seen, out);
        }
    }
}

fn tree_dot_from<'a>(
    root_id: &'a str,
    by_id: &HashMap<&str, &IssueHeading>,
    children: &HashMap<&str, Vec<&'a str>>,
    blockers: &'a HashMap<&str, Vec<String>>,
    out: &mut String,
) {
    use std::fmt::Write as _;
    let _ = writeln!(out, "digraph vissue_tree {{");
    let _ = writeln!(out, "  rankdir=LR;");
    let _ = writeln!(
        out,
        "  node [shape=box, fontname=\"Jost\", style=filled, fillcolor=\"#E0F2F1\"];"
    );
    let mut visited: HashSet<&str> = HashSet::new();
    let mut stack = vec![root_id];
    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        if let Some(h) = by_id.get(id) {
            let _ = writeln!(
                out,
                "  \"{}\" [label=\"{}\\n{} [#{}]\"];",
                dot_quoted(&h.id),
                dot_quoted(&h.title),
                dot_quoted(&h.state),
                dot_quoted(&h.priority.to_string())
            );
            if let Some(kids) = children.get(id) {
                for kid in kids {
                    let _ = writeln!(
                        out,
                        "  \"{}\" -> \"{}\" [color=\"#00897B\"];",
                        dot_quoted(&h.id),
                        dot_quoted(kid)
                    );
                    stack.push(kid);
                }
            }
            if let Some(blocked) = blockers.get(id) {
                for b in blocked {
                    let _ = writeln!(
                        out,
                        "  \"{}\" -> \"{}\" [style=dashed, color=\"#FF7043\", label=\"blocks\"];",
                        dot_quoted(b),
                        dot_quoted(&h.id)
                    );
                    stack.push(b.as_str());
                }
            }
        }
    }
    let _ = writeln!(out, "}}");
}

fn dot_quoted(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "")
}

fn known_issue_id(issues: &[IssueRec], id: &str) -> bool {
    issues.iter().any(|r| r.heading.id == id)
}

fn walk_hit(rec: &IssueRec, relation: &str) -> WalkHit {
    WalkHit {
        id: rec.heading.id.clone(),
        project: rec.project.clone(),
        state: rec.heading.state.clone(),
        title: rec.heading.title.clone(),
        relation: relation.to_string(),
    }
}
