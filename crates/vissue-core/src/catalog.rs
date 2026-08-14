//! In-memory query facade over a parsed issue catalog.

use chrono::Local;
use std::collections::{HashMap, HashSet};
use std::fs;

use crate::config::Layout;
use crate::error::Error;
use crate::graph::DependencyGraph;
use crate::model::{IssueHeading, READY_STATES};
use crate::related::related_hits_from;
use crate::report::parse_org_date;
use crate::store::{list_projects, project_selected, IssueDoc};
use crate::views::{
    AgendaRow, ClaimRow, Excerpt, IssueDetail, IssueRec, IssueRow, ListQuery, SearchHit, TreeNode,
    WalkHit,
};

pub(crate) const BODY_EXCERPT_MAX_LINES: usize = 40;
pub(crate) const BODY_EXCERPT_MAX_CHARS: usize = 4000;

/// Snapshot every heading across every project, with the path `detail` and
/// `excerpt` need.
pub fn load_recs(layout: &Layout) -> anyhow::Result<Vec<IssueRec>> {
    let mut recs = Vec::new();
    for project in list_projects(layout)? {
        let path = layout.project_issues_path(&project);
        let doc = IssueDoc::parse_file(&project, &path)?;
        for heading in doc.headings {
            recs.push(IssueRec {
                project: project.clone(),
                heading,
                path: path.clone(),
            });
        }
    }
    Ok(recs)
}

/// Read-only queries over a cached `&[IssueRec]`.
pub struct CatalogService<'a> {
    issues: &'a [IssueRec],
}

impl<'a> CatalogService<'a> {
    pub fn from_recs(issues: &'a [IssueRec]) -> Self {
        Self { issues }
    }

    fn rec(&self, id: &str) -> Result<&IssueRec, Error> {
        self.issues
            .iter()
            .find(|r| r.heading.id == id)
            .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })
    }

    pub fn issues_rows(&self, q: ListQuery) -> Result<Vec<IssueRow>, Error> {
        issues_rows_from(self.issues, q)
    }

    pub fn ready(&self, project: Option<&str>) -> Result<Vec<IssueRow>, Error> {
        issues_rows_from(
            self.issues,
            ListQuery {
                project: project.map(str::to_string),
                ready: true,
                ..ListQuery::default()
            },
        )
    }

    pub fn detail(&self, id: &str) -> Result<IssueDetail, Error> {
        Ok(issue_detail(self.rec(id)?))
    }

    pub fn excerpt(&self, id: &str) -> Result<Excerpt, Error> {
        excerpt_from(self.rec(id)?)
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, Error> {
        search_hits_from(self.issues, query, limit)
    }

    pub fn claims(
        &self,
        holder: Option<&str>,
        project: Option<&str>,
    ) -> Result<Vec<ClaimRow>, Error> {
        claims_from(self.issues, holder, project)
    }

    pub fn agenda(&self, days: i64, project: Option<&str>) -> Result<Vec<AgendaRow>, Error> {
        agenda_rows_from(self.issues, days, project)
    }

    pub fn tree(&self, id: &str) -> Result<TreeNode, Error> {
        tree_from(self.issues, id)
    }

    pub fn related(
        &self,
        id: &str,
        depth: usize,
        limit: usize,
    ) -> Result<Vec<crate::views::RelatedHit>, Error> {
        related_hits_from(self.issues, id, depth, limit)
    }

    pub fn children(&self, id: &str) -> Result<Vec<WalkHit>, Error> {
        children_from(self.issues, id)
    }

    pub fn ancestors(&self, id: &str, depth: usize) -> Result<Vec<WalkHit>, Error> {
        walk_from(self.issues, id, depth, WalkKind::Ancestors)
    }

    pub fn impact(&self, id: &str, depth: usize) -> Result<Vec<WalkHit>, Error> {
        walk_from(self.issues, id, depth, WalkKind::Impact)
    }

    pub fn backlinks(&self, id: &str) -> Result<Vec<WalkHit>, Error> {
        backlinks_from(self.issues, id)
    }
}

/// List/ready rows, same filters and sort as [`crate::agent::issues_json`].
pub fn issues_rows_from(issues: &[IssueRec], q: ListQuery) -> Result<Vec<IssueRow>, Error> {
    let active_blockers: HashSet<&str> = if q.ready {
        issues
            .iter()
            .filter(|r| r.heading.state != "DONE" && r.heading.state != "CANCELLED")
            .map(|r| r.heading.id.as_str())
            .collect()
    } else {
        HashSet::new()
    };

    let mut rows: Vec<(char, String, String, IssueRow)> = Vec::new();
    for rec in issues {
        if !project_selected(&rec.project, q.project.as_deref()) {
            continue;
        }
        if let Some(state) = q.state.as_deref() {
            if rec.heading.state != state {
                continue;
            }
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
        }
        if let Some(needle) = q.query.as_deref() {
            if !list_query_matches(&rec.heading, needle) {
                continue;
            }
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

fn list_query_matches(h: &IssueHeading, needle: &str) -> bool {
    let needle = needle.to_lowercase();
    if h.id.to_lowercase().contains(&needle) || h.title.to_lowercase().contains(&needle) {
        return true;
    }
    if h.tags()
        .iter()
        .any(|tag| tag.to_lowercase().contains(&needle))
    {
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
        tags: rec.heading.tags(),
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
    }
}

/// On-disk heading range, capped and screened for secrets.
pub fn excerpt_from(rec: &IssueRec) -> Result<Excerpt, Error> {
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

pub fn search_hits_from(
    issues: &[IssueRec],
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, Error> {
    let needle = query.to_lowercase();
    let mut hits: Vec<(char, String, String, SearchHit)> = Vec::new();
    for rec in issues {
        let h = &rec.heading;
        if !search_haystack(h).to_lowercase().contains(&needle) {
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
                snippet: search_snippet(h, &needle),
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

fn search_haystack(h: &IssueHeading) -> String {
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
    for tag in h.tags() {
        hay.push_str(&tag);
        hay.push(' ');
    }
    hay.push_str(&h.body);
    hay
}

fn search_snippet(h: &IssueHeading, needle: &str) -> String {
    let mut candidates = vec![h.id.clone(), h.title.clone()];
    for (k, v) in &h.properties {
        candidates.push(format!("{k}:{v}"));
    }
    candidates.extend(h.tags());
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

pub fn claims_from(
    issues: &[IssueRec],
    holder: Option<&str>,
    project: Option<&str>,
) -> Result<Vec<ClaimRow>, Error> {
    let today = Local::now().date_naive();
    let mut rows: Vec<(String, ClaimRow)> = Vec::new();
    for rec in issues {
        if !project_selected(&rec.project, project) {
            continue;
        }
        let Some(who) = rec.heading.claimed_by() else {
            continue;
        };
        if let Some(filter) = holder {
            if who != filter {
                continue;
            }
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

pub fn agenda_rows_from(
    issues: &[IssueRec],
    days: i64,
    project: Option<&str>,
) -> Result<Vec<AgendaRow>, Error> {
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

pub fn tree_from(issues: &[IssueRec], id: &str) -> Result<TreeNode, Error> {
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

pub fn children_from(issues: &[IssueRec], parent_id: &str) -> Result<Vec<WalkHit>, Error> {
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

fn walk_from(
    issues: &[IssueRec],
    id: &str,
    depth: usize,
    kind: WalkKind,
) -> Result<Vec<WalkHit>, Error> {
    let pairs: Vec<_> = issues
        .iter()
        .map(|r| (r.project.clone(), r.heading.clone()))
        .collect();
    let graph = DependencyGraph::from_issues(&pairs).map_err(Error::from)?;
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

pub fn backlinks_from(issues: &[IssueRec], target_id: &str) -> Result<Vec<WalkHit>, Error> {
    let mut out = Vec::new();
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
        if !hit && rec.heading.body.contains(target_id) {
            out.push(walk_hit(rec, "body mention"));
        }
    }
    Ok(out)
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
