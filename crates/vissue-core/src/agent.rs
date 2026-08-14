//! Verbs shaped for a program rather than a person: structured rows, claiming,
//! a body excerpt, and a hygiene checklist.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::fs;

use crate::config::Layout;
use crate::model::READY_STATES;
use crate::ops;
use crate::report;
use crate::store::{find_by_id, list_projects, load_all, project_selected};

const BODY_EXCERPT_MAX_LINES: usize = 40;
const BODY_EXCERPT_MAX_CHARS: usize = 4000;

/// Issue rows as JSON, filtered the same way [`report::list`] filters them.
pub fn issues_json(
    layout: &Layout,
    project_filter: Option<&str>,
    state_filter: Option<&str>,
    ready_only: bool,
) -> Result<Value> {
    let all = load_all(layout)?;
    let active_blockers: std::collections::HashSet<String> = all
        .iter()
        .filter(|(_, h)| h.state != "DONE" && h.state != "CANCELLED")
        .map(|(_, h)| h.id.clone())
        .collect();

    let mut rows: Vec<(char, String, String, Value)> = Vec::new();
    for (project, h) in &all {
        if !project_selected(project, project_filter) {
            continue;
        }
        if let Some(s) = state_filter {
            if h.state != s {
                continue;
            }
        }
        if ready_only {
            if !READY_STATES.contains(&h.state.as_str()) {
                continue;
            }
            if h.blocked_by().iter().any(|b| active_blockers.contains(b)) {
                continue;
            }
        }
        rows.push((
            h.priority,
            h.state.clone(),
            h.id.clone(),
            json!({
                "id": h.id,
                "state": h.state,
                "priority": h.priority.to_string(),
                "title": h.title,
                "project": project,
                "blocked_by": h.blocked_by(),
                "claimed_by": h.claimed_by(),
                "claimed_at": h.claimed_at(),
            }),
        ));
    }
    rows.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    Ok(Value::Array(rows.into_iter().map(|r| r.3).collect()))
}

/// One issue as JSON, including its file and line range.
pub fn show_json(layout: &Layout, id: &str) -> Result<Value> {
    let (h, path, project) =
        find_by_id(layout, id)?.ok_or_else(|| anyhow!("issue {id} not found"))?;
    Ok(json!({
        "id": h.id,
        "project": project,
        "title": h.title,
        "state": h.state,
        "priority": h.priority.to_string(),
        "properties": h.properties,
        "org_tags": h.org_tags,
        "tags": h.tags(),
        "blocked_by": h.blocked_by(),
        "parent": h.parent(),
        "claimed_by": h.claimed_by(),
        "claimed_at": h.claimed_at(),
        "file": format!("{}:{}-{}", path.display(), h.line_start, h.line_end),
        "line_start": h.line_start,
        "line_end": h.line_end,
    }))
}

/// Take an issue: move it to STARTED and stamp the claim.
pub fn claim(layout: &Layout, id: &str, force: bool) -> Result<String> {
    let report = ops::claim(layout, id, force)?;
    let detail = report::show(layout, id)?;
    Ok(format!("{report}{detail}"))
}

/// The first lines of an issue's file range, capped and screened for secrets.
pub fn body_excerpt(layout: &Layout, id: &str) -> Result<String> {
    let (h, path, _project) =
        find_by_id(layout, id)?.ok_or_else(|| anyhow!("issue {id} not found"))?;
    let content = fs::read_to_string(&path)?;
    let lines: Vec<&str> = content.lines().collect();
    let from = h.line_start.saturating_sub(1).min(lines.len());
    let to = h
        .line_end
        .min(lines.len())
        .min(from + BODY_EXCERPT_MAX_LINES);
    let mut excerpt = lines[from..to].join("\n");
    if excerpt.len() > BODY_EXCERPT_MAX_CHARS {
        excerpt.truncate(BODY_EXCERPT_MAX_CHARS);
        excerpt.push_str("\n...");
    }
    if let Some(marker) = secret_marker(&excerpt) {
        return Ok(format!(
            "(excerpt suppressed: {marker} looks like secret material; open {} directly)\n",
            path.display()
        ));
    }
    Ok(format!(
        "id: {id}\nfile: {}:{}-{}\n--- excerpt (lines {}-{}) ---\n{excerpt}\n",
        path.display(),
        h.line_start,
        h.line_end,
        from + 1,
        to
    ))
}

/// The marker that makes an excerpt look like it carries a credential.
///
/// A guard against handing an agent a secret by accident, not a redaction
/// guarantee: it screens the shapes credentials are usually written in, and
/// SECURITY.md says plainly that the answer is to keep them out of issue
/// bodies. Widening it is cheap; relying on it is not.
fn secret_marker(excerpt: &str) -> Option<&'static str> {
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
    use crate::config::DEFAULT_PREFIX;
    use crate::ops::{create, update, CreateOpts};
    use crate::store::IssueDoc;

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
            "-----BEGIN OPENSSH PRIVATE KEY-----",
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
