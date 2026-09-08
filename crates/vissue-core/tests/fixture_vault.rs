//! End-to-end checks against the synthetic fixture tracker in `tests/fixture_vault`.

#![allow(deprecated_safe_2024, missing_docs)]

use std::fs;
use std::path::{Path, PathBuf};

use vissue_core::catalog::{CatalogService, excerpt_from, load_recs};
use vissue_core::config::{DEFAULT_PREFIX, Layout};
use vissue_core::error::Error;
use vissue_core::graph::DependencyGraph;
use vissue_core::mirror::{self, Format};
use vissue_core::report;
use vissue_core::store::{self, IssueDoc};
use vissue_core::views::ListQuery;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixture_vault")
}

fn fixture_layout() -> Layout {
    Layout::new(fixture_root(), DEFAULT_PREFIX)
}

fn copy_tree(src: &Path, dest: &Path) {
    fs::create_dir_all(dest).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let target = dest.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// Copy the fixture into a temporary directory so a test may write to it.
fn writable_copy() -> (tempfile::TempDir, Layout) {
    let dir = tempfile::tempdir().unwrap();
    copy_tree(
        &fixture_root().join(DEFAULT_PREFIX),
        &dir.path().join(DEFAULT_PREFIX),
    );
    let layout = Layout::new(dir.path().to_path_buf(), DEFAULT_PREFIX);
    (dir, layout)
}

#[test]
fn the_fixture_holds_two_projects_and_six_issues() {
    let layout = fixture_layout();
    assert_eq!(
        store::list_projects(&layout).unwrap(),
        vec!["atlas", "beacon"]
    );
    assert_eq!(store::load_all(&layout).unwrap().len(), 6);
}

#[test]
fn parsing_and_rewriting_reproduces_the_file_byte_for_byte() {
    let (_dir, layout) = writable_copy();
    for project in ["atlas", "beacon"] {
        let path = layout.project_issues_path(project);
        let before = fs::read_to_string(&path).unwrap();
        IssueDoc::parse_file(project, &path)
            .unwrap()
            .write()
            .unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert_eq!(before, after, "{project} changed on a parse and rewrite");
    }
}

#[test]
fn a_clock_line_survives_the_rewrite() {
    let (_dir, layout) = writable_copy();
    let path = layout.project_issues_path("atlas");
    IssueDoc::parse_file("atlas", &path)
        .unwrap()
        .write()
        .unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("CLOCK: [2026-01-14 Wed 09:12]--[2026-01-14 Wed 10:42] =>  1:30"),
        "{text}"
    );
}

#[test]
fn export_rows_carry_the_documented_schema() {
    let layout = fixture_layout();
    let jsonl = report::export(&layout, Some("atlas")).unwrap();
    let rows: Vec<serde_json::Value> = jsonl
        .lines()
        .map(|l| serde_json::from_str(l).expect("each line is one JSON object"))
        .collect();
    assert_eq!(rows.len(), 4);

    let parser = rows
        .iter()
        .find(|r| r["id"] == "atlas-1a2b")
        .expect("the parser issue is exported");
    for key in [
        "id",
        "project",
        "title",
        "state",
        "priority",
        "properties",
        "logbook",
        "body",
        "line_start",
        "line_end",
    ] {
        assert!(!parser[key].is_null(), "missing {key} in {parser}");
    }
    assert_eq!(parser["project"], "atlas");
    assert_eq!(parser["state"], "STARTED");
    assert_eq!(parser["priority"], "A");
    // Tags ride the heading, where Org reads them, and the export names both
    // the Org run and the union a caller filters on.
    assert_eq!(parser["org_tags"][0], "parser");
    assert_eq!(parser["org_tags"][1], "core");
    assert_eq!(parser["tags"][0], "parser");
    let all_tags: Vec<&str> = parser["all_tags"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(all_tags.contains(&"parser"), "{all_tags:?}");
    assert!(all_tags.contains(&"issues"), "{all_tags:?}");
    assert!(all_tags.contains(&"noexport"), "{all_tags:?}");
    assert_eq!(parser["logbook"][0]["to"], "STARTED");
    assert_eq!(parser["logbook"][0]["from"], "TODO");
    assert!(
        parser["body"]
            .as_str()
            .unwrap()
            .starts_with("Scope: read the header block")
    );
}

#[test]
fn export_covers_every_project_when_unfiltered() {
    let layout = fixture_layout();
    assert_eq!(report::export(&layout, None).unwrap().lines().count(), 6);
}

#[test]
fn ready_hides_blocked_and_closed_work() {
    let layout = fixture_layout();
    let ready = report::ready(&layout, None).unwrap();
    assert!(ready.contains("atlas-1a2b"), "{ready}");
    assert!(ready.contains("atlas-2c3d"), "{ready}");
    assert!(ready.contains("beacon-5j6k"), "{ready}");
    assert!(
        !ready.contains("atlas-3e4f"),
        "blocked issue listed: {ready}"
    );
    assert!(
        !ready.contains("atlas-4g5h"),
        "closed issue listed: {ready}"
    );
    assert_eq!(report::count(&layout, None, None, true).unwrap(), "3\n");
}

#[test]
fn ready_hides_an_issue_blocked_by_another_project() {
    let (_dir, layout) = writable_copy();
    let beacon = layout.project_issues_path("beacon");
    let text = fs::read_to_string(&beacon).unwrap();
    let text = text.replace(
        ":ID:         beacon-5j6k\n",
        ":ID:         beacon-5j6k\n:BLOCKED_BY: atlas-1a2b\n",
    );
    fs::write(&beacon, text).unwrap();

    let ready = report::ready(&layout, None).unwrap();
    assert!(
        !ready.contains("beacon-5j6k"),
        "cross-project blocker ignored: {ready}"
    );
    assert_eq!(report::count(&layout, None, None, true).unwrap(), "2\n");
}

#[test]
fn export_includes_clock_raw_logbook_lines() {
    let layout = fixture_layout();
    let jsonl = report::export(&layout, Some("atlas")).unwrap();
    let rows: Vec<serde_json::Value> = jsonl
        .lines()
        .map(|l| serde_json::from_str(l).expect("each line is one JSON object"))
        .collect();
    let parser = rows
        .iter()
        .find(|r| r["id"] == "atlas-1a2b")
        .expect("the parser issue is exported");
    let logbook = parser["logbook"].as_array().expect("logbook array");
    assert!(
        logbook.iter().any(|entry| entry["raw"]
            .as_str()
            .is_some_and(|raw| raw.contains("CLOCK:"))),
        "CLOCK raw missing from export: {parser}"
    );
}

#[test]
fn claims_lists_only_claimed_issues_and_honors_filters() {
    let layout = fixture_layout();
    let all = report::claims(&layout, None, None, false).unwrap();
    assert!(all.contains("atlas-1a2b"), "{all}");
    assert!(all.contains("fixture-agent"), "{all}");
    assert!(!all.contains("atlas-2c3d"), "unclaimed issue listed: {all}");

    let by_holder = report::claims(&layout, Some("fixture-agent"), None, false).unwrap();
    assert!(by_holder.contains("atlas-1a2b"), "{by_holder}");
    let by_other = report::claims(&layout, Some("nobody"), None, false).unwrap();
    assert_eq!(by_other, "no live claims\n");
    let by_project = report::claims(&layout, None, Some("beacon"), false).unwrap();
    assert_eq!(by_project, "no live claims\n");

    let json = report::claims(&layout, None, None, true).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let rows = parsed.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "atlas-1a2b");
    assert_eq!(rows[0]["holder"], "fixture-agent");
    assert_eq!(rows[0]["claimed_at"], "[2026-01-14 Wed 09:12]");
    assert!(rows[0]["age_days"].as_i64().unwrap() >= 0);
}

#[test]
fn counts_respect_project_and_state_filters() {
    let layout = fixture_layout();
    assert_eq!(report::count(&layout, None, None, false).unwrap(), "6\n");
    assert_eq!(
        report::count(&layout, Some("atlas"), None, false).unwrap(),
        "4\n"
    );
    assert_eq!(
        report::count(&layout, None, Some("TODO"), false).unwrap(),
        "2\n"
    );
}

#[test]
fn list_sorts_by_priority_then_state_then_id() {
    let layout = fixture_layout();
    let listed = report::list(&layout, None, None, false).unwrap();
    let ids: Vec<&str> = listed
        .lines()
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![
            "atlas-3e4f",  // [#A] BLOCKED
            "atlas-1a2b",  // [#A] STARTED
            "atlas-2c3d",  // [#B] TODO
            "beacon-5j6k", // [#B] TODO
            "beacon-6m7n", // [#C] CANCELLED
            "atlas-4g5h",  // [#C] DONE
        ]
    );
}

#[test]
fn the_tree_shows_children_and_blockers() {
    let layout = fixture_layout();
    let ascii = report::tree(&layout, "atlas-1a2b", "ascii").unwrap();
    assert!(ascii.starts_with("atlas-1a2b STARTED"), "{ascii}");
    assert!(ascii.contains("  atlas-2c3d TODO"), "{ascii}");
    assert!(
        !ascii.contains("atlas-4g5h"),
        "unrelated issue in tree: {ascii}"
    );

    let dot = report::tree(&layout, "atlas-1a2b", "dot").unwrap();
    assert!(dot.starts_with("digraph vissue_tree {"), "{dot}");
    assert!(
        dot.contains("\"atlas-1a2b\" -> \"atlas-2c3d\" [color=\"#00897B\"];"),
        "{dot}"
    );

    assert!(report::tree(&layout, "atlas-1a2b", "svg").is_err());
    assert!(report::tree(&layout, "atlas-nope", "ascii").is_err());
}

#[test]
fn backlinks_name_the_relation() {
    let layout = fixture_layout();
    let text = report::backlinks(&layout, "atlas-1a2b").unwrap();
    assert!(text.contains("atlas-3e4f"), "{text}");
    assert!(text.contains("(blocked-by)"), "{text}");
    assert!(text.contains("atlas-2c3d"), "{text}");
    assert!(text.contains("(parent)"), "{text}");
}

#[test]
fn check_passes_and_counts_the_corpus() {
    let layout = fixture_layout();
    let out = report::check(&layout).unwrap();
    assert_eq!(out.errors, 0, "{}", out.text);
    assert_eq!(out.warnings, 0, "{}", out.text);
    assert!(
        out.text
            .contains("checked 6 issue(s) across 2 project(s): 0 error(s), 0 warning(s)"),
        "{}",
        out.text
    );
}

#[test]
fn agenda_orders_overdue_then_soonest_and_respects_the_horizon() {
    let (_dir, layout) = writable_copy();
    let path = layout.project_issues_path("atlas");
    let mut doc = IssueDoc::parse_file("atlas", &path).unwrap();
    let today = chrono::Local::now().date_naive();
    let stamp = |d: chrono::NaiveDate| format!("<{}>", d.format("%Y-%m-%d %a"));
    // 1a2b: deadline three days ago (overdue). 2c3d: scheduled in two days.
    // 3e4f: deadline far past the horizon, must not appear.
    doc.headings
        .iter_mut()
        .find(|h| h.id == "atlas-1a2b")
        .unwrap()
        .properties
        .insert("DEADLINE".into(), stamp(today - chrono::Duration::days(3)));
    doc.headings
        .iter_mut()
        .find(|h| h.id == "atlas-2c3d")
        .unwrap()
        .properties
        .insert("SCHEDULED".into(), stamp(today + chrono::Duration::days(2)));
    doc.headings
        .iter_mut()
        .find(|h| h.id == "atlas-3e4f")
        .unwrap()
        .properties
        .insert("DEADLINE".into(), stamp(today + chrono::Duration::days(90)));
    doc.write().unwrap();

    let out = report::agenda(&layout, 14, Some("atlas")).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    assert!(
        lines[0].contains("atlas-1a2b") && lines[0].contains("3d overdue"),
        "{out}"
    );
    assert!(
        lines[1].contains("atlas-2c3d") && lines[1].contains("in 2d"),
        "{out}"
    );
    assert!(!out.contains("atlas-3e4f"), "{out}");

    // A 100-day horizon pulls the far deadline in.
    let wide = report::agenda(&layout, 100, Some("atlas")).unwrap();
    assert!(wide.contains("atlas-3e4f"), "{wide}");
}

#[test]
fn cycles_reports_a_real_loop_once_in_edge_order() {
    let (_dir, layout) = writable_copy();
    let path = layout.project_issues_path("atlas");
    let mut doc = IssueDoc::parse_file("atlas", &path).unwrap();
    // atlas-1a2b -> atlas-2c3d -> atlas-1a2b is a genuine blocker loop.
    doc.headings
        .iter_mut()
        .find(|h| h.id == "atlas-1a2b")
        .unwrap()
        .properties
        .insert("BLOCKED_BY".into(), "atlas-2c3d".into());
    doc.headings
        .iter_mut()
        .find(|h| h.id == "atlas-2c3d")
        .unwrap()
        .properties
        .insert("BLOCKED_BY".into(), "atlas-1a2b".into());
    doc.write().unwrap();

    let out = report::cycles(&layout).unwrap();
    assert_eq!(out, "atlas-1a2b -> atlas-2c3d -> atlas-1a2b\n");
}

#[test]
fn cycles_ignores_a_shared_blocker_diamond() {
    let (_dir, layout) = writable_copy();
    let path = layout.project_issues_path("atlas");
    let mut doc = IssueDoc::parse_file("atlas", &path).unwrap();
    // 1a2b and 2c3d both wait on 3e4f: a diamond, not a loop. The fixture's
    // 3e4f ships blocked by 1a2b, which would close a genuine loop here, so
    // that edge is cleared first.
    let heading = doc
        .headings
        .iter_mut()
        .find(|h| h.id == "atlas-3e4f")
        .unwrap();
    heading.properties.remove("BLOCKED_BY");
    heading.properties.remove("BLOCKER");
    doc.headings
        .iter_mut()
        .find(|h| h.id == "atlas-1a2b")
        .unwrap()
        .properties
        .insert("BLOCKED_BY".into(), "atlas-3e4f".into());
    doc.headings
        .iter_mut()
        .find(|h| h.id == "atlas-2c3d")
        .unwrap()
        .properties
        .insert("BLOCKED_BY".into(), "atlas-3e4f,atlas-1a2b".into());
    doc.write().unwrap();

    let out = report::cycles(&layout).unwrap();
    assert_eq!(out, "no cycles\n");
}

#[test]
fn check_reports_a_broken_blocker_edge() {
    let (_dir, layout) = writable_copy();
    let path = layout.project_issues_path("atlas");
    let mut doc = IssueDoc::parse_file("atlas", &path).unwrap();
    doc.headings
        .iter_mut()
        .find(|h| h.id == "atlas-3e4f")
        .unwrap()
        .properties
        .insert("BLOCKED_BY".into(), "atlas-gone".into());
    doc.write().unwrap();

    let out = report::check(&layout).unwrap();
    assert_eq!(out.errors, 1, "{}", out.text);
    assert!(
        out.text
            .contains("[err]  atlas-3e4f (in atlas) :BLOCKED_BY: atlas-gone -> not found"),
        "{}",
        out.text
    );
}

#[test]
fn cycles_are_reported_when_the_blocker_graph_closes() {
    let layout = fixture_layout();
    assert_eq!(report::cycles(&layout).unwrap(), "no cycles\n");

    let (_dir, layout) = writable_copy();
    let path = layout.project_issues_path("atlas");
    let mut doc = IssueDoc::parse_file("atlas", &path).unwrap();
    doc.headings
        .iter_mut()
        .find(|h| h.id == "atlas-1a2b")
        .unwrap()
        .properties
        .insert("BLOCKED_BY".into(), "atlas-3e4f".into());
    doc.write().unwrap();
    let text = report::cycles(&layout).unwrap();
    assert!(text.contains("atlas-1a2b -> atlas-3e4f"), "{text}");
}

#[test]
fn stale_reports_open_issues_with_old_creation_dates() {
    let layout = fixture_layout();
    let text = report::stale(&layout, 30, None).unwrap();
    // Every open fixture issue was created well over 30 days ago.
    assert!(text.contains("atlas-1a2b"), "{text}");
    assert!(text.contains("beacon-5j6k"), "{text}");
    assert!(
        !text.contains("atlas-4g5h"),
        "closed issue reported: {text}"
    );
    assert!(
        report::stale(&layout, 30, Some("beacon"))
            .unwrap()
            .lines()
            .all(|l| l.starts_with("beacon-"))
    );
}

#[test]
fn the_roadmap_groups_by_project_and_state() {
    let layout = fixture_layout();
    let md = report::roadmap(&layout, None).unwrap();
    assert!(md.starts_with("# Roadmap\n"), "{md}");
    assert!(md.contains("## atlas"), "{md}");
    assert!(md.contains("### STARTED"), "{md}");
    assert!(
        md.contains("- **atlas-3e4f** [#A] Publish the release notes :: deadline <2026-03-01 Sun> :: blocked by atlas-1a2b"),
        "{md}"
    );
    assert!(md.contains("### Closed (1 items)"), "{md}");
}

#[test]
fn the_mirror_projects_selected_projects_only() {
    let layout = fixture_layout();
    let org = mirror::render(&layout, &["atlas".to_string()], Format::Org, None).unwrap();
    assert!(
        org.contains("# MIRROR: generated by `vissue mirror`"),
        "{org}"
    );
    assert!(org.contains("# Projects: atlas"), "{org}");
    assert!(org.contains("* atlas"), "{org}");
    assert!(!org.contains("* beacon"), "{org}");
    assert!(
        org.contains("** BLOCKED [#A] Publish the release notes"),
        "{org}"
    );
    assert!(org.contains(":ID:         atlas-3e4f"), "{org}");
    assert!(org.contains(":BLOCKED_BY: atlas-1a2b"), "{org}");
    assert!(
        org.contains("Waits on the parser landing, because the notes quote its error text."),
        "{org}"
    );
}

#[test]
fn the_mirror_covers_both_projects_by_default() {
    let layout = fixture_layout();
    let org = mirror::render(&layout, &[], Format::Org, None).unwrap();
    assert!(org.contains("# Projects: atlas, beacon"), "{org}");
    assert!(org.contains("* atlas"), "{org}");
    assert!(org.contains("* beacon"), "{org}");
    for id in [
        "atlas-1a2b",
        "atlas-2c3d",
        "atlas-3e4f",
        "atlas-4g5h",
        "beacon-5j6k",
        "beacon-6m7n",
    ] {
        assert!(org.contains(id), "missing {id} in the mirror");
    }
}

/// The sync stamp carries a wall-clock field, which is the only part of a
/// mirror that may differ between two runs over an unchanged tracker.
fn without_stamp_time(text: &str) -> String {
    text.lines()
        .map(|l| match l.find(" at=") {
            Some(i) if l.contains("SYNC:") => {
                let tail = l[i + 4..].find(' ').map(|j| &l[i + 4 + j..]).unwrap_or("");
                format!("{} at=<time>{}", &l[..i], tail)
            }
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_mirror_is_deterministic_apart_from_the_stamp_time() {
    let layout = fixture_layout();
    let once = mirror::render(&layout, &[], Format::Org, None).unwrap();
    let twice = mirror::render(&layout, &[], Format::Org, None).unwrap();
    assert_eq!(without_stamp_time(&once), without_stamp_time(&twice));
}

#[test]
fn the_fixture_digest_is_stable_and_moves_only_with_the_corpus() {
    let layout = fixture_layout();
    let first = vissue_core::digest::corpus_digest(&layout, &[]).unwrap();
    let second = vissue_core::digest::corpus_digest(&layout, &[]).unwrap();
    assert_eq!(first.combined, second.combined, "digest is not stable");
    assert_eq!(first.issues, 6);
    assert_eq!(first.projects.len(), 2);

    let (_dir, writable) = writable_copy();
    let before = vissue_core::digest::corpus_digest(&writable, &[]).unwrap();
    vissue_core::ops::update(&writable, "atlas-2c3d", Some("DONE"), None, None, None).unwrap();
    let after = vissue_core::digest::corpus_digest(&writable, &[]).unwrap();

    assert_ne!(
        before.combined, after.combined,
        "an edit did not move the digest"
    );
    assert_ne!(
        before.digest_of("atlas"),
        after.digest_of("atlas"),
        "the edited project did not move"
    );
    assert_eq!(
        before.digest_of("beacon"),
        after.digest_of("beacon"),
        "an untouched project moved"
    );
}

#[test]
fn a_mirror_is_fresh_when_written_and_stale_once_the_corpus_moves() {
    let _guard = EVENTS_ENV.lock().unwrap_or_else(|p| p.into_inner());
    let (dir, layout) = writable_copy();
    let path = dir.path().join("issues-mirror.org");

    let text = mirror::render(&layout, &["atlas".to_string()], Format::Org, None).unwrap();
    fs::write(&path, &text).unwrap();

    let stamp = mirror::SyncStamp::find(&text).expect("no stamp written");
    assert_eq!(stamp.projects.len(), 1);
    assert_eq!(stamp.projects[0].0, "atlas");

    let fresh = mirror::check(&layout, &path, &[]).unwrap();
    assert!(fresh.fresh, "{}", fresh.report);
    assert!(
        fresh.report.starts_with("fresh: digest="),
        "{}",
        fresh.report
    );

    // Move the mirrored project; the file on disk is now behind.
    vissue_core::ops::update(&layout, "atlas-2c3d", Some("DONE"), None, None, None).unwrap();
    let stale = mirror::check(&layout, &path, &[]).unwrap();
    assert!(!stale.fresh, "{}", stale.report);
    assert!(stale.report.contains("stale:"), "{}", stale.report);
    assert!(stale.report.contains("moved: atlas"), "{}", stale.report);
    assert!(
        stale.report.contains(&stamp.digest),
        "the report does not quote the stamped digest: {}",
        stale.report
    );

    // Regenerating makes it fresh again.
    fs::write(
        &path,
        mirror::render(&layout, &["atlas".to_string()], Format::Org, None).unwrap(),
    )
    .unwrap();
    assert!(mirror::check(&layout, &path, &[]).unwrap().fresh);
}

#[test]
fn a_change_outside_the_mirrored_projects_leaves_it_fresh() {
    let _guard = EVENTS_ENV.lock().unwrap_or_else(|p| p.into_inner());
    let (dir, layout) = writable_copy();
    let path = dir.path().join("atlas-only.org");
    fs::write(
        &path,
        mirror::render(&layout, &["atlas".to_string()], Format::Org, None).unwrap(),
    )
    .unwrap();

    vissue_core::ops::update(&layout, "beacon-5j6k", Some("DONE"), None, None, None).unwrap();
    let verdict = mirror::check(&layout, &path, &[]).unwrap();
    assert!(
        verdict.fresh,
        "an unrelated project moved the mirror's verdict: {}",
        verdict.report
    );
}

#[test]
fn a_file_without_a_stamp_reads_as_stale() {
    let (dir, layout) = writable_copy();
    let path = dir.path().join("unstamped.org");
    fs::write(&path, "#+TITLE: hand written\n\n* alpha\n").unwrap();
    let verdict = mirror::check(&layout, &path, &["atlas".to_string()]).unwrap();
    assert!(!verdict.fresh);
    assert!(
        verdict.report.contains("no SYNC stamp"),
        "{}",
        verdict.report
    );
}

#[test]
fn check_names_a_parent_cycle() {
    // Every :PARENT: id resolves, so the edge checks pass; the loop still
    // makes the hierarchy unwalkable and has to be reported.
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
    fs::create_dir_all(layout.projects_dir().join("loop")).unwrap();
    fs::write(
        layout.project_issues_path("loop"),
        r#"#+TITLE: loop issues
#+TODO: TODO STARTED BLOCKED | DONE CANCELLED

* TODO [#A] First
:PROPERTIES:
:ID:         loop-aaaa
:PARENT:     loop-bbbb
:CREATED:    [2026-08-14 Fri]
:END:

* TODO [#A] Second
:PROPERTIES:
:ID:         loop-bbbb
:PARENT:     loop-aaaa
:CREATED:    [2026-08-14 Fri]
:END:
"#,
    )
    .unwrap();

    let report = report::check(&layout).unwrap();
    assert_eq!(report.errors, 1, "{}", report.text);
    assert!(report.text.contains("parent cycle"), "{}", report.text);
    // Reported once, not once per entry point into the loop.
    assert_eq!(
        report.text.matches("parent cycle").count(),
        1,
        "{}",
        report.text
    );
}

#[test]
fn a_search_finds_body_and_property_text() {
    let layout = fixture_layout();
    assert!(
        report::search(&layout, "backoff table", 10)
            .unwrap()
            .contains("beacon-5j6k")
    );
    // A tag is searchable whether it sits on the heading or in the drawer,
    // and the scan folds case either way.
    assert!(
        report::search(&layout, "PARSER", 10)
            .unwrap()
            .contains("atlas-1a2b")
    );
    assert!(
        report::search(&layout, "core", 10)
            .unwrap()
            .contains("atlas-1a2b")
    );
    // FILETAGS are inherited (manual 6.1): every heading matches `issues`.
    assert!(
        report::search(&layout, "issues", 10)
            .unwrap()
            .contains("atlas-1a2b")
    );
    // Still reaches drawer properties: TYPE on the same issue.
    assert!(
        report::search(&layout, "feature", 10)
            .unwrap()
            .contains("atlas-1a2b")
    );
    assert_eq!(
        report::search(&layout, "nothing matches this", 10).unwrap(),
        ""
    );
}

#[test]
fn children_lists_issues_under_a_parent() {
    let layout = fixture_layout();
    let text = report::children(&layout, "atlas-1a2b").unwrap();
    assert!(text.contains("atlas-2c3d"), "{text}");
    assert_eq!(text.lines().count(), 1, "{text}");
    // A design document may also be a parent.
    assert!(
        report::children(&layout, "beacon-design-0001")
            .unwrap()
            .contains("beacon-5j6k")
    );
}

#[test]
fn related_uses_org_relations_and_emits_org_links() {
    let layout = fixture_layout();
    let text = report::related(&layout, "atlas-1a2b", 2, 10, "text").unwrap();
    assert!(text.contains("atlas-3e4f"), "{text}");
    assert!(text.contains("blocks"), "{text}");
    assert!(text.contains("atlas-2c3d"), "{text}");
    assert!(text.contains("child"), "{text}");
    assert!(text.contains("term:parser"), "{text}");
    assert!(!text.contains("term:created"), "{text}");

    let org = report::related(&layout, "atlas-1a2b", 2, 10, "org").unwrap();
    assert!(org.contains("[[id:atlas-3e4f][atlas-3e4f]]"), "{org}");
}

#[test]
fn related_reads_org_body_links_and_discovered_from_properties() {
    let (_dir, layout) = writable_copy();
    let path = layout.project_issues_path("atlas");
    let mut text = fs::read_to_string(&path).unwrap();
    text = text.replace(
        "Done-when: a malformed header names the offending line number.",
        "Done-when: a malformed header names the offending line number.\nSee [[id:atlas-3e4f][the release notes]] for the downstream contract.",
    );
    // Anchored on the id rather than on whichever property happens to sit last,
    // so a new property on the fixture heading does not quietly stop this
    // setup from applying.
    text = text.replace(
        ":ID:         atlas-4g5h\n",
        ":ID:         atlas-4g5h\n:DISCOVERED_FROM: atlas-1a2b\n",
    );
    fs::write(path, text).unwrap();

    let related = report::related(&layout, "atlas-1a2b", 1, 10, "text").unwrap();
    assert!(related.contains("atlas-3e4f"), "{related}");
    assert!(related.contains("org_link"), "{related}");
    assert!(related.contains("atlas-4g5h"), "{related}");
    assert!(related.contains("discovered_from"), "{related}");
}

#[test]
fn show_reports_the_metadata_the_range_and_the_body() {
    let layout = fixture_layout();
    let text = report::show(&layout, "atlas-2c3d").unwrap();
    assert!(text.contains("ID:       atlas-2c3d"), "{text}");
    assert!(text.contains("Project:  atlas"), "{text}");
    assert!(text.contains("State:    TODO"), "{text}");
    assert!(text.contains("Priority: [#B]"), "{text}");
    assert!(text.contains("atlas/issues.org:"), "{text}");
    // The body says what to do, so printing the range and stopping leaves
    // every reader to go fetch it by hand.
    assert!(
        text.contains("Scope: one row per parsed record"),
        "show must print the body: {text}"
    );
}

#[test]
fn show_says_so_when_there_is_no_body() {
    let (_dir, layout) = writable_copy();
    let id = vissue_core::ops::create(
        &layout,
        "atlas",
        "Nothing written yet",
        vissue_core::ops::CreateOpts {
            quiet: true,
            ..Default::default()
        },
    )
    .unwrap()
    .trim()
    .to_string();
    let text = report::show(&layout, &id).unwrap();
    assert!(text.contains("(no body"), "{text}");
}

/// The org export is the whole heading, not a preview of it.
///
/// `body_excerpt` caps at 40 lines, which silently drops the tail of any
/// longer issue. That is fine for a glance and wrong when the issue is being
/// handed to someone as the thing to work from.
#[test]
fn the_org_export_does_not_truncate_a_long_issue() {
    let (_dir, layout) = writable_copy();
    let body: String = (1..=60)
        .map(|i| format!("Requirement {i}: this must survive the export."))
        .collect::<Vec<_>>()
        .join("\n");
    let id = vissue_core::ops::create(
        &layout,
        "atlas",
        "A long specification",
        vissue_core::ops::CreateOpts {
            quiet: true,
            body: Some(&body),
            ..Default::default()
        },
    )
    .unwrap()
    .trim()
    .to_string();

    let org = vissue_core::agent::org_text(&layout, &id).unwrap();
    assert!(org.starts_with("* TODO"), "{org}");
    assert!(org.contains(&format!(":ID:         {id}")), "{org}");
    assert!(
        org.contains("Requirement 60: this must survive the export."),
        "the tail was dropped"
    );
    assert!(org.ends_with('\n'), "org text is written to a file as-is");

    // The preview stops early, which is the difference being pinned.
    let excerpt = vissue_core::agent::body_excerpt(&layout, &id).unwrap();
    assert!(
        !excerpt.contains("Requirement 60:"),
        "the excerpt is supposed to be capped: {excerpt}"
    );
}

#[test]
fn the_org_export_refuses_an_id_it_cannot_find() {
    let layout = fixture_layout();
    assert!(vissue_core::agent::org_text(&layout, "atlas-zzzz").is_err());
}

/// Credential-shaped text is refused here exactly as it is in a preview.
#[test]
fn the_org_export_keeps_the_secret_screen() {
    let (_dir, layout) = writable_copy();
    let carrier = concat!("-----BEGIN OPENSSH ", "PRIVATE KEY-----");
    let id = vissue_core::ops::create(
        &layout,
        "atlas",
        "Carries a key by mistake",
        vissue_core::ops::CreateOpts {
            quiet: true,
            body: Some(carrier),
            ..Default::default()
        },
    )
    .unwrap()
    .trim()
    .to_string();

    let err = vissue_core::agent::org_text(&layout, &id).unwrap_err();
    let text = format!("{err:#}");
    assert!(text.contains("secret material"), "{text}");
    assert!(!text.contains(carrier), "the refusal repeated the material");
}

/// `VISSUE_EVENTS` is process-global, so the two tests that depend on its value
/// take turns rather than racing each other inside the shared test binary.
static EVENTS_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn a_create_and_an_update_announce_themselves_to_pollers() {
    let _guard = EVENTS_ENV.lock().unwrap_or_else(|p| p.into_inner());
    let (_dir, layout) = writable_copy();
    let events_dir = layout.projects_dir();
    assert!(
        !vissue_core::events::log_path(&events_dir).exists(),
        "the fixture copy starts with no event stream"
    );
    let before = vissue_core::events::generation(&layout);

    let created = vissue_core::ops::create(
        &layout,
        "atlas",
        "an issue that should wake a poller",
        vissue_core::ops::CreateOpts {
            quiet: true,
            ..Default::default()
        },
    )
    .unwrap();
    let id = created.trim().to_string();

    let log = vissue_core::events::log_path(&events_dir);
    assert!(
        log.is_file(),
        "create wrote no event log at {}",
        log.display()
    );
    assert!(
        vissue_core::events::gen_path(&events_dir).is_file(),
        "create wrote no generation file"
    );
    let after_create = vissue_core::events::generation(&layout);
    assert!(
        after_create > before,
        "generation did not advance: {before} -> {after_create}"
    );

    let events = vissue_core::events::since(&layout, before, 10).unwrap();
    assert!(!events.is_empty(), "no events recorded for the create");
    let write = events
        .iter()
        .find(|e| e.kind == "issues_write")
        .expect("a create records an issues_write event");
    assert_eq!(write.project.as_deref(), Some("atlas"));
    assert!(
        write.path.as_deref().unwrap().ends_with("atlas/issues.org"),
        "{write:?}"
    );

    // The debounce window suppresses a second log line but must still move the
    // generation, so a poller cannot sleep through the update.
    vissue_core::ops::update(&layout, &id, Some("STARTED"), None, None, None).unwrap();
    let after_update = vissue_core::events::generation(&layout);
    assert!(
        after_update > after_create,
        "update did not advance the generation: {after_create} -> {after_update}"
    );
}

#[test]
fn event_emission_can_be_switched_off() {
    let _guard = EVENTS_ENV.lock().unwrap_or_else(|p| p.into_inner());
    let (_dir, layout) = writable_copy();
    vissue_core::process_env::override_var("VISSUE_EVENTS", Some("0"));
    let result = vissue_core::ops::create(
        &layout,
        "atlas",
        "a quiet issue",
        vissue_core::ops::CreateOpts {
            quiet: true,
            ..Default::default()
        },
    );
    vissue_core::process_env::clear_override("VISSUE_EVENTS");
    result.unwrap();

    assert!(
        !vissue_core::events::log_path(&layout.projects_dir()).exists(),
        "an event log appeared despite VISSUE_EVENTS=0"
    );
}

#[test]
fn a_claimed_issue_shows_its_holder_and_age() {
    let layout = fixture_layout();
    let listed = report::list(&layout, Some("atlas"), Some("STARTED"), false).unwrap();
    assert!(
        listed.contains("(claimed") && listed.contains("by fixture-agent"),
        "{listed}"
    );

    let shown = report::show(&layout, "atlas-1a2b").unwrap();
    assert!(shown.contains("Claimed:  fixture-agent since"), "{shown}");

    // An unclaimed issue renders exactly as it did before claims existed.
    let unclaimed = report::list(&layout, Some("atlas"), Some("TODO"), false).unwrap();
    assert!(!unclaimed.contains("claimed"), "{unclaimed}");
    assert!(
        !report::show(&layout, "atlas-2c3d")
            .unwrap()
            .contains("Claimed:")
    );
}

#[test]
fn claiming_stamps_the_identity_and_releasing_keeps_the_history() {
    let _guard = EVENTS_ENV.lock().unwrap_or_else(|p| p.into_inner());
    let (_dir, layout) = writable_copy();
    vissue_core::process_env::override_var("VISSUE_AGENT", Some("test-runner-1"));
    let claimed = vissue_core::ops::claim(&layout, "atlas-2c3d", false);
    vissue_core::process_env::clear_override("VISSUE_AGENT");
    claimed.unwrap();

    let h = store::find_by_id(&layout, "atlas-2c3d").unwrap().unwrap().0;
    assert_eq!(h.state, "STARTED");
    assert_eq!(h.claimed_by(), Some("test-runner-1"));
    assert!(h.claimed_at().is_some(), "no claim timestamp written");
    assert_eq!(h.claim_age_days(chrono::Local::now().date_naive()), Some(0));

    // Closing gives the claim up, and the logbook keeps who held it.
    vissue_core::ops::update(&layout, "atlas-2c3d", Some("DONE"), None, None, None).unwrap();
    let h = store::find_by_id(&layout, "atlas-2c3d").unwrap().unwrap().0;
    assert_eq!(h.claimed_by(), None, "claim survived the close");
    assert_eq!(h.claimed_at(), None);
    assert!(
        h.logbook
            .iter()
            .any(|e| e.note.as_deref().unwrap_or("").contains("test-runner-1")),
        "no logbook record of who held it: {:?}",
        h.logbook
    );
}

#[test]
fn a_block_keeps_the_claim_but_a_reset_to_todo_gives_it_up() {
    let _guard = EVENTS_ENV.lock().unwrap_or_else(|p| p.into_inner());
    let (_dir, layout) = writable_copy();

    // atlas-1a2b arrives STARTED and claimed in the fixture.
    vissue_core::ops::update(&layout, "atlas-1a2b", Some("BLOCKED"), None, None, None).unwrap();
    let h = store::find_by_id(&layout, "atlas-1a2b").unwrap().unwrap().0;
    assert_eq!(
        h.claimed_by(),
        Some("fixture-agent"),
        "a blocked issue is still held"
    );

    vissue_core::ops::update(&layout, "atlas-1a2b", Some("TODO"), None, None, None).unwrap();
    let h = store::find_by_id(&layout, "atlas-1a2b").unwrap().unwrap().0;
    assert_eq!(h.claimed_by(), None, "returning to TODO gives the claim up");
}

#[test]
fn a_claim_held_by_another_identity_needs_force() {
    let _guard = EVENTS_ENV.lock().unwrap_or_else(|p| p.into_inner());
    let (_dir, layout) = writable_copy();
    vissue_core::process_env::override_var("VISSUE_AGENT", Some("someone-else"));
    let refused = vissue_core::ops::claim(&layout, "atlas-1a2b", false);
    let forced = vissue_core::ops::claim(&layout, "atlas-1a2b", true);
    vissue_core::process_env::clear_override("VISSUE_AGENT");

    let err = refused.unwrap_err().to_string();
    assert!(err.contains("claimed by fixture-agent"), "{err}");
    assert!(err.contains("--force"), "{err}");

    assert!(forced.unwrap().contains("taken over from fixture-agent"));
    let h = store::find_by_id(&layout, "atlas-1a2b").unwrap().unwrap().0;
    assert_eq!(h.claimed_by(), Some("someone-else"));
    assert!(
        h.logbook
            .iter()
            .any(|e| e.note.as_deref().unwrap_or("").contains("fixture-agent")),
        "the takeover lost the previous holder"
    );
}

#[test]
fn hygiene_reports_a_claim_that_has_gone_stale() {
    let layout = fixture_layout();
    // The fixture claim was taken in January, so any small threshold trips it.
    let text = vissue_core::agent::hygiene(&layout, Some(7)).unwrap();
    assert!(text.contains("claim held"), "{text}");
    assert!(text.contains("atlas-1a2b by fixture-agent"), "{text}");
    assert!(text.contains("stale_claims=1"), "{text}");

    // A threshold wider than the claim age reports nothing stale.
    let wide = vissue_core::agent::hygiene(&layout, Some(100_000)).unwrap();
    assert!(wide.contains("stale_claims=0"), "{wide}");
}

#[test]
fn the_mirror_carries_the_claimant() {
    let layout = fixture_layout();
    let org = mirror::render(&layout, &["atlas".to_string()], Format::Org, None).unwrap();
    assert!(org.contains(":CLAIMED_BY: fixture-agent"), "{org}");
    assert!(org.contains(":CLAIMED_AT: [2026-01-14 Wed 09:12]"), "{org}");
}

#[test]
fn a_configured_prefix_finds_no_projects_where_there_are_none() {
    let layout = Layout::new(fixture_root(), "Elsewhere");
    assert!(store::list_projects(&layout).unwrap().is_empty());
    assert_eq!(report::count(&layout, None, None, false).unwrap(), "0\n");
}

#[test]
fn issues_rows_cover_the_fixture_and_use_claimed_by() {
    let layout = fixture_layout();
    let recs = load_recs(&layout).unwrap();
    let rows = CatalogService::from_recs(&recs)
        .issues_rows(ListQuery::default())
        .unwrap();
    assert_eq!(rows.len(), 6);

    let listed = serde_json::to_value(&rows).unwrap();
    for row in listed.as_array().unwrap() {
        assert!(
            row.get("claimed_by").is_some(),
            "list row missing claimed_by: {row}"
        );
        assert!(row.get("holder").is_none(), "list row leaked holder: {row}");
    }

    let via_json = vissue_core::agent::issues_json(&layout, None, None, false).unwrap();
    assert_eq!(via_json.as_array().unwrap().len(), 6);
    assert!(via_json[0].get("claimed_by").is_some(), "{via_json}");
    assert!(via_json[0].get("holder").is_none(), "{via_json}");
    assert_eq!(via_json, listed);

    let claimed = rows.iter().find(|r| r.id == "atlas-1a2b").unwrap();
    assert_eq!(claimed.claimed_by.as_deref(), Some("fixture-agent"));
    assert_eq!(
        claimed.claimed_at.as_deref(),
        Some("[2026-01-14 Wed 09:12]")
    );
}

#[test]
fn claims_rows_use_holder_not_claimed_by() {
    let layout = fixture_layout();
    let recs = load_recs(&layout).unwrap();
    let rows = CatalogService::from_recs(&recs).claims(None, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "atlas-1a2b");
    assert_eq!(rows[0].holder.as_deref(), Some("fixture-agent"));
    assert_eq!(
        rows[0].claimed_at.as_deref(),
        Some("[2026-01-14 Wed 09:12]")
    );
    assert!(rows[0].age_days >= 0);

    let structured = serde_json::to_value(&rows).unwrap();
    assert!(structured[0].get("holder").is_some(), "{structured}");
    assert!(
        structured[0].get("claimed_by").is_none(),
        "claims row leaked claimed_by: {structured}"
    );

    let via_report: serde_json::Value =
        serde_json::from_str(&report::claims(&layout, None, None, true).unwrap()).unwrap();
    assert_eq!(via_report, structured);
}

#[test]
fn excerpt_from_reads_the_on_disk_heading_range() {
    let layout = fixture_layout();
    let recs = load_recs(&layout).unwrap();
    let rec = recs
        .iter()
        .find(|r| r.heading.id == "atlas-1a2b")
        .expect("parser issue");
    let excerpt = excerpt_from(rec).unwrap();
    assert!(!excerpt.suppressed, "{excerpt:?}");
    assert!(
        excerpt.text.contains(":CLAIMED_BY: fixture-agent"),
        "PROPERTIES drawer missing from excerpt: {}",
        excerpt.text
    );
    assert!(
        excerpt.text.contains("CLOCK:"),
        "LOGBOOK missing from excerpt: {}",
        excerpt.text
    );
    assert!(
        excerpt.text.contains("Scope: read the header block"),
        "{}",
        excerpt.text
    );
    assert_eq!(excerpt.id, "atlas-1a2b");
    assert!(
        excerpt.file.ends_with("atlas/issues.org"),
        "{}",
        excerpt.file
    );

    let wrapped = vissue_core::agent::body_excerpt(&layout, "atlas-1a2b").unwrap();
    assert!(wrapped.contains(":CLAIMED_BY: fixture-agent"), "{wrapped}");
    assert!(
        wrapped.contains("* STARTED [#A] Parse the manifest header"),
        "{wrapped}"
    );
}

#[test]
fn excerpt_from_suppresses_a_credential_shaped_line() {
    let (_dir, layout) = writable_copy();
    let path = layout.project_issues_path("atlas");
    let text = fs::read_to_string(&path).unwrap().replace(
        "Scope: read the header block before the first record.",
        "Scope: read the header block before the first record.\napi_key = 9f8e7d6c5b4a3210ff",
    );
    fs::write(&path, text).unwrap();

    let recs = load_recs(&layout).unwrap();
    let rec = recs
        .iter()
        .find(|r| r.heading.id == "atlas-1a2b")
        .expect("parser issue");
    let excerpt = excerpt_from(rec).unwrap();
    assert!(excerpt.suppressed, "{excerpt:?}");
    assert!(
        excerpt.text.contains("excerpt suppressed"),
        "{}",
        excerpt.text
    );
    assert!(
        !excerpt.text.contains("9f8e7d6c5b4a3210ff"),
        "secret leaked: {}",
        excerpt.text
    );
}

#[test]
fn claim_as_stamps_the_passed_identity() {
    let _guard = EVENTS_ENV.lock().unwrap_or_else(|p| p.into_inner());
    let (_dir, layout) = writable_copy();
    vissue_core::process_env::override_var("VISSUE_AGENT", Some("env-agent"));
    let claimed = vissue_core::ops::claim_as(&layout, "atlas-2c3d", false, "passed-identity");
    vissue_core::process_env::clear_override("VISSUE_AGENT");
    claimed.unwrap();

    let h = store::find_by_id(&layout, "atlas-2c3d").unwrap().unwrap().0;
    assert_eq!(h.claimed_by(), Some("passed-identity"));
    assert_ne!(h.claimed_by(), Some("env-agent"));
}

#[test]
fn claim_conflict_is_matchable_without_parsing_display() {
    let _guard = EVENTS_ENV.lock().unwrap_or_else(|p| p.into_inner());
    let (_dir, layout) = writable_copy();
    vissue_core::process_env::override_var("VISSUE_AGENT", Some("someone-else"));
    let refused = vissue_core::ops::claim(&layout, "atlas-1a2b", false);
    vissue_core::process_env::clear_override("VISSUE_AGENT");

    let err = refused.unwrap_err();
    match &err {
        Error::ClaimConflict {
            id,
            holder,
            claimed_at,
        } => {
            assert_eq!(id, "atlas-1a2b");
            assert_eq!(holder, "fixture-agent");
            assert_eq!(claimed_at.as_deref(), Some("[2026-01-14 Wed 09:12]"));
        }
        other => panic!("expected ClaimConflict, got {other:?}"),
    }
    assert!(err.to_string().contains("claimed by fixture-agent"));
    assert!(err.to_string().contains("since [2026-01-14 Wed 09:12]"));
    assert!(err.to_string().contains("--force"));
}

#[test]
fn accepts_edge_cycle_is_a_typed_blocker_cycle() {
    let layout = fixture_layout();
    let all = store::load_all(&layout).unwrap();
    let graph = DependencyGraph::from_issues(&all).unwrap();
    // atlas-3e4f already waits on atlas-1a2b; the reverse edge closes a loop.
    let err = graph.accepts_edge("atlas-3e4f", "atlas-1a2b").unwrap_err();
    match &err {
        Error::BlockerCycle { blocker, issue } => {
            assert_eq!(blocker, "atlas-3e4f");
            assert_eq!(issue, "atlas-1a2b");
        }
        other => panic!("expected BlockerCycle, got {other:?}"),
    }
    assert!(err.to_string().contains("blocker cycle"));
}

#[test]
fn ready_from_is_corpus_wide() {
    let (_dir, layout) = writable_copy();
    let beacon = layout.project_issues_path("beacon");
    let text = fs::read_to_string(&beacon).unwrap().replace(
        ":ID:         beacon-5j6k\n",
        ":ID:         beacon-5j6k\n:BLOCKED_BY: atlas-1a2b\n",
    );
    fs::write(&beacon, text).unwrap();

    let recs = load_recs(&layout).unwrap();
    let ready = CatalogService::from_recs(&recs)
        .ready(Some("beacon"))
        .unwrap();
    assert!(
        ready.iter().all(|r| r.id != "beacon-5j6k"),
        "cross-project blocker ignored: {ready:?}"
    );
    assert_eq!(
        CatalogService::from_recs(&recs).ready(None).unwrap().len(),
        2
    );
}

#[test]
fn missing_ids_are_typed_issue_not_found() {
    let layout = fixture_layout();
    let recs = load_recs(&layout).unwrap();
    let catalog = CatalogService::from_recs(&recs);

    match catalog.detail("no-such") {
        Err(Error::IssueNotFound { id }) => assert_eq!(id, "no-such"),
        other => panic!("expected IssueNotFound from detail, got {other:?}"),
    }
    match catalog.children("no-such") {
        Err(Error::IssueNotFound { id }) => assert_eq!(id, "no-such"),
        other => panic!("expected IssueNotFound from children, got {other:?}"),
    }
    match catalog.backlinks("no-such") {
        Err(Error::IssueNotFound { id }) => assert_eq!(id, "no-such"),
        other => panic!("expected IssueNotFound from backlinks, got {other:?}"),
    }

    let err = vissue_core::ops::claim(&layout, "no-such", false).unwrap_err();
    match &err {
        Error::IssueNotFound { id } => assert_eq!(id, "no-such"),
        other => panic!("expected IssueNotFound from claim, got {other:?}"),
    }

    // A known issue with no children is empty, not not-found. A design-document
    // parent is not an IssueRec but still has children.
    assert!(catalog.children("atlas-4g5h").unwrap().is_empty());
    assert!(
        catalog
            .children("beacon-design-0001")
            .unwrap()
            .iter()
            .any(|hit| hit.id == "beacon-5j6k")
    );
}

#[test]
fn claiming_a_closed_issue_is_typed_invalid_state() {
    let _guard = EVENTS_ENV.lock().unwrap_or_else(|p| p.into_inner());
    let (_dir, layout) = writable_copy();
    let err = vissue_core::ops::claim(&layout, "atlas-4g5h", false).unwrap_err();
    match &err {
        Error::InvalidState { id, state } => {
            assert_eq!(id, "atlas-4g5h");
            assert_eq!(state, "DONE");
        }
        other => panic!("expected InvalidState, got {other:?}"),
    }
    assert!(err.to_string().contains("cannot claim"));
}

/// Set one property on one issue in the writable copy, then re-check.
fn check_after<F>(edit: F) -> report::CheckReport
where
    F: FnOnce(&Layout),
{
    let (_dir, layout) = writable_copy();
    edit(&layout);
    report::check(&layout).unwrap()
}

fn set_property(layout: &Layout, project: &str, id: &str, key: &str, value: &str) {
    let path = layout.project_issues_path(project);
    let mut doc = IssueDoc::parse_file(project, &path).unwrap();
    doc.headings
        .iter_mut()
        .find(|h| h.id == id)
        .unwrap()
        .properties
        .insert(key.into(), value.into());
    doc.write().unwrap();
}

#[test]
fn check_reports_a_parent_that_names_nothing() {
    let out = check_after(|layout| {
        set_property(layout, "atlas", "atlas-2c3d", "PARENT", "atlas-gone");
    });
    assert_eq!(out.errors, 1, "{}", out.text);
    assert!(
        out.text
            .contains("[err]  atlas-2c3d (in atlas) :PARENT: atlas-gone -> not found"),
        "{}",
        out.text
    );
}

/// Insert a property line into the raw org text of one issue.
///
/// `check` guards against what a person types into the file by hand, and
/// some of that cannot be produced through `IssueDoc`: the writer renders
/// DEADLINE as a planning line, and a value it cannot parse does not
/// survive the round trip.
fn insert_property_line(layout: &Layout, project: &str, id: &str, line: &str) {
    let path = layout.project_issues_path(project);
    let text = fs::read_to_string(&path).unwrap();
    let anchor = format!(":ID:         {id}");
    let at = text
        .find(&anchor)
        .unwrap_or_else(|| panic!("no {id} in {path:?}"));
    let eol = at + text[at..].find('\n').unwrap() + 1;
    let mut out = String::with_capacity(text.len() + line.len() + 1);
    out.push_str(&text[..eol]);
    out.push_str(line);
    out.push('\n');
    out.push_str(&text[eol..]);
    fs::write(&path, out).unwrap();
}

#[test]
fn check_reports_dates_it_cannot_parse() {
    let deadline = check_after(|layout| {
        insert_property_line(layout, "atlas", "atlas-2c3d", ":DEADLINE:   next tuesday");
    });
    assert_eq!(deadline.errors, 1, "{}", deadline.text);
    assert!(
        deadline
            .text
            .contains(":DEADLINE: next tuesday -> unparseable"),
        "{}",
        deadline.text
    );

    let scheduled = check_after(|layout| {
        insert_property_line(layout, "atlas", "atlas-2c3d", ":SCHEDULED:  soon");
    });
    assert_eq!(scheduled.errors, 1, "{}", scheduled.text);
    assert!(
        scheduled.text.contains(":SCHEDULED: soon -> unparseable"),
        "{}",
        scheduled.text
    );
}

#[test]
fn open_work_without_a_creation_date_is_a_warning_not_an_error() {
    let out = check_after(|layout| {
        let path = layout.project_issues_path("atlas");
        let mut doc = IssueDoc::parse_file("atlas", &path).unwrap();
        let h = doc
            .headings
            .iter_mut()
            .find(|h| h.id == "atlas-2c3d")
            .unwrap();
        h.properties.remove("CREATED");
        doc.write().unwrap();
    });
    assert_eq!(out.errors, 0, "{}", out.text);
    assert_eq!(out.warnings, 1, "{}", out.text);
    assert!(
        out.text
            .contains("[warn] atlas-2c3d (in atlas) state=TODO but :CREATED: is missing"),
        "{}",
        out.text
    );
}

#[test]
fn check_reports_an_id_that_names_two_issues() {
    let out = check_after(|layout| {
        // Give a beacon issue an id atlas already uses: every blocker and
        // parent edge pointing at it becomes ambiguous.
        let path = layout.project_issues_path("beacon");
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, text.replace("beacon-5j6k", "atlas-2c3d")).unwrap();
    });
    assert!(out.errors >= 1, "{}", out.text);
    assert!(
        out.text.contains("duplicate id: atlas-2c3d"),
        "{}",
        out.text
    );
}

#[test]
fn check_names_a_parent_loop_that_every_edge_check_would_pass() {
    let out = check_after(|layout| {
        // atlas-2c3d already names atlas-1a2b as its parent. Close the loop.
        set_property(layout, "atlas", "atlas-1a2b", "PARENT", "atlas-2c3d");
    });
    assert!(out.errors >= 1, "{}", out.text);
    assert!(out.text.contains("[err]  parent cycle:"), "{}", out.text);
    assert!(out.text.contains("atlas-1a2b"), "{}", out.text);
    assert!(out.text.contains("atlas-2c3d"), "{}", out.text);
}

#[test]
fn a_clean_corpus_still_counts_what_it_checked() {
    let out = check_after(|_| {});
    assert_eq!(out.errors, 0, "{}", out.text);
    assert!(out.text.contains("checked 6 issue(s)"), "{}", out.text);
}

#[test]
fn folding_an_inbox_stamps_each_heading_and_is_idempotent() {
    let (dir, layout) = writable_copy();
    let inbox = dir.path().join("inbox.org");
    fs::write(
        &inbox,
        "* TODO Rotate the signing key\nThe old one expires this quarter.\n\
         * TODO Write the migration note\n",
    )
    .unwrap();

    let first = vissue_core::ops::fold(&layout, &inbox, "atlas").unwrap();
    assert!(first.starts_with("folded 2:"), "{first}");

    let stamped = fs::read_to_string(&inbox).unwrap();
    assert_eq!(stamped.matches(":VISSUE_ID:").count(), 2, "{stamped}");
    assert_eq!(stamped.matches("* DONE").count(), 2, "{stamped}");
    assert!(!stamped.contains("* TODO"), "{stamped}");
    // The body under a heading travels with it.
    let body = vissue_core::report::search(&layout, "expires this quarter", 10).unwrap();
    assert!(body.contains("Rotate the signing key"), "{body}");

    // A stamped heading is skipped, so a second run creates nothing.
    let second = vissue_core::ops::fold(&layout, &inbox, "atlas").unwrap();
    assert_eq!(second, "folded 0 (nothing unstamped)\n");
    assert_eq!(fs::read_to_string(&inbox).unwrap(), stamped);
}

#[test]
fn folding_a_file_with_no_headings_leaves_it_alone() {
    let (dir, layout) = writable_copy();
    let inbox = dir.path().join("empty.org");
    fs::write(&inbox, "Just a note to myself.\n").unwrap();
    let out = vissue_core::ops::fold(&layout, &inbox, "atlas").unwrap();
    assert_eq!(out, "folded 0 (nothing unstamped)\n");
    assert_eq!(
        fs::read_to_string(&inbox).unwrap(),
        "Just a note to myself.\n"
    );
}

#[test]
fn refiling_into_the_project_an_issue_already_sits_in_does_nothing() {
    let (_dir, layout) = writable_copy();
    let before = fs::read_to_string(layout.project_issues_path("atlas")).unwrap();
    let out = vissue_core::ops::refile(&layout, "atlas-2c3d", "atlas").unwrap();
    assert!(out.contains("already in atlas"), "{out}");
    assert_eq!(
        fs::read_to_string(layout.project_issues_path("atlas")).unwrap(),
        before,
        "a no-op refile rewrote the file"
    );
}

#[test]
fn refiling_moves_the_heading_and_keeps_the_id() {
    let (_dir, layout) = writable_copy();
    let out = vissue_core::ops::refile(&layout, "atlas-2c3d", "beacon").unwrap();
    assert!(out.contains("atlas-2c3d"), "{out}");
    let atlas = fs::read_to_string(layout.project_issues_path("atlas")).unwrap();
    let beacon = fs::read_to_string(layout.project_issues_path("beacon")).unwrap();
    assert!(!atlas.contains("atlas-2c3d"), "{atlas}");
    // The id does not follow the project, so cross-project edges still resolve.
    assert!(beacon.contains("atlas-2c3d"), "{beacon}");
    let detail = CatalogService::from_recs(&load_recs(&layout).unwrap())
        .detail("atlas-2c3d")
        .unwrap();
    assert_eq!(detail.project, "beacon");
}

#[test]
fn an_update_that_asks_for_what_is_already_true_reports_no_change() {
    let (_dir, layout) = writable_copy();
    // atlas-2c3d is already TODO.
    let out =
        vissue_core::ops::update(&layout, "atlas-2c3d", Some("TODO"), None, None, None).unwrap();
    assert_eq!(out.report, "atlas-2c3d: no change\n");
    assert!(out.hints.is_empty(), "{:?}", out.hints);
}

#[test]
fn clearing_the_last_blocker_returns_a_blocked_issue_to_todo() {
    let (_dir, layout) = writable_copy();
    // atlas-3e4f is BLOCKED by atlas-1a2b in the fixture.
    let out = vissue_core::ops::update(&layout, "atlas-3e4f", None, None, None, Some("atlas-1a2b"))
        .unwrap();
    assert!(
        out.report.contains("blocked_by -= atlas-1a2b"),
        "{}",
        out.report
    );
    assert!(
        out.report.contains("BLOCKED -> TODO"),
        "nothing holds it now: {}",
        out.report
    );
    let detail = CatalogService::from_recs(&load_recs(&layout).unwrap())
        .detail("atlas-3e4f")
        .unwrap();
    assert_eq!(detail.state, "TODO");
}

/// A markdown body must not split the issue in two.
///
/// An asterisk in the first column opens an org headline. A body carrying a
/// bullet list used to be written straight through, so the next read saw a
/// heading with no `:ID:`, stopped parsing the file, and dropped every issue
/// in that project out of `list`.
#[test]
fn a_body_with_markdown_bullets_does_not_break_the_file() {
    let (_dir, layout) = writable_copy();
    let body = "Findings:\n\n* first bullet\n* second bullet\n\n** nested bullet";
    let id = vissue_core::ops::create(
        &layout,
        "atlas",
        "Carries a bullet list",
        vissue_core::ops::CreateOpts {
            quiet: true,
            body: Some(body),
            ..Default::default()
        },
    )
    .unwrap()
    .trim()
    .to_string();

    // Every issue that was there is still there, plus the new one.
    let recs = load_recs(&layout).unwrap();
    assert!(recs.iter().any(|r| r.heading.id == id), "the new issue");
    assert!(
        recs.iter().any(|r| r.heading.id == "atlas-1a2b"),
        "the file still parses, so its other issues survive"
    );
    assert_eq!(report::check(&layout).unwrap().errors, 0);

    // The text is kept, indented enough that org does not read a headline.
    let detail = CatalogService::from_recs(&recs).detail(&id).unwrap();
    assert!(detail.body.contains("first bullet"), "{}", detail.body);
    assert!(detail.body.contains("nested bullet"), "{}", detail.body);
    for line in detail.body.lines() {
        assert!(
            !line.starts_with("* "),
            "a body line still ends the issue: {line:?}"
        );
    }
    // A deeper heading is a child of the issue, not the end of it, so it is
    // left as written.
    assert!(
        detail.body.lines().any(|l| l.starts_with("** ")),
        "a nested heading was indented needlessly: {}",
        detail.body
    );
}

/// Writing what was read changes nothing.
///
/// The escape is applied on every write, so a file that has been through it
/// once must not drift further on the next pass.
#[test]
fn rewriting_an_escaped_body_is_stable() {
    let (_dir, layout) = writable_copy();
    vissue_core::ops::create(
        &layout,
        "atlas",
        "Carries a bullet list",
        vissue_core::ops::CreateOpts {
            quiet: true,
            body: Some("* bullet one\n* bullet two"),
            ..Default::default()
        },
    )
    .unwrap();

    let path = layout.project_issues_path("atlas");
    let once = fs::read_to_string(&path).unwrap();
    // A no-op parse and write cycle.
    IssueDoc::parse_file("atlas", &path)
        .unwrap()
        .write()
        .unwrap();
    let twice = fs::read_to_string(&path).unwrap();
    assert_eq!(once, twice, "the escape moved the text on a second write");
}

#[test]
fn appending_records_a_report_under_the_heading() {
    let (_dir, layout) = writable_copy();
    let before = CatalogService::from_recs(&load_recs(&layout).unwrap())
        .detail("atlas-2c3d")
        .unwrap()
        .body;

    let out = vissue_core::ops::append_body_as(
        &layout,
        "atlas-2c3d",
        "## What changed\n\n* took a Read instead of a String\n\nOpen doubt: back-pressure.",
        "worker-1",
    )
    .unwrap();
    assert!(out.contains("atlas-2c3d"), "{out}");

    let after = CatalogService::from_recs(&load_recs(&layout).unwrap())
        .detail("atlas-2c3d")
        .unwrap()
        .body;
    // What was already there is kept, and the report is added under it.
    assert!(after.starts_with(before.trim_end()), "{after}");
    assert!(after.contains("worker-1"), "the report names who wrote it");
    assert!(after.contains("## What changed"), "{after}");
    assert!(after.contains("Open doubt: back-pressure."), "{after}");
    // Markdown bullets do not end the issue.
    assert_eq!(load_recs(&layout).unwrap().len(), 6);
    assert_eq!(report::check(&layout).unwrap().errors, 0);
}

#[test]
fn two_reports_stack_rather_than_replace() {
    let (_dir, layout) = writable_copy();
    vissue_core::ops::append_body_as(&layout, "atlas-2c3d", "first pass", "worker-1").unwrap();
    vissue_core::ops::append_body_as(&layout, "atlas-2c3d", "second pass", "worker-2").unwrap();
    let body = CatalogService::from_recs(&load_recs(&layout).unwrap())
        .detail("atlas-2c3d")
        .unwrap()
        .body;
    let first = body.find("first pass").expect("first");
    let second = body.find("second pass").expect("second");
    assert!(first < second, "reports are in the order they were written");
    assert!(
        body.contains("worker-1") && body.contains("worker-2"),
        "{body}"
    );
}

#[test]
fn appending_to_an_issue_with_no_body_does_not_lead_with_blank_lines() {
    let (_dir, layout) = writable_copy();
    let id = vissue_core::ops::create(
        &layout,
        "atlas",
        "Nothing written yet",
        vissue_core::ops::CreateOpts {
            quiet: true,
            ..Default::default()
        },
    )
    .unwrap()
    .trim()
    .to_string();
    vissue_core::ops::append_body_as(&layout, &id, "the first word on it", "worker-1").unwrap();
    let body = CatalogService::from_recs(&load_recs(&layout).unwrap())
        .detail(&id)
        .unwrap()
        .body;
    assert!(!body.starts_with('\n'), "{body:?}");
    assert!(body.contains("the first word on it"), "{body}");
}

#[test]
fn appending_nothing_is_refused() {
    let (_dir, layout) = writable_copy();
    assert!(vissue_core::ops::append_body_as(&layout, "atlas-2c3d", "   \n\n", "w").is_err());
    assert!(vissue_core::ops::append_body_as(&layout, "atlas-zzzz", "text", "w").is_err());
}

/// Text that is not ASCII survives a write and a read.
///
/// The parser works in bytes in places, and slicing a multi-byte character
/// in half has already cost this file one panic. Accents, an em dash, a
/// non-Latin script and an emoji all go through a title, which is the field
/// most likely to be sliced.
#[test]
fn unicode_survives_the_round_trip() {
    let (_dir, layout) = writable_copy();
    let title = "Café não inicia — 日本語 🧪";
    let body = "Résumé du problème: le café ne démarre pas.\n日本語の本文もある。\nEmoji: 🧪🔬";
    let id = vissue_core::ops::create(
        &layout,
        "atlas",
        title,
        vissue_core::ops::CreateOpts {
            quiet: true,
            body: Some(body),
            ..Default::default()
        },
    )
    .unwrap()
    .trim()
    .to_string();

    let read = || {
        CatalogService::from_recs(&load_recs(&layout).unwrap())
            .detail(&id)
            .unwrap()
    };
    assert_eq!(read().title, title);
    assert!(
        read().body.contains("日本語の本文もある。"),
        "{}",
        read().body
    );
    assert!(read().body.contains("🧪🔬"), "{}", read().body);

    // The priority cookie sits immediately before the title, so setting one
    // moves the boundary the parser slices at.
    vissue_core::ops::update(&layout, &id, None, Some('A'), None, None).unwrap();
    let after = read();
    assert_eq!(after.title, title, "the title lost a byte to the cookie");
    assert_eq!(after.priority, "A");
    assert_eq!(report::check(&layout).unwrap().errors, 0);
}

/// A file written by another editor still parses.
///
/// Org files are edited by people and by other tools, so vissue reads what
/// it is given: a missing final newline, and CRLF endings. A write of its
/// own normalises the endings to LF, which is worth knowing because it
/// shows up as a whole-file diff the first time.
#[test]
fn a_file_from_another_editor_is_read_and_normalised() {
    let (_dir, layout) = writable_copy();
    let path = layout.project_issues_path("atlas");
    let before = load_recs(&layout).unwrap().len();

    // No final newline.
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.trim_end_matches('\n')).unwrap();
    assert_eq!(load_recs(&layout).unwrap().len(), before, "lost an issue");
    assert_eq!(report::check(&layout).unwrap().errors, 0);

    // CRLF throughout.
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace('\n', "\r\n")).unwrap();
    let recs = load_recs(&layout).unwrap();
    assert_eq!(recs.len(), before, "CRLF cost an issue");
    let first = recs
        .iter()
        .find(|r| r.heading.id == "atlas-1a2b")
        .expect("atlas-1a2b");
    assert!(
        !first.heading.title.contains('\r'),
        "a carriage return reached the title: {:?}",
        first.heading.title
    );

    // A vissue write settles the file on LF.
    vissue_core::ops::note(&layout, "atlas-1a2b", "after crlf").unwrap();
    let bytes = fs::read(&path).unwrap();
    assert!(
        !bytes.contains(&b'\r'),
        "a carriage return survived a vissue write"
    );
    assert_eq!(load_recs(&layout).unwrap().len(), before);
    assert_eq!(report::check(&layout).unwrap().errors, 0);
}

/// What another tool wrote is still there after vissue writes.
///
/// This is the promise the format rests on: an issues.org is shared with
/// Emacs and with whatever else a person points at it, so a rewrite that
/// dropped an unknown property or a preamble keyword would quietly destroy
/// someone else's data. Nothing tested it.
#[test]
fn a_rewrite_keeps_what_another_tool_put_there() {
    let (_dir, layout) = writable_copy();
    let path = layout.project_issues_path("atlas");

    let planted = fs::read_to_string(&path)
        .unwrap()
        // Preamble keywords vissue has no use for, and a plain comment.
        .replace(
            "#+TODO:",
            "#+STARTUP: overview\n# a human comment in the preamble\n\
             #+PROPERTY: Effort_ALL 1 2 3\n#+TODO:",
        )
        // Properties belonging to something else entirely.
        .replacen(
            ":CREATED:",
            ":EXTERNAL_REF: abc123\n:CUSTOM_ID:  my-anchor\n:NOTER_DOCUMENT:  ~/doc.pdf\n:ROAM_REFS:  cite:key\n:CREATED:",
            1,
        );
    fs::write(&path, &planted).unwrap();
    assert_eq!(report::check(&layout).unwrap().errors, 0, "planted file");

    // Any write rewrites the whole file, so one note is the whole test.
    vissue_core::ops::note(&layout, "atlas-1a2b", "touched").unwrap();

    let after = fs::read_to_string(&path).unwrap();
    for kept in [
        "#+STARTUP: overview",
        "# a human comment in the preamble",
        "#+PROPERTY: Effort_ALL 1 2 3",
        ":EXTERNAL_REF:",
        ":CUSTOM_ID:",
        ":NOTER_DOCUMENT:",
        ":ROAM_REFS:",
        "abc123",
        "my-anchor",
        "~/doc.pdf",
        "cite:key",
    ] {
        assert!(after.contains(kept), "a rewrite dropped {kept:?}");
    }

    // They reach a consumer as well as the file.
    let detail = CatalogService::from_recs(&load_recs(&layout).unwrap())
        .detail("atlas-1a2b")
        .unwrap();
    assert_eq!(
        detail.properties.get("EXTERNAL_REF").map(String::as_str),
        Some("abc123")
    );
    assert_eq!(
        detail.properties.get("CUSTOM_ID").map(String::as_str),
        Some("my-anchor")
    );

    // And they keep the order they were written in, after :ID:, which vissue
    // hoists. A drawer reshuffled on every write is a diff on every write.
    let drawer: Vec<&str> = after
        .lines()
        .skip_while(|l| !l.contains(":PROPERTIES:"))
        .skip(1)
        .take_while(|l| !l.contains(":END:"))
        .filter_map(|l| l.trim().split(':').nth(1))
        .collect();
    assert_eq!(drawer.first(), Some(&"ID"), "{drawer:?}");
    let trace = drawer.iter().position(|k| *k == "EXTERNAL_REF");
    let custom = drawer.iter().position(|k| *k == "CUSTOM_ID");
    let created = drawer.iter().position(|k| *k == "CREATED");
    assert!(
        trace < custom && custom < created,
        "the drawer was reshuffled: {drawer:?}"
    );
}

/// Replace text in a project's raw file, once.
///
/// `check` guards against what a person types by hand, and much of that cannot be
/// produced through `IssueDoc`: a preamble keyword the writer always emits, a
/// priority cookie outside the declared range, a second tag from an exclusive group.
fn edit_raw(layout: &Layout, project: &str, from: &str, to: &str) {
    let path = layout.project_issues_path(project);
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains(from), "no {from:?} in {path:?}");
    fs::write(&path, text.replacen(from, to, 1)).unwrap();
}

/// Drop every preamble line starting with `prefix`.
///
/// Every, not the first: `#+TAGS:` is on two lines in the fixture, and a check for
/// whether the keyword is present at all is unmoved by dropping one of them.
fn drop_preamble_lines(layout: &Layout, project: &str, prefix: &str) {
    let path = layout.project_issues_path(project);
    let text = fs::read_to_string(&path).unwrap();
    let kept: Vec<&str> = text.lines().filter(|l| !l.starts_with(prefix)).collect();
    assert!(kept.len() < text.lines().count(), "no {prefix} in {path:?}");
    fs::write(&path, format!("{}\n", kept.join("\n"))).unwrap();
}

/// Remove one property line from one issue.
fn drop_property_line(layout: &Layout, project: &str, id: &str, key: &str) {
    let path = layout.project_issues_path(project);
    let text = fs::read_to_string(&path).unwrap();
    let at = text
        .find(&format!(":ID:         {id}"))
        .unwrap_or_else(|| panic!("no {id} in {path:?}"));
    let end = text[at..].find(":END:").expect("a drawer end") + at;
    let drawer = &text[at..end];
    let line = drawer
        .lines()
        .find(|l| l.trim_start().starts_with(&format!(":{key}:")))
        .unwrap_or_else(|| panic!("no :{key}: on {id}"));
    let cut = format!("{line}\n");
    fs::write(&path, text.replacen(&cut, "", 1)).unwrap();
}

/// A finding names the project and the reader can act on it.
///
/// One assertion per finding rather than a count, because a count says a warning
/// arrived and not which: several of these fire on the same corpus, and a test that
/// only counts passes when the wrong one fires.
fn assert_finds(out: &report::CheckReport, needle: &str) {
    assert!(out.text.contains(needle), "no {needle:?} in:\n{}", out.text);
}

#[test]
fn check_wants_a_protocol_stamp_and_reads_the_one_it_finds() {
    let missing = check_after(|l| drop_preamble_lines(l, "atlas", "#+VISSUE:"));
    assert_finds(
        &missing,
        "[warn] atlas: preamble has no #+VISSUE: protocol stamp",
    );

    // Behind, which a reader can fix by normalising.
    let behind = check_after(|l| edit_raw(l, "atlas", "#+VISSUE: 1", "#+VISSUE: 0"));
    assert_finds(&behind, "[warn] atlas: #+VISSUE: 0 is behind protocol 1");
    assert_eq!(behind.errors, 0, "{}", behind.text);

    // Ahead, which it cannot: this binary does not know what the newer one wrote.
    let ahead = check_after(|l| edit_raw(l, "atlas", "#+VISSUE: 1", "#+VISSUE: 9"));
    assert_finds(&ahead, "[err]  atlas: #+VISSUE: 9 is newer than this");
    assert_eq!(ahead.errors, 1, "{}", ahead.text);
}

#[test]
fn check_names_each_preamble_keyword_org_needs() {
    let category = check_after(|l| drop_preamble_lines(l, "atlas", "#+CATEGORY:"));
    assert_finds(&category, "[warn] atlas: preamble has no #+CATEGORY:");

    let filetags = check_after(|l| drop_preamble_lines(l, "atlas", "#+FILETAGS:"));
    assert_finds(&filetags, "[warn] atlas: preamble has no #+FILETAGS:");

    let tags = check_after(|l| drop_preamble_lines(l, "atlas", "#+TAGS:"));
    assert_finds(&tags, "[warn] atlas: preamble has no #+TAGS:");

    let priorities = check_after(|l| drop_preamble_lines(l, "atlas", "#+PRIORITIES:"));
    assert_finds(&priorities, "[warn] atlas: preamble has no #+PRIORITIES:");
}

/// A tracker without `noexport` is one a vault publish will put on the web, which is
/// the one preamble warning that is about disclosure rather than about Org's
/// conveniences.
#[test]
fn check_says_when_a_publish_would_export_the_tracker() {
    let out = check_after(|l| {
        edit_raw(
            l,
            "atlas",
            "#+FILETAGS: :issues:atlas:noexport:",
            "#+FILETAGS: :issues:atlas:",
        )
    });
    assert_finds(&out, "[warn] atlas: #+FILETAGS: has no noexport");
}

#[test]
fn check_counts_headings_that_fight_org_over_a_property() {
    let in_drawer = check_after(|l| set_property(l, "atlas", "atlas-2c3d", "PRIORITY", "A"));
    assert_finds(
        &in_drawer,
        "[warn] atlas: 1 heading(s) put :PRIORITY: in the drawer",
    );

    let computed = check_after(|l| set_property(l, "atlas", "atlas-2c3d", "BLOCKED", "t"));
    assert_finds(&computed, "[warn] atlas: 1 heading(s) set a compute");

    let effort =
        check_after(|l| insert_property_line(l, "atlas", "atlas-2c3d", ":Effort:     a while"));
    assert_finds(&effort, "[warn] atlas: 1 heading(s) have an Effort value");
}

#[test]
fn check_counts_the_two_ways_a_blocker_is_spelled_wrong() {
    // The property Org reads is BLOCKED_BY, and BLOCKEDBY is silently nothing.
    // Raw, because a write through IssueDoc folds the alias into :BLOCKED_BY: and
    // there would be nothing left to find.
    let typo =
        check_after(|l| insert_property_line(l, "atlas", "atlas-2c3d", ":BLOCKEDBY: atlas-1a2b"));
    assert_finds(&typo, "[warn] atlas: 1 heading(s) use :BLOCKEDBY:");

    // :BLOCKER: is org-edna's, and an id list in it is not an edna form.
    let as_ids =
        check_after(|l| insert_property_line(l, "atlas", "atlas-2c3d", ":BLOCKER:   atlas-1a2b"));
    assert_finds(&as_ids, "[warn] atlas: 1 heading(s) use :BLOCKER: as");
}

#[test]
fn check_counts_a_type_that_is_not_on_the_heading() {
    // Through IssueDoc the two stay in step, because the writer renders the tag from
    // the property. A person deleting the tag by hand is what this is about.
    let out = check_after(|l| edit_raw(l, "atlas", ":reporting:feature:", ":reporting:"));
    assert_finds(&out, "[warn] atlas: 1 heading(s) have :TYPE:");
}

/// `#+TAGS: { bug feature task chore plan }` is an exclusive group, so two of them on
/// one heading is a choice Org will not let a person make through its own interface.
#[test]
fn check_counts_a_heading_carrying_two_tags_from_one_exclusive_group() {
    let out = check_after(|l| {
        edit_raw(
            l,
            "atlas",
            ":parser:core:feature:",
            ":parser:core:feature:task:",
        )
    });
    assert_finds(&out, "[warn] atlas: 1 heading(s) carry more than one tag");
}

#[test]
fn check_counts_a_priority_outside_the_declared_range() {
    let out = check_after(|l| edit_raw(l, "atlas", "[#B] Emit a summary", "[#D] Emit a summary"));
    assert_finds(&out, "[warn] atlas: 1 heading(s) have a [#");
}

/// An id with a slash is an org-gcal event, which means a calendar sync owns the
/// heading and a tracker write would fight it.
#[test]
fn check_reports_an_id_a_calendar_sync_owns() {
    let out = check_after(|l| edit_raw(l, "atlas", "atlas-3e4f", "cal/2026abc"));
    assert_finds(&out, "[err]  atlas: 1 heading(s) use an org-gcal event");
}

#[test]
fn check_counts_a_done_parent_with_an_open_child() {
    // atlas-4g5h is DONE and atlas-2c3d is TODO.
    let out = check_after(|l| set_property(l, "atlas", "atlas-2c3d", "PARENT", "atlas-4g5h"));
    assert_finds(&out, "[warn] atlas: 1 DONE heading(s) st");
    assert_eq!(out.errors, 0, "{}", out.text);
}

#[test]
fn check_wants_a_creation_date_on_work_that_is_open() {
    let out = check_after(|l| drop_property_line(l, "atlas", "atlas-2c3d", "CREATED"));
    assert_finds(
        &out,
        "[warn] atlas-2c3d (in atlas) state=TODO but :CREATED: is missing",
    );
}

/// A heading that is DONE while its body says it was rejected records the wrong
/// thing: the state a reader sees and the reason underneath it disagree.
#[test]
fn check_reads_a_done_body_that_says_it_was_rejected() {
    let out = check_after(|l| {
        let path = l.project_issues_path("atlas");
        let text = fs::read_to_string(&path).unwrap();
        let at = text.find(":ID:         atlas-4g5h").unwrap();
        let end = text[at..].find(":END:").unwrap() + at + ":END:".len();
        let mut edited = String::from(&text[..end]);
        edited.push_str("\nSuperseded by atlas-1a2b, so not doing this.\n");
        edited.push_str(&text[end..]);
        fs::write(&path, edited).unwrap();
    });
    assert_finds(
        &out,
        "[warn] atlas-4g5h (in atlas) is DONE but the body reads as a reject",
    );
}

#[test]
fn check_reports_a_heading_holding_a_state_a_sibling_already_settled() {
    let out = check_after(|l| {
        set_property(
            l,
            "atlas",
            "atlas-2c3d",
            "SIBLING_TERMINAL",
            "atlas-4g5h DONE",
        );
    });
    assert_finds(&out, "[warn] atlas-2c3d (in atlas) holds TODO and sibling");
}

/// A body claiming one issue came out of another, with no edge either way, is a
/// claim the graph does not carry: `related` and `impact` cannot see it.
#[test]
fn check_reports_a_discovery_claim_with_no_edge_behind_it() {
    let out = check_after(|l| {
        let path = l.project_issues_path("atlas");
        let text = fs::read_to_string(&path).unwrap();
        let at = text.find(":ID:         atlas-3e4f").unwrap();
        let end = text[at..].find(":END:").unwrap() + at + ":END:".len();
        let mut edited = String::from(&text[..end]);
        edited.push_str("\nDiscovered while reading [[id:atlas-4g5h][the rename]].\n");
        edited.push_str(&text[end..]);
        fs::write(&path, edited).unwrap();
    });
    assert_finds(
        &out,
        "[warn] atlas-3e4f (in atlas) mentions [[id:atlas-4g5h]] as discovered",
    );
}

/// An ORDERED parent means the children are a sequence, so starting the second while
/// the first is open is the one thing that ordering forbids.
#[test]
fn check_counts_work_started_out_of_order_under_an_ordered_parent() {
    let out = check_after(|l| {
        // atlas-4g5h is DONE and sits after atlas-2c3d, which is TODO.
        set_property(l, "atlas", "atlas-4g5h", "PARENT", "atlas-1a2b");
        set_property(l, "atlas", "atlas-1a2b", "ORDERED", "t");
    });
    assert_finds(&out, "[warn] atlas: 1 heading(s) started or closed ");
    assert_eq!(out.errors, 0, "{}", out.text);
}

/// A blocker ring resolves edge by edge and still cannot be scheduled, so it is the
/// graph rather than any one heading that has to report it.
#[test]
fn check_reports_a_blocker_ring_the_edge_checks_all_pass() {
    let out = check_after(|l| {
        // atlas-3e4f is already blocked by atlas-1a2b.
        set_property(l, "atlas", "atlas-1a2b", "BLOCKED_BY", "atlas-3e4f");
    });
    assert_finds(&out, "[err]  blocker graph:");
    assert_eq!(out.errors, 1, "{}", out.text);
}
