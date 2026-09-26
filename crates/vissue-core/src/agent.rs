//! Verbs shaped for a program rather than a person: structured rows, claiming,
//! a body excerpt, and a hygiene checklist.

use crate::error::Result;
use serde_json::Value;
use std::fmt::Write as _;

use crate::catalog::{CatalogService, excerpt_from, format_body_excerpt, load_recs};
use crate::config::Layout;
use crate::error::Error;
use crate::ops;
use crate::report;
use crate::store::{list_projects, load_all};
use crate::views::ListQuery;

/// Issue rows as JSON, filtered the same way [`report::list`] filters them.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read, or the rows cannot be
/// serialized.
pub fn issues_json(
    layout: &Layout,
    project_filter: Option<&str>,
    state_filter: Option<&str>,
    ready_only: bool,
) -> Result<Value> {
    Ok(serde_json::to_value(issues_rows(
        layout,
        project_filter,
        state_filter,
        ready_only,
    )?)?)
}

/// The same rows, still typed.
///
/// A caller that has to publish the shape it returns needs the type rather than
/// the value: a schema taken from `IssueRow` cannot disagree with what this
/// hands back, and one written beside it can.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read or the filter is not a state.
pub fn issues_rows(
    layout: &Layout,
    project_filter: Option<&str>,
    state_filter: Option<&str>,
    ready_only: bool,
) -> Result<Vec<crate::views::IssueRow>> {
    let recs = load_recs(layout)?;
    issues_rows_in(&recs, project_filter, state_filter, ready_only)
}

/// [`issues_rows`] over a corpus the caller already holds.
///
/// # Errors
///
/// Does not fail for a parsed corpus.
pub fn issues_rows_in(
    recs: &[crate::views::IssueRec],
    project_filter: Option<&str>,
    state_filter: Option<&str>,
    ready_only: bool,
) -> Result<Vec<crate::views::IssueRow>> {
    CatalogService::from_recs(recs).issues_rows(ListQuery {
        project: project_filter.map(str::to_string),
        state: state_filter.map(str::to_string),
        ready: ready_only,
        ..ListQuery::default()
    })
}

/// One issue as JSON, including its file and line range.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read, `id` is not in it, or the
/// detail cannot be serialized.
pub fn show_json(layout: &Layout, id: &str) -> Result<Value> {
    Ok(serde_json::to_value(show_detail(layout, id)?)?)
}

/// The same card, still typed. See [`issues_rows`] for why both exist.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read or `id` is not in it.
pub fn show_detail(layout: &Layout, id: &str) -> Result<crate::views::IssueDetail> {
    let recs = load_recs(layout)?;
    CatalogService::from_recs(&recs).detail(id)
}

/// Take an issue: move it to STARTED and stamp the claim.
///
/// # Errors
///
/// Returns an error if `id` is not in the corpus, the issue is DONE or
/// CANCELLED, another identity holds it and `force` is false, or the file
/// cannot be rewritten.
pub fn claim(layout: &Layout, id: &str, force: bool) -> Result<String> {
    let report = ops::claim(layout, id, force)?;
    let detail = report::show(layout, id)?;
    Ok(format!("{report}{detail}"))
}

/// The first lines of an issue's file range, capped and screened for secrets.
///
/// # Errors
///
/// Returns an error if `id` is not in the corpus, or the heading's file
/// cannot be read.
pub fn body_excerpt(layout: &Layout, id: &str) -> Result<String> {
    let recs = load_recs(layout)?;
    let rec = recs
        .iter()
        .find(|r| r.heading.id == id)
        .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
    Ok(format_body_excerpt(&excerpt_from(rec)?))
}

/// One issue's org text, in full, ready to write to a file.
///
/// [`body_excerpt`] is a preview and truncates; this does not. It is what a
/// caller wants when the issue is being handed to someone as the thing to
/// work from, rather than glanced at.
///
/// # Errors
///
/// Returns an error if `id` is not in the corpus, the heading's file cannot
/// be read, or the heading looks like secret material.
pub fn org_text(layout: &Layout, id: &str) -> Result<String> {
    let recs = load_recs(layout)?;
    let rec = recs
        .iter()
        .find(|r| r.heading.id == id)
        .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
    let mut text = crate::catalog::org_text_from(rec)?;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    Ok(text)
}

/// Issues waiting on this one.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
pub fn waiting_on(layout: &Layout, id: &str) -> Result<String> {
    report::backlinks(layout, id)
}

/// The agent and CI checklist: issues claimed but not actionable, claims that
/// have gone stale, plus the corpus validation summary.
///
/// `stale_days` overrides the configured threshold when given.
///
/// # Errors
///
/// Returns an error if the corpus or configuration cannot be read.
pub fn hygiene(layout: &Layout, stale_days: Option<i64>) -> Result<String> {
    let mut out = String::new();
    writeln!(out, "=== vissue hygiene ===")?;
    writeln!(
        out,
        "[note] agents write through vissue / MCP; never Write or StrReplace issues.org"
    )?;

    // Compare ids, not rendered rows: `id_length` is configurable, so one id
    // can be a prefix of another and a row match would pair the wrong issues.
    let ready_ids: std::collections::HashSet<String> = issues_json(layout, None, None, true)?
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut started_not_ready = 0usize;
    for (project, h) in load_all(layout)? {
        if h.state != "STARTED" || ready_ids.contains(&h.id) {
            continue;
        }
        started_not_ready += 1;
        writeln!(
            out,
            "[warn] STARTED but not ready (blockers?): {} ({project})  {}",
            h.id, h.title
        )?;
    }

    let threshold = match stale_days {
        Some(d) => d,
        None => {
            crate::config::VissueConfig::load(layout)?
                .issues
                .stale_claim_days
        }
    };
    let today = chrono::Local::now().date_naive();
    let mut stale_claims = 0usize;
    let mut unclaimed_started = 0usize;
    struct HolderRow {
        count: usize,
        last: Option<chrono::NaiveDate>,
    }
    let mut holders: std::collections::BTreeMap<String, HolderRow> =
        std::collections::BTreeMap::new();
    for (project, h) in load_all(layout)? {
        if h.state == "STARTED" && h.claimed_by().is_none() {
            unclaimed_started += 1;
            writeln!(out, "[warn] STARTED with no claimant: {} ({project})", h.id)?;
            continue;
        }
        let Some(who) = h.claimed_by() else {
            continue;
        };
        if h.state != "STARTED" && h.state != "BLOCKED" {
            continue;
        }
        let last = h.last_activity_date();
        let age = h.last_activity_age_days(today);
        if age.is_some_and(|d| d > threshold) {
            stale_claims += 1;
        }
        let row = holders.entry(who.to_string()).or_insert(HolderRow {
            count: 0,
            last: None,
        });
        row.count += 1;
        row.last = match (row.last, last) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
    }
    let mut holder_rows: Vec<(String, HolderRow)> = holders.into_iter().collect();
    holder_rows.sort_by(|a, b| match (a.1.last, b.1.last) {
        (Some(x), Some(y)) => x.cmp(&y).then_with(|| a.0.cmp(&b.0)),
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, None) => a.0.cmp(&b.0),
    });
    let stale_holders = holder_rows
        .iter()
        .filter(|(_, row)| row.last.is_some_and(|d| (today - d).num_days() > threshold))
        .count();
    if !holder_rows.is_empty() {
        writeln!(
            out,
            "holders ({} live, {stale_holders} stale over {threshold}d):",
            holder_rows.len()
        )?;
        for (name, row) in &holder_rows {
            let age = row.last.map(|d| (today - d).num_days());
            let age_txt = age.map(|d| format!("{d}d")).unwrap_or_else(|| "?d".into());
            let last_txt = row
                .last
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "?".into());
            let flag = if age.is_some_and(|d| d > threshold) {
                "stale"
            } else {
                "    "
            };
            writeln!(
                out,
                "  {:>5}  {:>5}  {last_txt}  {flag}  {name}",
                row.count, age_txt
            )?;
        }
    }

    // Only where the tracker says the handoff matters. Elsewhere it would
    // report every answered question and closed review as a hole.
    let mut closed_without_a_product = 0usize;
    if crate::config::VissueConfig::load(layout)?
        .issues
        .expect_deeds
    {
        for (project, h) in load_all(layout)? {
            if h.state != "DONE" || !h.deeds().is_empty() {
                continue;
            }
            closed_without_a_product += 1;
            writeln!(
                out,
                "[warn] closed without naming what it made: {} ({project})  {}",
                h.id, h.title
            )?;
        }
    }

    let check = report::check(layout)?;
    if check.errors == 0 {
        writeln!(out, "[ok] check passed")?;
    } else {
        writeln!(out, "[fail] check found {} error(s)", check.errors)?;
        for line in check.text.lines().filter(|l| l.starts_with("[err]")) {
            writeln!(out, "{line}")?;
        }
    }
    writeln!(
        out,
        "summary: started_not_ready={started_not_ready} stale_claims={stale_claims} stale_holders={stale_holders} holders={} unclaimed_started={unclaimed_started} closed_without_a_product={closed_without_a_product} projects={} errors={} warnings={}",
        holder_rows.len(),
        list_projects(layout)?.len(),
        check.errors,
        check.warnings
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::secret_marker;
    use crate::config::DEFAULT_PREFIX;
    use crate::ops::{CreateOpts, create, update};
    use crate::store::IssueDoc;
    use std::fs;

    fn layout_with_two_issues() -> (tempfile::TempDir, Layout, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        fs::create_dir_all(layout.projects_dir()).unwrap();
        create(&layout, "sample", "first", CreateOpts::default()).unwrap();
        create(&layout, "sample", "blocker", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let first = doc.headings[0].id.clone();
        let blocker = doc.headings[1].id.clone();
        (dir, layout, first, blocker)
    }

    #[test]
    fn claim_moves_an_open_issue_to_started() {
        let (_dir, layout, first, _blocker) = layout_with_two_issues();
        let text = claim(&layout, &first, false).unwrap();
        assert!(text.starts_with(&format!("claimed {first}")), "{text}");
        assert!(text.contains("State:    STARTED"), "{text}");
    }

    #[test]
    fn claim_refuses_a_closed_issue() {
        let (_dir, layout, first, _blocker) = layout_with_two_issues();
        update(&layout, &first, Some("DONE"), None, None, None).unwrap();
        let err = claim(&layout, &first, false).unwrap_err();
        assert!(err.to_string().contains("cannot claim"), "{err}");
    }

    #[test]
    fn ready_json_drops_blocked_issues() {
        let (_dir, layout, first, blocker) = layout_with_two_issues();
        update(&layout, &first, None, None, Some(&blocker), None).unwrap();
        let rows = issues_json(&layout, None, None, true).unwrap();
        let ids: Vec<&str> = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec![blocker.as_str()], "{rows}");
    }

    #[test]
    fn show_json_carries_the_file_range() {
        let (_dir, layout, first, _blocker) = layout_with_two_issues();
        let row = show_json(&layout, &first).unwrap();
        assert_eq!(row["id"].as_str(), Some(first.as_str()));
        assert_eq!(row["project"].as_str(), Some("sample"));
        assert!(
            row["file"].as_str().unwrap().contains("issues.org:"),
            "{row}"
        );
    }

    /// Work that closed naming nothing is a hole in the handoff, but only on a
    /// tracker that expects one. Reporting it everywhere would flag every
    /// answered question and closed review, which is how a checklist stops
    /// being read.
    #[test]
    fn closed_work_that_named_no_product_is_reported_only_where_it_is_expected() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        fs::create_dir_all(layout.projects_dir()).unwrap();
        create(&layout, "sample", "made something", CreateOpts::default()).unwrap();
        create(&layout, "sample", "made nothing", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let (first, second) = (doc.headings[0].id.clone(), doc.headings[1].id.clone());
        crate::ops::deed(&layout, &first, &["deed-file-thing".to_string()], &[]).unwrap();
        for id in [&first, &second] {
            update(&layout, id, Some("DONE"), None, None, None).unwrap();
        }

        let quiet = hygiene(&layout, None).unwrap();
        assert!(
            quiet.contains("closed_without_a_product=0"),
            "off by default: {quiet}"
        );
        assert!(!quiet.contains("closed without naming"), "{quiet}");

        fs::write(
            dir.path().join("vissue.toml"),
            "[issues]\nexpect_deeds = true\n",
        )
        .unwrap();
        let strict = hygiene(&layout, None).unwrap();
        assert!(
            strict.contains("closed_without_a_product=1"),
            "one of the two named nothing: {strict}"
        );
        assert!(
            strict.contains(&second) && !strict.contains(&format!("made: {first}")),
            "the one that cited a deed is not a hole: {strict}"
        );
    }

    #[test]
    fn hygiene_flags_a_started_issue_that_is_blocked() {
        let (_dir, layout, first, blocker) = layout_with_two_issues();
        update(&layout, &first, Some("STARTED"), None, None, None).unwrap();
        // Blocking would flip the state, so write the edge without the state move.
        let path = layout.project_issues_path("sample");
        let mut doc = IssueDoc::parse_file("sample", &path).unwrap();
        doc.headings
            .iter_mut()
            .find(|h| h.id == first)
            .unwrap()
            .properties
            .insert("BLOCKED_BY".into(), blocker.clone());
        doc.write().unwrap();

        let text = hygiene(&layout, None).unwrap();
        assert!(
            text.contains("never Write or StrReplace issues.org"),
            "{text}"
        );
        assert!(text.contains("STARTED but not ready"), "{text}");
        assert!(text.contains("started_not_ready=1"), "{text}");
        assert!(text.contains("[ok] check passed"), "{text}");
    }

    #[test]
    fn hygiene_groups_claims_by_holder() {
        let (_dir, layout, first, _blocker) = layout_with_two_issues();
        crate::ops::claim_as(&layout, &first, false, "quiet-holder").unwrap();

        let text = hygiene(&layout, Some(7)).unwrap();
        assert!(
            text.contains("holders (1 live, 0 stale over 7d):"),
            "{text}"
        );
        assert!(text.contains("quiet-holder"), "{text}");
        assert!(text.contains("stale_claims=0"), "{text}");
        assert!(text.contains("stale_holders=0"), "{text}");
        assert!(text.contains("holders=1"), "{text}");
        assert!(
            !text.contains("claim held"),
            "per-issue stale lines came back: {text}"
        );
    }

    #[test]
    fn a_note_from_someone_else_does_not_keep_a_dead_holder_live() {
        let (_dir, layout, first, _blocker) = layout_with_two_issues();
        crate::ops::claim_as(&layout, &first, false, "dead-host").unwrap();
        let path = layout.project_issues_path("sample");
        let mut doc = IssueDoc::parse_file("sample", &path).unwrap();
        doc.headings
            .iter_mut()
            .find(|h| h.id == first)
            .unwrap()
            .properties
            .insert("CLAIMED_AT".into(), "[2026-01-11 Sun]".into());
        doc.write().unwrap();
        crate::ops::note(&layout, &first, "progress from another seat").unwrap();

        let text = hygiene(&layout, Some(7)).unwrap();
        assert!(text.contains("dead-host"), "{text}");
        assert!(text.contains("stale_holders=1"), "{text}");
        assert!(text.contains("stale_claims=1"), "{text}");
        assert!(
            text.contains("stale"),
            "a later note kept the dead holder live: {text}"
        );
        assert!(
            !text.contains("  0d"),
            "the later note counted as this holder's activity: {text}"
        );
    }

    #[test]
    fn body_excerpt_returns_the_heading_range() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        fs::create_dir_all(layout.projects_dir()).unwrap();
        create(
            &layout,
            "sample",
            "documented",
            CreateOpts {
                body: Some("Scope: the excerpt path.\nDone-when: it reads back."),
                ..Default::default()
            },
        )
        .unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let text = body_excerpt(&layout, &doc.headings[0].id).unwrap();
        assert!(text.contains("Scope: the excerpt path."), "{text}");
        assert!(text.contains("Done-when: it reads back."), "{text}");
    }

    #[test]
    fn the_secret_screen_reads_shapes_not_substrings() {
        // Suppressed: the shapes a credential is actually written in.
        for carrier in [
            // Assembled rather than written out: a literal PEM header in a
            // source file is exactly what the private-key hook looks for.
            concat!("-----BEGIN OPENSSH ", "PRIVATE KEY-----"),
            "aws_secret_access_key = wJalrXUtnFEMI",
            "Authorization: Bearer abcdefghijklmno",
            "api_key = 9f8e7d6c5b4a3210ff",
            "token: ghp_0123456789abcdefghij",
            "AKIAIOSFODNN7EXAMPLE is the key",
        ] {
            assert!(
                secret_marker(carrier).is_some(),
                "missed a credential: {carrier:?}"
            );
        }
        // Not suppressed: ordinary prose. A substring screen flags every one
        // of these -- "making" holds "aki", "task-force" holds "sk-".
        for prose in [
            "Scope: read the header block before the first record.",
            "making the parser reject a bad manifest",
            "the task-force agreed on the schema",
            "deployments in Asia are slower",
            "next-token: reviewed by the release owner",
            "Deadline: the parser lands before the notes.",
            "See the design note for the token grammar.",
        ] {
            assert_eq!(secret_marker(prose), None, "false positive: {prose:?}");
        }
    }

    #[test]
    fn body_excerpt_suppresses_apparent_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        fs::create_dir_all(layout.projects_dir()).unwrap();
        create(
            &layout,
            "sample",
            "leaky",
            CreateOpts {
                body: Some("token: api_key=whatever-it-was"),
                ..Default::default()
            },
        )
        .unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let text = body_excerpt(&layout, &doc.headings[0].id).unwrap();
        assert!(text.contains("excerpt suppressed"), "{text}");
        assert!(!text.contains("whatever-it-was"), "{text}");
    }
}
