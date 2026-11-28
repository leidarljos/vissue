//! The op log folds to the same in-memory issues the org parser produces,
//! and fold order follows the op set rather than the order the ops were handed in.

#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

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
fn a_zero_size_composite_claiming_too_many_elements_is_refused() {
    let op = Op {
        parents: Vec::new(),
        lamport: 1,
        seat: "seat".to_string(),
        project: "proj".to_string(),
        issue: IssueKey::derive("proj", "proj-1"),
        kind: OpKind::SetTarget {
            target_id: "t".to_string(),
            fields: vec![("a".to_string(), "b".to_string())],
        },
        wall: 0,
    };
    let segment = op.canonical_segment().unwrap();
    assert_eq!(segment.len(), 208);
    let mut words: Vec<u64> = segment
        .chunks_exact(8)
        .map(|chunk| u64::from_le_bytes(chunk.try_into().unwrap()))
        .collect();
    let tag_at = words
        .iter()
        .position(|word| *word == (1u64 << 2) | (2u64 << 48))
        .unwrap();
    let ptr_at = words
        .iter()
        .enumerate()
        .find_map(|(idx, word)| {
            if (word & 3) != 1 || ((word >> 32) & 7) != 7 {
                return None;
            }
            let raw = ((word >> 2) & 0x3fff_ffff) as u32;
            let offset = if (raw & (1 << 29)) != 0 {
                (raw | 0xc000_0000) as i32
            } else {
                raw as i32
            };
            let at = idx as i64 + 1 + i64::from(offset);
            (usize::try_from(at).ok() == Some(tag_at)).then_some(idx)
        })
        .unwrap();
    assert_eq!(words[ptr_at] >> 35, 2);
    let offset_bits = (words[ptr_at] >> 2) & 0x3fff_ffff;
    words[ptr_at] = 1 | (offset_bits << 2) | (7 << 32);
    words[tag_at] = 1u64 << 30;
    let mut crafted = Vec::new();
    for word in &words {
        crafted.extend_from_slice(&word.to_le_bytes());
    }
    assert_eq!(crafted.len(), 208);
    assert!(Op::from_message_bytes(&framed(&crafted)).is_err());
}

fn pointer_target(idx: usize, word: u64) -> Option<usize> {
    if word & 3 > 1 {
        return None;
    }
    let raw = ((word >> 2) & 0x3fff_ffff) as u32;
    let offset = if raw & (1 << 29) != 0 {
        (raw | 0xc000_0000) as i32
    } else {
        raw as i32
    };
    usize::try_from(idx as i64 + 1 + i64::from(offset)).ok()
}

fn retarget(from: usize, to: usize, template: u64) -> u64 {
    let offset = to as i64 - from as i64 - 1;
    assert!(
        (-(1 << 29)..(1 << 29)).contains(&offset),
        "offset {offset} from {from} to {to}"
    );
    let raw = (offset as u32 as u64) & 0x3fff_ffff;
    (template & !(0x3fff_ffffu64 << 2)) | (raw << 2)
}

/// One set-target whose field texts all name a single blob.
///
/// `elements == 1` is the canonical segment. A larger count inserts pointer
/// words and aims every text at that blob, which is not canonical.
fn aliased_set_target(elements: usize, blob_len: usize) -> Vec<u8> {
    assert!(elements >= 1);
    let op = Op {
        parents: Vec::new(),
        lamport: 1,
        seat: "seat".to_string(),
        project: "proj".to_string(),
        issue: IssueKey::derive("proj", "proj-1"),
        kind: OpKind::SetTarget {
            target_id: "t".to_string(),
            fields: vec![("k".to_string(), "v".repeat(blob_len))],
        },
        wall: 0,
    };
    let segment = op.canonical_segment().unwrap();
    let mut words: Vec<u64> = segment
        .chunks_exact(8)
        .map(|chunk| u64::from_le_bytes(chunk.try_into().unwrap()))
        .collect();
    let tag_at = words
        .iter()
        .position(|word| *word == (1u64 << 2) | (2u64 << 48))
        .unwrap();
    let name_word = words[tag_at + 1];
    let value_word = words[tag_at + 2];
    assert_ne!(name_word, 0);
    assert_ne!(value_word, 0);
    let name_at = pointer_target(tag_at + 1, name_word).unwrap();
    let value_at = pointer_target(tag_at + 2, value_word).unwrap();
    let name_bytes = (name_word >> 35) as usize;
    assert_eq!(name_at, tag_at + 3);
    assert_eq!(value_at, name_at + name_bytes.div_ceil(8));
    let extra = (elements - 1) * 2;
    let name_to = name_at + extra;
    let value_to = value_at + extra;
    words.splice(tag_at + 3..tag_at + 3, std::iter::repeat_n(0u64, extra));
    let ptr_at = words
        .iter()
        .enumerate()
        .find_map(|(idx, word)| {
            if (word & 3) != 1 || ((word >> 32) & 7) != 7 {
                return None;
            }
            (pointer_target(idx, *word) == Some(tag_at)).then_some(idx)
        })
        .unwrap();
    let body = (elements as u64) * 2;
    words[ptr_at] = (words[ptr_at] & ((1u64 << 35) - 1)) | (body << 35);
    words[tag_at] = ((elements as u64) << 2) | (2u64 << 48);
    let shared = if elements == 1 { name_to } else { value_to };
    for index in 0..elements {
        let base = tag_at + 1 + index * 2;
        words[base] = retarget(
            base,
            shared,
            if elements == 1 { name_word } else { value_word },
        );
        words[base + 1] = retarget(base + 1, value_to, value_word);
    }
    let mut crafted = Vec::with_capacity(words.len() * 8);
    for word in &words {
        crafted.extend_from_slice(&word.to_le_bytes());
    }
    framed(&crafted)
}

#[test]
fn aliased_byte_pointers_are_refused_within_the_segment_budget() {
    let name = "aliased_byte_pointers_are_refused_within_the_segment_budget";
    if std::env::var_os("VISSUE_OP_ALIAS_CHILD").is_some() {
        let bytes = aliased_set_target(12_000, 50_000);
        assert!(bytes.len() < 1024 * 1024, "{}", bytes.len());
        let err = Op::from_message_bytes(&bytes).unwrap_err();
        assert!(
            err.to_string().contains("op segment is not canonical"),
            "{err}"
        );
        return;
    }

    let op = Op {
        parents: Vec::new(),
        lamport: 1,
        seat: "seat".to_string(),
        project: "proj".to_string(),
        issue: IssueKey::derive("proj", "proj-1"),
        kind: OpKind::SetTarget {
            target_id: "t".to_string(),
            fields: vec![("k".to_string(), "v".repeat(8))],
        },
        wall: 0,
    };
    assert_eq!(aliased_set_target(1, 8), op.message_bytes().unwrap());

    let err = Op::from_message_bytes(&aliased_set_target(4, 32)).unwrap_err();
    assert!(
        err.to_string().contains("op segment is not canonical"),
        "{err}"
    );

    let body = "b".repeat(200_000);
    let wide_body = Op {
        parents: Vec::new(),
        lamport: 1,
        seat: "seat".to_string(),
        project: "proj".to_string(),
        issue: IssueKey::derive("proj", "proj-1"),
        kind: OpKind::SetBody { body },
        wall: 0,
    };
    let decoded = Op::from_message_bytes(&wide_body.message_bytes().unwrap()).unwrap();
    assert_eq!(decoded.id().unwrap(), wide_body.id().unwrap());
    match decoded.kind {
        OpKind::SetBody { body: got } => assert_eq!(got.len(), 200_000),
        other => panic!("set-body, got {other:?}"),
    }

    let fields: Vec<(String, String)> = (0..4_000)
        .map(|index| (format!("f{index:04}"), "v".to_string()))
        .collect();
    let wide_target = Op {
        parents: Vec::new(),
        lamport: 1,
        seat: "seat".to_string(),
        project: "proj".to_string(),
        issue: IssueKey::derive("proj", "proj-1"),
        kind: OpKind::SetTarget {
            target_id: "t".to_string(),
            fields,
        },
        wall: 0,
    };
    let segment = wide_target.canonical_segment().unwrap();
    assert!(segment.len() >= 128_000, "{}", segment.len());
    let decoded = Op::from_message_bytes(&wide_target.message_bytes().unwrap()).unwrap();
    assert_eq!(decoded.id().unwrap(), wide_target.id().unwrap());

    // 768 MiB. The crafted op asks for about a gigabyte of copies; this cap
    // is what makes that decode abort. A follow budget of eight times the
    // segment returns before the copy and fits under the cap.
    let exe = std::env::current_exe().unwrap();
    let out = Command::new("bash")
        .arg("-c")
        .arg("ulimit -v 786432 && exec \"$1\" --exact \"$2\"")
        .arg("bash")
        .arg(&exe)
        .arg(name)
        .env("VISSUE_OP_ALIAS_CHILD", "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "status {:?}\n{stdout}\n{stderr}",
        out.status
    );
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

fn child(parents: &[&Op], issue: &Op, kind: OpKind, wall: i64) -> Op {
    let lamport = parents
        .iter()
        .map(|parent| parent.lamport)
        .max()
        .unwrap_or(0)
        + 1;
    wire(&Op {
        parents: parents.iter().map(|parent| parent.id().unwrap()).collect(),
        lamport,
        seat: issue.seat.clone(),
        project: issue.project.clone(),
        issue: issue.issue,
        kind,
        wall,
    })
}

fn wall_at(mut make: impl FnMut(i64) -> Op, mut keep: impl FnMut(&Op) -> bool) -> Op {
    (0..64)
        .map(&mut make)
        .find(|op| keep(op))
        .expect("a wall with that op id order")
}

fn deeds(folded: &Folded) -> Option<String> {
    folded.issues[&("proj".to_string(), "proj-1".to_string())]
        .properties
        .get("DEEDS")
        .cloned()
}

fn blocked(folded: &Folded) -> Option<String> {
    folded.issues[&("proj".to_string(), "proj-1".to_string())]
        .properties
        .get("BLOCKED_BY")
        .cloned()
}

/// A set write removes the adds it observed. A concurrent add keeps its own
/// element. Op id order and input order fold to the same bytes.
fn a_removal_drops_elements_it_saw() {
    let root = wire(&Op::create("proj", "proj-1", "Title", "seat").unwrap());
    let old = child(
        &[&root],
        &root,
        OpKind::AddDeed {
            accession: "old".to_string(),
        },
        0,
    );
    let old_id = old.id().unwrap();
    let stay_after = wall_at(
        |wall| {
            child(
                &[&root],
                &root,
                OpKind::AddDeed {
                    accession: "stay".to_string(),
                },
                wall,
            )
        },
        |op| op.id().unwrap() > old_id,
    );
    let stay_before = wall_at(
        |wall| {
            child(
                &[&root],
                &root,
                OpKind::AddDeed {
                    accession: "stay".to_string(),
                },
                wall,
            )
        },
        |op| op.id().unwrap() < old_id,
    );
    for stay in [stay_after, stay_before] {
        let set = child(
            &[&old],
            &root,
            OpKind::SetField {
                name: "property:DEEDS".to_string(),
                value: "S".to_string(),
            },
            3,
        );
        let set_id = set.id().unwrap();
        let stay_id = stay.id().unwrap();
        let mut ops = vec![root.clone(), old.clone(), stay.clone(), set.clone()];
        assert_folds_same(&mut ops, |folded| {
            assert_eq!(
                deeds(folded).as_deref(),
                Some("S stay"),
                "{:?}",
                folded.conflicts
            );
            let clash = clash_ids(folded, "property:DEEDS");
            assert!(clash.contains(&stay_id), "{clash:?}");
            assert!(clash.contains(&set_id), "{clash:?}");
            assert!(!clash.contains(&old_id), "{clash:?}");
            assert_eq!(clash.len(), 2, "{clash:?}");
        });
        let later = child(
            &[&set],
            &root,
            OpKind::AddDeed {
                accession: "d".to_string(),
            },
            4,
        );
        let mut ops = vec![root.clone(), old.clone(), stay, set, later];
        assert_folds_same(&mut ops, |folded| {
            assert_eq!(deeds(folded).as_deref(), Some("S d stay"));
            assert!(
                folded
                    .conflicts
                    .iter()
                    .all(|clash| clash.field != "property:DEEDS"),
                "{:?}",
                folded.conflicts
            );
        });
    }

    let link_old = child(
        &[&root],
        &root,
        OpKind::Link {
            kind: "blocked-by".to_string(),
            other: "old".to_string(),
        },
        0,
    );
    let link_old_id = link_old.id().unwrap();
    let link_stay = wall_at(
        |wall| {
            child(
                &[&root],
                &root,
                OpKind::Link {
                    kind: "blocked-by".to_string(),
                    other: "stay".to_string(),
                },
                wall,
            )
        },
        |op| op.id().unwrap() > link_old_id,
    );
    let link_set = child(
        &[&link_old],
        &root,
        OpKind::SetField {
            name: "property:BLOCKED_BY".to_string(),
            value: "S".to_string(),
        },
        3,
    );
    let link_set_id = link_set.id().unwrap();
    let link_stay_id = link_stay.id().unwrap();
    let mut ops = vec![root.clone(), link_old, link_stay, link_set];
    assert_folds_same(&mut ops, |folded| {
        assert_eq!(blocked(folded).as_deref(), Some("S stay"));
        let clash = clash_ids(folded, "property:BLOCKED_BY");
        assert!(clash.contains(&link_stay_id) && clash.contains(&link_set_id));
        assert!(!clash.contains(&link_old_id));
        assert_eq!(clash.len(), 2);
    });

    let stay = wall_at(
        |wall| {
            child(
                &[&root],
                &root,
                OpKind::AddDeed {
                    accession: "stay".to_string(),
                },
                wall,
            )
        },
        |op| op.id().unwrap() > old_id,
    );
    let resolved = child(
        &[&old],
        &root,
        OpKind::Resolve {
            field: "property:DEEDS".to_string(),
            value: "S".to_string(),
        },
        5,
    );
    let resolved_id = resolved.id().unwrap();
    let stay_id = stay.id().unwrap();
    let mut ops = vec![root.clone(), old.clone(), stay, resolved];
    assert_folds_same(&mut ops, |folded| {
        assert_eq!(deeds(folded).as_deref(), Some("S stay"));
        let clash = clash_ids(folded, "property:DEEDS");
        assert!(clash.contains(&stay_id) && clash.contains(&resolved_id));
        assert!(!clash.contains(&old_id));
        assert_eq!(clash.len(), 2);
    });

    let set = child(
        &[&old],
        &root,
        OpKind::SetField {
            name: "property:DEEDS".to_string(),
            value: "S".to_string(),
        },
        3,
    );
    let set_id = set.id().unwrap();
    for stay in [
        wall_at(
            |wall| {
                child(
                    &[&old],
                    &root,
                    OpKind::AddDeed {
                        accession: "stay".to_string(),
                    },
                    wall,
                )
            },
            |op| op.id().unwrap() > set_id,
        ),
        wall_at(
            |wall| {
                child(
                    &[&old],
                    &root,
                    OpKind::AddDeed {
                        accession: "stay".to_string(),
                    },
                    wall,
                )
            },
            |op| op.id().unwrap() < set_id,
        ),
    ] {
        let stay_id = stay.id().unwrap();
        let mut ops = vec![root.clone(), old.clone(), stay, set.clone()];
        assert_folds_same(&mut ops, |folded| {
            assert_eq!(deeds(folded).as_deref(), Some("S stay"));
            let clash = clash_ids(folded, "property:DEEDS");
            assert!(clash.contains(&stay_id) && clash.contains(&set_id));
            assert!(!clash.contains(&old_id));
            assert_eq!(clash.len(), 2);
        });
    }

    let left = child(
        &[&root],
        &root,
        OpKind::AddDeed {
            accession: "a".to_string(),
        },
        1,
    );
    let right = child(
        &[&root],
        &root,
        OpKind::AddDeed {
            accession: "b".to_string(),
        },
        2,
    );
    let left_id = left.id().unwrap();
    let right_id = right.id().unwrap();
    let extra = wall_at(
        |wall| {
            child(
                &[&root],
                &root,
                OpKind::AddDeed {
                    accession: "c".to_string(),
                },
                wall,
            )
        },
        |op| {
            let id = op.id().unwrap();
            id > left_id && id > right_id
        },
    );
    let both = child(
        &[&left, &right],
        &root,
        OpKind::SetField {
            name: "property:DEEDS".to_string(),
            value: "S".to_string(),
        },
        3,
    );
    let added = child(
        &[&both],
        &root,
        OpKind::AddDeed {
            accession: "d".to_string(),
        },
        4,
    );
    let mut ops = vec![root, left, right, extra, both, added];
    assert_folds_same(&mut ops, |folded| {
        assert_eq!(deeds(folded).as_deref(), Some("S c d"));
        assert!(
            folded
                .conflicts
                .iter()
                .all(|clash| clash.field != "property:DEEDS"),
            "{:?}",
            folded.conflicts
        );
    });
}

/// A parented create and a concurrent deed share a parent. The deed is kept
/// when its op id sorts first and when the create sorts first.
fn a_write_before_the_first_birth_is_kept() {
    let anchor = wire(&Op::create("proj", "proj-0", "Other", "seat").unwrap());
    let template = Op::create("proj", "proj-1", "Created", "seat").unwrap();
    let birth_kind = template.kind.clone();
    let deed_kind = OpKind::AddDeed {
        accession: "keep-me".to_string(),
    };
    let mut deed_first = None;
    let mut birth_first = None;
    for birth_wall in 0..24 {
        for deed_wall in 0..24 {
            let birth = child(&[&anchor], &template, birth_kind.clone(), birth_wall);
            let deed = child(&[&anchor], &template, deed_kind.clone(), deed_wall);
            let birth_key = (birth.lamport, birth.id().unwrap());
            let deed_key = (deed.lamport, deed.id().unwrap());
            if deed_key < birth_key && deed_first.is_none() {
                deed_first = Some((birth, deed));
            } else if birth_key < deed_key && birth_first.is_none() {
                birth_first = Some((birth, deed));
            }
        }
    }
    let cases = [
        deed_first.expect("a deed that sorts before the birth"),
        birth_first.expect("a birth that sorts before the deed"),
    ];
    for (birth, deed) in cases {
        let mut ops = vec![anchor.clone(), birth, deed];
        assert_folds_same(&mut ops, |folded| {
            assert_eq!(deeds(folded).as_deref(), Some("keep-me"));
            let heading = &folded.issues[&("proj".to_string(), "proj-1".to_string())];
            assert_eq!(heading.title, "Created");
            assert!(
                folded
                    .issues
                    .contains_key(&("proj".to_string(), "proj-0".to_string()))
            );
        });
    }
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
                other: "pq".to_string(),
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
                Some("pq r t")
            );
            let clash = clash_ids(folded, "property:BLOCKED_BY");
            assert_eq!(clash.len(), 3, "{clash:?}");
            for id in ids {
                assert!(clash.contains(&id), "{clash:?}");
            }
        });
    }
}

fn headed(id: &str, title: &str, body: &str, owner: &str) -> IssueHeading {
    let mut heading = plain_heading(id, title);
    heading.body = body.to_string();
    heading
        .properties
        .insert("OWNER".to_string(), owner.to_string());
    heading.property_order.push("OWNER".to_string());
    heading
}

/// Two births of one short id both stay. The later birth by `(lamport, op id)`
/// is shown as `id~2`, and each heading keeps its own body and owner.
fn duplicate_births_keep_both_headings() {
    let first = headed("proj-dup", "First", "body-one", "ada");
    let second = headed("proj-dup", "Second", "body-two", "bo");
    let mut ops: Vec<Op> = describe("proj", &[first, second])
        .unwrap()
        .iter()
        .map(wire)
        .collect();
    let first_id = ops[0].id().unwrap();
    let second_id = ops[1].id().unwrap();
    let earlier_title = if (ops[0].lamport, first_id) < (ops[1].lamport, second_id) {
        "First"
    } else {
        "Second"
    };
    assert_folds_same(&mut ops, |folded| {
        assert_eq!(folded.issues.len(), 2, "{:?}", folded.issues.keys());
        let mut by_title = BTreeMap::new();
        for ((project, id), heading) in &folded.issues {
            assert_eq!(project, "proj");
            by_title.insert(
                heading.title.clone(),
                (
                    id.clone(),
                    heading.body.clone(),
                    heading.properties.get("OWNER").cloned(),
                ),
            );
        }
        assert_eq!(by_title["First"].1, "body-one");
        assert_eq!(by_title["First"].2.as_deref(), Some("ada"));
        assert_eq!(by_title["Second"].1, "body-two");
        assert_eq!(by_title["Second"].2.as_deref(), Some("bo"));
        assert_ne!(by_title["First"].0, by_title["Second"].0);
        assert_eq!(by_title[earlier_title].0, "proj-dup");
        assert!(
            by_title.values().any(|row| row.0 == "proj-dup~2"),
            "{by_title:?}"
        );
        let clash = clash_ids(folded, "issue");
        assert!(clash.contains(&first_id) && clash.contains(&second_id));
        assert_eq!(clash.len(), 2, "{clash:?}");
    });

    let left = wire(&Op::create("proj", "proj-same", "Left", "seat").unwrap());
    let right = wire(&Op::create("proj", "proj-same", "Right", "seat").unwrap());
    let left_id = left.id().unwrap();
    let right_id = right.id().unwrap();
    let earlier = if (left.lamport, left_id) < (right.lamport, right_id) {
        "Left"
    } else {
        "Right"
    };
    let mut ops = vec![left, right];
    assert_folds_same(&mut ops, |folded| {
        assert_eq!(folded.issues.len(), 2, "{:?}", folded.issues.keys());
        let mut titles = BTreeMap::new();
        for ((_, id), heading) in &folded.issues {
            titles.insert(heading.title.clone(), id.clone());
        }
        assert_eq!(titles[earlier], "proj-same");
        assert!(titles.values().any(|id| id == "proj-same~2"), "{titles:?}");
        let clash = clash_ids(folded, "issue");
        assert!(clash.contains(&left_id) && clash.contains(&right_id));
        assert_eq!(clash.len(), 2, "{clash:?}");
    });
}

/// A log line and a retire note that sort before a concurrent create are kept.
/// The same ops fold to the same bytes in every input order.
fn a_log_and_a_retire_note_wait_for_the_birth() {
    let anchor = wire(&Op::create("proj", "proj-0", "Other", "seat").unwrap());
    let template = Op::create("proj", "proj-1", "Created", "seat").unwrap();
    let birth_kind = template.kind.clone();
    let log_kind = OpKind::AppendLog {
        entry: note("keep-log", "[2026-05-01 Fri]"),
    };
    let retire_kind = OpKind::Retire {
        reason: "done".to_string(),
    };
    for kind in [log_kind, retire_kind] {
        let mut early = None;
        let mut late = None;
        for birth_wall in 0..24 {
            for other_wall in 0..24 {
                let birth = child(&[&anchor], &template, birth_kind.clone(), birth_wall);
                let other = child(&[&anchor], &template, kind.clone(), other_wall);
                let birth_key = (birth.lamport, birth.id().unwrap());
                let other_key = (other.lamport, other.id().unwrap());
                if other_key < birth_key && early.is_none() {
                    early = Some((birth, other));
                } else if birth_key < other_key && late.is_none() {
                    late = Some((birth, other));
                }
            }
        }
        let note_text = if matches!(kind, OpKind::Retire { .. }) {
            "done"
        } else {
            "keep-log"
        };
        for (birth, other) in [
            early.expect("the note sorts before the birth"),
            late.expect("the birth sorts before the note"),
        ] {
            let birth_id = birth.id().unwrap();
            let other_id = other.id().unwrap();
            let expect_state = if matches!(kind, OpKind::Retire { .. })
                && (other.lamport, other_id) > (birth.lamport, birth_id)
            {
                "CANCELLED"
            } else {
                "TODO"
            };
            let mut ops = vec![anchor.clone(), birth, other];
            assert_folds_same(&mut ops, |folded| {
                let heading = &folded.issues[&("proj".to_string(), "proj-1".to_string())];
                assert!(
                    heading
                        .logbook
                        .iter()
                        .any(|entry| entry.note.as_deref() == Some(note_text)),
                    "{:?} {}",
                    heading.logbook,
                    heading.state
                );
                assert_eq!(heading.state, expect_state);
                assert!(
                    folded
                        .issues
                        .contains_key(&("proj".to_string(), "proj-0".to_string()))
                );
            });
        }
    }
}

/// A child of an add that is concurrent with a removal must not copy the
/// removed element back. Deeds, blocked-by links, and resolve agree.
fn a_descendant_of_a_concurrent_add_stays_removed() {
    let root = wire(&Op::create("proj", "proj-1", "Title", "seat").unwrap());
    check_descendant(
        &root,
        OpKind::AddDeed {
            accession: "old".to_string(),
        },
        OpKind::AddDeed {
            accession: "mid".to_string(),
        },
        OpKind::SetField {
            name: "property:DEEDS".to_string(),
            value: "S".to_string(),
        },
        OpKind::AddDeed {
            accession: "d".to_string(),
        },
        deeds,
    );
    check_descendant(
        &root,
        OpKind::AddDeed {
            accession: "old".to_string(),
        },
        OpKind::AddDeed {
            accession: "mid".to_string(),
        },
        OpKind::Resolve {
            field: "property:DEEDS".to_string(),
            value: "S".to_string(),
        },
        OpKind::AddDeed {
            accession: "d".to_string(),
        },
        deeds,
    );
    check_descendant(
        &root,
        OpKind::Link {
            kind: "blocked-by".to_string(),
            other: "old".to_string(),
        },
        OpKind::Link {
            kind: "blocked-by".to_string(),
            other: "mid".to_string(),
        },
        OpKind::SetField {
            name: "property:BLOCKED_BY".to_string(),
            value: "S".to_string(),
        },
        OpKind::Link {
            kind: "blocked-by".to_string(),
            other: "d".to_string(),
        },
        blocked,
    );
    check_descendant(
        &root,
        OpKind::Link {
            kind: "blocked-by".to_string(),
            other: "old".to_string(),
        },
        OpKind::Link {
            kind: "blocked-by".to_string(),
            other: "mid".to_string(),
        },
        OpKind::Resolve {
            field: "property:BLOCKED_BY".to_string(),
            value: "S".to_string(),
        },
        OpKind::Link {
            kind: "blocked-by".to_string(),
            other: "d".to_string(),
        },
        blocked,
    );
}

fn check_descendant(
    root: &Op,
    old_kind: OpKind,
    mid_kind: OpKind,
    set_kind: OpKind,
    child_kind: OpKind,
    read: fn(&Folded) -> Option<String>,
) {
    let old = child(&[root], root, old_kind, 0);
    let mut mid_first = None;
    let mut set_first = None;
    for mid_wall in 0..24 {
        for set_wall in 0..24 {
            let mid = child(&[&old], root, mid_kind.clone(), mid_wall);
            let set = child(&[&old], root, set_kind.clone(), set_wall);
            let mid_id = mid.id().unwrap();
            let set_id = set.id().unwrap();
            if mid_id < set_id && mid_first.is_none() {
                mid_first = Some((mid, set));
            } else if set_id < mid_id && set_first.is_none() {
                set_first = Some((mid, set));
            }
        }
    }
    for (mid, set) in [
        mid_first.expect("the add sorts before the removal"),
        set_first.expect("the removal sorts before the add"),
    ] {
        let descendant = child(&[&mid], root, child_kind.clone(), 1);
        let mut ops = vec![root.clone(), old.clone(), mid, set, descendant];
        assert_folds_same(&mut ops, |folded| {
            assert_eq!(read(folded).as_deref(), Some("S d mid"), "{folded:?}");
        });
    }
}

/// A generated `~N` never takes a real short id. Two births of `proj-1`
/// and a real `proj-1~2` all stay, and every input order folds to the
/// same bytes. The later birth takes the smallest free suffix, `~3`.
fn a_generated_suffix_does_not_replace_a_real_id() {
    let alpha = wire(&Op::create("proj", "proj-1", "Alpha", "seat").unwrap());
    let beta = wire(&Op::create("proj", "proj-1", "Beta", "seat").unwrap());
    let real = wire(&Op::create("proj", "proj-1~2", "Real", "seat").unwrap());
    let alpha_key = (alpha.lamport, alpha.id().unwrap());
    let beta_key = (beta.lamport, beta.id().unwrap());
    let (earlier, later) = if alpha_key < beta_key {
        ("Alpha", "Beta")
    } else {
        ("Beta", "Alpha")
    };
    let mut ops = vec![alpha, beta, real];
    assert_folds_same(&mut ops, |folded| {
        assert_eq!(folded.issues.len(), 3, "{:?}", folded.issues.keys());
        let mut titles = BTreeMap::new();
        for ((project, id), heading) in &folded.issues {
            assert_eq!(project, "proj");
            assert_eq!(heading.id, *id);
            assert_eq!(
                heading.properties.get("ID").map(String::as_str),
                Some(id.as_str())
            );
            titles.insert(heading.title.clone(), id.clone());
        }
        assert_eq!(titles[earlier], "proj-1");
        assert_eq!(titles["Real"], "proj-1~2");
        assert_eq!(titles[later], "proj-1~3");
    });
}

#[test]
fn concurrent_ops_fold_to_the_same_bytes_in_every_order() {
    a_generated_suffix_does_not_replace_a_real_id();
    duplicate_births_keep_both_headings();
    a_log_and_a_retire_note_wait_for_the_birth();
    a_descendant_of_a_concurrent_add_stays_removed();
    a_removal_drops_elements_it_saw();
    a_write_before_the_first_birth_is_kept();
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
            accession: "ab".to_string(),
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
            summary: "wide".to_string(),
        },
        6,
    ));
    let nested = wire(&spawned(
        &create,
        OpKind::AddRevision {
            rev_id: "2".to_string(),
            summary: "x".to_string(),
        },
        7,
    ));
    let create_id = create.id().unwrap();
    let genesis_id = genesis.id().unwrap();
    let create_earlier = (create.lamport, create_id) < (genesis.lamport, genesis_id);
    let create_shown = if create_earlier { "proj-1" } else { "proj-1~2" };
    let genesis_shown = if create_earlier { "proj-1~2" } else { "proj-1" };
    let mut ops = vec![
        create, genesis, owner, area, spaced, other_deed, ballot, wide, nested,
    ];
    let mut seen = 0usize;
    let mut expected: Option<Vec<u8>> = None;
    permute(&mut ops, 0, &mut |order| {
        let folded = fold(order).expect("the concurrent set folds");
        assert_eq!(folded.issues.len(), 2, "{:?}", folded.issues.keys());
        let heading = &folded.issues[&("proj".to_string(), create_shown.to_string())];
        let generated = &folded.issues[&("proj".to_string(), genesis_shown.to_string())];
        assert_eq!(heading.title, "Created");
        assert_eq!(generated.title, "Genesis");
        assert_eq!(
            heading.property_order,
            ["ID", "AREA", "DEEDS", "OWNER"]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            heading.properties.get("DEEDS").map(String::as_str),
            Some("ab c")
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
        assert!(drawers.contains("1 wide"), "{drawers}");
        assert!(drawers.contains("2 x"), "{drawers}");
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

fn genesis_of(heading: IssueHeading) -> Op {
    wire(&describe("proj", &[heading]).unwrap().pop().unwrap())
}

fn both_ways(ops: &[Op], mut check: impl FnMut(&Folded)) {
    let forward = fold(ops).expect("fold");
    let mut reversed = ops.to_vec();
    reversed.reverse();
    let backward = fold(&reversed).expect("fold");
    assert_eq!(
        folded_bytes(&forward),
        folded_bytes(&backward),
        "input order changed the fold"
    );
    check(&forward);
}

fn heading_of(folded: &Folded) -> IssueHeading {
    folded.issues[&("proj".to_string(), "proj-1".to_string())].clone()
}

fn line_is(text: &str, line: &str) -> bool {
    text.lines().filter(|item| *item == line).count() == 1
}

/// Legacy drawer bytes stay, and a value is accepted only when the base
/// writer and reader return that same value on one line.
#[test]
fn genesis_lines_survive_the_next_op() {
    legacy_drawers_survive_the_next_op();
    let root = a_retarget_keeps_the_later_status();
    quoted_tokens_survive_in_the_drawers(&root);
    a_duplicated_genesis_line_gains_the_next_op();
    pre_birth_lines_fold_to_one_order();
    let (anchor, template, link_kind) = duplicate_cites_fold_the_same_either_way();
    a_clear_drops_only_the_lines_it_observed(&template, &link_kind);
    let post_birth = a_replaced_revision_matches_across_birth(&anchor, &template);
    target_fields_keep_the_order_the_op_named(anchor, &template, post_birth);
    let before = folded_bytes(&fold(std::slice::from_ref(&root)).unwrap());
    a_value_base_cannot_store_is_refused(&root);
    a_heading_round_trips_when_the_base_reader_accepts_it();
    two_adds_and_a_clear_share_one_drawer();
    a_pre_birth_descendant_keeps_every_line();
    a_drawer_type_without_a_headline_tag_folds();
    let after = folded_bytes(&fold(std::slice::from_ref(&root)).unwrap());
    assert_eq!(before, after);
}

fn legacy_drawers_survive_the_next_op() {
    let mut legacy = plain_heading("proj-1", "Genesis");
    legacy
        .properties
        .insert("DEEDS".to_string(), "a  b".to_string());
    legacy.property_order.push("DEEDS".to_string());
    let votes = "\
:VOTES:
# a vote note
[2026-01-01 Thu] ann: ship: after the audit
:END:
";
    let notes = ":NOTES:\nkeep-me\n:END:\n";
    let clash_drawer = ":MERGE_CONFLICT:\n%old\n:END:\n";
    let hidden = ":VOTES:\nzed: hidden\n:END:\n";
    let revisions = "\
:REVISIONS:
1 alpha
1 beta
# a note
1 \"alpha\"
:END:
";
    let targets = "\
:TARGETS:
t1 status open
t1 status shut
retarget zzz
:END:
";
    let links = ":LINKS:\n# a note\nrelates \"kept\"\n:END:\n";
    legacy.extra_drawers = vec![
        votes.to_string(),
        notes.to_string(),
        clash_drawer.to_string(),
        hidden.to_string(),
        revisions.to_string(),
        targets.to_string(),
        links.to_string(),
    ];
    let born = wire(&genesis_of(legacy));
    let revision = wire(&spawned(
        &born,
        OpKind::AddRevision {
            rev_id: "2".to_string(),
            summary: "gamma".to_string(),
        },
        1,
    ));
    let vote = wire(&spawned(
        &born,
        OpKind::Ballot {
            voter: "bob".to_string(),
            rev: String::new(),
            choice: "yes".to_string(),
            stamp: "[2026-01-02 Fri]".to_string(),
        },
        2,
    ));
    let votes_after = "\
:VOTES:
# a vote note
[2026-01-01 Thu] ann: ship: after the audit
[2026-01-02 Fri] bob: yes
:END:
";
    let revisions_after = "\
:REVISIONS:
1 alpha
1 beta
# a note
1 \"alpha\"
2 gamma
:END:
";
    let born_id = born.id().unwrap();
    both_ways(&[born, revision, vote], |folded| {
        let heading = heading_of(folded);
        assert_eq!(
            heading.properties.get("DEEDS").map(String::as_str),
            Some("a  b")
        );
        assert_eq!(
            heading.extra_drawers,
            vec![
                votes_after.to_string(),
                notes.to_string(),
                clash_drawer.to_string(),
                hidden.to_string(),
                revisions_after.to_string(),
                targets.to_string(),
                links.to_string(),
            ]
        );
        for field in ["revision:1", "target:t1:status"] {
            let clash = folded
                .conflicts
                .iter()
                .find(|clash| clash.field == field)
                .unwrap_or_else(|| panic!("{field} missing in {:?}", folded.conflicts));
            assert!(clash.ops.contains(&born_id), "{field} {:?}", clash.ops);
        }
    });
}

fn a_retarget_keeps_the_later_status() -> Op {
    let root = wire(&Op::create("proj", "proj-1", "Title", "seat").unwrap());
    let open = wire(&spawned(
        &root,
        OpKind::SetTarget {
            target_id: "retarget".to_string(),
            fields: vec![("status".to_string(), "open".to_string())],
        },
        1,
    ));
    let shut = wire(&spawned(
        &open,
        OpKind::SetTarget {
            target_id: "retarget".to_string(),
            fields: vec![("status".to_string(), "closed".to_string())],
        },
        2,
    ));
    let folded = fold(&[root.clone(), open, shut]).unwrap();
    let text = drawers(&folded, "proj-1");
    assert!(line_is(&text, "retarget status closed"), "{text}");
    assert!(!text.contains("retarget status open"), "{text}");
    assert!(folded.conflicts.is_empty(), "{:?}", folded.conflicts);
    root
}

fn quoted_tokens_survive_in_the_drawers(root: &Op) {
    let smith = wire(&spawned(
        root,
        OpKind::Ballot {
            voter: "ann smith".to_string(),
            rev: String::new(),
            choice: "yes".to_string(),
            stamp: "[2026-01-01 Thu]".to_string(),
        },
        1,
    ));
    let quoted = wire(&spawned(
        root,
        OpKind::Ballot {
            voter: "ann\"x".to_string(),
            rev: String::new(),
            choice: "yes".to_string(),
            stamp: "[2026-01-01 Thu]".to_string(),
        },
        2,
    ));
    let slash = wire(&spawned(
        root,
        OpKind::Ballot {
            voter: "a\\b".to_string(),
            rev: String::new(),
            choice: "yes".to_string(),
            stamp: "[2026-01-01 Thu]".to_string(),
        },
        3,
    ));
    let tabbed = wire(&spawned(
        root,
        OpKind::Ballot {
            voter: "a\tb".to_string(),
            rev: String::new(),
            choice: "a\tb".to_string(),
            stamp: "[2026-01-01 Thu]".to_string(),
        },
        4,
    ));
    let deed = wire(&spawned(
        root,
        OpKind::AddDeed {
            accession: "deed-a\"b".to_string(),
        },
        5,
    ));
    let link = wire(&spawned(
        root,
        OpKind::Link {
            kind: "blocked-by".to_string(),
            other: "proj-a\"b".to_string(),
        },
        6,
    ));
    let cites = wire(&spawned(
        root,
        OpKind::Link {
            kind: "cites".to_string(),
            other: "proj-a\"b".to_string(),
        },
        7,
    ));
    both_ways(
        &[
            (*root).clone(),
            smith,
            quoted,
            slash,
            tabbed,
            deed,
            link,
            cites,
        ],
        |folded| {
            let heading = heading_of(folded);
            assert_eq!(
                heading.properties.get("DEEDS").map(String::as_str),
                Some("deed-a\"b")
            );
            assert_eq!(
                heading.properties.get("BLOCKED_BY").map(String::as_str),
                Some("proj-a\"b")
            );
            let text = heading.extra_drawers.concat();
            for line in [
                "[2026-01-01 Thu] ann smith: yes",
                "[2026-01-01 Thu] ann\"x: yes",
                "[2026-01-01 Thu] a\\b: yes",
                "[2026-01-01 Thu] a\tb: a\tb",
                "cites proj-a\"b",
            ] {
                assert!(line_is(&text, line), "missing {line} in {text}");
            }
        },
    );
}

fn a_duplicated_genesis_line_gains_the_next_op() {
    let mut duplicated = plain_heading("proj-1", "Dup");
    let dup_votes = "\
:VOTES:
[2026-01-01 Thu] ann: yes
[2026-01-01 Thu] ann: no
:END:
";
    let dup_revisions = "\
:REVISIONS:
1 alpha
1 beta
:END:
";
    duplicated.extra_drawers = vec![dup_votes.to_string(), dup_revisions.to_string()];
    let dup_born = wire(&genesis_of(duplicated));
    let recast = wire(&spawned(
        &dup_born,
        OpKind::Ballot {
            voter: "ann".to_string(),
            rev: String::new(),
            choice: "later".to_string(),
            stamp: "[2026-01-02 Fri]".to_string(),
        },
        1,
    ));
    let nine = wire(&spawned(
        &dup_born,
        OpKind::AddRevision {
            rev_id: "1".to_string(),
            summary: "nine".to_string(),
        },
        2,
    ));
    let dup_born_id = dup_born.id().unwrap();
    let recast_id = recast.id().unwrap();
    let nine_id = nine.id().unwrap();
    both_ways(&[dup_born, recast, nine], |folded| {
        let heading = heading_of(folded);
        assert_eq!(
            heading
                .extra_drawers
                .iter()
                .find(|drawer| drawer.starts_with(":VOTES:"))
                .map(String::as_str),
            Some(
                "\
:VOTES:
[2026-01-01 Thu] ann: yes
[2026-01-01 Thu] ann: no
[2026-01-02 Fri] ann: later
:END:
"
            )
        );
        assert_eq!(
            heading
                .extra_drawers
                .iter()
                .find(|drawer| drawer.starts_with(":REVISIONS:"))
                .map(String::as_str),
            Some(
                "\
:REVISIONS:
1 alpha
1 beta
1 nine
:END:
"
            )
        );
        let revision = folded
            .conflicts
            .iter()
            .find(|clash| clash.field == "revision:1")
            .unwrap_or_else(|| panic!("{:?}", folded.conflicts));
        assert_eq!(
            revision.ops,
            vec![dup_born_id, nine_id],
            "revision:1 {:?}",
            revision.ops
        );
        let ballot = folded
            .conflicts
            .iter()
            .find(|clash| clash.field == "ballot:3:ann")
            .unwrap_or_else(|| panic!("{:?}", folded.conflicts));
        assert_eq!(
            ballot.ops,
            vec![dup_born_id, recast_id],
            "ballot {:?}",
            ballot.ops
        );
    });
}

fn pre_birth_lines_fold_to_one_order() {
    let anchor = wire(&Op::create("proj", "proj-0", "Anchor", "seat").unwrap());
    let template = Op::create("proj", "proj-1", "Born", "seat").unwrap();
    let ann = child(
        &[&anchor],
        &template,
        OpKind::Ballot {
            voter: "ann".to_string(),
            rev: String::new(),
            choice: "yes".to_string(),
            stamp: "[2026-01-01 Thu]".to_string(),
        },
        1,
    );
    let bob = child(
        &[&anchor],
        &template,
        OpKind::Ballot {
            voter: "bob".to_string(),
            rev: String::new(),
            choice: "no".to_string(),
            stamp: "[2026-01-01 Thu]".to_string(),
        },
        2,
    );
    let alpha = child(
        &[&anchor],
        &template,
        OpKind::AddRevision {
            rev_id: "1".to_string(),
            summary: "alpha".to_string(),
        },
        3,
    );
    let beta = child(
        &[&anchor],
        &template,
        OpKind::AddRevision {
            rev_id: "2".to_string(),
            summary: "beta".to_string(),
        },
        4,
    );
    let birth = child(
        &[&ann, &bob, &alpha, &beta],
        &template,
        template.kind.clone(),
        5,
    );
    let mut ballot_lines = [
        (ann.lamport, ann.id().unwrap(), "[2026-01-01 Thu] ann: yes"),
        (bob.lamport, bob.id().unwrap(), "[2026-01-01 Thu] bob: no"),
    ];
    let mut revision_lines = [
        (alpha.lamport, alpha.id().unwrap(), "1 alpha"),
        (beta.lamport, beta.id().unwrap(), "2 beta"),
    ];
    ballot_lines.sort();
    revision_lines.sort();
    let expect_drawers = format!(
        ":REVISIONS:\n{}\n{}\n:END:\n:VOTES:\n{}\n{}\n:END:\n",
        revision_lines[0].2, revision_lines[1].2, ballot_lines[0].2, ballot_lines[1].2
    );
    let early = vec![anchor, ann, bob, alpha, beta, birth];
    let mut seen = None;
    for _ in 0..40 {
        let folded = fold(&early).expect("pre-birth fold");
        let text = drawers(&folded, "proj-1");
        assert!(folded.conflicts.is_empty(), "{:?}", folded.conflicts);
        match &seen {
            None => seen = Some(text),
            Some(first) => assert_eq!(first, &text, "fold order changed the drawers"),
        }
    }
    let mut reversed = early.clone();
    reversed.reverse();
    let backward = drawers(&fold(&reversed).expect("reversed"), "proj-1");
    assert_eq!(seen.as_deref(), Some(backward.as_str()));
    assert_eq!(seen.as_deref(), Some(expect_drawers.as_str()));
}

fn duplicate_cites_fold_the_same_either_way() -> (Op, Op, OpKind) {
    let anchor = wire(&Op::create("proj", "proj-0", "Anchor", "seat").unwrap());
    let template = Op::create("proj", "proj-1", "Dup", "seat").unwrap();
    let mut dup_links = plain_heading("proj-1", "Dup");
    dup_links.extra_drawers = vec![
        "\
:LINKS:
# link-note
cites proj-2
cites proj-2
:END:
"
        .to_string(),
    ];
    let birth_kind = OpKind::GenesisImport {
        heading: Box::new(dup_links),
        residue: String::new(),
    };
    let link_kind = OpKind::Link {
        kind: "cites".to_string(),
        other: "proj-2".to_string(),
    };
    let mut link_first = None;
    let mut birth_first = None;
    for birth_wall in 0..24 {
        for link_wall in 0..24 {
            let birth = child(&[&anchor], &template, birth_kind.clone(), birth_wall);
            let link = child(&[&anchor], &template, link_kind.clone(), link_wall);
            let birth_id = birth.id().unwrap();
            let link_id = link.id().unwrap();
            if link_id < birth_id && link_first.is_none() {
                link_first = Some((birth.clone(), link.clone()));
            }
            if birth_id < link_id && birth_first.is_none() {
                birth_first = Some((birth, link));
            }
            if link_first.is_some() && birth_first.is_some() {
                break;
            }
        }
        if link_first.is_some() && birth_first.is_some() {
            break;
        }
    }
    let mut link_drawers = Vec::new();
    for (birth, link) in [link_first.unwrap(), birth_first.unwrap()] {
        let birth_id = birth.id().unwrap();
        let link_id = link.id().unwrap();
        both_ways(&[anchor.clone(), birth, link], |folded| {
            let text = drawers(folded, "proj-1");
            assert_eq!(
                text.lines().filter(|line| *line == "cites proj-2").count(),
                3,
                "{text}"
            );
            assert!(line_is(&text, "# link-note"), "{text}");
            let clash = clash_ids(folded, "link:5:citesproj-2");
            assert!(clash.contains(&birth_id), "{clash:?}");
            assert!(clash.contains(&link_id), "{clash:?}");
            assert_eq!(clash.len(), 2, "{clash:?}");
            link_drawers.push(text);
        });
    }
    assert_eq!(
        link_drawers[0], link_drawers[1],
        "delivery order changed the drawer"
    );
    (anchor, template, link_kind)
}

fn a_clear_drops_only_the_lines_it_observed(template: &Op, link_kind: &OpKind) {
    let mut cleared = plain_heading("proj-1", "Dup");
    cleared.extra_drawers = vec![
        "\
:LINKS:
# link-note
cites proj-2
cites proj-2
cites proj-9
:END:
"
        .to_string(),
    ];
    let clear_birth = genesis_of(cleared);
    let clear_kind = OpKind::SetField {
        name: "link:5:citesproj-2".to_string(),
        value: String::new(),
    };
    let mut clear_first = None;
    let mut add_first = None;
    for clear_wall in 0..24 {
        for add_wall in 0..24 {
            let clear = child(&[&clear_birth], template, clear_kind.clone(), clear_wall);
            let add = child(&[&clear_birth], template, (*link_kind).clone(), add_wall);
            let clear_id = clear.id().unwrap();
            let add_id = add.id().unwrap();
            if clear_id < add_id && clear_first.is_none() {
                clear_first = Some((clear.clone(), add.clone()));
            }
            if add_id < clear_id && add_first.is_none() {
                add_first = Some((clear, add));
            }
            if clear_first.is_some() && add_first.is_some() {
                break;
            }
        }
        if clear_first.is_some() && add_first.is_some() {
            break;
        }
    }
    let clear_lines = "\
:LINKS:
# link-note
cites proj-9
cites proj-2
:END:
";
    for (clear, add) in [clear_first.unwrap(), add_first.unwrap()] {
        both_ways(&[clear_birth.clone(), clear, add], |folded| {
            let links = folded.issues[&("proj".to_string(), "proj-1".to_string())]
                .extra_drawers
                .iter()
                .find(|drawer| drawer.starts_with(":LINKS:"))
                .map(String::as_str);
            assert_eq!(links, Some(clear_lines));
            assert!(
                folded
                    .conflicts
                    .iter()
                    .all(|clash| clash.field != "link:5:citesproj-2"),
                "one remaining line is not a clash: {:?}",
                folded.conflicts
            );
        });
    }
}

fn a_replaced_revision_matches_across_birth(anchor: &Op, template: &Op) -> Op {
    let post_birth = wire(&Op::create("proj", "proj-1", "Title", "seat").unwrap());
    let old_rev = OpKind::AddRevision {
        rev_id: "5".to_string(),
        summary: "old".to_string(),
    };
    let other_rev = OpKind::AddRevision {
        rev_id: "7".to_string(),
        summary: "other".to_string(),
    };
    let new_rev = OpKind::AddRevision {
        rev_id: "5".to_string(),
        summary: "new".to_string(),
    };
    let mut post_pair = None;
    for left_wall in 0..20 {
        for right_wall in 0..20 {
            let left = spawned(&post_birth, old_rev.clone(), left_wall);
            let right = spawned(&post_birth, other_rev.clone(), right_wall);
            if left.id().unwrap() < right.id().unwrap() {
                post_pair = Some((left, right));
                break;
            }
        }
        if post_pair.is_some() {
            break;
        }
    }
    let (left, right) = post_pair.unwrap();
    let replaced = spawned(&left, new_rev.clone(), 40);
    let post = fold(&[
        post_birth.clone(),
        left.clone(),
        right.clone(),
        replaced.clone(),
    ])
    .unwrap();
    let post_rev = post.issues[&("proj".to_string(), "proj-1".to_string())]
        .extra_drawers
        .iter()
        .find(|drawer| drawer.starts_with(":REVISIONS:"))
        .cloned()
        .unwrap();
    assert_eq!(post_rev, ":REVISIONS:\n5 new\n7 other\n:END:\n");
    let mut spaced_rev = plain_heading("proj-1", "Legacy");
    spaced_rev.extra_drawers = vec![
        "\
:REVISIONS:
1 2 3
:END:
"
        .to_string(),
    ];
    let spaced_birth = genesis_of(spaced_rev);
    let gamma = spawned(
        &spaced_birth,
        OpKind::AddRevision {
            rev_id: "1".to_string(),
            summary: "gamma".to_string(),
        },
        1,
    );
    let spaced = drawers(&fold(&[spaced_birth, gamma]).unwrap(), "proj-1");
    assert!(line_is(&spaced, "1 2 3"), "{spaced}");
    assert!(line_is(&spaced, "1 gamma"), "{spaced}");
    let mut pre_pair = None;
    for left_wall in 0..20 {
        for right_wall in 0..20 {
            let left = child(&[anchor], template, old_rev.clone(), left_wall);
            let right = child(&[anchor], template, other_rev.clone(), right_wall);
            if left.id().unwrap() < right.id().unwrap() {
                pre_pair = Some((left, right));
                break;
            }
        }
        if pre_pair.is_some() {
            break;
        }
    }
    let (pre_left, pre_right) = pre_pair.unwrap();
    let pre_replaced = child(&[&pre_left], template, new_rev, 41);
    let pre_birth = child(
        &[&pre_left, &pre_right, &pre_replaced],
        template,
        template.kind.clone(),
        42,
    );
    let pre = fold(&[
        (*anchor).clone(),
        pre_left,
        pre_right,
        pre_replaced,
        pre_birth,
    ])
    .unwrap();
    let pre_rev = pre.issues[&("proj".to_string(), "proj-1".to_string())]
        .extra_drawers
        .iter()
        .find(|drawer| drawer.starts_with(":REVISIONS:"))
        .cloned()
        .unwrap();
    assert_eq!(pre_rev, post_rev);
    post_birth
}

fn target_fields_keep_the_order_the_op_named(anchor: Op, template: &Op, post_birth: Op) {
    let target_kind = OpKind::SetTarget {
        target_id: "t1".to_string(),
        fields: vec![
            ("z".to_string(), "1".to_string()),
            ("a".to_string(), "2".to_string()),
        ],
    };
    let targeted = spawned(&post_birth, target_kind.clone(), 3);
    let after_birth = drawers(&fold(&[post_birth, targeted]).unwrap(), "proj-1");
    let early_target = child(&[&anchor], template, target_kind.clone(), 3);
    let early_birth = child(&[&early_target], template, template.kind.clone(), 4);
    let before_birth = drawers(
        &fold(&[anchor.clone(), early_target, early_birth]).unwrap(),
        "proj-1",
    );
    assert_eq!(before_birth, after_birth);
    assert!(
        after_birth.find("t1 z 1").unwrap() < after_birth.find("t1 a 2").unwrap(),
        "{after_birth}"
    );
    let mut between = None;
    for first_wall in 0..16 {
        for set_wall in 0..16 {
            for second_wall in 0..16 {
                let first = child(&[&anchor], template, template.kind.clone(), first_wall);
                let set = child(&[&anchor], template, target_kind.clone(), set_wall);
                let second = child(&[&anchor], template, template.kind.clone(), second_wall);
                let first_id = first.id().unwrap();
                let set_id = set.id().unwrap();
                let second_id = second.id().unwrap();
                if first_id < set_id && set_id < second_id {
                    between = Some((first, set, second));
                    break;
                }
            }
            if between.is_some() {
                break;
            }
        }
        if between.is_some() {
            break;
        }
    }
    let (first, set, second) = between.expect("a SetTarget between two births");
    let both = fold(&[anchor, first, set, second]).unwrap();
    for id in ["proj-1", "proj-1~2"] {
        let text = drawers(&both, id);
        let zed = text.find("t1 z 1").unwrap_or_else(|| panic!("{id} {text}"));
        let ay = text.find("t1 a 2").unwrap_or_else(|| panic!("{id} {text}"));
        assert!(zed < ay, "{id} {text}");
    }
}

fn a_value_base_cannot_store_is_refused(root: &Op) {
    let refused: &[(&str, OpKind)] = &[
        (
            "property:NOTE",
            OpKind::SetField {
                name: "property:NOTE".to_string(),
                value: "hello\n:DEEDS:    stolen".to_string(),
            },
        ),
        (
            "property:NOTE",
            OpKind::SetField {
                name: "property:NOTE".to_string(),
                value: "a\u{0001}b".to_string(),
            },
        ),
        (
            "property:NOTE",
            OpKind::SetField {
                name: "property:NOTE".to_string(),
                value: String::new(),
            },
        ),
        (
            "property:ID",
            OpKind::SetField {
                name: "property:ID".to_string(),
                value: "other-id".to_string(),
            },
        ),
        (
            "choice",
            OpKind::Ballot {
                voter: "ann".to_string(),
                rev: String::new(),
                choice: "a\nb".to_string(),
                stamp: "[2026-01-01 Thu]".to_string(),
            },
        ),
    ];
    for (snippet, kind) in refused {
        let op = spawned(root, kind.clone(), 9);
        let err = Op::from_message_bytes(&op.message_bytes().expect(snippet)).expect_err(snippet);
        let text = err.to_string();
        assert!(text.contains(snippet), "{snippet} not in {text}");
        assert!(
            fold(&[(*root).clone(), op]).is_err(),
            "{snippet} changed the tracker"
        );
    }
    let mut note_heading = plain_heading("proj-1", "Genesis");
    note_heading
        .properties
        .insert("NOTE".to_string(), "hello\n:DEEDS:    stolen".to_string());
    note_heading.property_order.push("NOTE".to_string());
    let title_heading = plain_heading("proj-1", "Hello\n* DONE [#A] other");
    let mut log_heading = plain_heading("proj-1", "Genesis");
    log_heading
        .logbook
        .push(note("keep\n:DEEDS:    stolen", "[2026-01-01 Thu 00:00]"));
    for (snippet, heading) in [
        ("property:NOTE", note_heading),
        ("title", title_heading),
        ("note", log_heading),
    ] {
        let op = Op {
            parents: vec![root.id().unwrap()],
            lamport: root.lamport + 1,
            seat: root.seat.clone(),
            project: root.project.clone(),
            issue: root.issue,
            kind: OpKind::GenesisImport {
                heading: Box::new(heading),
                residue: String::new(),
            },
            wall: 9,
        };
        let err = Op::from_message_bytes(&op.message_bytes().expect(snippet)).expect_err(snippet);
        let text = err.to_string();
        assert!(text.contains(snippet), "{snippet} not in {text}");
        assert!(
            fold(&[(*root).clone(), op]).is_err(),
            "{snippet} changed the tracker"
        );
    }
}

fn a_heading_round_trips_when_the_base_reader_accepts_it() {
    let round_trip = |heading: IssueHeading| {
        let ops = describe("proj", std::slice::from_ref(&heading)).expect("describe");
        fold(&ops).expect("fold").issues[&("proj".to_string(), "proj-1".to_string())].clone()
    };
    let mut waiting = plain_heading("proj-1", "Gate");
    waiting.state = "WAITING".to_string();
    waiting.priority = 'A';
    assert_eq!(round_trip(waiting).state, "WAITING");
    let idea = "\
#+TODO: IDEA WAIT | DONE CANCELLED

* IDEA [#A] Gate
:PROPERTIES:
:ID:         proj-1
:OWNER:      ada
:END:
";
    let idea_doc = IssueDoc::parse("proj", PathBuf::from("issues.org"), idea).unwrap();
    let idea_folded = fold(&describe("proj", &idea_doc.headings).unwrap()).unwrap();
    assert_eq!(
        idea_folded.issues[&("proj".to_string(), "proj-1".to_string())].state,
        "IDEA"
    );
    let mut empty_note = plain_heading("proj-1", "Gate");
    empty_note
        .properties
        .insert("NOTE".to_string(), String::new());
    empty_note.property_order.push("NOTE".to_string());
    assert_eq!(
        round_trip(empty_note)
            .properties
            .get("NOTE")
            .map(String::as_str),
        Some("")
    );
    let mut spaced = plain_heading("proj-1", "Gate");
    spaced
        .properties
        .insert("MY KEY".to_string(), "kept".to_string());
    spaced.property_order.push("MY KEY".to_string());
    assert_eq!(
        round_trip(spaced)
            .properties
            .get("MY KEY")
            .map(String::as_str),
        Some("kept")
    );
    assert_eq!(round_trip(plain_heading("proj-1", "")).title, "");
    assert_eq!(
        round_trip(plain_heading("proj-1", "a\u{0001}b")).title,
        "a\u{0001}b"
    );
    let mut injected = plain_heading("proj-1", "Gate");
    injected.extra_drawers = vec![
        "\
:NOTES:
ok
:END:
* DONE [#A] Injected
:PROPERTIES:
:ID:         proj-stolen
:DEEDS:      stolen
:END:
"
        .to_string(),
    ];
    assert!(
        describe("proj", &[injected]).is_err(),
        "a drawer past :END: folded"
    );
}

fn two_adds_and_a_clear_share_one_drawer() {
    let dup_link = "\
:LINKS:
# link-note
cites proj-2
cites proj-2
cites proj-9
:END:
";
    let mut two_adds = plain_heading("proj-1", "Dup");
    two_adds.extra_drawers = vec![dup_link.to_string()];
    let two_birth = genesis_of(two_adds);
    let two_template = Op::create("proj", "proj-1", "Dup", "seat").unwrap();
    let add_kind = OpKind::Link {
        kind: "cites".to_string(),
        other: "proj-2".to_string(),
    };
    let clear_kind = OpKind::SetField {
        name: "link:5:citesproj-2".to_string(),
        value: String::new(),
    };
    let mut clear_before = None;
    let mut clear_after = None;
    let mut clear_between = None;
    for left in 0..16 {
        for right in 0..16 {
            for clear_wall in 0..16 {
                let left_add = child(&[&two_birth], &two_template, add_kind.clone(), left);
                let right_add = child(&[&two_birth], &two_template, add_kind.clone(), right);
                let clear = child(&[&two_birth], &two_template, clear_kind.clone(), clear_wall);
                let left_id = left_add.id().unwrap();
                let right_id = right_add.id().unwrap();
                let clear_id = clear.id().unwrap();
                if left_id == right_id {
                    continue;
                }
                let set = (two_birth.clone(), left_add, right_add, clear);
                if clear_id < left_id && clear_id < right_id && clear_before.is_none() {
                    clear_before = Some(set);
                } else if clear_id > left_id && clear_id > right_id && clear_after.is_none() {
                    clear_after = Some(set);
                } else if ((left_id < clear_id && clear_id < right_id)
                    || (right_id < clear_id && clear_id < left_id))
                    && clear_between.is_none()
                {
                    clear_between = Some(set);
                }
                if clear_before.is_some() && clear_after.is_some() && clear_between.is_some() {
                    break;
                }
            }
            if clear_before.is_some() && clear_after.is_some() && clear_between.is_some() {
                break;
            }
        }
        if clear_before.is_some() && clear_after.is_some() && clear_between.is_some() {
            break;
        }
    }
    let mut two_drawers = Vec::new();
    for (birth, left_add, right_add, clear) in [
        clear_before.expect("clear sorts first"),
        clear_between.expect("clear sorts between the adds"),
        clear_after.expect("clear sorts last"),
    ] {
        let ids = [
            birth.id().unwrap(),
            left_add.id().unwrap(),
            right_add.id().unwrap(),
            clear.id().unwrap(),
        ];
        let mut ops = vec![birth, left_add, right_add, clear];
        let mut seen = None;
        permute(&mut ops, 0, &mut |order| {
            let folded = fold(order).expect("two adds and a clear");
            let text = drawers(&folded, "proj-1");
            assert_eq!(
                text.lines().filter(|line| *line == "cites proj-2").count(),
                2,
                "{text}"
            );
            assert!(line_is(&text, "# link-note"), "{text}");
            assert!(line_is(&text, "cites proj-9"), "{text}");
            let clash = clash_ids(&folded, "link:5:citesproj-2");
            assert_eq!(clash.len(), 4, "{clash:?}");
            for id in ids {
                assert!(clash.contains(&id), "{clash:?}");
            }
            match &seen {
                None => seen = Some(text),
                Some(first) => assert_eq!(first, &text),
            }
        });
        two_drawers.push(seen.expect("a drawer"));
    }
    assert_eq!(two_drawers[0], two_drawers[1]);
    assert_eq!(two_drawers[0], two_drawers[2]);
    let observed_add = child(&[&two_birth], &two_template, add_kind.clone(), 1);
    let observed_clear = child(&[&observed_add], &two_template, clear_kind.clone(), 2);
    let observed = fold(&[two_birth.clone(), observed_add, observed_clear]).unwrap();
    let observed_text = drawers(&observed, "proj-1");
    assert_eq!(
        observed_text
            .lines()
            .filter(|line| *line == "cites proj-2")
            .count(),
        0,
        "{observed_text}"
    );
    assert!(line_is(&observed_text, "cites proj-9"), "{observed_text}");
    assert!(
        observed
            .conflicts
            .iter()
            .all(|clash| clash.field != "link:5:citesproj-2"),
        "{:?}",
        observed.conflicts
    );
}

fn a_pre_birth_descendant_keeps_every_line() {
    let two_template = Op::create("proj", "proj-1", "Dup", "seat").unwrap();
    let add_kind = OpKind::Link {
        kind: "cites".to_string(),
        other: "proj-2".to_string(),
    };
    let anchor = wire(&Op::create("proj", "proj-0", "Anchor", "seat").unwrap());
    let title = child(
        &[&anchor],
        &anchor,
        OpKind::SetField {
            name: "title".to_string(),
            value: "Renamed".to_string(),
        },
        1,
    );
    let mut pre_heading = plain_heading("proj-1", "Dup");
    pre_heading.extra_drawers = vec![
        "\
:LINKS:
# link-note
cites proj-2
cites proj-2

cites proj-9
:END:
"
        .to_string(),
        ":NOTES:\nkeep-me\n:END:\n".to_string(),
        ":LINKS:\nsecond-drawer\n:END:\n".to_string(),
    ];
    let pre_kind = OpKind::GenesisImport {
        heading: Box::new(pre_heading),
        residue: String::new(),
    };
    let mut child_first = None;
    let mut birth_first = None;
    for birth_wall in 0..20 {
        for first_wall in 0..12 {
            for second_wall in 0..12 {
                let birth = child(&[&title], &two_template, pre_kind.clone(), birth_wall);
                let first = child(&[&anchor], &two_template, add_kind.clone(), first_wall);
                let second = child(&[&first], &two_template, add_kind.clone(), second_wall);
                let birth_id = birth.id().unwrap();
                let second_id = second.id().unwrap();
                if second_id < birth_id && child_first.is_none() {
                    child_first = Some((birth.clone(), first.clone(), second.clone()));
                }
                if birth_id < second_id && birth_first.is_none() {
                    birth_first = Some((birth, first, second));
                }
                if child_first.is_some() && birth_first.is_some() {
                    break;
                }
            }
            if child_first.is_some() && birth_first.is_some() {
                break;
            }
        }
        if child_first.is_some() && birth_first.is_some() {
            break;
        }
    }
    let mut pre_drawers = Vec::new();
    for (birth, first, second) in [
        child_first.expect("the child sorts before the birth"),
        birth_first.expect("the birth sorts before the child"),
    ] {
        let birth_id = birth.id().unwrap();
        let first_id = first.id().unwrap();
        let second_id = second.id().unwrap();
        let mut ops = vec![anchor.clone(), title.clone(), birth, first, second];
        let mut seen = None;
        permute(&mut ops, 0, &mut |order| {
            let folded = fold(order).expect("pre-birth descendants");
            let text = drawers(&folded, "proj-1");
            assert_eq!(
                text.lines().filter(|line| *line == "cites proj-2").count(),
                4,
                "{text}"
            );
            assert!(text.contains("keep-me"), "{text}");
            assert!(text.contains("second-drawer"), "{text}");
            assert!(line_is(&text, "# link-note"), "{text}");
            let clash = clash_ids(&folded, "link:5:citesproj-2");
            assert!(clash.contains(&birth_id), "{clash:?}");
            assert!(clash.contains(&first_id), "{clash:?}");
            assert!(clash.contains(&second_id), "{clash:?}");
            assert_eq!(clash.len(), 3, "{clash:?}");
            match &seen {
                None => seen = Some(text),
                Some(first) => assert_eq!(first, &text),
            }
        });
        pre_drawers.push(seen.expect("a pre-birth drawer"));
    }
    assert_eq!(pre_drawers[0], pre_drawers[1]);
}

fn a_drawer_type_without_a_headline_tag_folds() {
    let typed = "\
* TODO [#B] Typed
:PROPERTIES:
:ID:         proj-1
:TYPE:       bug
:END:
";
    let typed_doc = IssueDoc::parse("proj", PathBuf::from("issues.org"), typed).unwrap();
    assert!(typed_doc.headings[0].org_tags.is_empty());
    let typed_folded = fold(&describe("proj", &typed_doc.headings).unwrap()).unwrap();
    assert_eq!(
        typed_folded.issues[&("proj".to_string(), "proj-1".to_string())],
        typed_doc.headings[0]
    );
}
