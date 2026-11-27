//! The op log folds to the same in-memory issues the org parser produces,
//! and fold order follows the op set rather than the order the ops were handed in.

#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use vissue_core::model::{IssueHeading, LogEntry};
use vissue_core::oplog::{IssueKey, Op, OpKind, describe, fold};
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

fn spawned(parent: &Op, kind: OpKind, wall: i64) -> Op {
    Op {
        parents: vec![parent.id().unwrap()],
        lamport: parent.lamport + 1,
        seat: parent.seat.clone(),
        project: parent.project.clone(),
        issue: parent.issue,
        kind,
        wall,
    }
}

fn framed(segment: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(8 + segment.len());
    msg.extend_from_slice(&0u32.to_le_bytes());
    msg.extend_from_slice(&u32::try_from(segment.len() / 8).unwrap().to_le_bytes());
    msg.extend_from_slice(segment);
    msg
}

fn heading(id: &str) -> IssueHeading {
    let mut properties = BTreeMap::new();
    properties.insert("ID".to_string(), id.to_string());
    properties.insert("AAA".to_string(), "1".to_string());
    properties.insert("ZZZ".to_string(), "2".to_string());
    IssueHeading {
        id: id.to_string(),
        title: "Imported".to_string(),
        state: "TODO".to_string(),
        priority: 'B',
        properties,
        org_tags: Vec::new(),
        statistics: None,
        property_order: vec!["ID".to_string()],
        extra_drawers: Vec::new(),
        body: String::new(),
        logbook: Vec::new(),
        line_start: 3,
        line_end: 9,
    }
}

fn drawers(folded: &vissue_core::oplog::Folded, id: &str) -> String {
    folded.issues[&("proj".to_string(), id.to_string())]
        .extra_drawers
        .concat()
}

#[test]
fn a_non_canonical_segment_is_refused() {
    let op = Op::create("proj", "proj-1", "Title", "seat").unwrap();
    let mut segment = op.canonical_segment().unwrap();
    segment.extend_from_slice(&0u64.to_le_bytes());
    segment.extend_from_slice(&0xdead_u64.to_le_bytes());
    assert!(Op::from_message_bytes(&framed(&segment)).is_err());
}

#[test]
fn a_non_derived_issue_key_is_refused() {
    let op = Op::create("proj", "proj-1", "Only", "seat").unwrap();
    let key = *IssueKey::derive("proj", "proj-1").as_bytes();
    let mut msg = op.message_bytes().unwrap();
    let pos = msg[8..]
        .windows(16)
        .position(|window| window == key)
        .unwrap();
    msg[8 + pos] ^= 0xff;
    assert!(Op::from_message_bytes(&msg).is_err());
}

#[test]
fn set_target_records_its_id_and_a_one_colon_target_field_is_refused() {
    let missing = Op {
        parents: Vec::new(),
        lamport: 1,
        seat: "seat".to_string(),
        project: "proj".to_string(),
        issue: IssueKey::derive("proj", "proj-missing"),
        kind: OpKind::SetTarget {
            target_id: "tgt-lost".to_string(),
            fields: Vec::new(),
        },
        wall: 0,
    };
    assert!(fold(&[missing]).is_err());

    let root = wire(&Op::create("proj", "proj-1", "Title", "seat").unwrap());
    let empty = wire(&spawned(
        &root,
        OpKind::SetTarget {
            target_id: "tgt-lost".to_string(),
            fields: Vec::new(),
        },
        1,
    ));
    let folded = fold(&[root.clone(), empty]).unwrap();
    let text = format!(
        "{:?}",
        folded.issues[&("proj".to_string(), "proj-1".to_string())]
    );
    assert!(text.contains("tgt-lost"), "{text}");

    let bad = spawned(
        &root,
        OpKind::SetField {
            name: "target:only".to_string(),
            value: "LOST".to_string(),
        },
        2,
    );
    assert!(Op::from_message_bytes(&bad.message_bytes().unwrap()).is_err());
    assert!(fold(&[root, bad]).is_err());
}

#[test]
fn an_empty_property_order_keeps_its_op_id() {
    let mut imported = heading("proj-1");
    imported.property_order.clear();
    let op = describe("proj", &[imported]).unwrap().pop().unwrap();
    let bytes = op.message_bytes().unwrap();
    let decoded = Op::from_message_bytes(&bytes).unwrap();
    assert_eq!(decoded.id().unwrap(), op.id().unwrap());
    assert_eq!(
        decoded.canonical_segment().unwrap(),
        op.canonical_segment().unwrap()
    );
    match &decoded.kind {
        OpKind::GenesisImport { heading, .. } => assert!(heading.property_order.is_empty()),
        other => panic!("genesis import, got {other:?}"),
    }
    let child = Op::set_field(&[&decoded], "proj", "proj-1", "title", "Next", "seat").unwrap();
    let folded = fold(&[decoded, child]).unwrap();
    assert_eq!(
        folded.issues[&("proj".to_string(), "proj-1".to_string())].title,
        "Next"
    );
}

fn assert_clash(chain: &[Op], left: OpKind, right: OpKind, field: &str) {
    for (wall_left, wall_right) in [(1_i64, 2_i64), (2, 1)] {
        let parent = chain.last().unwrap();
        let left_op = wire(&spawned(parent, left.clone(), wall_left));
        let right_op = wire(&spawned(parent, right.clone(), wall_right));
        let mut first = None;
        for (left_first, right_first) in [(&left_op, &right_op), (&right_op, &left_op)] {
            let mut ops = chain.to_vec();
            ops.push(left_first.clone());
            ops.push(right_first.clone());
            let folded = fold(&ops).unwrap();
            let clash = folded
                .conflicts
                .iter()
                .find(|clash| clash.field == field)
                .unwrap_or_else(|| panic!("{field} missing in {:?}", folded.conflicts));
            assert!(clash.ops.contains(&left_op.id().unwrap()));
            assert!(clash.ops.contains(&right_op.id().unwrap()));
            match &first {
                None => first = Some(folded.issues.clone()),
                Some(issues) => assert_eq!(issues, &folded.issues),
            }
        }
    }
}

#[test]
fn concurrent_property_writes_clash_in_both_orders() {
    let root = wire(&Op::create("proj", "proj-1", "Title", "seat").unwrap());
    assert_clash(
        std::slice::from_ref(&root),
        OpKind::AddDeed {
            accession: "deed-a".to_string(),
        },
        OpKind::SetField {
            name: "property:DEEDS".to_string(),
            value: "deed-b".to_string(),
        },
        "property:DEEDS",
    );
    assert_clash(
        std::slice::from_ref(&root),
        OpKind::Link {
            kind: "blocked-by".to_string(),
            other: "proj-x".to_string(),
        },
        OpKind::SetField {
            name: "property:BLOCKED_BY".to_string(),
            value: "proj-y".to_string(),
        },
        "property:BLOCKED_BY",
    );
    let base = wire(&spawned(
        &root,
        OpKind::SetBody {
            body: "alpha\nbeta".to_string(),
        },
        1,
    ));
    assert_clash(
        &[root, base],
        OpKind::SetBody {
            body: String::new(),
        },
        OpKind::SetBody {
            body: "alpha\nbeta!".to_string(),
        },
        "body",
    );
}

#[test]
fn votes_merge_per_voter_and_a_recast_replaces_one_line() {
    let root = wire(&Op::create("proj", "proj-v", "Title", "seat").unwrap());
    let alpha = wire(&spawned(
        &root,
        OpKind::Ballot {
            voter: "agent-a".to_string(),
            rev: String::new(),
            choice: "ship".to_string(),
            stamp: "[2026-03-01 Sun]".to_string(),
        },
        1,
    ));
    let beta = wire(&spawned(
        &root,
        OpKind::Ballot {
            voter: "agent-b".to_string(),
            rev: String::new(),
            choice: "hold".to_string(),
            stamp: "[2026-03-02 Mon]".to_string(),
        },
        2,
    ));
    let both = fold(&[root.clone(), alpha.clone(), beta.clone()]).unwrap();
    let text = drawers(&both, "proj-v");
    assert!(text.contains("agent-a"), "{text}");
    assert!(text.contains("agent-b"), "{text}");

    let recast = wire(&spawned(
        &alpha,
        OpKind::Ballot {
            voter: "agent-a".to_string(),
            rev: String::new(),
            choice: "later".to_string(),
            stamp: "[2026-03-05 Thu]".to_string(),
        },
        1,
    ));
    let folded = fold(&[root, alpha, beta, recast]).unwrap();
    let text = drawers(&folded, "proj-v");
    assert!(text.contains("agent-a: later"), "{text}");
    assert!(text.contains("agent-b: hold"), "{text}");
    assert!(!text.contains("agent-a: ship"), "{text}");

    let mut imported = heading("proj-g");
    imported.extra_drawers = vec![
        ":VOTES:\n[2026-03-01 Sun] agent-a: ship\n[2026-03-02 Mon] agent-b: ship\n:END:\n"
            .to_string(),
    ];
    let genesis = wire(&describe("proj", &[imported]).unwrap().pop().unwrap());
    let third = wire(&spawned(
        &genesis,
        OpKind::Ballot {
            voter: "agent-b".to_string(),
            rev: String::new(),
            choice: "hold".to_string(),
            stamp: "[2026-03-04 Wed]".to_string(),
        },
        1,
    ));
    let folded = fold(&[genesis, third]).unwrap();
    let text = drawers(&folded, "proj-g");
    assert!(text.contains("agent-a: ship"), "{text}");
    assert!(text.contains("agent-b: hold"), "{text}");
    assert!(!text.contains("agent-b: ship"), "{text}");
}
