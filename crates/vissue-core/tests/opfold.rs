//! The op log folds to the same in-memory issues the org parser produces,
//! and fold order follows the op set rather than the order the ops were handed in.

#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use vissue_core::model::{IssueHeading, LogEntry};
use vissue_core::oplog::{Folded, IssueKey, Op, OpId, OpKind, describe, fold};
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

fn plain_heading(id: &str, title: &str) -> IssueHeading {
    let mut properties = BTreeMap::new();
    properties.insert("ID".to_string(), id.to_string());
    IssueHeading {
        id: id.to_string(),
        title: title.to_string(),
        state: "TODO".to_string(),
        priority: 'B',
        properties,
        org_tags: Vec::new(),
        statistics: None,
        property_order: vec!["ID".to_string()],
        extra_drawers: Vec::new(),
        body: String::new(),
        logbook: Vec::new(),
        line_start: 1,
        line_end: 4,
    }
}

fn folded_bytes(folded: &vissue_core::oplog::Folded) -> Vec<u8> {
    let mut buf = Vec::new();
    for ((project, id), heading) in &folded.issues {
        buf.extend(project.as_bytes());
        buf.push(0);
        buf.extend(id.as_bytes());
        buf.push(0);
        buf.extend(heading.render().as_bytes());
        buf.extend(format!("{:?}", heading.property_order).as_bytes());
        buf.extend(format!("{:?}", heading.extra_drawers).as_bytes());
    }
    for clash in &folded.conflicts {
        buf.extend(format!("{} {} {} ", clash.project, clash.id, clash.field).as_bytes());
        for op in &clash.ops {
            buf.extend(op.as_bytes());
        }
        buf.push(0);
    }
    buf
}

fn permute(ops: &mut [Op], start: usize, visit: &mut impl FnMut(&[Op])) {
    if start == ops.len() {
        visit(ops);
        return;
    }
    for index in start..ops.len() {
        ops.swap(start, index);
        permute(ops, start + 1, visit);
        ops.swap(start, index);
    }
}

fn parented(parent: &Op, issue: &Op, kind: OpKind, wall: i64) -> Op {
    Op {
        parents: vec![parent.id().unwrap()],
        lamport: parent.lamport + 1,
        seat: issue.seat.clone(),
        project: issue.project.clone(),
        issue: issue.issue,
        kind,
        wall,
    }
}

fn assert_folds_same(ops: &mut [Op], mut check: impl FnMut(&Folded)) {
    let mut expected: Option<Vec<u8>> = None;
    permute(ops, 0, &mut |order| {
        let folded = fold(order).expect("the concurrent set folds");
        check(&folded);
        let bytes = folded_bytes(&folded);
        match &expected {
            None => expected = Some(bytes),
            Some(first) => assert_eq!(first, &bytes),
        }
    });
}

fn clash_ids(folded: &Folded, field: &str) -> Vec<OpId> {
    folded
        .conflicts
        .iter()
        .find(|clash| clash.field == field)
        .unwrap_or_else(|| panic!("{field} missing in {:?}", folded.conflicts))
        .ops
        .clone()
}

fn earlier_writes_survive_a_later_birth() {
    let other = wire(&Op::create("proj", "proj-0", "Other", "seat").unwrap());
    let create = wire(&Op::create("proj", "proj-1", "Created", "seat").unwrap());
    let create_id = create.id().unwrap();
    let genesis_kind = describe("proj", &[plain_heading("proj-1", "Genesis")])
        .unwrap()
        .pop()
        .unwrap()
        .kind;
    let deed_kind = OpKind::AddDeed {
        accession: "keep-me".to_string(),
    };
    let owner_kind = OpKind::SetField {
        name: "property:OWNER".to_string(),
        value: "ada".to_string(),
    };
    let mut chosen = None;
    for wall in 0..64 {
        let genesis = parented(&other, &create, genesis_kind.clone(), wall);
        let genesis_id = genesis.id().unwrap();
        let Some(deed) = (0..64)
            .map(|w| parented(&other, &create, deed_kind.clone(), w))
            .find(|op| {
                let id = op.id().unwrap();
                id < create_id && id < genesis_id
            })
        else {
            continue;
        };
        let deed_id = deed.id().unwrap();
        let Some(owner) = (0..64)
            .map(|w| parented(&other, &create, owner_kind.clone(), w))
            .find(|op| {
                let id = op.id().unwrap();
                id < create_id && id < genesis_id && id != deed_id
            })
        else {
            continue;
        };
        chosen = Some((wire(&genesis), wire(&deed), wire(&owner)));
        break;
    }
    let (genesis, deed, owner) = chosen.expect("a deed and a property sort before the create");
    let genesis_id = genesis.id().unwrap();
    let deed_id = deed.id().unwrap();
    let owner_id = owner.id().unwrap();
    assert!(deed_id < create_id && owner_id < create_id);
    assert!(deed_id < genesis_id && owner_id < genesis_id);
    let mut ops = vec![other, create, genesis, deed, owner];
    assert_folds_same(&mut ops, |folded| {
        let heading = &folded.issues[&("proj".to_string(), "proj-1".to_string())];
        assert_eq!(
            heading.properties.get("DEEDS").map(String::as_str),
            Some("keep-me")
        );
        assert_eq!(
            heading.properties.get("OWNER").map(String::as_str),
            Some("ada")
        );
        let births = clash_ids(folded, "issue");
        assert!(births.contains(&create_id));
        assert!(births.contains(&genesis_id));
        assert_eq!(births.len(), 2);
        for clash in &folded.conflicts {
            assert!(clash.ops.len() >= 2, "{:?}", folded.conflicts);
        }
    });
}

fn scalar_on_a_set_keeps_the_union() {
    let walls = [
        [1_i64, 2, 3],
        [1, 3, 2],
        [2, 1, 3],
        [2, 3, 1],
        [3, 1, 2],
        [3, 2, 1],
    ];
    for [left, mid, right] in walls {
        let root = wire(&Op::create("proj", "proj-1", "Title", "seat").unwrap());
        let deed_a = wire(&spawned(
            &root,
            OpKind::AddDeed {
                accession: "a".to_string(),
            },
            left,
        ));
        let deed_b = wire(&spawned(
            &root,
            OpKind::AddDeed {
                accession: "b".to_string(),
            },
            mid,
        ));
        let set = wire(&spawned(
            &root,
            OpKind::SetField {
                name: "property:DEEDS".to_string(),
                value: "S".to_string(),
            },
            right,
        ));
        let ids = [
            deed_a.id().unwrap(),
            deed_b.id().unwrap(),
            set.id().unwrap(),
        ];
        let mut ops = vec![root, deed_a, deed_b, set];
        assert_folds_same(&mut ops, |folded| {
            let heading = &folded.issues[&("proj".to_string(), "proj-1".to_string())];
            assert_eq!(
                heading.properties.get("DEEDS").map(String::as_str),
                Some("S a b")
            );
            let clash = clash_ids(folded, "property:DEEDS");
            assert_eq!(clash.len(), 3, "{clash:?}");
            for id in ids {
                assert!(clash.contains(&id), "{clash:?}");
            }
        });

        let root = wire(&Op::create("proj", "proj-1", "Title", "seat").unwrap());
        let wide = wire(&spawned(
            &root,
            OpKind::Link {
                kind: "blocked-by".to_string(),
                other: "p q".to_string(),
            },
            left,
        ));
        let narrow = wire(&spawned(
            &root,
            OpKind::Link {
                kind: "blocked-by".to_string(),
                other: "r".to_string(),
            },
            mid,
        ));
        let set = wire(&spawned(
            &root,
            OpKind::SetField {
                name: "property:BLOCKED_BY".to_string(),
                value: "t".to_string(),
            },
            right,
        ));
        let ids = [wide.id().unwrap(), narrow.id().unwrap(), set.id().unwrap()];
        let mut ops = vec![root, wide, narrow, set];
        assert_folds_same(&mut ops, |folded| {
            let heading = &folded.issues[&("proj".to_string(), "proj-1".to_string())];
            assert_eq!(
                heading.properties.get("BLOCKED_BY").map(String::as_str),
                Some("p q r t")
            );
            let clash = clash_ids(folded, "property:BLOCKED_BY");
            assert_eq!(clash.len(), 3, "{clash:?}");
            for id in ids {
                assert!(clash.contains(&id), "{clash:?}");
            }
        });
    }
}

#[test]
fn concurrent_ops_fold_to_the_same_bytes_in_every_order() {
    earlier_writes_survive_a_later_birth();
    scalar_on_a_set_keeps_the_union();
    let create = wire(&Op::create("proj", "proj-1", "Created", "seat").unwrap());
    let genesis = wire(
        &describe("proj", &[plain_heading("proj-1", "Genesis")])
            .unwrap()
            .pop()
            .unwrap(),
    );
    let owner = wire(&spawned(
        &create,
        OpKind::SetField {
            name: "property:OWNER".to_string(),
            value: "ada".to_string(),
        },
        1,
    ));
    let area = wire(&spawned(
        &create,
        OpKind::SetField {
            name: "property:AREA".to_string(),
            value: "9".to_string(),
        },
        2,
    ));
    let spaced = wire(&spawned(
        &create,
        OpKind::AddDeed {
            accession: "a  b".to_string(),
        },
        3,
    ));
    let other_deed = wire(&spawned(
        &create,
        OpKind::AddDeed {
            accession: "c".to_string(),
        },
        4,
    ));
    let ballot = wire(&spawned(
        &create,
        OpKind::Ballot {
            voter: "ann".to_string(),
            rev: String::new(),
            choice: "yes".to_string(),
            stamp: "[2026-04-01 Wed]".to_string(),
        },
        5,
    ));
    let wide = wire(&spawned(
        &create,
        OpKind::AddRevision {
            rev_id: "1".to_string(),
            summary: "2 3".to_string(),
        },
        6,
    ));
    let nested = wire(&spawned(
        &create,
        OpKind::AddRevision {
            rev_id: "1 2".to_string(),
            summary: "x".to_string(),
        },
        7,
    ));
    let create_id = create.id().unwrap();
    let genesis_id = genesis.id().unwrap();
    let later_title = if (create.lamport, create_id) > (genesis.lamport, genesis_id) {
        "Created"
    } else {
        "Genesis"
    };
    let mut ops = vec![
        create, genesis, owner, area, spaced, other_deed, ballot, wide, nested,
    ];
    let mut seen = 0usize;
    let mut expected: Option<Vec<u8>> = None;
    permute(&mut ops, 0, &mut |order| {
        let folded = fold(order).expect("the concurrent set folds");
        let heading = &folded.issues[&("proj".to_string(), "proj-1".to_string())];
        assert_eq!(heading.title, later_title);
        assert_eq!(
            heading.property_order,
            ["ID", "AREA", "DEEDS", "OWNER"]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            heading.properties.get("DEEDS").map(String::as_str),
            Some("a  b c")
        );
        assert_eq!(
            heading.properties.get("OWNER").map(String::as_str),
            Some("ada")
        );
        assert_eq!(
            heading.properties.get("AREA").map(String::as_str),
            Some("9")
        );
        let drawers = heading.extra_drawers.concat();
        let revisions = drawers.find(":REVISIONS:").expect("revisions drawer");
        let votes = drawers.find(":VOTES:").expect("votes drawer");
        assert!(revisions < votes, "{drawers}");
        assert!(drawers.contains("1 2 3"), "{drawers}");
        assert!(drawers.contains("1 2 x"), "{drawers}");
        assert!(drawers.contains("[2026-04-01 Wed] ann: yes"), "{drawers}");
        let mut sorted = heading.extra_drawers.clone();
        sorted.sort();
        assert_eq!(heading.extra_drawers, sorted);
        assert_eq!(folded.conflicts.len(), 1, "{:?}", folded.conflicts);
        let clash = &folded.conflicts[0];
        assert!(clash.ops.contains(&create_id));
        assert!(clash.ops.contains(&genesis_id));
        assert_eq!(clash.ops.len(), 2);
        let bytes = folded_bytes(&folded);
        match &expected {
            None => expected = Some(bytes),
            Some(first) => assert_eq!(first, &bytes),
        }
        seen += 1;
    });
    assert_eq!(seen, 362_880);
}
