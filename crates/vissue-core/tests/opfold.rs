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

fn genesis_of(heading: IssueHeading) -> Op {
    wire(&describe("proj", &[heading]).unwrap().pop().unwrap())
}

fn issue_text(folded: &Folded) -> String {
    let heading = &folded.issues[&("proj".to_string(), "proj-1".to_string())];
    let mut text = String::new();
    for value in heading.properties.values() {
        text.push_str(value);
        text.push('\n');
    }
    text.push_str(&heading.extra_drawers.concat());
    text
}

/// Genesis and a scalar set value must use the same keys as the op that
/// rewrites the drawer. Each row is one heading and the next op; every
/// input order keeps the genesis line.
#[test]
fn genesis_lines_survive_the_next_op() {
    let mut deeds = plain_heading("proj-1", "Genesis");
    deeds
        .properties
        .insert("DEEDS".to_string(), "a  b".to_string());
    deeds.property_order.push("DEEDS".to_string());
    let deeds_born = genesis_of(deeds);
    let deed = wire(&spawned(
        &deeds_born,
        OpKind::AddDeed {
            accession: "c".to_string(),
        },
        1,
    ));

    let mut linked = plain_heading("proj-1", "Genesis");
    linked.extra_drawers = vec![
        ":LINKS:\nrelates kept\n:END:\n".to_string(),
        ":TARGETS:\nt1 status open\n:END:\n".to_string(),
    ];
    let linked_born = genesis_of(linked);
    let link = wire(&spawned(
        &linked_born,
        OpKind::Link {
            kind: "relates".to_string(),
            other: "new".to_string(),
        },
        1,
    ));
    let target = wire(&spawned(
        &linked_born,
        OpKind::SetTarget {
            target_id: "t2".to_string(),
            fields: vec![("status".to_string(), "shut".to_string())],
        },
        2,
    ));

    let mut revised = plain_heading("proj-1", "Genesis");
    revised.extra_drawers = vec![":REVISIONS:\n1 2 3\n1 2 x\n:END:\n".to_string()];
    let revised_born = genesis_of(revised);
    let revision = wire(&spawned(
        &revised_born,
        OpKind::AddRevision {
            rev_id: "9".to_string(),
            summary: "z".to_string(),
        },
        1,
    ));

    let mut voted = plain_heading("proj-1", "Genesis");
    voted.extra_drawers = vec![
        ":VOTES:\n[rev=1] [2026-01-01 Thu] ann: yes\n[rev=2] [2026-01-02 Fri] ann: no\n:END:\n"
            .to_string(),
    ];
    let voted_born = genesis_of(voted);
    let ballot = wire(&spawned(
        &voted_born,
        OpKind::Ballot {
            voter: "bob".to_string(),
            rev: String::new(),
            choice: "yes".to_string(),
            stamp: "[2026-01-03 Sat]".to_string(),
        },
        1,
    ));

    let rows: [(&str, Vec<Op>, &[&str]); 4] = [
        ("deeds", vec![deeds_born, deed], &["a  b"]),
        (
            "links and targets",
            vec![linked_born, link, target],
            &["relates kept", "t1 status open"],
        ),
        (
            "revisions",
            vec![revised_born, revision],
            &["1 2 3", "1 2 x"],
        ),
        (
            "votes",
            vec![voted_born, ballot],
            &["[rev=1]", "ann: yes", "[rev=2]", "ann: no"],
        ),
    ];
    let mut report = String::new();
    for (label, mut ops, needles) in rows {
        let mut expected: Option<Vec<u8>> = None;
        let mut case_lost = String::new();
        permute(&mut ops, 0, &mut |order| {
            if !case_lost.is_empty() {
                return;
            }
            let Ok(folded) = fold(order) else {
                case_lost.push_str("fold error\n");
                return;
            };
            if !folded.conflicts.is_empty() {
                case_lost.push_str(&format!("clash {:?}\n", folded.conflicts));
            }
            let text = issue_text(&folded);
            for needle in needles {
                if !text.contains(needle) {
                    case_lost.push_str(&format!("lost {needle} in {text}\n"));
                }
            }
            let bytes = folded_bytes(&folded);
            if let Some(first) = &expected
                && first != &bytes
            {
                case_lost.push_str("orders differ\n");
            }
            expected = Some(bytes);
        });
        if !case_lost.is_empty() {
            report.push_str(label);
            report.push('\n');
            report.push_str(&case_lost);
        }
    }
    assert!(report.is_empty(), "{report}");
}
