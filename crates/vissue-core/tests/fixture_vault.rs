//! End-to-end checks against the synthetic fixture tracker in `tests/fixture_vault`.

use std::fs;
use std::path::{Path, PathBuf};

use vissue_core::config::{Layout, DEFAULT_PREFIX};
use vissue_core::mirror::{self, Format};
use vissue_core::report;
use vissue_core::store::{self, IssueDoc};

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
    assert_eq!(parser["properties"]["TAGS"], "parser,core");
    assert_eq!(parser["logbook"][0]["to"], "STARTED");
    assert_eq!(parser["logbook"][0]["from"], "TODO");
    assert!(parser["body"]
        .as_str()
        .unwrap()
        .starts_with("Scope: read the header block"));
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
    doc.headings
        .iter_mut()
        .find(|h| h.id == "atlas-3e4f")
        .unwrap()
        .properties
        .remove("BLOCKED_BY");
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
    assert!(report::stale(&layout, 30, Some("beacon"))
        .unwrap()
        .lines()
        .all(|l| l.starts_with("beacon-")));
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
fn a_search_finds_body_and_property_text() {
    let layout = fixture_layout();
    assert!(report::search(&layout, "backoff table", 10)
        .unwrap()
        .contains("beacon-5j6k"));
    assert!(report::search(&layout, "PARSER,CORE", 10)
        .unwrap()
        .contains("atlas-1a2b"));
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
    assert!(report::children(&layout, "beacon-design-0001")
        .unwrap()
        .contains("beacon-5j6k"));
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
    assert!(
        org.contains("[[file:Software/atlas/issues.org::#atlas-3e4f][atlas-3e4f]]"),
        "{org}"
    );
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
    text = text.replace(
        ":TYPE:       chore\n:END:",
        ":TYPE:       chore\n:DISCOVERED_FROM: atlas-1a2b\n:END:",
    );
    fs::write(path, text).unwrap();

    let related = report::related(&layout, "atlas-1a2b", 1, 10, "text").unwrap();
    assert!(related.contains("atlas-3e4f"), "{related}");
    assert!(related.contains("org_link"), "{related}");
    assert!(related.contains("atlas-4g5h"), "{related}");
    assert!(related.contains("discovered_from"), "{related}");
}

#[test]
fn show_reports_the_file_range_without_the_body() {
    let layout = fixture_layout();
    let text = report::show(&layout, "atlas-2c3d").unwrap();
    assert!(text.contains("ID:       atlas-2c3d"), "{text}");
    assert!(text.contains("Project:  atlas"), "{text}");
    assert!(text.contains("State:    TODO"), "{text}");
    assert!(text.contains("Priority: [#B]"), "{text}");
    assert!(text.contains("atlas/issues.org:"), "{text}");
    assert!(
        !text.contains("Scope: one row per parsed record"),
        "show must not print the body: {text}"
    );
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
    std::env::set_var("VISSUE_EVENTS", "0");
    let result = vissue_core::ops::create(
        &layout,
        "atlas",
        "a quiet issue",
        vissue_core::ops::CreateOpts {
            quiet: true,
            ..Default::default()
        },
    );
    std::env::remove_var("VISSUE_EVENTS");
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
    assert!(!report::show(&layout, "atlas-2c3d")
        .unwrap()
        .contains("Claimed:"));
}

#[test]
fn claiming_stamps_the_identity_and_releasing_keeps_the_history() {
    let _guard = EVENTS_ENV.lock().unwrap_or_else(|p| p.into_inner());
    let (_dir, layout) = writable_copy();
    std::env::set_var("VISSUE_AGENT", "test-runner-1");
    let claimed = vissue_core::ops::claim(&layout, "atlas-2c3d", false);
    std::env::remove_var("VISSUE_AGENT");
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
    std::env::set_var("VISSUE_AGENT", "someone-else");
    let refused = vissue_core::ops::claim(&layout, "atlas-1a2b", false);
    let forced = vissue_core::ops::claim(&layout, "atlas-1a2b", true);
    std::env::remove_var("VISSUE_AGENT");

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
