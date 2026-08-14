//! The catalog query surface, over a corpus built in memory.
//!
//! These go through the `*_from` entry points rather than a tracker on disk,
//! so a case can state exactly the corpus it needs: a claim held by someone,
//! a deadline that is already overdue, a blocker that names nothing.

use std::collections::BTreeMap;
use std::path::PathBuf;

use vissue_core::catalog::{
    agenda_rows_from, backlinks_from, children_from, claims_from, excerpt_from, issues_rows_from,
    search_hits_from, tree_from, tree_text_from, CatalogService,
};
use vissue_core::error::Error;
use vissue_core::model::{IssueHeading, LogEntry};
use vissue_core::views::{IssueRec, ListQuery};

/// One issue. Everything optional is set through the builder methods so a
/// test names only the fields it depends on.
fn issue(project: &str, id: &str, state: &str, title: &str) -> IssueRec {
    let mut properties = BTreeMap::new();
    properties.insert("ID".to_string(), id.to_string());
    properties.insert("CREATED".to_string(), "[2026-01-02 Fri]".to_string());
    IssueRec {
        project: project.to_string(),
        heading: IssueHeading {
            id: id.to_string(),
            title: title.to_string(),
            state: state.to_string(),
            priority: 'B',
            properties,
            org_tags: Vec::new(),
            property_order: vec!["ID".to_string(), "CREATED".to_string()],
            body: String::new(),
            logbook: Vec::new(),
            line_start: 1,
            line_end: 6,
        },
        path: PathBuf::from(format!("/tmp/{project}/issues.org")),
    }
}

fn with_property(mut rec: IssueRec, key: &str, value: &str) -> IssueRec {
    rec.heading
        .properties
        .insert(key.to_string(), value.to_string());
    rec
}

fn with_priority(mut rec: IssueRec, priority: char) -> IssueRec {
    rec.heading.priority = priority;
    rec
}

fn with_body(mut rec: IssueRec, body: &str) -> IssueRec {
    rec.heading.body = body.to_string();
    rec
}

fn with_org_tags(mut rec: IssueRec, tags: &[&str]) -> IssueRec {
    rec.heading.org_tags = tags.iter().map(|t| t.to_string()).collect();
    rec
}

/// A corpus with the shapes the query verbs actually branch on.
fn corpus() -> Vec<IssueRec> {
    vec![
        with_org_tags(
            with_body(
                with_priority(
                    issue("atlas", "atlas-1a2b", "STARTED", "Parse the header"),
                    'A',
                ),
                "Scope: read the header block before the first record.",
            ),
            &["parser", "core"],
        ),
        with_property(
            with_property(
                with_priority(
                    issue("atlas", "atlas-3e4f", "BLOCKED", "Publish the notes"),
                    'A',
                ),
                "BLOCKED_BY",
                "atlas-1a2b",
            ),
            "DEADLINE",
            "<2020-03-01 Sun>",
        ),
        with_property(
            issue("atlas", "atlas-2c3d", "TODO", "Emit a summary table"),
            "PARENT",
            "atlas-1a2b",
        ),
        issue("atlas", "atlas-4g5h", "DONE", "Rename the config key"),
        issue("beacon", "beacon-5j6k", "TODO", "Document the retry policy"),
    ]
}

fn claimed_corpus() -> Vec<IssueRec> {
    let mut recs = corpus();
    recs[0]
        .heading
        .properties
        .insert("CLAIMED_BY".into(), "worker-1".into());
    recs[0]
        .heading
        .properties
        .insert("CLAIMED_AT".into(), "[2026-01-03 Sat]".into());
    recs
}

fn ids(rows: &[vissue_core::views::IssueRow]) -> Vec<&str> {
    rows.iter().map(|r| r.id.as_str()).collect()
}

#[test]
fn rows_are_ordered_by_priority_then_state_then_id() {
    let recs = corpus();
    let rows = issues_rows_from(&recs, ListQuery::default()).unwrap();
    assert_eq!(rows.len(), 5);
    // [#A] first; within a priority, state then id.
    assert_eq!(&ids(&rows)[..2], &["atlas-3e4f", "atlas-1a2b"]);
}

#[test]
fn a_project_filter_folds_case_and_a_state_filter_narrows() {
    let recs = corpus();
    let by_project = issues_rows_from(
        &recs,
        ListQuery {
            project: Some("ATLAS".into()),
            ..ListQuery::default()
        },
    )
    .unwrap();
    assert_eq!(by_project.len(), 4, "{:?}", ids(&by_project));

    let by_state = issues_rows_from(
        &recs,
        ListQuery {
            state: Some("TODO".into()),
            ..ListQuery::default()
        },
    )
    .unwrap();
    assert_eq!(ids(&by_state), vec!["atlas-2c3d", "beacon-5j6k"]);
}

#[test]
fn ready_drops_closed_work_and_anything_an_open_blocker_holds() {
    let recs = corpus();
    let ready = CatalogService::from_recs(&recs).ready(None).unwrap();
    let ready = ids(&ready);
    assert!(ready.contains(&"atlas-1a2b"), "{ready:?}");
    assert!(ready.contains(&"atlas-2c3d"), "{ready:?}");
    assert!(!ready.contains(&"atlas-3e4f"), "blocked: {ready:?}");
    assert!(!ready.contains(&"atlas-4g5h"), "closed: {ready:?}");
}

#[test]
fn a_blocker_that_names_nothing_does_not_hold_an_issue_back() {
    // The edge is reported by `check`; it must not park the work forever.
    let mut recs = corpus();
    recs[4]
        .heading
        .properties
        .insert("BLOCKED_BY".into(), "atlas-nope".into());
    let ready = CatalogService::from_recs(&recs).ready(None).unwrap();
    assert!(ids(&ready).contains(&"beacon-5j6k"), "{:?}", ids(&ready));
}

#[test]
fn closing_a_blocker_is_not_enough_on_its_own() {
    // `ready` asks for TODO or STARTED. An issue sitting in BLOCKED stays out
    // even once nothing holds it, which is why clearing the edge moves the
    // state back to TODO rather than leaving that to the reader.
    let mut recs = corpus();
    recs[0].heading.state = "DONE".into();
    let still_blocked = CatalogService::from_recs(&recs).ready(None).unwrap();
    assert!(!ids(&still_blocked).contains(&"atlas-3e4f"));

    recs[1].heading.state = "TODO".into();
    recs[1].heading.properties.remove("BLOCKED_BY");
    let freed = CatalogService::from_recs(&recs).ready(None).unwrap();
    assert!(ids(&freed).contains(&"atlas-3e4f"), "{:?}", ids(&freed));
}

#[test]
fn limit_and_offset_page_the_rows() {
    let recs = corpus();
    let page = issues_rows_from(
        &recs,
        ListQuery {
            limit: Some(2),
            offset: Some(1),
            ..ListQuery::default()
        },
    )
    .unwrap();
    assert_eq!(page.len(), 2);
    let all = issues_rows_from(&recs, ListQuery::default()).unwrap();
    assert_eq!(ids(&page), ids(&all)[1..3].to_vec());
}

#[test]
fn detail_carries_the_tags_and_the_file_range() {
    let recs = corpus();
    let detail = CatalogService::from_recs(&recs)
        .detail("atlas-1a2b")
        .unwrap();
    assert_eq!(detail.project, "atlas");
    assert_eq!(detail.org_tags, vec!["parser", "core"]);
    assert!(detail.tags.contains(&"parser".to_string()));
    assert_eq!(detail.line_start, 1);
    // `file` is the range an editor opens, not just the path.
    assert!(detail.file.ends_with("issues.org:1-6"), "{}", detail.file);
}

#[test]
fn detail_carries_the_logbook() {
    let mut rec = issue("atlas", "atlas-1a2b", "STARTED", "Parse the header");
    rec.heading.logbook = vec![
        LogEntry {
            timestamp: "[2026-01-14 Wed 09:12]".into(),
            from_state: Some("TODO".into()),
            to_state: Some("STARTED".into()),
            note: None,
            raw: None,
        },
        LogEntry {
            timestamp: String::new(),
            from_state: None,
            to_state: None,
            note: None,
            raw: Some("CLOCK: [2026-01-14 Wed 09:12]--[2026-01-14 Wed 10:42] =>  1:30".into()),
        },
        LogEntry {
            timestamp: "[2026-01-14 Wed 11:00]".into(),
            from_state: None,
            to_state: None,
            note: Some("parser landed".into()),
            raw: None,
        },
    ];
    let detail = CatalogService::from_recs(&[rec])
        .detail("atlas-1a2b")
        .unwrap();
    assert_eq!(detail.logbook.len(), 3);
    assert_eq!(detail.logbook[0].from_state.as_deref(), Some("TODO"));
    assert_eq!(detail.logbook[0].to_state.as_deref(), Some("STARTED"));
    assert!(
        detail.logbook[1]
            .raw
            .as_deref()
            .is_some_and(|raw| raw.starts_with("CLOCK:")),
        "{:?}",
        detail.logbook[1]
    );
    assert_eq!(detail.logbook[2].note.as_deref(), Some("parser landed"));
}

#[test]
fn detail_names_an_id_it_cannot_find() {
    let recs = corpus();
    let err = CatalogService::from_recs(&recs)
        .detail("atlas-zzzz")
        .unwrap_err();
    assert!(matches!(err, Error::IssueNotFound { ref id } if id == "atlas-zzzz"));
}

#[test]
fn search_reads_the_body_the_tags_and_the_id() {
    let recs = corpus();
    let service = CatalogService::from_recs(&recs);
    for needle in ["header block", "PARSER", "atlas-1a2b"] {
        let hits = service.search(needle, 10).unwrap();
        assert!(
            hits.iter().any(|h| h.id == "atlas-1a2b"),
            "{needle:?} missed the issue"
        );
    }
    assert!(service
        .search("nothing matches this", 10)
        .unwrap()
        .is_empty());
}

#[test]
fn search_respects_its_limit() {
    let recs = corpus();
    assert_eq!(search_hits_from(&recs, "the", 2).unwrap().len(), 2);
}

#[test]
fn claims_lists_the_holder_and_narrows_by_holder_and_project() {
    let recs = claimed_corpus();
    let all = claims_from(&recs, None, None).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, "atlas-1a2b");
    assert_eq!(all[0].holder.as_deref(), Some("worker-1"));
    assert!(all[0].age_days >= 0);

    assert_eq!(claims_from(&recs, Some("worker-1"), None).unwrap().len(), 1);
    assert!(claims_from(&recs, Some("nobody"), None).unwrap().is_empty());
    assert!(claims_from(&recs, None, Some("beacon")).unwrap().is_empty());
}

#[test]
fn an_overdue_deadline_sorts_ahead_and_reports_its_age() {
    let recs = corpus();
    let rows = agenda_rows_from(&recs, 30, None).unwrap();
    let overdue = rows
        .iter()
        .find(|r| r.id == "atlas-3e4f")
        .expect("the dated issue is in the agenda");
    assert_eq!(overdue.kind, "deadline");
    assert!(overdue.overdue_days > 0, "{overdue:?}");
}

#[test]
fn the_agenda_keeps_a_blocked_issue_and_narrows_by_project() {
    // A blocked issue's date does not stop mattering while it waits.
    let recs = corpus();
    assert!(agenda_rows_from(&recs, 30, Some("atlas"))
        .unwrap()
        .iter()
        .any(|r| r.state == "BLOCKED"));
    assert!(agenda_rows_from(&recs, 30, Some("beacon"))
        .unwrap()
        .is_empty());
}

#[test]
fn a_tree_carries_children_and_the_blockers_of_each_node() {
    let recs = corpus();
    let tree = tree_from(&recs, "atlas-1a2b").unwrap();
    assert_eq!(tree.id, "atlas-1a2b");
    assert!(tree.children.iter().any(|c| c.id == "atlas-2c3d"));

    let blocked = tree_from(&recs, "atlas-3e4f").unwrap();
    assert_eq!(blocked.blocked_by, vec!["atlas-1a2b"]);
}

#[test]
fn every_walk_refuses_an_id_the_corpus_does_not_hold() {
    let recs = corpus();
    let service = CatalogService::from_recs(&recs);
    assert!(matches!(
        tree_from(&recs, "atlas-zzzz").unwrap_err(),
        Error::IssueNotFound { .. }
    ));
    assert!(matches!(
        children_from(&recs, "atlas-zzzz").unwrap_err(),
        Error::IssueNotFound { .. }
    ));
    assert!(matches!(
        service.ancestors("atlas-zzzz", 2).unwrap_err(),
        Error::IssueNotFound { .. }
    ));
    assert!(matches!(
        service.impact("atlas-zzzz", 2).unwrap_err(),
        Error::IssueNotFound { .. }
    ));
    assert!(matches!(
        backlinks_from(&recs, "atlas-zzzz").unwrap_err(),
        Error::IssueNotFound { .. }
    ));
}

#[test]
fn children_of_a_real_issue_with_none_is_empty_rather_than_an_error() {
    let recs = corpus();
    assert!(children_from(&recs, "atlas-4g5h").unwrap().is_empty());
}

#[test]
fn ancestors_and_impact_walk_opposite_directions() {
    let recs = corpus();
    let service = CatalogService::from_recs(&recs);
    let ancestors = service.ancestors("atlas-3e4f", 3).unwrap();
    assert!(
        ancestors.iter().any(|h| h.id == "atlas-1a2b"),
        "{ancestors:?}"
    );
    let impact = service.impact("atlas-1a2b", 3).unwrap();
    assert!(impact.iter().any(|h| h.id == "atlas-3e4f"), "{impact:?}");
}

#[test]
fn backlinks_name_the_relation_that_points_at_the_issue() {
    let recs = corpus();
    let hits = backlinks_from(&recs, "atlas-1a2b").unwrap();
    let relations: Vec<&str> = hits.iter().map(|h| h.relation.as_str()).collect();
    assert!(relations.contains(&"blocked-by"), "{relations:?}");
    assert!(relations.contains(&"parent"), "{relations:?}");
}

#[test]
fn related_ranks_a_declared_edge_above_shared_words() {
    let recs = corpus();
    let hits = CatalogService::from_recs(&recs)
        .related("atlas-1a2b", 2, 10)
        .unwrap();
    assert!(!hits.is_empty());
    let top = &hits[0];
    assert!(
        top.evidence
            .iter()
            .any(|e| e.contains("blocks") || e.contains("child") || e.contains("org_distance")),
        "a declared relation should lead: {:?}",
        top.evidence
    );
}

#[test]
fn an_excerpt_reads_the_file_range_and_suppresses_a_credential() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("issues.org");
    std::fs::write(
        &path,
        "* TODO [#B] Ordinary\n:PROPERTIES:\n:ID:         demo-aaaa\n:END:\n\nScope: nothing secret.\n",
    )
    .unwrap();
    let mut rec = issue("demo", "demo-aaaa", "TODO", "Ordinary");
    rec.path = path.clone();
    let excerpt = excerpt_from(&rec).unwrap();
    assert!(!excerpt.suppressed);
    assert!(excerpt.text.contains("Ordinary"), "{}", excerpt.text);

    std::fs::write(
        &path,
        "* TODO [#B] Leaky\n:PROPERTIES:\n:ID:         demo-aaaa\n:END:\n\napi_key = 9f8e7d6c5b4a3210ff\n",
    )
    .unwrap();
    let suppressed = excerpt_from(&rec).unwrap();
    assert!(suppressed.suppressed, "{}", suppressed.text);
    assert!(!suppressed.text.contains("9f8e7d6c5b4a3210ff"));
}

#[test]
fn an_excerpt_of_a_file_that_is_gone_is_an_error_not_a_panic() {
    let mut rec = issue("demo", "demo-aaaa", "TODO", "Missing file");
    rec.path = PathBuf::from("/nonexistent/does/not/exist.org");
    assert!(matches!(excerpt_from(&rec).unwrap_err(), Error::Other(_)));
}

#[test]
fn tree_text_from_ascii_and_dot_name_the_root() {
    let recs = corpus();
    let root = tree_from(&recs, "atlas-1a2b").unwrap().id;
    let ascii = tree_text_from(&recs, &root, "ascii").unwrap();
    assert!(ascii.contains("atlas-1a2b"), "{ascii}");
    let dot = tree_text_from(&recs, &root, "dot").unwrap();
    assert!(dot.contains("digraph"), "{dot}");
    let err = tree_text_from(&recs, "missing-zzzz", "ascii").unwrap_err();
    assert!(matches!(err, Error::IssueNotFound { .. }));
}

/// Two issues naming each other as parent.
///
/// Nothing stops a person from writing this in an org file, so every walk
/// over the parent edges has to terminate rather than recurse until the
/// stack runs out.
fn cyclic_corpus() -> Vec<IssueRec> {
    vec![
        with_property(
            issue("atlas", "atlas-aaaa", "TODO", "First half of the loop"),
            "PARENT",
            "atlas-bbbb",
        ),
        with_property(
            issue("atlas", "atlas-bbbb", "TODO", "Second half of the loop"),
            "PARENT",
            "atlas-aaaa",
        ),
    ]
}

#[test]
fn a_parent_cycle_stops_instead_of_recurring_forever() {
    let recs = cyclic_corpus();
    let tree = tree_from(&recs, "atlas-aaaa").unwrap();
    assert_eq!(tree.id, "atlas-aaaa");
    // The loop closes on the second visit, and the repeat carries no state:
    // that is how a reader tells it apart from a real node.
    let child = &tree.children[0];
    assert_eq!(child.id, "atlas-bbbb");
    let repeat = &child.children[0];
    assert_eq!(repeat.id, "atlas-aaaa");
    assert!(repeat.state.is_empty(), "{repeat:?}");
    assert!(repeat.children.is_empty(), "{repeat:?}");
}

#[test]
fn the_ascii_tree_says_where_a_cycle_closed() {
    let recs = cyclic_corpus();
    let text = tree_text_from(&recs, "atlas-aaaa", "ascii").unwrap();
    assert!(text.contains("(cycle, stopping)"), "{text}");
    assert_eq!(
        text.matches("atlas-aaaa").count(),
        2,
        "the root appears once as itself and once as the cycle: {text}"
    );
}

#[test]
fn the_ascii_tree_indents_children_and_names_blockers() {
    let recs = corpus();
    let text = tree_text_from(&recs, "atlas-1a2b", "ascii").unwrap();
    let child = text
        .lines()
        .find(|l| l.contains("atlas-2c3d"))
        .unwrap_or_else(|| panic!("no child row in {text}"));
    assert!(child.starts_with("  "), "a child is indented: {child:?}");
    assert!(child.contains("Emit a summary table"), "{child:?}");

    // A blocked issue names what holds it, one line per blocker.
    let blocked = tree_text_from(&recs, "atlas-3e4f", "ascii").unwrap();
    assert!(blocked.contains("* blocked-by atlas-1a2b"), "{blocked}");
}

#[test]
fn the_dot_tree_draws_both_kinds_of_edge() {
    let recs = corpus();
    let dot = tree_text_from(&recs, "atlas-1a2b", "dot").unwrap();
    assert!(dot.contains("digraph vissue_tree {"), "{dot}");
    assert!(dot.trim_end().ends_with('}'), "{dot}");
    // A parent edge is solid; a blocker edge is dashed and labelled.
    assert!(
        dot.contains("\"atlas-1a2b\" -> \"atlas-2c3d\""),
        "no parent edge: {dot}"
    );
    let blocked = tree_text_from(&recs, "atlas-3e4f", "dot").unwrap();
    assert!(
        blocked.contains("label=\"blocks\"") && blocked.contains("style=dashed"),
        "no blocker edge: {blocked}"
    );
}

#[test]
fn a_dot_tree_survives_a_cycle() {
    let recs = cyclic_corpus();
    let dot = tree_text_from(&recs, "atlas-aaaa", "dot").unwrap();
    assert!(dot.contains("digraph"), "{dot}");
    assert!(dot.contains("atlas-bbbb"), "{dot}");
}

#[test]
fn an_unknown_tree_format_names_the_ones_that_work() {
    let recs = corpus();
    let err = tree_text_from(&recs, "atlas-1a2b", "svg").unwrap_err();
    let text = err.to_string();
    assert!(text.contains("svg"), "{text}");
    assert!(text.contains("ascii") && text.contains("dot"), "{text}");
}

#[test]
fn children_of_an_id_the_corpus_does_not_hold_is_an_error() {
    let recs = corpus();
    assert!(matches!(
        children_from(&recs, "atlas-zzzz").unwrap_err(),
        Error::IssueNotFound { .. }
    ));
}

#[test]
fn backlinks_report_a_discovered_from_edge_and_a_bare_mention() {
    let mut recs = corpus();
    recs.push(with_property(
        issue("atlas", "atlas-6i7j", "TODO", "Fell out of the parser work"),
        "DISCOVERED_FROM",
        "atlas-1a2b",
    ));
    recs.push(with_body(
        issue(
            "atlas",
            "atlas-7k8l",
            "TODO",
            "Unrelated, but talks about it",
        ),
        "Same failure as atlas-1a2b, different file.",
    ));

    let hits = backlinks_from(&recs, "atlas-1a2b").unwrap();
    let by_id: Vec<(&str, &str)> = hits
        .iter()
        .map(|h| (h.id.as_str(), h.relation.as_str()))
        .collect();
    assert!(
        by_id.contains(&("atlas-6i7j", "discovered-from")),
        "{by_id:?}"
    );
    assert!(by_id.contains(&("atlas-7k8l", "body mention")), "{by_id:?}");
    // A declared edge wins: a mention is only reported when nothing else is.
    assert!(
        !by_id
            .iter()
            .any(|(id, rel)| *id == "atlas-6i7j" && *rel == "body mention"),
        "{by_id:?}"
    );
}

#[test]
fn an_issue_is_not_its_own_backlink() {
    let recs = with_body(
        issue("atlas", "atlas-9m0n", "TODO", "Mentions itself"),
        "See atlas-9m0n for the rest.",
    );
    let hits = backlinks_from(std::slice::from_ref(&recs), "atlas-9m0n").unwrap();
    assert!(hits.is_empty(), "{hits:?}");
}

#[test]
fn backlinks_of_an_unknown_id_is_an_error() {
    let recs = corpus();
    assert!(matches!(
        backlinks_from(&recs, "atlas-zzzz").unwrap_err(),
        Error::IssueNotFound { .. }
    ));
}
