//! Mutating verbs: create, update, and move issues between projects.

use anyhow::{anyhow, bail, Context, Result};
use chrono::NaiveDate;
use std::collections::BTreeMap;

use crate::config::{Layout, VissueConfig};
use crate::graph::DependencyGraph;
use crate::model::{today_inactive_bracket, IssueHeading, LogEntry, TODO_KEYWORDS};
use crate::store::{
    collect_org_ids, detect_project_from_ctx, find_by_id, generate_id, load_all,
    resolve_existing_project_case, with_issues_lock, with_issues_locks, IssueDoc,
};

/// Resolve the project to act on. An explicit name wins; otherwise walk up from
/// the current directory for `.project-ctx.toml` and read `[project].name`.
/// Neither available is an error, so nothing is ever guessed silently.
pub fn resolve_project(layout: &Layout, explicit: Option<&str>) -> Result<String> {
    if let Some(p) = explicit {
        if p.is_empty() {
            bail!("--project given but empty");
        }
        return resolve_existing_project_case(layout, p);
    }
    let cwd = std::env::current_dir()?;
    let detected = detect_project_from_ctx(&cwd).ok_or_else(|| {
        anyhow!(
            "no --project given and no .project-ctx.toml found walking up from {}",
            cwd.display()
        )
    })?;
    resolve_existing_project_case(layout, &detected)
}

/// Optional fields on a new issue.
#[derive(Debug, Default, Clone, Copy)]
pub struct CreateOpts<'a> {
    pub priority: Option<char>,
    pub issue_type: Option<&'a str>,
    pub deadline: Option<&'a str>,
    pub scheduled: Option<&'a str>,
    pub tags: Option<&'a str>,
    pub parent: Option<&'a str>,
    /// Print only the new id.
    pub quiet: bool,
    /// Body prose written under the properties drawer.
    pub body: Option<&'a str>,
}

/// Append a new TODO issue to the project's file and return the status text.
pub fn create(layout: &Layout, project: &str, title: &str, opts: CreateOpts<'_>) -> Result<String> {
    let project = resolve_existing_project_case(layout, project)?;
    let cfg = VissueConfig::load(layout)?;
    let priority = opts.priority.unwrap_or(cfg.issues.default_priority);
    if !"ABC".contains(priority) {
        bail!("invalid priority {priority:?}; allowed: A B C");
    }
    let path = layout.project_issues_path(&project);

    // Validating the parent scans every org file, so do it outside the lock.
    if let Some(p) = opts.parent {
        if !collect_org_ids(layout)?.contains(p) {
            bail!("--parent {p} does not refer to any known id");
        }
    }

    with_issues_lock(&path, || {
        let mut doc = IssueDoc::parse_file(&project, &path)?;
        let id = generate_id(&project, &doc.known_ids(), cfg.issues.id_length);

        let mut props = BTreeMap::new();
        props.insert("ID".into(), id.clone());
        props.insert("CREATED".into(), today_inactive_bracket());
        if let Some(t) = opts.issue_type {
            props.insert("TYPE".into(), t.into());
        }
        if let Some(d) = opts.deadline {
            validate_org_date(d)?;
            props.insert("DEADLINE".into(), d.into());
        }
        if let Some(s) = opts.scheduled {
            validate_org_date(s)?;
            props.insert("SCHEDULED".into(), s.into());
        }
        if let Some(tags) = opts.tags {
            let normalized: Vec<String> = tags
                .split([',', ':'])
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !normalized.is_empty() {
                props.insert("TAGS".into(), normalized.join(","));
            }
        }
        if let Some(p) = opts.parent {
            props.insert("PARENT".into(), p.into());
        }

        doc.headings.push(IssueHeading {
            id: id.clone(),
            title: title.to_string(),
            state: "TODO".into(),
            priority,
            properties: props,
            property_order: Vec::new(),
            body: match opts.body {
                Some(b) if !b.trim().is_empty() => format!("{}\n", b.trim_end()),
                _ => String::new(),
            },
            logbook: Vec::new(),
            line_start: 0,
            line_end: 0,
        });
        doc.write()?;

        if opts.quiet {
            Ok(format!("{id}\n"))
        } else {
            Ok(format!(
                "{id}  TODO  [#{priority}]  {title}\nfile: {}\n",
                path.display()
            ))
        }
    })
}

pub(crate) fn validate_org_date(s: &str) -> Result<()> {
    let inner = s
        .trim_start_matches(['<', '['])
        .trim_end_matches(['>', ']']);
    let token = inner.split_whitespace().next().unwrap_or("");
    NaiveDate::parse_from_str(token, "%Y-%m-%d").with_context(|| {
        format!("expected org date like <YYYY-MM-DD> or [YYYY-MM-DD], got {s:?}")
    })?;
    Ok(())
}

/// Change state, priority, or blocker edges. Adding a blocker to an open issue
/// moves it to BLOCKED; clearing the last blocker moves it back to TODO.
pub fn update(
    layout: &Layout,
    id: &str,
    new_state: Option<&str>,
    new_priority: Option<char>,
    block_add: Option<&str>,
    block_clear: Option<&str>,
) -> Result<UpdateOutcome> {
    let (_h0, path, project) =
        find_by_id(layout, id)?.ok_or_else(|| anyhow!("issue {id} not found"))?;
    let identity = crate::config::identity(layout);
    let graph = if block_add.is_some() {
        Some(DependencyGraph::from_issues(&load_all(layout)?)?)
    } else {
        None
    };

    let (final_state, changed) = with_issues_lock(&path, || {
        let mut doc = IssueDoc::parse_file(&project, &path)?;
        let h = doc
            .headings
            .iter_mut()
            .find(|x| x.id == id)
            .ok_or_else(|| anyhow!("issue {id} not found"))?;

        let mut changed = Vec::new();

        if let Some(s) = new_state {
            if !TODO_KEYWORDS.contains(&s) {
                bail!("invalid state {s:?}; allowed: {TODO_KEYWORDS:?}");
            }
            if h.state != s {
                let from = h.state.clone();
                h.record_state_change(s);
                changed.push(format!("state {from} -> {s}"));
                for note in settle_claim(h, &from, s, &identity) {
                    changed.push(note);
                }
            }
        }

        if let Some(p) = new_priority {
            if !"ABC".contains(p) {
                bail!("invalid priority {p:?}; allowed: A B C");
            }
            if h.priority != p {
                h.priority = p;
                changed.push(format!("priority -> [#{p}]"));
            }
        }

        if let Some(blk) = block_add {
            let mut current = h.blocked_by();
            if !current.iter().any(|x| x == blk) {
                if let Some(graph) = &graph {
                    graph.accepts_edge(blk, id)?;
                }
                current.push(blk.to_string());
                h.properties.insert("BLOCKED_BY".into(), current.join(","));
                if h.state == "TODO" || h.state == "STARTED" {
                    let from = h.state.clone();
                    h.record_state_change("BLOCKED");
                    changed.push(format!("state {from} -> BLOCKED (auto on block)"));
                }
                changed.push(format!("blocked_by += {blk}"));
            }
        }

        if let Some(blk) = block_clear {
            let mut current = h.blocked_by();
            let before = current.len();
            current.retain(|x| x != blk);
            if current.len() < before {
                if current.is_empty() {
                    h.properties.remove("BLOCKED_BY");
                    if h.state == "BLOCKED" {
                        let from = h.state.clone();
                        h.record_state_change("TODO");
                        changed.push("state BLOCKED -> TODO (auto on unblock)".to_string());
                        for note in settle_claim(h, &from, "TODO", &identity) {
                            changed.push(note);
                        }
                    }
                } else {
                    h.properties.insert("BLOCKED_BY".into(), current.join(","));
                }
                changed.push(format!("blocked_by -= {blk}"));
            }
        }

        if changed.is_empty() {
            return Ok((None, Vec::new()));
        }

        let final_state = h.state.clone();
        doc.write()?;
        Ok((Some(final_state), changed))
    })?;

    if changed.is_empty() {
        return Ok(UpdateOutcome {
            report: format!("{id}: no change\n"),
            hints: Vec::new(),
        });
    }

    let mut hints = Vec::new();
    if matches!(final_state.as_deref(), Some("DONE") | Some("CANCELLED")) {
        for (other_project, other) in load_all(layout)? {
            if !other.blocked_by().iter().any(|b| b == id) {
                continue;
            }
            if other.state == "DONE" || other.state == "CANCELLED" {
                continue;
            }
            hints.push(format!(
                "{} (in {}) lists this as a blocker; clear with `vissue update {} --unblock {}`",
                other.id, other_project, other.id, id
            ));
        }
    }
    Ok(UpdateOutcome {
        report: format!("{id}: {}\n", changed.join(", ")),
        hints,
    })
}

/// States that keep a claim: someone still holds the issue even when it is
/// waiting on something else. Leaving for TODO, DONE, or CANCELLED gives it up.
fn keeps_claim(state: &str) -> bool {
    matches!(state, "STARTED" | "BLOCKED")
}

/// Take or give up the claim as the state moves.
///
/// Entering STARTED unclaimed stamps the identity; leaving for a state that
/// holds no claim releases it, and the logbook keeps who held it and since
/// when.
fn settle_claim(h: &mut IssueHeading, from: &str, to: &str, identity: &str) -> Vec<String> {
    let mut notes = Vec::new();
    if to == "STARTED" && h.claimed_by().is_none() {
        h.set_claim(identity);
        notes.push(format!("claimed by {identity}"));
    } else if keeps_claim(from) && !keeps_claim(to) {
        if let Some((who, _when)) = h.release_claim() {
            notes.push(format!("claim released ({who})"));
        }
    }
    notes
}

/// Take an issue: move it to STARTED and stamp the claim.
///
/// A claim held by another identity is refused unless `force`, which records
/// the takeover in the logbook rather than losing it.
pub fn claim(layout: &Layout, id: &str, force: bool) -> Result<String> {
    let (_h0, path, project) =
        find_by_id(layout, id)?.ok_or_else(|| anyhow!("issue {id} not found"))?;
    let identity = crate::config::identity(layout);

    let report = with_issues_lock(&path, || {
        let mut doc = IssueDoc::parse_file(&project, &path)?;
        let h = doc
            .headings
            .iter_mut()
            .find(|x| x.id == id)
            .ok_or_else(|| anyhow!("issue {id} not found"))?;

        if h.state == "DONE" || h.state == "CANCELLED" {
            bail!("{id} is already {}; cannot claim", h.state);
        }
        if let Some(holder) = h.claimed_by() {
            if holder != identity && !force {
                bail!(
                    "{id} is claimed by {holder} since {}; pass --force to take it over",
                    h.claimed_at().unwrap_or("an unknown time")
                );
            }
            if holder != identity {
                let previous = holder.to_string();
                h.release_claim();
                h.set_claim(&identity);
                h.record_state_change("STARTED");
                doc.write()?;
                return Ok(format!("claimed {id} (taken over from {previous})\n"));
            }
        }

        let was = h.state.clone();
        h.record_state_change("STARTED");
        if h.claimed_by().is_none() {
            h.set_claim(&identity);
        }
        doc.write()?;
        if was == "STARTED" {
            Ok(format!("claimed {id} by {identity}\n"))
        } else {
            Ok(format!("claimed {id} by {identity} ({was} -> STARTED)\n"))
        }
    })?;
    Ok(report)
}

/// What an update changed, plus advice about issues left dangling by it.
#[derive(Debug, Clone)]
pub struct UpdateOutcome {
    pub report: String,
    pub hints: Vec<String>,
}

/// Add a dated note to the top of an issue's logbook. State, claim, and
/// properties stay untouched, so an agent can record progress without owning
/// the issue.
pub fn note(layout: &Layout, id: &str, text: &str) -> Result<String> {
    // One line in the drawer: fold internal whitespace, and swap double
    // quotes for singles so the rendered `- Note: "..."` line re-parses.
    let text = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('"', "'");
    if text.is_empty() {
        bail!("note text is empty");
    }
    let (_h0, path, project) =
        find_by_id(layout, id)?.ok_or_else(|| anyhow!("issue {id} not found"))?;
    with_issues_lock(&path, || {
        let mut doc = IssueDoc::parse_file(&project, &path)?;
        let h = doc
            .headings
            .iter_mut()
            .find(|x| x.id == id)
            .ok_or_else(|| anyhow!("issue {id} not found"))?;
        // Newest first, matching state transitions and claim releases. A
        // drawer written from both ends reads as sorted by neither.
        h.logbook.insert(
            0,
            LogEntry {
                timestamp: LogEntry::now(),
                from_state: None,
                to_state: None,
                note: Some(text.clone()),
                raw: None,
            },
        );
        doc.write()?;
        Ok(format!("{id}: noted\n"))
    })
}

/// Fold an inbox-convention org file into tracked issues.
///
/// Each top-level `* TODO <title>` heading that does not already carry a
/// `:VISSUE_ID:` line becomes an issue in `project` (body = the heading's
/// text up to the next heading). The heading is then flipped to DONE and
/// stamped with the assigned id in place, which is what makes a second run
/// a no-op: stamped headings are skipped, so folding is idempotent.
pub fn fold(layout: &Layout, inbox: &std::path::Path, project: &str) -> Result<String> {
    let project = resolve_existing_project_case(layout, project)?;
    let text = std::fs::read_to_string(inbox)
        .with_context(|| format!("read inbox {}", inbox.display()))?;
    let lines: Vec<String> = text.lines().map(str::to_string).collect();

    struct Entry {
        line: usize,
        title: String,
        body: String,
        stamped: bool,
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(title) = lines[i].strip_prefix("* TODO ") {
            let start = i + 1;
            let end = lines[start..]
                .iter()
                .position(|l| l.starts_with("* "))
                .map(|off| start + off)
                .unwrap_or(lines.len());
            let stamped = lines[start..end]
                .iter()
                .any(|l| l.trim_start().starts_with(":VISSUE_ID:"));
            let body = lines[start..end].join("\n").trim().to_string();
            entries.push(Entry {
                line: i,
                title: title.trim().to_string(),
                body,
                stamped,
            });
            i = end;
        } else {
            i += 1;
        }
    }

    // Stamping inserts lines, so rewrite from the bottom up to keep the
    // recorded line numbers valid.
    let mut out = lines.clone();
    let mut created: Vec<String> = Vec::new();
    for e in entries.iter().rev() {
        if e.stamped {
            continue;
        }
        let printed = create(
            layout,
            &project,
            &e.title,
            CreateOpts {
                quiet: true,
                body: if e.body.is_empty() {
                    None
                } else {
                    Some(&e.body)
                },
                ..CreateOpts::default()
            },
        )?;
        let id = printed.trim().to_string();
        out[e.line] = format!("* DONE {}", e.title);
        out.insert(e.line + 1, format!(":VISSUE_ID: {id}"));
        created.push(id);
    }
    created.reverse();

    if created.is_empty() {
        return Ok("folded 0 (nothing unstamped)\n".into());
    }
    let mut rendered = out.join("\n");
    if text.ends_with('\n') {
        rendered.push('\n');
    }
    std::fs::write(inbox, rendered).with_context(|| format!("write inbox {}", inbox.display()))?;
    Ok(format!("folded {}: {}\n", created.len(), created.join(" ")))
}

/// Move one issue's heading to another project's file. The id is not
/// regenerated, so cross-project blocker edges keep resolving.
pub fn refile(layout: &Layout, id: &str, to_project: &str) -> Result<String> {
    let to_project = resolve_existing_project_case(layout, to_project)?;
    let target_path = layout.project_issues_path(&to_project);
    let (_heading, src_path, src_project) =
        find_by_id(layout, id)?.ok_or_else(|| anyhow!("issue {id} not found"))?;
    if src_project == to_project {
        return Ok(format!("{id} already in {to_project}; nothing to do\n"));
    }
    with_issues_locks(&[&src_path, &target_path], || {
        let mut src_doc = IssueDoc::parse_file(&src_project, &src_path)?;
        let heading = src_doc
            .remove(id)
            .ok_or_else(|| anyhow!("issue {id} not found in {}", src_path.display()))?;

        // Two files cannot be replaced in one atomic step, so choose which
        // half-finished state a failure leaves behind. Writing the target
        // first means a failed source write duplicates the id, which `check`
        // reports and a person can resolve; the other order deletes the issue
        // with nothing left naming it.
        let mut tgt_doc = IssueDoc::parse_file(&to_project, &target_path)?;
        tgt_doc.upsert(heading);
        tgt_doc.write()?;
        src_doc.write()?;
        Ok(())
    })?;
    Ok(format!("{id}: {src_project} -> {to_project}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_PREFIX;
    use std::fs;
    use std::path::Path;

    fn fresh_layout(dir: &Path) -> Layout {
        fs::create_dir_all(dir.join(DEFAULT_PREFIX)).unwrap();
        Layout::new(dir, DEFAULT_PREFIX)
    }

    fn issue_at(layout: &Layout, project: &str, id: &str) -> IssueHeading {
        IssueDoc::parse_file(project, &layout.project_issues_path(project))
            .unwrap()
            .headings
            .into_iter()
            .find(|h| h.id == id)
            .expect("issue not found")
    }

    fn only_id(layout: &Layout, project: &str) -> String {
        IssueDoc::parse_file(project, &layout.project_issues_path(project))
            .unwrap()
            .headings[0]
            .id
            .clone()
    }

    #[test]
    fn create_rejects_a_parent_that_does_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        let err = create(
            &layout,
            "sample",
            "child without parent",
            CreateOpts {
                parent: Some("sample-zzz9"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("does not refer to any known id"));
    }

    #[test]
    fn create_accepts_a_parent_defined_in_a_design_document() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        let parent_id = "sample-spec-20260615";
        let project_dir = layout.projects_dir().join("sample");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(
            project_dir.join("design.org"),
            format!("#+TITLE: sample design\n\n* Design\n:PROPERTIES:\n:ID:         {parent_id}\n:END:\n"),
        )
        .unwrap();

        create(
            &layout,
            "sample",
            "child under design",
            CreateOpts {
                parent: Some(parent_id),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(only_id(&layout, "sample").starts_with("sample-"));
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        assert_eq!(doc.headings[0].parent(), Some(parent_id));
    }

    #[test]
    fn a_state_update_writes_a_logbook_entry() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "first", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");
        update(&layout, &id, Some("STARTED"), None, None, None).unwrap();
        let h = issue_at(&layout, "sample", &id);
        assert_eq!(h.state, "STARTED");
        assert_eq!(h.logbook[0].from_state.as_deref(), Some("TODO"));
        assert_eq!(h.logbook[0].to_state.as_deref(), Some("STARTED"));
    }

    #[test]
    fn blocking_and_unblocking_drive_the_state() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "first", CreateOpts::default()).unwrap();
        create(&layout, "sample", "blocker", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let first = doc.headings[0].id.clone();
        let blocker = doc.headings[1].id.clone();

        update(&layout, &first, None, None, Some(&blocker), None).unwrap();
        let h = issue_at(&layout, "sample", &first);
        assert_eq!(h.state, "BLOCKED");
        assert!(h.blocked_by().contains(&blocker));

        update(&layout, &first, None, None, None, Some(&blocker)).unwrap();
        let h = issue_at(&layout, "sample", &first);
        assert_eq!(h.state, "TODO");
        assert!(h.blocked_by().is_empty());
    }

    #[test]
    fn auto_unblock_to_todo_releases_the_claim() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "first", CreateOpts::default()).unwrap();
        create(&layout, "sample", "blocker", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let first = doc.headings[0].id.clone();
        let blocker = doc.headings[1].id.clone();

        crate::agent::claim(&layout, &first, false).unwrap();
        update(&layout, &first, None, None, Some(&blocker), None).unwrap();
        assert!(issue_at(&layout, "sample", &first).claimed_by().is_some());

        update(&layout, &first, None, None, None, Some(&blocker)).unwrap();
        let h = issue_at(&layout, "sample", &first);
        assert_eq!(h.state, "TODO");
        assert!(h.claimed_by().is_none(), "claim stuck on TODO: {h:?}");
    }

    #[test]
    fn blocker_cycle_is_rejected_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "first", CreateOpts::default()).unwrap();
        create(&layout, "sample", "second", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let first = doc.headings[0].id.clone();
        let second = doc.headings[1].id.clone();

        update(&layout, &first, None, None, Some(&second), None).unwrap();
        let err = update(&layout, &second, None, None, Some(&first), None).unwrap_err();
        assert!(err.to_string().contains("blocker cycle"), "{err}");
        assert!(issue_at(&layout, "sample", &second).blocked_by().is_empty());
    }

    #[test]
    fn closing_a_blocker_reports_the_issues_still_pointing_at_it() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "first", CreateOpts::default()).unwrap();
        create(&layout, "sample", "blocker", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let first = doc.headings[0].id.clone();
        let blocker = doc.headings[1].id.clone();
        update(&layout, &first, None, None, Some(&blocker), None).unwrap();

        let outcome = update(&layout, &blocker, Some("DONE"), None, None, None).unwrap();
        assert_eq!(outcome.hints.len(), 1, "{:?}", outcome.hints);
        assert!(outcome.hints[0].contains(&first), "{:?}", outcome.hints);
    }

    #[test]
    fn refile_moves_the_heading_between_projects() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "source", "the issue", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "source");
        refile(&layout, &id, "target").unwrap();

        let src = IssueDoc::parse_file("source", &layout.project_issues_path("source")).unwrap();
        let tgt = IssueDoc::parse_file("target", &layout.project_issues_path("target")).unwrap();
        assert!(src.headings.is_empty());
        assert_eq!(tgt.headings[0].id, id);
    }

    #[test]
    fn deadlines_must_parse_as_org_dates() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        let err = create(
            &layout,
            "sample",
            "bad date",
            CreateOpts {
                deadline: Some("not-a-date"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("expected org date"));

        for (i, d) in ["<2026-05-15 Fri>", "[2026-05-15]"].iter().enumerate() {
            create(
                &layout,
                "sample",
                &format!("issue {i}"),
                CreateOpts {
                    deadline: Some(d),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        assert_eq!(doc.headings.len(), 2);
        assert!(doc.headings.iter().all(|h| h.deadline().is_some()));
    }

    #[test]
    fn tags_are_normalised_to_a_comma_list() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(
            &layout,
            "sample",
            "tagged",
            CreateOpts {
                tags: Some("rust: perf ,, scaling"),
                ..Default::default()
            },
        )
        .unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        assert_eq!(
            doc.headings[0].properties.get("TAGS").map(|s| s.as_str()),
            Some("rust,perf,scaling")
        );
    }

    #[test]
    fn resolve_project_needs_a_name_from_somewhere() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        assert_eq!(
            resolve_project(&layout, Some("fromcli")).unwrap(),
            "fromcli"
        );
        assert!(resolve_project(&layout, Some(""))
            .unwrap_err()
            .to_string()
            .contains("empty"));
    }

    /// Parallel creates must not lose headings or fail the temporary rename.
    #[test]
    fn concurrent_creates_preserve_every_heading() {
        use std::sync::Arc;
        use std::thread;

        let dir = tempfile::tempdir().unwrap();
        let layout = Arc::new(fresh_layout(dir.path()));
        let n = 24usize;
        let handles: Vec<_> = (0..n)
            .map(|i| {
                let layout = Arc::clone(&layout);
                thread::spawn(move || {
                    create(
                        &layout,
                        "sample",
                        &format!("parallel title {i}"),
                        CreateOpts {
                            quiet: true,
                            ..Default::default()
                        },
                    )
                })
            })
            .collect();
        let mut ids: Vec<String> = handles
            .into_iter()
            .map(|h| {
                h.join()
                    .expect("thread panicked")
                    .expect("create failed")
                    .trim()
                    .to_string()
            })
            .collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "expected {n} unique ids, got {ids:?}");

        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let mut on_disk: Vec<String> = doc.headings.iter().map(|h| h.id.clone()).collect();
        on_disk.sort();
        assert_eq!(on_disk, ids);
    }

    #[test]
    fn note_appends_to_the_logbook_and_leaves_state_alone() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "carries a note", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");

        let out = note(&layout, &id, "first pass done,\n  \"quoted\" bit next").unwrap();
        assert_eq!(out, format!("{id}: noted\n"));

        let h = issue_at(&layout, "sample", &id);
        assert_eq!(h.state, "TODO");
        assert!(h.claimed_by().is_none());
        let notes: Vec<&str> = h.logbook.iter().filter_map(|e| e.note.as_deref()).collect();
        // Whitespace collapses to single spaces; double quotes become single.
        assert_eq!(notes, vec!["first pass done, 'quoted' bit next"]);
    }

    #[test]
    fn the_logbook_reads_newest_first_however_an_entry_arrived() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "ordered", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");

        note(&layout, &id, "first note").unwrap();
        update(&layout, &id, Some("STARTED"), None, None, None).unwrap();
        note(&layout, &id, "second note").unwrap();

        let h = issue_at(&layout, "sample", &id);
        let summary: Vec<String> = h
            .logbook
            .iter()
            .map(|e| match (&e.note, &e.to_state) {
                (Some(note), _) => note.clone(),
                (_, Some(to)) => format!("state:{to}"),
                _ => "?".into(),
            })
            .collect();
        assert_eq!(
            summary,
            vec!["second note", "state:STARTED", "first note"],
            "{h:?}"
        );
    }

    #[test]
    fn note_rejects_empty_text_and_unknown_ids() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "target", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");
        assert!(note(&layout, &id, "   ").is_err());
        assert!(note(&layout, "sample-zzz9", "text").is_err());
    }

    #[test]
    fn fold_creates_issues_and_stamps_the_inbox_idempotently() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "seed", CreateOpts::default()).unwrap();

        let inbox = dir.path().join("inbox.org");
        fs::write(
            &inbox,
            "#+TITLE: inbox\n\n\
             * TODO first discovered thing\nSome body line.\nAnother line.\n\
             * DONE already handled elsewhere\n\
             * TODO second discovered thing\n",
        )
        .unwrap();

        let out = fold(&layout, &inbox, "sample").unwrap();
        assert!(out.starts_with("folded 2: "), "got: {out}");

        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let titles: Vec<&str> = doc.headings.iter().map(|h| h.title.as_str()).collect();
        assert!(titles.contains(&"first discovered thing"));
        assert!(titles.contains(&"second discovered thing"));
        let folded = doc
            .headings
            .iter()
            .find(|h| h.title == "first discovered thing")
            .unwrap();
        assert!(folded.body.contains("Some body line."));

        // Headings flipped to DONE and stamped with the assigned id.
        let stamped = fs::read_to_string(&inbox).unwrap();
        assert_eq!(stamped.matches("* DONE ").count(), 3);
        assert_eq!(stamped.matches(":VISSUE_ID: sample-").count(), 2);
        assert!(!stamped.contains("* TODO "));

        // Second fold finds nothing unstamped and creates nothing.
        let again = fold(&layout, &inbox, "sample").unwrap();
        assert_eq!(again, "folded 0 (nothing unstamped)\n");
        let doc2 = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        assert_eq!(doc2.headings.len(), doc.headings.len());
    }
}
