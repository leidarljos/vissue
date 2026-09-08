//! The catalog query surface, over a corpus built in memory.
//!
//! These go through the `*_from` entry points rather than a tracker on disk,
//! so a case can state exactly the corpus it needs: a claim held by someone,
//! a deadline that is already overdue, a blocker that names nothing.

#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use vissue_core::catalog::{
    CatalogService, agenda_rows_from, backlinks_from, children_from, claims_from, excerpt_from,
    issues_rows_from, recall_from, search_hits_from, tree_from, tree_text_from,
};
use vissue_core::error::Error;
use vissue_core::model::IssueHeading;
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
            statistics: None,
            property_order: vec!["ID".to_string(), "CREATED".to_string()],
            extra_drawers: Vec::new(),
            body: String::new(),
            logbook: Vec::new(),
            line_start: 1,
            line_end: 6,
        },
        path: PathBuf::from(format!("/tmp/{project}/issues.org")),
        tag_settings: vissue_core::org::TagSettings::default(),
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
    // `file` carries the range an editor opens: path, start, end.
    assert!(detail.file.ends_with("issues.org:1-6"), "{}", detail.file);
}

#[test]
fn ready_waits_on_an_ordered_sibling() {
    let mut recs = corpus();
    let parent_id = recs[1].heading.id.clone();
    recs[1]
        .heading
        .properties
        .insert("ORDERED".into(), "t".into());
    recs[2]
        .heading
        .properties
        .insert("PARENT".into(), parent_id.clone());
    recs[2].heading.line_start = 20;
    recs[3]
        .heading
        .properties
        .insert("PARENT".into(), parent_id);
    recs[3].heading.line_start = 40;
    recs[3].heading.state = "TODO".into();
    recs[3].heading.properties.remove("BLOCKED_BY");
    let later = recs[3].heading.id.clone();
    let cat = CatalogService::from_recs(&recs);
    let held = cat.ready(None).unwrap();
    let ready = ids(&held);
    assert!(
        !ready.contains(&later.as_str()),
        "later ORDERED sibling was ready: {ready:?}"
    );
    recs[2].heading.state = "DONE".into();
    let cat = CatalogService::from_recs(&recs);
    let freed = cat.ready(None).unwrap();
    let ready = ids(&freed);
    assert!(
        ready.contains(&later.as_str()),
        "later sibling still held after earlier DONE: {ready:?}"
    );
}

#[test]
fn search_matches_filetags_and_a_group_tag() {
    let mut recs = corpus();
    recs[0].tag_settings.filetags = vec!["issues".into(), "atlas".into()];
    recs[0].tag_settings.hierarchies = vec![("area".into(), vec!["core".into(), "cli".into()])];
    let cat = CatalogService::from_recs(&recs);
    let by_file = cat.search("issues", 10).unwrap();
    assert!(by_file.iter().any(|h| h.id == "atlas-1a2b"), "{by_file:?}");
    let by_group = cat.search("area", 10).unwrap();
    assert!(
        by_group.iter().any(|h| h.id == "atlas-1a2b"),
        "group tag area should match heading tagged core: {by_group:?}"
    );
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
    assert!(
        service
            .search("nothing matches this", 10)
            .unwrap()
            .is_empty()
    );
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
    assert!(
        agenda_rows_from(&recs, 30, Some("atlas"))
            .unwrap()
            .iter()
            .any(|r| r.state == "BLOCKED")
    );
    assert!(
        agenda_rows_from(&recs, 30, Some("beacon"))
            .unwrap()
            .is_empty()
    );
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
fn backlinks_report_a_pivoted_to_edge() {
    let mut recs = corpus();
    recs.push(with_property(
        issue("atlas", "atlas-8m9n", "CANCELLED", "Old approach"),
        "PIVOTED_TO",
        "atlas-1a2b",
    ));

    let hits = backlinks_from(&recs, "atlas-1a2b").unwrap();
    let by_id: Vec<(&str, &str)> = hits
        .iter()
        .map(|h| (h.id.as_str(), h.relation.as_str()))
        .collect();
    assert!(by_id.contains(&("atlas-8m9n", "pivoted-to")), "{by_id:?}");
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

/// The score of one hit.
fn tagged_score(hits: &[vissue_core::views::RelatedHit], id: &str) -> f64 {
    hits.iter().find(|h| h.id == id).map_or(0.0, |h| h.score)
}

/// Evidence recorded for one hit, or a panic naming what did come back.
fn evidence_for(hits: &[vissue_core::views::RelatedHit], id: &str) -> Vec<String> {
    hits.iter()
        .find(|h| h.id == id)
        .unwrap_or_else(|| {
            panic!(
                "no {id} among {:?}",
                hits.iter().map(|h| h.id.as_str()).collect::<Vec<_>>()
            )
        })
        .evidence
        .clone()
}

/// An edge is named from both ends, and differently from each.
///
/// `blocks` and `blocked_by` are the same edge read from opposite directions, as are
/// `parent` and `child`. A reader asking about the blocked issue wants to know it is
/// waiting, and one asking about the blocker wants to know something waits on it, so
/// naming both ends the same would lose which.
#[test]
fn related_names_a_declared_edge_from_the_end_it_is_asked_from() {
    let recs = corpus();
    let service = CatalogService::from_recs(&recs);

    // atlas-3e4f is BLOCKED_BY atlas-1a2b, and atlas-2c3d has it as PARENT.
    let from_blocked = service.related("atlas-3e4f", 2, 10).unwrap();
    assert!(
        evidence_for(&from_blocked, "atlas-1a2b").contains(&"blocked_by".to_string()),
        "{:?}",
        evidence_for(&from_blocked, "atlas-1a2b")
    );

    let from_child = service.related("atlas-2c3d", 2, 10).unwrap();
    assert!(
        evidence_for(&from_child, "atlas-1a2b").contains(&"parent".to_string()),
        "{:?}",
        evidence_for(&from_child, "atlas-1a2b")
    );

    // And the other way round, which is the direction already relied on.
    let from_blocker = service.related("atlas-1a2b", 2, 10).unwrap();
    let back = evidence_for(&from_blocker, "atlas-3e4f");
    assert!(back.contains(&"blocks".to_string()), "{back:?}");
    let down = evidence_for(&from_blocker, "atlas-2c3d");
    assert!(down.contains(&"child".to_string()), "{down:?}");
}

/// Provenance is an edge too, and it also reads differently from each end.
#[test]
fn related_names_provenance_from_the_end_it_is_asked_from() {
    let mut recs = corpus();
    // atlas-4g5h came out of atlas-1a2b, and beacon-5j6k replaced atlas-2c3d.
    recs[3] = with_property(recs[3].clone(), "DISCOVERED_FROM", "atlas-1a2b");
    recs[2] = with_property(recs[2].clone(), "PIVOTED_TO", "beacon-5j6k");
    let service = CatalogService::from_recs(&recs);

    let from_origin = service.related("atlas-1a2b", 1, 10).unwrap();
    let found = evidence_for(&from_origin, "atlas-4g5h");
    assert!(found.contains(&"discovered_from".to_string()), "{found:?}");

    let from_discovery = service.related("atlas-4g5h", 1, 10).unwrap();
    let source = evidence_for(&from_discovery, "atlas-1a2b");
    assert!(source.contains(&"source_of".to_string()), "{source:?}");

    let from_abandoned = service.related("atlas-2c3d", 1, 10).unwrap();
    let onwards = evidence_for(&from_abandoned, "beacon-5j6k");
    assert!(onwards.contains(&"pivoted_to".to_string()), "{onwards:?}");

    let from_successor = service.related("beacon-5j6k", 1, 10).unwrap();
    let back = evidence_for(&from_successor, "atlas-2c3d");
    assert!(back.contains(&"successor_of".to_string()), "{back:?}");
}

/// Two issues carrying the same tag are related by that alone, and two in one project
/// are weakly related by that alone. Both are weak on purpose: a shared tag scores far
/// under a declared edge, and a shared project barely registers.
#[test]
fn related_scores_a_shared_tag_and_a_shared_project() {
    let mut recs = corpus();
    recs[3] = with_org_tags(recs[3].clone(), &["parser", "docs"]);
    let hits = CatalogService::from_recs(&recs)
        .related("atlas-1a2b", 1, 10)
        .unwrap();

    let tagged = evidence_for(&hits, "atlas-4g5h");
    assert!(tagged.contains(&"shared_tags".to_string()), "{tagged:?}");
    assert!(tagged.contains(&"same_project".to_string()), "{tagged:?}");

    // The other project shares neither, so it is here on its words or not at all.
    let elsewhere = hits.iter().find(|h| h.id == "beacon-5j6k");
    assert!(
        elsewhere.is_none_or(|h| !h.evidence.contains(&"same_project".to_string())),
        "{:?}",
        elsewhere.map(|h| &h.evidence)
    );

    // A declared edge outranks both, which is what the weights are for. Two of them
    // tie here and the id settles it, so the assertion is on the score rather than on
    // which of the pair came first.
    let declared = hits[0].score;
    assert!(
        declared > 1_000.0 && declared > tagged_score(&hits, "atlas-4g5h") * 10.0,
        "{hits:?}"
    );
}

/// The shared tag carries weight, not just a label.
///
/// Two issues alike in every way the scorer looks at, one of them carrying a tag the
/// target also carries. Asserting the label alone would pass with the weight set to
/// zero, which is a scorer that records the reason and ignores it.
#[test]
fn a_shared_tag_is_worth_more_than_the_words_that_come_with_it() {
    let recs = vec![
        with_org_tags(
            issue("atlas", "atlas-target", "TODO", "Parse the header"),
            &["parser", "core"],
        ),
        with_org_tags(
            issue("atlas", "atlas-tagged", "TODO", "Unrelated wording here"),
            &["parser"],
        ),
        issue("atlas", "atlas-plain", "TODO", "Unrelated wording here"),
    ];
    let hits = CatalogService::from_recs(&recs)
        .related("atlas-target", 1, 10)
        .unwrap();
    let tagged = tagged_score(&hits, "atlas-tagged");
    let plain = tagged_score(&hits, "atlas-plain");
    assert!(
        tagged - plain >= 25.0,
        "the tag is worth {}, and matching its word alone would be worth a few: {hits:?}",
        tagged - plain
    );
}

#[test]
fn related_returns_no_more_hits_than_the_limit() {
    let recs = corpus();
    let service = CatalogService::from_recs(&recs);
    let all = service.related("atlas-1a2b", 2, 10).unwrap();
    assert!(all.len() > 1, "{all:?}");

    let capped = service.related("atlas-1a2b", 2, 1).unwrap();
    assert_eq!(capped.len(), 1);
    // The one kept is the one that ranked first, not an arbitrary survivor.
    assert_eq!(capped[0].id, all[0].id);
}

/// Two hits that score the same are ordered by id, so the ranking is total and a
/// caller reading the top of the list twice reads the same thing.
#[test]
fn related_breaks_a_score_tie_on_the_id() {
    let recs = vec![
        issue("atlas", "atlas-target", "TODO", "Parse the header"),
        issue("atlas", "atlas-zzzz", "TODO", "Unrelated wording here"),
        issue("atlas", "atlas-aaaa", "TODO", "Unrelated wording here"),
    ];
    let hits = CatalogService::from_recs(&recs)
        .related("atlas-target", 1, 10)
        .unwrap();
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(
        (hits[0].score - hits[1].score).abs() < f64::EPSILON,
        "{hits:?}"
    );
    assert_eq!(
        [hits[0].id.as_str(), hits[1].id.as_str()],
        ["atlas-aaaa", "atlas-zzzz"],
        "{hits:?}"
    );
}

/// Sharing a project is worth something, and barely anything.
///
/// Two issues alike in every way the scorer looks at, one of them in the target's
/// project. The gap is the whole weight, which is small on purpose: a tracker where
/// most issues sit in one project would otherwise rank every one of them as related.
#[test]
fn sharing_a_project_is_worth_a_little_and_not_nothing() {
    let recs = vec![
        issue("atlas", "atlas-target", "TODO", "Parse the header"),
        issue("atlas", "atlas-near", "TODO", "Unrelated wording here"),
        issue("beacon", "beacon-far", "TODO", "Unrelated wording here"),
    ];
    let hits = CatalogService::from_recs(&recs)
        .related("atlas-target", 1, 10)
        .unwrap();
    let near = tagged_score(&hits, "atlas-near");
    let far = tagged_score(&hits, "beacon-far");
    assert!(
        (near - far - 2.0).abs() < 1e-9,
        "the project is worth {}: {hits:?}",
        near - far
    );
}

/// The working set for a node is what the plan says it stands on: the parent
/// chain above it and the products of what blocks it. Nothing is ranked, so the
/// assertion is on membership rather than on an order a scorer chose.
#[test]
fn recall_gathers_the_plan_and_the_products_of_the_blockers() {
    let issues = vec![
        issue("keys", "keys-e0pl", "TODO", "Epic: Colemak leader sequence"),
        with_property(
            with_property(
                issue("keys", "keys-cata", "DONE", "Catalog of bindable actions"),
                "PARENT",
                "keys-e0pl",
            ),
            "DEEDS",
            "deed-file-catalog",
        ),
        with_property(
            with_property(
                issue("keys", "keys-toml", "TODO", "keys.toml schema"),
                "PARENT",
                "keys-e0pl",
            ),
            "BLOCKED_BY",
            "keys-cata",
        ),
    ];

    let set = recall_from(&issues, "keys-toml", 1, false).unwrap();
    assert_eq!(set.id, "keys-toml");
    assert_eq!(
        set.plan.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["keys-e0pl"],
        "the parent chain is the plan the node sits in"
    );
    assert_eq!(set.inputs.len(), 1);
    assert_eq!(set.inputs[0].id, "keys-cata");
    assert_eq!(set.inputs[0].relation, "blocked-by");
    assert_eq!(set.inputs[0].deeds, vec!["deed-file-catalog".to_string()]);
    assert!(set.produced.is_empty());
}

/// The parent chain is reported outermost first, so a reader gets the plan
/// before the node inside it rather than the other way up.
#[test]
fn the_plan_reads_from_the_outermost_parent_down() {
    let issues = vec![
        issue("keys", "keys-root", "TODO", "Programme"),
        with_property(
            issue("keys", "keys-mid", "TODO", "Epic"),
            "PARENT",
            "keys-root",
        ),
        with_property(
            issue("keys", "keys-leaf", "TODO", "Task"),
            "PARENT",
            "keys-mid",
        ),
    ];

    let set = recall_from(&issues, "keys-leaf", 1, false).unwrap();
    assert_eq!(
        set.plan.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["keys-root", "keys-mid"]
    );
}

/// A `:PARENT:` cycle is something `check` reports rather than prevents, so the
/// walk has to stop on its own. Hanging the command that explains an issue is a
/// worse failure than the bad edge it was asked about.
#[test]
fn a_parent_cycle_stops_the_plan_walk() {
    let issues = vec![
        with_property(issue("p", "p-a", "TODO", "a"), "PARENT", "p-b"),
        with_property(issue("p", "p-b", "TODO", "b"), "PARENT", "p-a"),
    ];

    let set = recall_from(&issues, "p-a", 1, false).unwrap();
    assert_eq!(
        set.plan.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["p-b"],
        "the walk stops at the first id it has already seen"
    );
}

/// A bounced issue's origin is an input no blocker edge carries: the work came
/// from there and nothing else points back at it.
#[test]
fn the_origin_of_a_bounce_is_an_input() {
    let issues = vec![
        with_property(
            issue("p", "p-src", "CANCELLED", "the original attempt"),
            "DEEDS",
            "deed-patch-attempt",
        ),
        with_property(
            issue("p", "p-new", "TODO", "the second attempt"),
            "DISCOVERED_FROM",
            "p-src",
        ),
    ];

    let set = recall_from(&issues, "p-new", 1, false).unwrap();
    assert_eq!(set.inputs.len(), 1);
    assert_eq!(set.inputs[0].relation, "discovered-from");
    assert_eq!(set.inputs[0].deeds, vec!["deed-patch-attempt".to_string()]);
}

/// An origin that also blocks the issue is one input, not two. The blocker edge
/// is the stronger statement and is reported.
#[test]
fn an_origin_that_also_blocks_is_reported_once() {
    let issues = vec![
        issue("p", "p-src", "DONE", "the original attempt"),
        with_property(
            with_property(
                issue("p", "p-new", "TODO", "the second attempt"),
                "DISCOVERED_FROM",
                "p-src",
            ),
            "BLOCKED_BY",
            "p-src",
        ),
    ];

    let set = recall_from(&issues, "p-new", 1, false).unwrap();
    assert_eq!(set.inputs.len(), 1, "{:?}", set.inputs);
    assert_eq!(set.inputs[0].relation, "blocked-by");
}

/// One hop is the default because a deed carries its own sources and `deedar
/// trail` walks them. Asking for more hops here has to actually widen the set,
/// or the flag is a lie.
#[test]
fn depth_widens_the_blocker_walk() {
    let issues = vec![
        with_property(
            issue("p", "p-first", "DONE", "first"),
            "DEEDS",
            "deed-file-first",
        ),
        with_property(
            with_property(
                issue("p", "p-mid", "DONE", "middle"),
                "BLOCKED_BY",
                "p-first",
            ),
            "DEEDS",
            "deed-file-middle",
        ),
        with_property(issue("p", "p-last", "TODO", "last"), "BLOCKED_BY", "p-mid"),
    ];

    let one = recall_from(&issues, "p-last", 1, false).unwrap();
    assert_eq!(
        one.inputs.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        vec!["p-mid"]
    );

    let two = recall_from(&issues, "p-last", 2, false).unwrap();
    assert_eq!(
        two.inputs.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        vec!["p-first", "p-mid"],
        "furthest first: the order the work happened"
    );
    assert_eq!(two.inputs[0].relation, "blocked-by:2");
}

/// An id that is not in the corpus is an error rather than an empty working
/// set, because an agent handed nothing would start work with no context and no
/// reason to think anything was missing.
#[test]
fn recall_of_an_unknown_id_is_an_error() {
    let issues = vec![issue("p", "p-a", "TODO", "a")];
    let err = recall_from(&issues, "p-nope", 1, false).unwrap_err();
    assert!(matches!(err, Error::IssueNotFound { .. }), "{err:?}");
}

/// An input that closed without naming a product still has to hand the next
/// unit something. The last thing said about it is what a reader falls back on.
#[test]
fn an_input_carries_the_last_thing_said_about_it() {
    let mut blocker = issue("p", "p-first", "DONE", "the groundwork");
    blocker.heading.logbook = vec![
        vissue_core::model::LogEntry {
            timestamp: "[2026-09-06 Sun]".into(),
            from_state: None,
            to_state: None,
            note: Some("landed without the fast path".into()),
            raw: None,
        },
        vissue_core::model::LogEntry {
            timestamp: "[2026-09-05 Sat]".into(),
            from_state: None,
            to_state: None,
            note: Some("started".into()),
            raw: None,
        },
    ];
    let issues = vec![
        blocker,
        with_property(
            issue("p", "p-next", "TODO", "the next step"),
            "BLOCKED_BY",
            "p-first",
        ),
    ];

    let set = recall_from(&issues, "p-next", 1, false).unwrap();
    assert_eq!(
        set.inputs[0].last_note.as_deref(),
        Some("landed without the fast path"),
        "the logbook is newest first, so the first note is the last word"
    );
}

/// Releasing a claim files a note of its own, and on a closed issue it is
/// almost always the newest one. Handing that back as the last word would mean
/// nearly every input reported the same sentence about a claim.
#[test]
fn the_last_word_is_not_the_tracker_talking_to_itself() {
    let mut blocker = issue("p", "p-first", "DONE", "the groundwork");
    blocker.heading.logbook = vec![
        vissue_core::model::LogEntry {
            timestamp: "[2026-09-07 Mon]".into(),
            from_state: None,
            to_state: None,
            note: Some("claim released: impl held since [2026-09-06 Sun]".into()),
            raw: None,
        },
        vissue_core::model::LogEntry {
            timestamp: "[2026-09-06 Sun]".into(),
            from_state: None,
            to_state: None,
            note: Some("landed without the fast path".into()),
            raw: None,
        },
    ];
    let issues = vec![
        blocker,
        with_property(
            issue("p", "p-next", "TODO", "the next step"),
            "BLOCKED_BY",
            "p-first",
        ),
    ];

    let set = recall_from(&issues, "p-next", 1, false).unwrap();
    assert_eq!(
        set.inputs[0].last_note.as_deref(),
        Some("landed without the fast path")
    );
}

/// Two issues citing one deed worked on the same product. That is declared, so
/// it outranks a resemblance and the evidence names the deed.
#[test]
fn a_shared_deed_is_evidence_of_a_relation() {
    let issues = vec![
        with_property(
            issue("p", "p-a", "DONE", "write the exporter"),
            "DEEDS",
            "deed-patch-exporter",
        ),
        with_property(
            issue("p", "p-b", "TODO", "unrelated words entirely"),
            "DEEDS",
            "deed-patch-exporter",
        ),
        issue("p", "p-c", "TODO", "write the exporter again"),
    ];

    let hits = CatalogService::from_recs(&issues)
        .related("p-a", 2, 10)
        .unwrap();
    let first = hits.first().expect("a hit");
    assert_eq!(
        first.id, "p-b",
        "a shared deed outranks a shared word: {hits:?}"
    );
    assert!(
        first
            .evidence
            .iter()
            .any(|e| e == "deed:deed-patch-exporter"),
        "the evidence names the deed: {:?}",
        first.evidence
    );
}

/// A `:PARENT:` may name any Org heading with an `:ID:` under the prefix, so a
/// design document can head a work hierarchy. That document is not an issue and
/// is exactly what a reader should open, so the plan names it instead of
/// stopping silently one step short.
#[test]
fn a_plan_headed_by_a_document_is_named_not_dropped() {
    let issues = vec![with_property(
        issue("spec", "spec-9k2m", "TODO", "Implement the retry table"),
        "PARENT",
        "spec-design-20260615",
    )];

    let set = recall_from(&issues, "spec-9k2m", 1, false).unwrap();
    assert_eq!(
        set.plan.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["spec-design-20260615"]
    );
    assert!(
        set.plan[0].title.contains("outside the tracker"),
        "the reader has to know why it has no state: {:?}",
        set.plan[0]
    );
}

/// The export carries the citations typed as well as in the drawer. A consumer
/// reading the JSONL should not have to split a drawer string on whichever
/// separator the author used, when the socket hands the same field over typed.
#[test]
fn the_export_row_types_the_deed_citations() {
    let dir = tempfile::tempdir().unwrap();
    let layout = vissue_core::config::Layout::new(dir.path(), vissue_core::DEFAULT_PREFIX);
    std::fs::create_dir_all(layout.projects_dir()).unwrap();
    vissue_core::ops::create(
        &layout,
        "api",
        "made two things",
        vissue_core::CreateOpts::default(),
    )
    .unwrap();
    let id = vissue_core::store::load_all(&layout).unwrap()[0]
        .1
        .id
        .clone();
    for accession in ["deed-file-one", "deed-patch-two"] {
        vissue_core::ops::deed(&layout, &id, &[accession.to_string()], &[]).unwrap();
    }

    let line = vissue_core::report::export(&layout, None).unwrap();
    let row: serde_json::Value = serde_json::from_str(line.trim()).expect("one json object");
    assert_eq!(
        row["deeds"],
        serde_json::json!(["deed-file-one", "deed-patch-two"]),
        "typed, in the order they were cited"
    );
    assert_eq!(
        row["properties"]["DEEDS"], "deed-file-one deed-patch-two",
        "and the drawer is still there verbatim"
    );
}

/// Forward, an issue names the products it stands on. Backwards, from a
/// product to everything depending on it, is the question you have exactly
/// when the product turns out to be wrong.
#[test]
fn backlinks_answer_a_deed_accession_as_a_citation() {
    let mut recs = corpus();
    recs.push(with_property(
        issue("atlas", "atlas-c1t1", "DONE", "Built the overlay"),
        "DEEDS",
        "deed-patch-overlay",
    ));
    recs.push(with_property(
        issue("keys", "keys-c2t2", "TODO", "Used the overlay"),
        "DEEDS",
        "deed-patch-overlay, deed-file-notes",
    ));
    recs.push(with_body(
        issue("keys", "keys-m3t3", "TODO", "Talks about it"),
        "Waiting on whatever deed-patch-overlay turns out to say.",
    ));

    let hits = backlinks_from(&recs, "deed-patch-overlay").unwrap();
    let by_id: Vec<(&str, &str)> = hits
        .iter()
        .map(|h| (h.id.as_str(), h.relation.as_str()))
        .collect();
    assert!(by_id.contains(&("atlas-c1t1", "cites")), "{by_id:?}");
    assert!(by_id.contains(&("keys-c2t2", "cites")), "{by_id:?}");
    assert!(by_id.contains(&("keys-m3t3", "body mention")), "{by_id:?}");
    assert_eq!(by_id.len(), 3, "nothing else cites it: {by_id:?}");

    // A deed nobody cited is an empty answer, not an error: the product may be
    // real and simply unused. An unknown issue id stays an error.
    assert!(
        backlinks_from(&recs, "deed-quote-unused")
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        backlinks_from(&recs, "atlas-zzzz").unwrap_err(),
        Error::IssueNotFound { .. }
    ));
}

/// A project named `deed` mints ids that look exactly like accessions. The
/// corpus decides, so a real id keeps its own meaning.
#[test]
fn a_known_id_is_an_issue_whatever_it_looks_like() {
    let mut recs = corpus();
    recs.push(issue("deed", "deed-patch-overlay", "TODO", "Not a deed"));
    recs.push(with_property(
        issue("deed", "deed-waits-on-it", "TODO", "Waits on it"),
        "BLOCKED_BY",
        "deed-patch-overlay",
    ));
    recs.push(with_property(
        issue("atlas", "atlas-c4t4", "DONE", "Cites the accession"),
        "DEEDS",
        "deed-patch-overlay",
    ));

    let hits = backlinks_from(&recs, "deed-patch-overlay").unwrap();
    let by_id: Vec<(&str, &str)> = hits
        .iter()
        .map(|h| (h.id.as_str(), h.relation.as_str()))
        .collect();
    assert!(
        by_id.contains(&("deed-waits-on-it", "blocked-by")),
        "{by_id:?}"
    );
    assert!(
        !by_id.iter().any(|(_, rel)| *rel == "cites"),
        "the id namespace wins outright: {by_id:?}"
    );
}
