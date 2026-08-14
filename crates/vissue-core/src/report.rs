//! Read-only verbs. Every function returns its text instead of printing, so a
//! CLI, an MCP server, and a library caller share one implementation.

use anyhow::{bail, Result};
use chrono::{Local, NaiveDate};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

use crate::catalog::{load_recs, CatalogService};
use crate::config::Layout;
use crate::graph::DependencyGraph;
use crate::model::{IssueHeading, READY_STATES};
pub use crate::related::related;
use crate::store::{collect_org_ids, find_by_id, list_projects, load_all, project_selected};
use crate::views::{IssueRec, IssueRow, ListQuery};

struct GraphIndex<'a> {
    by_id: HashMap<&'a str, &'a IssueHeading>,
    children: HashMap<&'a str, Vec<&'a str>>,
    blockers: HashMap<&'a str, Vec<&'a str>>,
}

impl<'a> GraphIndex<'a> {
    fn new(all: &'a [(String, IssueHeading)]) -> Self {
        let mut index = Self {
            by_id: HashMap::with_capacity(all.len()),
            children: HashMap::new(),
            blockers: HashMap::new(),
        };
        for (_, h) in all {
            index.by_id.insert(h.id.as_str(), h);
        }
        for (_, h) in all {
            if let Some(parent) = h.parent() {
                index
                    .children
                    .entry(parent)
                    .or_default()
                    .push(h.id.as_str());
            }
            let blockers = blocker_ids(h).collect::<Vec<_>>();
            if !blockers.is_empty() {
                index.blockers.insert(h.id.as_str(), blockers);
            }
        }
        for children in index.children.values_mut() {
            children.sort_unstable();
        }
        index
    }
}

fn blocker_ids(h: &IssueHeading) -> impl Iterator<Item = &str> {
    h.properties
        .get("BLOCKED_BY")
        .into_iter()
        .flat_map(|raw| raw.split(|c: char| c == ',' || c.is_whitespace()))
        .map(str::trim)
        .filter(|id| !id.is_empty())
}

/// One row per issue: id, state, priority cookie, title.
pub fn list(
    layout: &Layout,
    project_filter: Option<&str>,
    state_filter: Option<&str>,
    ready_only: bool,
) -> Result<String> {
    let recs = load_recs(layout)?;
    let rows = CatalogService::from_recs(&recs).issues_rows(ListQuery {
        project: project_filter.map(str::to_string),
        state: state_filter.map(str::to_string),
        ready: ready_only,
        ..ListQuery::default()
    })?;
    Ok(format_issue_rows(&recs, &rows))
}

fn format_issue_rows(recs: &[IssueRec], rows: &[IssueRow]) -> String {
    let mut out = String::new();
    for row in rows {
        let suffix = recs
            .iter()
            .find(|r| r.heading.id == row.id)
            .map(|r| claim_suffix(&r.heading))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "{:<22} {:<9} [#{}]  {}{}",
            row.id, row.state, row.priority, row.title, suffix
        );
    }
    out
}

/// ` (claimed 3d by <identity>)`, or nothing when no one holds the issue.
/// Only a claimed issue grows the suffix, so an unclaimed corpus renders
/// exactly as it did before claims existed.
pub(crate) fn claim_suffix(h: &IssueHeading) -> String {
    let Some(who) = h.claimed_by() else {
        return String::new();
    };
    match h.claim_age_days(Local::now().date_naive()) {
        Some(days) => format!("  (claimed {days}d by {who})"),
        None => format!("  (claimed by {who})"),
    }
}

/// Actionable issues: TODO or STARTED with no open blocker.
pub fn ready(layout: &Layout, project_filter: Option<&str>) -> Result<String> {
    let recs = load_recs(layout)?;
    let rows = CatalogService::from_recs(&recs).ready(project_filter)?;
    Ok(format_issue_rows(&recs, &rows))
}

/// One issue's metadata and file range. The body stays in the file: an editor
/// opens the range when the prose is wanted.
pub fn show(layout: &Layout, id: &str) -> Result<String> {
    let (h, path, project) =
        find_by_id(layout, id)?.ok_or_else(|| anyhow::anyhow!("issue {id} not found"))?;
    let mut out = String::new();
    writeln!(out, "ID:       {}", h.id)?;
    writeln!(out, "Project:  {project}")?;
    writeln!(out, "Title:    {}", h.title)?;
    writeln!(out, "State:    {}", h.state)?;
    writeln!(out, "Priority: [#{}]", h.priority)?;
    if let Some(who) = h.claimed_by() {
        match h.claim_age_days(Local::now().date_naive()) {
            Some(days) => writeln!(
                out,
                "Claimed:  {who} since {} ({days}d)",
                h.claimed_at().unwrap_or("?")
            )?,
            None => writeln!(out, "Claimed:  {who}")?,
        }
    }
    let tags = h.tags();
    if !tags.is_empty() {
        writeln!(out, "Tags:     {}", tags.join(", "))?;
    }
    if h.properties.iter().any(|(k, _)| k != "ID") {
        writeln!(out, "Properties:")?;
        for (k, v) in &h.properties {
            if k == "ID" {
                continue;
            }
            writeln!(out, "  {k}: {v}")?;
        }
    }
    writeln!(
        out,
        "File:     {}:{}-{}",
        path.display(),
        h.line_start,
        h.line_end
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "(body lives in file; open the range above to read or edit)"
    )?;
    Ok(out)
}

/// Case-insensitive substring scan over id, title, properties, and body. Linear
/// in the corpus, which is the right cost until the issue count climbs.
pub fn search(layout: &Layout, query: &str, limit: usize) -> Result<String> {
    let recs = load_recs(layout)?;
    let hits = CatalogService::from_recs(&recs).search(query, limit)?;
    let mut out = String::new();
    for h in hits {
        let _ = writeln!(
            out,
            "{:<22} {:<9} [#{}]  {}  ({})",
            h.id, h.state, h.priority, h.title, h.project
        );
    }
    Ok(out)
}

/// Issues whose `:PARENT:` points at `parent_id`.
pub fn children(layout: &Layout, parent_id: &str) -> Result<String> {
    let mut rows: Vec<(String, IssueHeading)> = load_all(layout)?
        .into_iter()
        .filter(|(_, h)| h.parent() == Some(parent_id))
        .collect();
    rows.sort_by(|a, b| {
        a.1.priority
            .cmp(&b.1.priority)
            .then_with(|| a.1.state.cmp(&b.1.state))
            .then_with(|| a.1.id.cmp(&b.1.id))
    });
    let mut out = String::new();
    for (project, h) in rows {
        let _ = writeln!(
            out,
            "{:<22} {:<9} [#{}]  {}  ({})",
            h.id, h.state, h.priority, h.title, project
        );
    }
    Ok(out)
}

/// Open issues whose `:CREATED:` is at least `days` old. An issue without a
/// parseable date is never stale, because its age is unknown.
pub fn stale(layout: &Layout, days: i64, project_filter: Option<&str>) -> Result<String> {
    let today = Local::now().date_naive();
    let cutoff = today - chrono::Duration::days(days);
    let mut rows: Vec<(String, IssueHeading, NaiveDate)> = Vec::new();
    for (project, h) in load_all(layout)? {
        if !project_selected(&project, project_filter) {
            continue;
        }
        if !READY_STATES.contains(&h.state.as_str()) {
            continue;
        }
        let Some(created) = h.properties.get("CREATED") else {
            continue;
        };
        let Some(parsed) = parse_org_date(created) else {
            continue;
        };
        if parsed <= cutoff {
            rows.push((project, h, parsed));
        }
    }
    rows.sort_by_key(|r| r.2);
    let mut out = String::new();
    for (project, h, created) in rows {
        let age = (today - created).num_days();
        let _ = writeln!(
            out,
            "{:<22} {:<9} [#{}]  {} ({}d, {})",
            h.id, h.state, h.priority, h.title, age, project
        );
    }
    Ok(out)
}

/// Every live claim, oldest first: the who-holds-what view. A claim is live
/// while its issue is STARTED or BLOCKED (release happens on TODO, DONE, or
/// CANCELLED), so this is the working set, not history.
pub fn claims(
    layout: &Layout,
    holder_filter: Option<&str>,
    project_filter: Option<&str>,
    json: bool,
) -> Result<String> {
    let recs = load_recs(layout)?;
    let rows = CatalogService::from_recs(&recs).claims(holder_filter, project_filter)?;

    if json {
        return Ok(format!("{}\n", serde_json::to_value(&rows)?));
    }

    let mut out = String::new();
    for row in &rows {
        let age_txt = if row.age_days < 0 {
            "?d".to_string()
        } else {
            format!("{}d", row.age_days)
        };
        let _ = writeln!(
            out,
            "{:<22} {:<9} [#{}]  {:>4}  {}  {} ({})",
            row.id,
            row.state,
            row.priority,
            age_txt,
            row.holder.as_deref().unwrap_or("?"),
            row.title,
            row.project
        );
    }
    if rows.is_empty() {
        out.push_str("no live claims\n");
    }
    Ok(out)
}

/// Dated open work in the next `days` days, plus anything already overdue.
/// One line per (issue, date kind): deadlines first within a day, soonest day
/// first, overdue on top with a negative day count.
pub fn agenda(layout: &Layout, days: i64, project_filter: Option<&str>) -> Result<String> {
    let today = Local::now().date_naive();
    let horizon = today + chrono::Duration::days(days);
    // kind sorts D before S so a same-day deadline outranks a scheduled start.
    let mut rows: Vec<(NaiveDate, char, String, IssueHeading)> = Vec::new();
    for (project, h) in load_all(layout)? {
        if !project_selected(&project, project_filter) {
            continue;
        }
        if !READY_STATES.contains(&h.state.as_str()) && h.state != "BLOCKED" {
            continue;
        }
        for (kind, value) in [('D', h.deadline()), ('S', h.scheduled())] {
            let Some(parsed) = value.and_then(parse_org_date) else {
                continue;
            };
            if parsed <= horizon {
                rows.push((parsed, kind, project.clone(), h.clone()));
            }
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.3.id.cmp(&b.3.id)));

    let mut out = String::new();
    for (date, kind, project, h) in rows {
        let delta = (date - today).num_days();
        let when = match delta {
            d if d < 0 => format!("{}d overdue", -d),
            0 => "today".to_string(),
            d => format!("in {d}d"),
        };
        let label = if kind == 'D' { "deadline" } else { "scheduled" };
        let _ = writeln!(
            out,
            "{date}  {label:<9} {when:<11} {:<22} {:<9} [#{}]  {}  ({})",
            h.id, h.state, h.priority, h.title, project
        );
    }
    if out.is_empty() {
        out.push_str("nothing dated in range\n");
    }
    Ok(out)
}

pub(crate) fn parse_org_date(s: &str) -> Option<NaiveDate> {
    let inner = s
        .trim_start_matches(['<', '['])
        .trim_end_matches(['>', ']']);
    let token = inner.split_whitespace().next()?;
    NaiveDate::parse_from_str(token, "%Y-%m-%d").ok()
}

/// The matching issue count and nothing else, for shell pipelines.
pub fn count(
    layout: &Layout,
    project_filter: Option<&str>,
    state_filter: Option<&str>,
    ready_only: bool,
) -> Result<String> {
    let all = load_all(layout)?;
    let active_blockers: HashSet<String> = if ready_only {
        all.iter()
            .filter(|(_, h)| h.state != "DONE" && h.state != "CANCELLED")
            .map(|(_, h)| h.id.clone())
            .collect()
    } else {
        HashSet::new()
    };
    let n = all
        .iter()
        .filter(|(project, h)| {
            if !project_selected(project, project_filter) {
                return false;
            }
            if let Some(s) = state_filter {
                if h.state != s {
                    return false;
                }
            }
            if ready_only {
                if !READY_STATES.contains(&h.state.as_str()) {
                    return false;
                }
                if blocker_ids(h).any(|b| active_blockers.contains(b)) {
                    return false;
                }
            }
            true
        })
        .count();
    Ok(format!("{n}\n"))
}

/// One JSON object per line: every property, the logbook, the body, and the
/// file line range. Round-trippable, and the seam other tools consume.
pub fn export(layout: &Layout, project_filter: Option<&str>) -> Result<String> {
    let mut out = String::new();
    for (project, h) in load_all(layout)? {
        if !project_selected(&project, project_filter) {
            continue;
        }
        let logbook: Vec<serde_json::Value> = h
            .logbook
            .iter()
            .map(|e| {
                let mut row = serde_json::json!({
                    "timestamp": e.timestamp,
                    "from": e.from_state,
                    "to": e.to_state,
                    "note": e.note,
                });
                if let Some(raw) = &e.raw {
                    row["raw"] = serde_json::Value::String(raw.clone());
                }
                row
            })
            .collect();
        let row = serde_json::json!({
            "id": h.id,
            "project": project,
            "title": h.title,
            "state": h.state,
            "priority": h.priority.to_string(),
            "properties": h.properties,
            "org_tags": h.org_tags,
            "tags": h.tags(),
            "logbook": logbook,
            "body": h.body,
            "line_start": h.line_start,
            "line_end": h.line_end,
        });
        let _ = writeln!(out, "{}", row);
    }
    Ok(out)
}

/// Children and blockers below `root_id`, as indented text or Graphviz DOT.
pub fn tree(layout: &Layout, root_id: &str, format: &str) -> Result<String> {
    let all = load_all(layout)?;
    let graph = GraphIndex::new(&all);
    if !graph.by_id.contains_key(root_id) {
        bail!("issue {root_id} not found");
    }
    let mut out = String::new();
    let root = graph.by_id.get(root_id).unwrap().id.as_str();
    match format {
        "ascii" | "text" => tree_ascii(&graph, root, 0, &mut HashSet::new(), &mut out),
        "dot" => tree_dot(&graph, root, &mut out),
        _ => bail!("unknown format {format:?}; allowed: ascii, dot"),
    }
    Ok(out)
}

fn tree_ascii<'a>(
    graph: &GraphIndex<'a>,
    id: &'a str,
    depth: usize,
    seen: &mut HashSet<&'a str>,
    out: &mut String,
) {
    if !seen.insert(id) {
        let _ = writeln!(out, "{}{id} (cycle, stopping)", "  ".repeat(depth));
        return;
    }
    let Some(h) = graph.by_id.get(id) else {
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
    if let Some(blockers) = graph.blockers.get(id) {
        for blocker in blockers {
            let _ = writeln!(out, "{}* blocked-by {blocker}", "  ".repeat(depth + 1));
        }
    }
    if let Some(kids) = graph.children.get(id) {
        for k in kids {
            tree_ascii(graph, k, depth + 1, seen, out);
        }
    }
}

/// Escape text for a Graphviz quoted string. Backslash goes first, or the
/// escape introduced for a quote is itself re-escaped; a raw newline would end
/// the statement early. Titles and ids are whatever someone committed to the
/// tracker, so neither is trusted here.
pub(crate) fn dot_quoted(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "")
}

fn tree_dot<'a>(graph: &GraphIndex<'a>, root_id: &str, out: &mut String) {
    let _ = writeln!(out, "digraph vissue_tree {{");
    let _ = writeln!(out, "  rankdir=LR;");
    let _ = writeln!(
        out,
        "  node [shape=box, fontname=\"Jost\", style=filled, fillcolor=\"#E0F2F1\"];"
    );
    let mut visited: HashSet<&str> = HashSet::new();
    let mut stack = vec![graph.by_id.get(root_id).unwrap().id.as_str()];
    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        if let Some(h) = graph.by_id.get(id) {
            let _ = writeln!(
                out,
                "  \"{}\" [label=\"{}\\n{} [#{}]\"];",
                dot_quoted(&h.id),
                dot_quoted(&h.title),
                dot_quoted(&h.state),
                dot_quoted(&h.priority.to_string())
            );
            if let Some(kids) = graph.children.get(id) {
                for k in kids {
                    let _ = writeln!(
                        out,
                        "  \"{}\" -> \"{}\" [color=\"#00897B\"];",
                        dot_quoted(&h.id),
                        dot_quoted(k)
                    );
                    stack.push(k);
                }
            }
            if let Some(blockers) = graph.blockers.get(id) {
                for b in blockers {
                    let _ = writeln!(
                        out,
                        "  \"{}\" -> \"{}\" [style=dashed, color=\"#FF7043\", label=\"blocks\"];",
                        dot_quoted(b),
                        dot_quoted(&h.id)
                    );
                    stack.push(b);
                }
            }
        }
    }
    let _ = writeln!(out, "}}");
}

/// Cycles in the blocker graph, one per line, or a line saying there are none.
pub fn cycles(layout: &Layout) -> Result<String> {
    let all = load_all(layout)?;
    let graph = GraphIndex::new(&all);

    // Colored depth-first search over BLOCKED_BY edges. Grey marks the
    // current stack, black a finished node, so a shared blocker reached
    // from two branches (a diamond) is never mistaken for a cycle.
    const WHITE: u8 = 0;
    const GREY: u8 = 1;
    const BLACK: u8 = 2;
    let mut color: HashMap<&str, u8> = HashMap::new();
    let mut found: Vec<Vec<String>> = Vec::new();

    fn dfs<'a>(
        id: &'a str,
        graph: &GraphIndex<'a>,
        color: &mut HashMap<&'a str, u8>,
        path: &mut Vec<&'a str>,
        found: &mut Vec<Vec<String>>,
    ) {
        color.insert(id, GREY);
        path.push(id);
        if let Some(blockers) = graph.blockers.get(id) {
            for b in blockers {
                if !graph.by_id.contains_key(b) {
                    continue; // a broken edge cannot close a loop; `check` reports it
                }
                match color.get(b).copied().unwrap_or(WHITE) {
                    GREY => {
                        let start = path.iter().position(|&x| x == *b).unwrap();
                        let mut cycle: Vec<String> =
                            path[start..].iter().map(|s| s.to_string()).collect();
                        // Rotate so the smallest id leads: one canonical form
                        // per cycle no matter where the walk entered it.
                        let min = cycle
                            .iter()
                            .enumerate()
                            .min_by(|a, b| a.1.cmp(b.1))
                            .map(|(i, _)| i)
                            .unwrap();
                        cycle.rotate_left(min);
                        cycle.push(cycle[0].clone());
                        if !found.contains(&cycle) {
                            found.push(cycle);
                        }
                    }
                    WHITE => dfs(b, graph, color, path, found),
                    _ => {}
                }
            }
        }
        path.pop();
        color.insert(id, BLACK);
    }

    for (_, start) in &all {
        if color.get(start.id.as_str()).copied().unwrap_or(WHITE) == WHITE {
            let mut path = Vec::new();
            dfs(start.id.as_str(), &graph, &mut color, &mut path, &mut found);
        }
    }

    let mut out = String::new();
    if found.is_empty() {
        let _ = writeln!(out, "no cycles");
    } else {
        for cycle in found {
            let _ = writeln!(out, "{}", cycle.join(" -> "));
        }
    }
    Ok(out)
}

/// Transitive blocker ancestors, limited to a bounded number of hops.
pub fn ancestors(layout: &Layout, id: &str, depth: usize) -> Result<String> {
    let graph = DependencyGraph::from_issues(&load_all(layout)?)?;
    let mut out = String::new();
    for (distance, ancestor) in graph.ancestors(id, depth)? {
        writeln!(out, "{distance} {ancestor}")?;
    }
    Ok(out)
}

/// Transitive issues waiting on this issue, limited to a bounded number of hops.
pub fn impact(layout: &Layout, id: &str, depth: usize) -> Result<String> {
    let graph = DependencyGraph::from_issues(&load_all(layout)?)?;
    let mut out = String::new();
    for (distance, descendant) in graph.descendants(id, depth)? {
        writeln!(out, "{distance} {descendant}")?;
    }
    Ok(out)
}

/// The whole blocker and parent graph as Graphviz DOT. Node fill encodes state.
pub fn graph(layout: &Layout, project_filter: Option<&str>) -> Result<String> {
    let all = load_all(layout)?;
    let graph = GraphIndex::new(&all);
    let mut out = String::new();
    writeln!(out, "digraph vissue_graph {{")?;
    writeln!(out, "  rankdir=LR;")?;
    writeln!(out, "  node [shape=box, fontname=\"Jost\", style=filled];")?;
    writeln!(out, "  edge [fontname=\"Jost\"];")?;
    for (project, h) in &all {
        if !project_selected(project, project_filter) {
            continue;
        }
        let fill = match h.state.as_str() {
            "DONE" => "#A5D6A7",
            "CANCELLED" => "#CFD8DC",
            "BLOCKED" => "#FFCC80",
            "STARTED" => "#80CBC4",
            _ => "#E0F2F1",
        };
        let _ = writeln!(
            out,
            "  \"{}\" [label=\"{}\\n{} [#{}]\", fillcolor=\"{}\"];",
            dot_quoted(&h.id),
            dot_quoted(&h.title),
            dot_quoted(&h.state),
            dot_quoted(&h.priority.to_string()),
            fill
        );
    }
    for (project, h) in &all {
        if !project_selected(project, project_filter) {
            continue;
        }
        if let Some(blockers) = graph.blockers.get(h.id.as_str()) {
            for b in blockers {
                writeln!(
                    out,
                    "  \"{}\" -> \"{}\" [color=\"#FF7043\"];",
                    dot_quoted(b),
                    dot_quoted(&h.id)
                )?;
            }
        }
        if let Some(parent) = h.parent() {
            writeln!(
                out,
                "  \"{}\" -> \"{}\" [color=\"#00897B\", style=dashed];",
                dot_quoted(parent),
                dot_quoted(&h.id)
            )?;
        }
    }
    writeln!(out, "}}")?;
    Ok(out)
}

/// A markdown roadmap grouped by project and state. Closed items collapse into
/// one section so the document stays about live work.
pub fn roadmap(layout: &Layout, project_filter: Option<&str>) -> Result<String> {
    let all = load_all(layout)?;
    let mut by_project: BTreeMap<String, Vec<&IssueHeading>> = BTreeMap::new();
    for (project, h) in &all {
        if !project_selected(project, project_filter) {
            continue;
        }
        by_project.entry(project.clone()).or_default().push(h);
    }
    let mut out = String::new();
    writeln!(out, "# Roadmap")?;
    writeln!(out)?;
    writeln!(
        out,
        "Generated from `vissue roadmap`. Source of truth lives in the per-project issues.org files."
    )?;
    writeln!(out)?;
    for (project, mut headings) in by_project {
        headings.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| a.state.cmp(&b.state))
                .then_with(|| a.id.cmp(&b.id))
        });
        let buckets = ["STARTED", "TODO", "BLOCKED"];
        let active: Vec<&&IssueHeading> = headings
            .iter()
            .filter(|h| buckets.contains(&h.state.as_str()))
            .collect();
        let closed: Vec<&&IssueHeading> = headings
            .iter()
            .filter(|h| h.state == "DONE" || h.state == "CANCELLED")
            .collect();
        if active.is_empty() && closed.is_empty() {
            continue;
        }
        writeln!(out, "## {project}")?;
        writeln!(out)?;
        for state in buckets {
            let in_state: Vec<&&IssueHeading> = active
                .iter()
                .copied()
                .filter(|h| h.state == state)
                .collect();
            if in_state.is_empty() {
                continue;
            }
            writeln!(out, "### {state}")?;
            writeln!(out)?;
            for h in in_state {
                let deadline = h
                    .deadline()
                    .map(|d| format!(" :: deadline {d}"))
                    .unwrap_or_default();
                let blockers = blocker_ids(h).collect::<Vec<_>>();
                let blocked_by = if blockers.is_empty() {
                    String::new()
                } else {
                    format!(" :: blocked by {}", blockers.join(", "))
                };
                writeln!(
                    out,
                    "- **{}** [#{}] {}{}{}",
                    h.id, h.priority, h.title, deadline, blocked_by
                )?;
            }
            writeln!(out)?;
        }
        if !closed.is_empty() {
            writeln!(out, "### Closed ({} items)", closed.len())?;
            writeln!(out)?;
            for h in closed.iter().take(10) {
                writeln!(
                    out,
                    "- {} [#{}] {} ({})",
                    h.id, h.priority, h.title, h.state
                )?;
            }
            if closed.len() > 10 {
                writeln!(out, "- ... and {} more", closed.len() - 10)?;
            }
            writeln!(out)?;
        }
    }
    Ok(out)
}

/// Outcome of [`check`]: the findings, and how many were errors.
#[derive(Debug, Clone)]
pub struct CheckReport {
    pub text: String,
    pub errors: usize,
    pub warnings: usize,
}

/// Validate the corpus: every parent and blocker id resolves, dates parse, open
/// issues carry a creation date, and ids are unique across projects.
pub fn check(layout: &Layout) -> Result<CheckReport> {
    let all = load_all(layout)?;
    let org_ids = collect_org_ids(layout)?;
    let mut out = String::new();

    let mut errors = 0usize;
    let mut warnings = 0usize;

    let mut by_id: HashMap<String, (String, &IssueHeading)> = HashMap::new();
    for (project, h) in &all {
        if let Some(prev) = by_id.insert(h.id.clone(), (project.clone(), h)) {
            // An error, not a note: an id that names two issues makes every
            // blocker and parent edge pointing at it ambiguous.
            writeln!(
                out,
                "[err]  duplicate id: {} appears in {} and {}",
                h.id, prev.0, project
            )?;
            errors += 1;
        }
    }

    for (project, h) in &all {
        if let Some(parent) = h.parent() {
            if !org_ids.contains(parent) {
                writeln!(
                    out,
                    "[err]  {} (in {}) :PARENT: {} -> not found",
                    h.id, project, parent
                )?;
                errors += 1;
            }
        }
        for blk in blocker_ids(h) {
            if !by_id.contains_key(blk) {
                writeln!(
                    out,
                    "[err]  {} (in {}) :BLOCKED_BY: {} -> not found",
                    h.id, project, blk
                )?;
                errors += 1;
            }
        }
        if let Some(d) = h.deadline() {
            if parse_org_date(d).is_none() {
                writeln!(
                    out,
                    "[err]  {} (in {}) :DEADLINE: {} -> unparseable",
                    h.id, project, d
                )?;
                errors += 1;
            }
        }
        if let Some(s) = h.scheduled() {
            if parse_org_date(s).is_none() {
                writeln!(
                    out,
                    "[err]  {} (in {}) :SCHEDULED: {} -> unparseable",
                    h.id, project, s
                )?;
                errors += 1;
            }
        }
        if matches!(h.state.as_str(), "TODO" | "STARTED") && !h.properties.contains_key("CREATED") {
            writeln!(
                out,
                "[warn] {} (in {}) state={} but :CREATED: is missing",
                h.id, project, h.state
            )?;
            warnings += 1;
        }
    }

    // A :PARENT: loop passes every edge check, because each id resolves, yet
    // it makes the hierarchy unwalkable: `tree` stops on it and prints
    // "(cycle, stopping)". Naming it here is what keeps a corpus that holds
    // one from reading as clean.
    let mut settled: HashSet<&str> = HashSet::new();
    for (_, h) in &all {
        if settled.contains(h.id.as_str()) {
            continue;
        }
        let mut path: Vec<&str> = Vec::new();
        let mut on_path: HashSet<&str> = HashSet::new();
        let mut cursor = h.id.as_str();
        loop {
            if settled.contains(cursor) {
                break;
            }
            if !on_path.insert(cursor) {
                let start = path.iter().position(|id| *id == cursor).unwrap_or(0);
                let mut loop_ids: Vec<&str> = path[start..].to_vec();
                loop_ids.push(cursor);
                writeln!(out, "[err]  parent cycle: {}", loop_ids.join(" -> "))?;
                errors += 1;
                break;
            }
            path.push(cursor);
            match by_id.get(cursor).and_then(|(_, owner)| owner.parent()) {
                Some(parent) if by_id.contains_key(parent) => cursor = parent,
                _ => break,
            }
        }
        settled.extend(path);
    }

    if errors == 0 {
        if let Err(err) = DependencyGraph::from_issues(&all) {
            writeln!(out, "[err]  blocker graph: {err}")?;
            errors += 1;
        }
    }

    writeln!(out)?;
    writeln!(
        out,
        "checked {} issue(s) across {} project(s): {} error(s), {} warning(s)",
        all.len(),
        list_projects(layout)?.len(),
        errors,
        warnings
    )?;
    Ok(CheckReport {
        text: out,
        errors,
        warnings,
    })
}

/// Every issue referring to `target_id` through a blocker edge, a parent link,
/// a discovered-from link, or a body mention. The relation is named on the row.
pub fn backlinks(layout: &Layout, target_id: &str) -> Result<String> {
    let all = load_all(layout)?;
    let mut out = String::new();
    for (project, h) in &all {
        if h.id == target_id {
            continue;
        }
        let mut hit = false;
        if blocker_ids(h).any(|b| b == target_id) {
            let _ = writeln!(out, "{:<22} (blocked-by) ({})", h.id, project);
            hit = true;
        }
        if h.parent() == Some(target_id) {
            let _ = writeln!(out, "{:<22} (parent) ({})", h.id, project);
            hit = true;
        }
        if h.properties.get("DISCOVERED_FROM").map(|s| s.as_str()) == Some(target_id) {
            let _ = writeln!(out, "{:<22} (discovered-from) ({})", h.id, project);
            hit = true;
        }
        if !hit && h.body.contains(target_id) {
            let _ = writeln!(out, "{:<22} (body mention) ({})", h.id, project);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_labels_escape_untrusted_issue_text() {
        assert_eq!(dot_quoted(r#"a "quoted" title"#), r#"a \"quoted\" title"#);
        // A trailing backslash would otherwise escape the closing quote and
        // let the rest of the title become DOT syntax.
        assert_eq!(dot_quoted(r"ends with\"), r"ends with\\");
        assert_eq!(dot_quoted("two\nlines"), "two\\nlines");
    }
}
