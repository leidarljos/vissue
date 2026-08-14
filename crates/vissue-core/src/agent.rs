//! Verbs shaped for a program rather than a person: structured rows, claiming,
//! a body excerpt, and a hygiene checklist.

use anyhow::Result;
use serde_json::Value;
use std::fmt::Write as _;

use crate::catalog::{excerpt_from, format_body_excerpt, load_recs, CatalogService};
use crate::config::Layout;
use crate::error::Error;
use crate::ops;
use crate::report;
use crate::store::{list_projects, load_all};
use crate::views::ListQuery;

/// Issue rows as JSON, filtered the same way [`report::list`] filters them.
pub fn issues_json(
    layout: &Layout,
    project_filter: Option<&str>,
    state_filter: Option<&str>,
    ready_only: bool,
) -> Result<Value> {
    let recs = load_recs(layout)?;
    let rows = CatalogService::from_recs(&recs).issues_rows(ListQuery {
        project: project_filter.map(str::to_string),
        state: state_filter.map(str::to_string),
        ready: ready_only,
        ..ListQuery::default()
    })?;
    Ok(serde_json::to_value(rows)?)
}

/// One issue as JSON, including its file and line range.
pub fn show_json(layout: &Layout, id: &str) -> Result<Value> {
    let recs = load_recs(layout)?;
    let detail = CatalogService::from_recs(&recs).detail(id)?;
    Ok(serde_json::to_value(detail)?)
}

/// Take an issue: move it to STARTED and stamp the claim.
pub fn claim(layout: &Layout, id: &str, force: bool) -> Result<String> {
    let report = ops::claim(layout, id, force)?;
    let detail = report::show(layout, id)?;
    Ok(format!("{report}{detail}"))
}

/// The first lines of an issue's file range, capped and screened for secrets.
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
pub fn waiting_on(layout: &Layout, id: &str) -> Result<String> {
    report::backlinks(layout, id)
}

/// The agent and CI checklist: issues claimed but not actionable, claims that
/// have gone stale, plus the corpus validation summary.
///
/// `stale_days` overrides the configured threshold when given.
pub fn hygiene(layout: &Layout, stale_days: Option<i64>) -> Result<String> {
    let mut out = String::new();
    writeln!(out, "=== vissue hygiene ===")?;

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
    for (project, h) in load_all(layout)? {
        if h.state != "STARTED" {
            continue;
        }
        match h.claimed_by() {
            None => {
                unclaimed_started += 1;
                writeln!(out, "[warn] STARTED with no claimant: {} ({project})", h.id)?;
            }
            Some(who) => {
                if let Some(days) = h.claim_age_days(today) {
                    if days > threshold {
                        stale_claims += 1;
                        writeln!(
                            out,
                            "[warn] claim held {days}d (over {threshold}d): {} by {who} ({project})",
                            h.id
                        )?;
                    }
                }
            }
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
        "summary: started_not_ready={started_not_ready} stale_claims={stale_claims} unclaimed_started={unclaimed_started} projects={} errors={} warnings={}",
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
    use crate::ops::{create, update, CreateOpts};
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
        assert!(text.contains("STARTED but not ready"), "{text}");
        assert!(text.contains("started_not_ready=1"), "{text}");
        assert!(text.contains("[ok] check passed"), "{text}");
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
