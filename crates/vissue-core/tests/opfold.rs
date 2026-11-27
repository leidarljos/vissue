//! The op log folds to the same in-memory issues the org parser produces,
//! and fold order follows the op set rather than the order the ops were handed in.

#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use vissue_core::model::LogEntry;
use vissue_core::oplog::{Op, describe, fold};
use vissue_core::store::IssueDoc;

fn wire(op: &Op) -> Op {
    let bytes = op.message_bytes().unwrap();
    let back = Op::from_message_bytes(&bytes).unwrap();
    assert_eq!(back.id().unwrap(), op.id().unwrap());
    back
}

fn note(text: &str, stamp: &str) -> LogEntry {
    LogEntry {
        timestamp: stamp.to_string(),
        from_state: None,
        to_state: None,
        note: Some(text.to_string()),
        raw: None,
    }
}

#[test]
fn folding_the_ops_that_describe_a_tracker_matches_the_parsed_org() {
    let org = "\
#+TODO: TODO STARTED BLOCKED | DONE CANCELLED

* TODO [#A] First title [1/1] :bug:
:PROPERTIES:
:ID:         proj-a1
:CREATED:    [2026-01-02 Fri]
:TYPE:       bug
:END:
:LOGBOOK:
- State \"STARTED\" from \"TODO\" [2026-01-03 Sat 09:00]
- Note: \"opened\" [2026-01-02 Fri 08:00]
:END:
:NOTES:
keep this
:END:

Body one.

* DONE [#C] Second title
:PROPERTIES:
:ID:         proj-b2
:CREATED:    [2026-01-04 Sun]
:BLOCKED_BY: proj-a1
:END:

Body two.
";
    let doc = IssueDoc::parse("proj", PathBuf::from("issues.org"), org).unwrap();
    let described = describe("proj", &doc.headings).unwrap();
    let mut decoded = Vec::new();
    for op in &described {
        let bytes = op.message_bytes().unwrap();
        let back = Op::from_message_bytes(&bytes).unwrap();
        assert_eq!(back.id().unwrap(), op.id().unwrap());
        assert_eq!(back, *op);
        decoded.push(back);
    }
    let folded = fold(&decoded).unwrap();
    let mut parsed = BTreeMap::new();
    for heading in &doc.headings {
        parsed.insert(("proj".to_string(), heading.id.clone()), heading.clone());
    }
    assert_eq!(folded.issues, parsed);
    assert!(folded.conflicts.is_empty());
}

#[test]
fn commuting_ops_fold_either_way_and_a_clash_is_reported() {
    let root = wire(&Op::create("proj", "proj-1", "Title", "seat").unwrap());
    let newer = Op::append_log(
        &[&root],
        "proj",
        "proj-1",
        note("later", "[2026-02-02 Mon 00:00]"),
        "seat",
    )
    .unwrap();
    let older = Op::append_log(
        &[&root],
        "proj",
        "proj-1",
        note("earlier", "[2026-02-01 Sun 00:00]"),
        "seat",
    )
    .unwrap();
    let newer = wire(&newer);
    let older = wire(&older);
    let forward = fold(&[root.clone(), newer.clone(), older.clone()]).unwrap();
    let reverse = fold(&[root.clone(), older, newer]).unwrap();
    assert!(forward.conflicts.is_empty());
    assert_eq!(forward.issues, reverse.issues);
    let key = ("proj".to_string(), "proj-1".to_string());
    let log = &forward.issues[&key].logbook;
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].note.as_deref(), Some("later"));
    assert_eq!(log[1].note.as_deref(), Some("earlier"));

    let left = wire(&Op::set_field(&[&root], "proj", "proj-1", "title", "Left", "seat").unwrap());
    let right = wire(&Op::set_field(&[&root], "proj", "proj-1", "title", "Right", "seat").unwrap());
    let left_first = fold(&[root.clone(), left.clone(), right.clone()]).unwrap();
    let right_first = fold(&[root, right.clone(), left.clone()]).unwrap();
    assert_eq!(left_first.issues, right_first.issues);
    assert_eq!(left_first.conflicts.len(), 1);
    let clash = &left_first.conflicts[0];
    assert_eq!(clash.field, "title");
    assert_eq!(clash.id, "proj-1");
    assert!(clash.ops.contains(&left.id().unwrap()));
    assert!(clash.ops.contains(&right.id().unwrap()));
    let title = &left_first.issues[&key].title;
    assert!(title == "Left" || title == "Right");
    let winner = if left.id().unwrap() > right.id().unwrap() {
        "Left"
    } else {
        "Right"
    };
    assert_eq!(title, winner);
}
