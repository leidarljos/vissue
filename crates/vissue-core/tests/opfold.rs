//! The op log folds to the same in-memory issues the org parser produces,
//! and fold order follows the op set rather than the order the ops were handed in.

#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use vissue_core::model::{IssueHeading, LogEntry};
use vissue_core::oplog::{Folded, IssueKey, Op, OpId, OpKind, describe, fold};
use vissue_core::store::IssueDoc;
use vissue_core::target::target_id;

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
                Some("\"p q\" r t")
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
            Some("\"a  b\" c")
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
        assert!(drawers.contains("\"1\" \"2 3\""), "{drawers}");
        assert!(drawers.contains("\"1 2\" \"x\""), "{drawers}");
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

fn reencode(heading: &IssueHeading, kind: OpKind) -> Folded {
    let born = genesis_of(heading.clone());
    let next = wire(&spawned(&born, kind, 1));
    fold(&[born, next]).expect("reencode")
}

fn heading_of(folded: &Folded) -> IssueHeading {
    folded.issues[&("proj".to_string(), "proj-1".to_string())].clone()
}

fn clash_of<'a>(folded: &'a Folded, field: &str) -> Option<&'a vissue_core::oplog::Conflict> {
    folded.conflicts.iter().find(|clash| clash.field == field)
}

fn line_is(text: &str, line: &str) -> bool {
    text.lines().filter(|item| *item == line).count() == 1
}

/// Genesis and a scalar set value must use the same codec as the op that
/// rewrites the drawer. Each row is one heading and the next op. Nothing
/// is dropped or duplicated, and a clash is recorded when the base records
/// one for a title. Re-encoding a line this code wrote yields that line.
#[test]
fn genesis_lines_survive_the_next_op() {
    let mut deed_heading = plain_heading("proj-1", "Genesis");
    deed_heading
        .properties
        .insert("DEEDS".to_string(), "a  b".to_string());
    deed_heading.property_order.push("DEEDS".to_string());
    let deeds_born = genesis_of(deed_heading);
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
            &["\"relates\" \"kept\"", "\"t1\" \"status\" \"open\""],
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

    // 1. A rendered list reloads as its elements. `a  b` stays one element.
    let mut listed = plain_heading("proj-1", "Genesis");
    listed
        .properties
        .insert("DEEDS".to_string(), "acc1 acc2".to_string());
    listed
        .properties
        .insert("BLOCKED_BY".to_string(), "a-1 a-2".to_string());
    listed.property_order.push("DEEDS".to_string());
    listed.property_order.push("BLOCKED_BY".to_string());
    let listed_born = genesis_of(listed);
    let add_acc = wire(&spawned(
        &listed_born,
        OpKind::AddDeed {
            accession: "acc1".to_string(),
        },
        1,
    ));
    let add_block = wire(&spawned(
        &listed_born,
        OpKind::Link {
            kind: "blocked-by".to_string(),
            other: "a-1".to_string(),
        },
        2,
    ));
    both_ways(&[listed_born, add_acc, add_block], |folded| {
        if deeds(folded).as_deref() != Some("acc1 acc2") {
            report.push_str(&format!("deed list {:?}\n", deeds(folded)));
        }
        if blocked(folded).as_deref() != Some("a-1 a-2") {
            report.push_str(&format!("blocker list {:?}\n", blocked(folded)));
        }
        if !folded.conflicts.is_empty() {
            report.push_str(&format!("set clash {:?}\n", folded.conflicts));
        }
    });
    let mut spaced_h = plain_heading("proj-1", "Genesis");
    spaced_h
        .properties
        .insert("DEEDS".to_string(), "a  b".to_string());
    spaced_h.property_order.push("DEEDS".to_string());
    let spaced_born = genesis_of(spaced_h);
    let spaced_add = wire(&spawned(
        &spaced_born,
        OpKind::AddDeed {
            accession: "c".to_string(),
        },
        1,
    ));
    let spaced = fold(&[spaced_born, spaced_add]).expect("spaced deeds");
    if deeds(&spaced).as_deref() != Some("\"a  b\" c") {
        report.push_str(&format!("spaced deed {:?}\n", deeds(&spaced)));
    }
    let spaced_again = reencode(
        &heading_of(&spaced),
        OpKind::AddDeed {
            accession: "c".to_string(),
        },
    );
    if deeds(&spaced_again).as_deref() != Some("\"a  b\" c") {
        report.push_str(&format!("set round trip {:?}\n", deeds(&spaced_again)));
    }

    // 2. A ballot line carries its rev, so two revs of one voter both survive.
    let vote_root = wire(&Op::create("proj", "proj-1", "Votes", "seat").unwrap());
    let yes = wire(&spawned(
        &vote_root,
        OpKind::Ballot {
            voter: "ann".to_string(),
            rev: "1".to_string(),
            choice: "yes".to_string(),
            stamp: "[2026-01-01 Thu]".to_string(),
        },
        1,
    ));
    let no = wire(&spawned(
        &yes,
        OpKind::Ballot {
            voter: "ann".to_string(),
            rev: "2".to_string(),
            choice: "no".to_string(),
            stamp: "[2026-01-02 Fri]".to_string(),
        },
        2,
    ));
    let voted = fold(&[vote_root, yes, no]).expect("ballots");
    let vote_text = drawers(&voted, "proj-1");
    let yes_line = "[rev=1] [2026-01-01 Thu] ann: yes";
    let no_line = "[rev=2] [2026-01-02 Fri] ann: no";
    if !line_is(&vote_text, yes_line) || !line_is(&vote_text, no_line) {
        report.push_str(&format!("ballot write {vote_text}\n"));
    }
    let cara = reencode(
        &heading_of(&voted),
        OpKind::Ballot {
            voter: "cara".to_string(),
            rev: String::new(),
            choice: "yes".to_string(),
            stamp: "[2026-02-01 Sun]".to_string(),
        },
    );
    let cara_text = drawers(&cara, "proj-1");
    if !line_is(&cara_text, yes_line) || !line_is(&cara_text, no_line) {
        report.push_str(&format!("ballot round trip {cara_text}\n"));
    }
    if !cara.conflicts.is_empty() {
        report.push_str(&format!("ballot clash {:?}\n", cara.conflicts));
    }

    // 3. A revision is keyed by id. A simple line updates. Spaced ids stay distinct.
    let mut alpha = plain_heading("proj-1", "Genesis");
    alpha.extra_drawers = vec![":REVISIONS:\n1 alpha\n:END:\n".to_string()];
    let alpha_born = genesis_of(alpha);
    let beta = wire(&spawned(
        &alpha_born,
        OpKind::AddRevision {
            rev_id: "1".to_string(),
            summary: "beta".to_string(),
        },
        1,
    ));
    let updated = fold(&[alpha_born, beta]).expect("revision update");
    let updated_text = drawers(&updated, "proj-1");
    if !line_is(&updated_text, "\"1\" \"beta\"") || updated_text.contains("1 alpha") {
        report.push_str(&format!("revision update {updated_text}\n"));
    }
    if !updated.conflicts.is_empty() {
        report.push_str(&format!("revision clash {:?}\n", updated.conflicts));
    }
    let rev_root = wire(&Op::create("proj", "proj-1", "Revs", "seat").unwrap());
    let wide = wire(&spawned(
        &rev_root,
        OpKind::AddRevision {
            rev_id: "1".to_string(),
            summary: "2 3".to_string(),
        },
        1,
    ));
    let nested = wire(&spawned(
        &wide,
        OpKind::AddRevision {
            rev_id: "1 2".to_string(),
            summary: "x".to_string(),
        },
        2,
    ));
    let written_revs = fold(&[rev_root, wide, nested]).expect("revision lines");
    let rev_text = drawers(&written_revs, "proj-1");
    if !line_is(&rev_text, "\"1\" \"2 3\"") || !line_is(&rev_text, "\"1 2\" \"x\"") {
        report.push_str(&format!("revision write {rev_text}\n"));
    }
    let rev_again = reencode(
        &heading_of(&written_revs),
        OpKind::AddRevision {
            rev_id: "1 2".to_string(),
            summary: "x".to_string(),
        },
    );
    let rev_again_text = drawers(&rev_again, "proj-1");
    if !line_is(&rev_again_text, "\"1\" \"2 3\"") || !line_is(&rev_again_text, "\"1 2\" \"x\"") {
        report.push_str(&format!("revision round trip {rev_again_text}\n"));
    }

    // 4. Genesis is a writer of each target, retarget, and revision slot.
    let anchor = wire(&Op::create("proj", "anchor-1", "Anchor", "seat").unwrap());
    let mut frontier_h = plain_heading("proj-1", "Genesis");
    frontier_h.extra_drawers = vec![
        ":TARGETS:\nt1 status open\nt2 owner ada\nretarget old\n:END:\n".to_string(),
        ":REVISIONS:\n9\n1 alpha\n:END:\n".to_string(),
    ];
    let frontier_born = genesis_of(frontier_h);
    let shut = wire(&parented(
        &anchor,
        &frontier_born,
        OpKind::SetTarget {
            target_id: "t1".to_string(),
            fields: vec![("status".to_string(), "shut".to_string())],
        },
        1,
    ));
    let moved = wire(&parented(
        &anchor,
        &frontier_born,
        OpKind::Retarget {
            target_id: "new".to_string(),
        },
        2,
    ));
    let hello = wire(&parented(
        &anchor,
        &frontier_born,
        OpKind::AddRevision {
            rev_id: "9".to_string(),
            summary: "hello".to_string(),
        },
        3,
    ));
    let retitle = wire(&parented(
        &anchor,
        &frontier_born,
        OpKind::SetField {
            name: "title".to_string(),
            value: "Other".to_string(),
        },
        4,
    ));
    both_ways(
        &[anchor, frontier_born, shut, moved, hello, retitle],
        |folded| {
            let text = issue_text(folded);
            for field in ["title", "target:t1:status", "retarget", "revision:9"] {
                match clash_of(folded, field) {
                    Some(clash) if clash.ops.len() == 2 => {}
                    other => report.push_str(&format!("frontier {field} {other:?}\n")),
                }
            }
            if folded.conflicts.len() != 4 {
                report.push_str(&format!("frontier count {:?}\n", folded.conflicts));
            }
            if !text.contains("\"t2\" \"owner\" \"ada\"") || !text.contains("\"1\" \"alpha\"") {
                report.push_str(&format!("frontier lost a line {text}\n"));
            }
            if folded.issues[&("proj".to_string(), "proj-1".to_string())].title != "Other" {
                report.push_str("frontier title\n");
            }
        },
    );

    // 5. A line the loader cannot split is copied through the rewrite.
    let mut raw_h = plain_heading("proj-1", "Genesis");
    raw_h.extra_drawers = vec![
        ":LINKS:\nsolo\nsee-also\nrelates kept\n:END:\n".to_string(),
        ":TARGETS:\nbare\norphan\nt1 status open\n:END:\n".to_string(),
        ":VOTES:\nnot a ballot\n:END:\n".to_string(),
    ];
    let raw_born = genesis_of(raw_h);
    let raw_link = wire(&spawned(
        &raw_born,
        OpKind::Link {
            kind: "relates".to_string(),
            other: "new".to_string(),
        },
        1,
    ));
    let raw_target = wire(&spawned(
        &raw_born,
        OpKind::SetTarget {
            target_id: "t2".to_string(),
            fields: vec![("status".to_string(), "shut".to_string())],
        },
        2,
    ));
    let raw_vote = wire(&spawned(
        &raw_born,
        OpKind::Ballot {
            voter: "bob".to_string(),
            rev: String::new(),
            choice: "yes".to_string(),
            stamp: "[2026-01-03 Sat]".to_string(),
        },
        3,
    ));
    let raw_folded = fold(&[raw_born, raw_link, raw_target, raw_vote]).expect("raw lines");
    let raw_text = issue_text(&raw_folded);
    for needle in [
        "solo",
        "see-also",
        "\"relates\" \"kept\"",
        "\"relates\" \"new\"",
        "bare",
        "orphan",
        "\"t1\" \"status\" \"open\"",
        "\"t2\" \"status\" \"shut\"",
        "not a ballot",
    ] {
        if !line_is(&raw_text, needle) {
            report.push_str(&format!("raw lost {needle} in {raw_text}\n"));
        }
    }
    let raw_again = reencode(
        &heading_of(&raw_folded),
        OpKind::Link {
            kind: "relates".to_string(),
            other: "new".to_string(),
        },
    );
    let raw_again_text = issue_text(&raw_again);
    for needle in [
        "solo",
        "see-also",
        "bare",
        "orphan",
        "\"relates\" \"kept\"",
        "\"relates\" \"new\"",
    ] {
        if !line_is(&raw_again_text, needle) {
            report.push_str(&format!(
                "raw round trip lost {needle} in {raw_again_text}\n"
            ));
        }
    }

    // 6. A minted target id is one token. The field key quotes a colon
    // inside it, and the drawer line keeps the id whole. Clash count is
    // what one causal write records: none.
    let main_id = target_id("origin", "main");
    let dev_id = target_id("origin", "dev");
    let mut minted = plain_heading("proj-1", "Genesis");
    minted.extra_drawers = vec![format!(":TARGETS:\n{main_id} status open\n:END:\n")];
    let minted_born = genesis_of(minted);
    let other_target = wire(&spawned(
        &minted_born,
        OpKind::SetTarget {
            target_id: dev_id.clone(),
            fields: vec![("status".to_string(), "shut".to_string())],
        },
        1,
    ));
    let main_open = format!("\"{main_id}\" \"status\" \"open\"");
    let dev_shut = format!("\"{dev_id}\" \"status\" \"shut\"");
    let minted_ops = [minted_born, other_target];
    both_ways(&minted_ops, |folded| {
        let text = issue_text(folded);
        for line in [&main_open, &dev_shut] {
            if !line_is(&text, line) {
                report.push_str(&format!("minted lost {line} in {text}\n"));
            }
        }
        if text.lines().any(|line| line.starts_with("sha256 ")) {
            report.push_str(&format!("minted split {text}\n"));
        }
        if !folded.conflicts.is_empty() {
            report.push_str(&format!("minted clash {:?}\n", folded.conflicts));
        }
    });
    let minted_again = reencode(
        &heading_of(&fold(&minted_ops).expect("minted")),
        OpKind::SetTarget {
            target_id: dev_id.clone(),
            fields: vec![("status".to_string(), "shut".to_string())],
        },
    );
    let minted_again_text = issue_text(&minted_again);
    for line in [&main_open, &dev_shut] {
        if !line_is(&minted_again_text, line) {
            report.push_str(&format!(
                "minted round trip lost {line} in {minted_again_text}\n"
            ));
        }
    }

    let mut same = plain_heading("proj-1", "Genesis");
    same.extra_drawers = vec![format!(":TARGETS:\n{main_id} status open\n:END:\n")];
    let same_born = genesis_of(same);
    let shut_same = wire(&spawned(
        &same_born,
        OpKind::SetTarget {
            target_id: main_id.clone(),
            fields: vec![("status".to_string(), "shut".to_string())],
        },
        1,
    ));
    let main_shut = format!("\"{main_id}\" \"status\" \"shut\"");
    both_ways(&[same_born, shut_same], |folded| {
        let text = issue_text(folded);
        if !line_is(&text, &main_shut) || line_is(&text, &main_open) {
            report.push_str(&format!("same target {text}\n"));
        }
        if text.lines().any(|line| line.starts_with("sha256 ")) {
            report.push_str(&format!("same target split {text}\n"));
        }
        if !folded.conflicts.is_empty() {
            report.push_str(&format!("same target clash {:?}\n", folded.conflicts));
        }
    });

    // 7. An id spelled `retarget` is a field, not the retarget record.
    // Three tokens are a field. The retarget record is the two-token line.
    let mut word = plain_heading("proj-1", "Genesis");
    word.extra_drawers = vec![":TARGETS:\nretarget status open\n:END:\n".to_string()];
    let word_born = genesis_of(word);
    let moved_word = wire(&spawned(
        &word_born,
        OpKind::Retarget {
            target_id: "zzz".to_string(),
        },
        1,
    ));
    let word_ops = [word_born, moved_word];
    both_ways(&word_ops, |folded| {
        let text = issue_text(folded);
        for line in ["\"retarget\" \"status\" \"open\"", "\"retarget\" \"zzz\""] {
            if !line_is(&text, line) {
                report.push_str(&format!("keyword lost {line} in {text}\n"));
            }
        }
        if !folded.conflicts.is_empty() {
            report.push_str(&format!("keyword clash {:?}\n", folded.conflicts));
        }
    });
    let word_again = reencode(
        &heading_of(&fold(&word_ops).expect("keyword")),
        OpKind::Retarget {
            target_id: "zzz".to_string(),
        },
    );
    let word_again_text = issue_text(&word_again);
    for line in ["\"retarget\" \"status\" \"open\"", "\"retarget\" \"zzz\""] {
        if !line_is(&word_again_text, line) {
            report.push_str(&format!(
                "keyword round trip lost {line} in {word_again_text}\n"
            ));
        }
    }

    // 8. Legacy list text decodes to the ids `split_id_list` returns.
    // A tab, a comma, a backslash, and a lone quote stay those ids.
    let mut tabbed = plain_heading("proj-1", "Genesis");
    tabbed
        .properties
        .insert("DEEDS".to_string(), "acc1\tacc2".to_string());
    tabbed.property_order.push("DEEDS".to_string());
    let tab_born = genesis_of(tabbed);
    let tab_add = wire(&spawned(
        &tab_born,
        OpKind::AddDeed {
            accession: "zz".to_string(),
        },
        1,
    ));
    let tab_ops = [tab_born, tab_add];
    both_ways(&tab_ops, |folded| {
        let heading = &folded.issues[&("proj".to_string(), "proj-1".to_string())];
        if heading.deeds() != ["acc1".to_string(), "acc2".to_string(), "zz".to_string()] {
            report.push_str(&format!("tab deeds {:?}\n", heading.deeds()));
        }
        if deeds(folded).as_deref() != Some("acc1 acc2 zz") {
            report.push_str(&format!("tab property {:?}\n", deeds(folded)));
        }
        if !folded.conflicts.is_empty() {
            report.push_str(&format!("tab clash {:?}\n", folded.conflicts));
        }
    });
    let tab_again = reencode(
        &heading_of(&fold(&tab_ops).expect("tab")),
        OpKind::AddDeed {
            accession: "zz".to_string(),
        },
    );
    if deeds(&tab_again).as_deref() != Some("acc1 acc2 zz") {
        report.push_str(&format!("tab round trip {:?}\n", deeds(&tab_again)));
    }

    let mut commas = plain_heading("proj-1", "Genesis");
    commas
        .properties
        .insert("BLOCKED_BY".to_string(), "a-1,a-2".to_string());
    commas.property_order.push("BLOCKED_BY".to_string());
    let comma_born = genesis_of(commas);
    let comma_add = wire(&spawned(
        &comma_born,
        OpKind::Link {
            kind: "blocked-by".to_string(),
            other: "a-1".to_string(),
        },
        1,
    ));
    both_ways(&[comma_born, comma_add], |folded| {
        if blocked(folded).as_deref() != Some("a-1 a-2") {
            report.push_str(&format!("comma property {:?}\n", blocked(folded)));
        }
        let heading = &folded.issues[&("proj".to_string(), "proj-1".to_string())];
        if heading.blocked_by() != ["a-1".to_string(), "a-2".to_string()] {
            report.push_str(&format!("comma ids {:?}\n", heading.blocked_by()));
        }
        if !folded.conflicts.is_empty() {
            report.push_str(&format!("comma clash {:?}\n", folded.conflicts));
        }
    });

    let mut slashed = plain_heading("proj-1", "Genesis");
    slashed
        .properties
        .insert("DEEDS".to_string(), "a\\b".to_string());
    slashed.property_order.push("DEEDS".to_string());
    let slash_born = genesis_of(slashed);
    let slash_add = wire(&spawned(
        &slash_born,
        OpKind::AddDeed {
            accession: "zz".to_string(),
        },
        1,
    ));
    both_ways(&[slash_born, slash_add], |folded| {
        let heading = &folded.issues[&("proj".to_string(), "proj-1".to_string())];
        if heading.deeds() != ["a\\b".to_string(), "zz".to_string()] {
            report.push_str(&format!("slash deeds {:?}\n", heading.deeds()));
        }
        if deeds(folded).as_deref() != Some("a\\b zz") {
            report.push_str(&format!("slash property {:?}\n", deeds(folded)));
        }
        if !folded.conflicts.is_empty() {
            report.push_str(&format!("slash clash {:?}\n", folded.conflicts));
        }
    });

    let mut quoted = plain_heading("proj-1", "Genesis");
    quoted
        .properties
        .insert("DEEDS".to_string(), "\"".to_string());
    quoted.property_order.push("DEEDS".to_string());
    let quote_born = genesis_of(quoted);
    let quote_add = wire(&spawned(
        &quote_born,
        OpKind::AddDeed {
            accession: "zz".to_string(),
        },
        1,
    ));
    both_ways(&[quote_born, quote_add], |folded| {
        let heading = &folded.issues[&("proj".to_string(), "proj-1".to_string())];
        if heading.deeds() != ["\"".to_string(), "zz".to_string()] {
            report.push_str(&format!("quote deeds {:?}\n", heading.deeds()));
        }
        if deeds(folded).as_deref() != Some("\" zz") {
            report.push_str(&format!("quote property {:?}\n", deeds(folded)));
        }
        if !folded.conflicts.is_empty() {
            report.push_str(&format!("quote clash {:?}\n", folded.conflicts));
        }
    });

    let mut wrapped = plain_heading("proj-1", "Genesis");
    wrapped
        .properties
        .insert("DEEDS".to_string(), "\"acc1\"".to_string());
    wrapped.property_order.push("DEEDS".to_string());
    let wrapped_born = genesis_of(wrapped);
    let wrapped_add = wire(&spawned(
        &wrapped_born,
        OpKind::AddDeed {
            accession: "zz".to_string(),
        },
        1,
    ));
    both_ways(&[wrapped_born, wrapped_add], |folded| {
        let heading = &folded.issues[&("proj".to_string(), "proj-1".to_string())];
        if heading.deeds() != ["\"acc1\"".to_string(), "zz".to_string()] {
            report.push_str(&format!("wrapped deeds {:?}\n", heading.deeds()));
        }
        if !folded.conflicts.is_empty() {
            report.push_str(&format!("wrapped clash {:?}\n", folded.conflicts));
        }
    });

    // 9. A newline in a link, a target value, a retarget id, or a ballot
    // choice is escaped by the same quote the deed codec uses. The tail
    // is not a second line. One causal write records no clash.
    let newline_rows: [(&str, OpKind, &str); 4] = [
        (
            "link",
            OpKind::Link {
                kind: "relates".to_string(),
                other: "a\nb".to_string(),
            },
            "\"relates\" \"a\\nb\"",
        ),
        (
            "target value",
            OpKind::SetTarget {
                target_id: "t1".to_string(),
                fields: vec![("status".to_string(), "a\nb".to_string())],
            },
            "\"t1\" \"status\" \"a\\nb\"",
        ),
        (
            "retarget",
            OpKind::Retarget {
                target_id: "a\nb".to_string(),
            },
            "\"retarget\" \"a\\nb\"",
        ),
        (
            "ballot",
            OpKind::Ballot {
                voter: "ann".to_string(),
                rev: String::new(),
                choice: "a\nb".to_string(),
                stamp: "[2026-01-01 Thu]".to_string(),
            },
            "[2026-01-01 Thu] ann: \"a\\nb\"",
        ),
    ];
    for (label, kind, line) in newline_rows {
        let born = genesis_of(plain_heading("proj-1", "Genesis"));
        let next = wire(&spawned(&born, kind.clone(), 1));
        let ops = [born, next];
        both_ways(&ops, |folded| {
            let text = issue_text(folded);
            if !line_is(&text, line) || text.lines().any(|item| item == "b") {
                report.push_str(&format!("newline {label} {text}\n"));
            }
            if !folded.conflicts.is_empty() {
                report.push_str(&format!("newline {label} clash {:?}\n", folded.conflicts));
            }
        });
        let again = reencode(&heading_of(&fold(&ops).expect(label)), kind);
        let again_text = issue_text(&again);
        if !line_is(&again_text, line) || again_text.lines().any(|item| item == "b") {
            report.push_str(&format!("newline round trip {label} {again_text}\n"));
        }
    }

    // A legacy token that starts with a quote keeps those characters.
    // The next write marks the drawer and quotes every token.
    let mut quoted_h = plain_heading("proj-1", "Genesis");
    quoted_h.extra_drawers = vec![
        ":LINKS:\n# a note\nrelates \"kept\"\n:END:\n".to_string(),
        ":TARGETS:\nt1 status \"open\"\n\"t1\" status open\nretarget \"zzz\"\n:END:\n".to_string(),
        ":REVISIONS:\n1 \"alpha\"\n\"\n:END:\n".to_string(),
    ];
    let quoted_born = genesis_of(quoted_h);
    let quoted_link = wire(&spawned(
        &quoted_born,
        OpKind::Link {
            kind: "see-also".to_string(),
            other: "x".to_string(),
        },
        1,
    ));
    let quoted_target = wire(&spawned(
        &quoted_born,
        OpKind::SetTarget {
            target_id: "t2".to_string(),
            fields: vec![("status".to_string(), "shut".to_string())],
        },
        2,
    ));
    let quoted_rev = wire(&spawned(
        &quoted_born,
        OpKind::AddRevision {
            rev_id: "9".to_string(),
            summary: "z".to_string(),
        },
        3,
    ));
    let quoted_ops = [quoted_born, quoted_link, quoted_target, quoted_rev];
    let quoted_lines = [
        "# a note",
        "\"relates\" \"\\\"kept\\\"\"",
        "\"see-also\" \"x\"",
        "\"t1\" \"status\" \"\\\"open\\\"\"",
        "\"\\\"t1\\\"\" \"status\" \"open\"",
        "\"retarget\" \"\\\"zzz\\\"\"",
        "\"t2\" \"status\" \"shut\"",
        "\"1\" \"\\\"alpha\\\"\"",
        "\"\\\"\"",
        "\"9\" \"z\"",
    ];
    both_ways(&quoted_ops, |folded| {
        let text = issue_text(folded);
        if text.lines().filter(|line| *line == ":CODEC: 1").count() != 3 {
            report.push_str(&format!("quote marker {text}\n"));
        }
        for line in quoted_lines {
            if !line_is(&text, line) {
                report.push_str(&format!("quote kept lost {line} in {text}\n"));
            }
        }
        for stripped in ["relates kept", "t1 status open", "1 alpha", "retarget zzz"] {
            if line_is(&text, stripped) {
                report.push_str(&format!("quote stripped {stripped} in {text}\n"));
            }
        }
        if text.contains("# \"a note\"") {
            report.push_str(&format!("comment quoted {text}\n"));
        }
        if !folded.conflicts.is_empty() {
            report.push_str(&format!("quote kept clash {:?}\n", folded.conflicts));
        }
    });
    let quoted_again = reencode(
        &heading_of(&fold(&quoted_ops).expect("quoted legacy")),
        OpKind::Link {
            kind: "see-also".to_string(),
            other: "x".to_string(),
        },
    );
    let quoted_again_text = issue_text(&quoted_again);
    for line in quoted_lines {
        if !line_is(&quoted_again_text, line) {
            report.push_str(&format!(
                "quote fixed point lost {line} in {quoted_again_text}\n"
            ));
        }
    }

    // `a  b c` is three ids. A leading or trailing space is not part of an id.
    let space_rows: [(&str, &str, &[&str]); 7] = [
        ("double", "a  b c", &["a", "b", "c", "zz"]),
        (
            "double acc",
            "acc1 acc2  acc3",
            &["acc1", "acc2", "acc3", "zz"],
        ),
        ("leading", " deed-a", &["deed-a", "zz"]),
        ("trailing", "deed-a ", &["deed-a", "zz"]),
        ("both spaces", " deed-a ", &["deed-a", "zz"]),
        (
            "leading pair",
            " deed-a deed-b",
            &["deed-a", "deed-b", "zz"],
        ),
        (
            "trailing pair",
            "deed-a deed-b ",
            &["deed-a", "deed-b", "zz"],
        ),
    ];
    for (label, raw, ids) in space_rows {
        let want: Vec<String> = ids.iter().map(|id| (*id).to_string()).collect();
        let mut heading = plain_heading("proj-1", "Genesis");
        heading
            .properties
            .insert("DEEDS".to_string(), raw.to_string());
        heading.property_order.push("DEEDS".to_string());
        let born = genesis_of(heading);
        let add = wire(&spawned(
            &born,
            OpKind::AddDeed {
                accession: "zz".to_string(),
            },
            1,
        ));
        let ops = [born, add];
        both_ways(&ops, |folded| {
            let heading = &folded.issues[&("proj".to_string(), "proj-1".to_string())];
            if heading.deeds() != want {
                report.push_str(&format!("space {label} deeds {:?}\n", heading.deeds()));
            }
            if !folded.conflicts.is_empty() {
                report.push_str(&format!("space {label} clash {:?}\n", folded.conflicts));
            }
        });
        let again = reencode(
            &heading_of(&fold(&ops).expect(label)),
            OpKind::AddDeed {
                accession: "zz".to_string(),
            },
        );
        let again_ids = again.issues[&("proj".to_string(), "proj-1".to_string())].deeds();
        if again_ids != want {
            report.push_str(&format!("space {label} fixed {again_ids:?}\n"));
        }
    }

    // A choice with a space or a colon stays the text parse_ballot returns.
    for choice in ["keep both", "ship it", "ship: after the audit"] {
        let root = wire(&Op::create("proj", "proj-1", "Vote", "seat").unwrap());
        let cast = wire(&spawned(
            &root,
            OpKind::Ballot {
                voter: "ann".to_string(),
                rev: String::new(),
                choice: choice.to_string(),
                stamp: "[2026-01-01 Thu]".to_string(),
            },
            1,
        ));
        let ops = [root, cast];
        let line = format!("[2026-01-01 Thu] ann: {choice}");
        both_ways(&ops, |folded| {
            let text = drawers(folded, "proj-1");
            if !line_is(&text, &line) || text.contains(&format!("ann: \"{choice}\"")) {
                report.push_str(&format!("choice {choice} {text}\n"));
            }
            if !folded.conflicts.is_empty() {
                report.push_str(&format!("choice {choice} clash {:?}\n", folded.conflicts));
            }
        });
        let again = reencode(
            &heading_of(&fold(&ops).expect(choice)),
            OpKind::Ballot {
                voter: "ann".to_string(),
                rev: String::new(),
                choice: choice.to_string(),
                stamp: "[2026-01-01 Thu]".to_string(),
            },
        );
        if !line_is(&drawers(&again, "proj-1"), &line) {
            report.push_str(&format!(
                "choice fixed {choice} {}\n",
                drawers(&again, "proj-1")
            ));
        }
    }
    let mut legacy_vote = plain_heading("proj-1", "Genesis");
    legacy_vote.extra_drawers =
        vec![":VOTES:\n[2026-01-01 Thu] ann: ship: after the audit\n:END:\n".to_string()];
    let legacy_born = genesis_of(legacy_vote);
    let legacy_line = "[2026-01-01 Thu] ann: ship: after the audit";
    let bob = wire(&spawned(
        &legacy_born,
        OpKind::Ballot {
            voter: "bob".to_string(),
            rev: String::new(),
            choice: "yes".to_string(),
            stamp: "[2026-01-03 Sat]".to_string(),
        },
        1,
    ));
    let legacy_ops = [legacy_born, bob];
    both_ways(&legacy_ops, |folded| {
        let text = drawers(folded, "proj-1");
        if !line_is(&text, legacy_line) || text.contains("ann: \"ship: after the audit\"") {
            report.push_str(&format!("legacy choice {text}\n"));
        }
        if !folded.conflicts.is_empty() {
            report.push_str(&format!("legacy choice clash {:?}\n", folded.conflicts));
        }
    });
    let recast = reencode(
        &heading_of(&fold(&legacy_ops).expect("legacy choice")),
        OpKind::Ballot {
            voter: "ann".to_string(),
            rev: String::new(),
            choice: "ship: after the audit".to_string(),
            stamp: "[2026-02-01 Sun]".to_string(),
        },
    );
    let recast_text = drawers(&recast, "proj-1");
    if !recast_text.contains("ann: ship: after the audit")
        || recast_text.contains("ann: \"ship: after the audit\"")
    {
        report.push_str(&format!("recast choice {recast_text}\n"));
    }

    // A newline in the voter, revision, or stamp stays one line.
    let break_rows: [(&str, OpKind, &str, &str); 3] = [
        (
            "voter",
            OpKind::Ballot {
                voter: "an\nn".to_string(),
                rev: String::new(),
                choice: "yes".to_string(),
                stamp: "[2026-01-01 Thu]".to_string(),
            },
            "[2026-01-01 Thu] \"an\\nn\": yes",
            "n: yes",
        ),
        (
            "rev",
            OpKind::Ballot {
                voter: "ann".to_string(),
                rev: "a\nb".to_string(),
                choice: "yes".to_string(),
                stamp: "[2026-01-01 Thu]".to_string(),
            },
            "[rev=\"a\\nb\"] [2026-01-01 Thu] ann: yes",
            "b] [2026-01-01 Thu] ann: yes",
        ),
        (
            "stamp",
            OpKind::Ballot {
                voter: "ann".to_string(),
                rev: String::new(),
                choice: "yes".to_string(),
                stamp: "[2026-01-01 Thu]\nextra".to_string(),
            },
            "\"[2026-01-01 Thu]\\nextra\" ann: yes",
            "extra ann: yes",
        ),
    ];
    for (label, kind, line, fragment) in break_rows {
        let root = wire(&Op::create("proj", "proj-1", "Vote", "seat").unwrap());
        let cast = wire(&spawned(&root, kind.clone(), 1));
        let ops = [root, cast];
        both_ways(&ops, |folded| {
            let text = drawers(folded, "proj-1");
            if !line_is(&text, line) || line_is(&text, fragment) {
                report.push_str(&format!("break {label} {text}\n"));
            }
            if !folded.conflicts.is_empty() {
                report.push_str(&format!("break {label} clash {:?}\n", folded.conflicts));
            }
        });
        let again = reencode(&heading_of(&fold(&ops).expect(label)), kind);
        let again_text = drawers(&again, "proj-1");
        if !line_is(&again_text, line) || line_is(&again_text, fragment) {
            report.push_str(&format!("break fixed {label} {again_text}\n"));
        }
    }

    // An empty target id or an empty field name is kept.
    let empty_root = wire(&Op::create("proj", "proj-1", "Empty", "seat").unwrap());
    let empty_id = wire(&spawned(
        &empty_root,
        OpKind::SetTarget {
            target_id: String::new(),
            fields: vec![("status".to_string(), "open".to_string())],
        },
        1,
    ));
    let empty_ops = [empty_root, empty_id];
    let empty_line = "\"\" \"status\" \"open\"";
    both_ways(&empty_ops, |folded| {
        let text = drawers(folded, "proj-1");
        if !line_is(&text, empty_line) {
            report.push_str(&format!("empty id {text}\n"));
        }
        if !folded.conflicts.is_empty() {
            report.push_str(&format!("empty id clash {:?}\n", folded.conflicts));
        }
    });
    let empty_again = reencode(
        &heading_of(&fold(&empty_ops).expect("empty id")),
        OpKind::SetTarget {
            target_id: String::new(),
            fields: vec![("status".to_string(), "open".to_string())],
        },
    );
    if !line_is(&drawers(&empty_again, "proj-1"), empty_line) {
        report.push_str(&format!(
            "empty id fixed {}\n",
            drawers(&empty_again, "proj-1")
        ));
    }
    let named_root = wire(&Op::create("proj", "proj-1", "Empty", "seat").unwrap());
    let named = wire(&spawned(
        &named_root,
        OpKind::SetTarget {
            target_id: "t1".to_string(),
            fields: vec![
                ("status".to_string(), "open".to_string()),
                (String::new(), "hidden".to_string()),
            ],
        },
        1,
    ));
    let named_ops = [named_root, named];
    let named_lines = ["\"t1\" \"status\" \"open\"", "\"t1\" \"\" \"hidden\""];
    both_ways(&named_ops, |folded| {
        let text = drawers(folded, "proj-1");
        for line in named_lines {
            if !line_is(&text, line) {
                report.push_str(&format!("empty field lost {line} in {text}\n"));
            }
        }
        if !folded.conflicts.is_empty() {
            report.push_str(&format!("empty field clash {:?}\n", folded.conflicts));
        }
    });
    let named_again = reencode(
        &heading_of(&fold(&named_ops).expect("empty field")),
        OpKind::SetTarget {
            target_id: "t1".to_string(),
            fields: vec![
                ("status".to_string(), "open".to_string()),
                (String::new(), "hidden".to_string()),
            ],
        },
    );
    let named_again_text = drawers(&named_again, "proj-1");
    for line in named_lines {
        if !line_is(&named_again_text, line) {
            report.push_str(&format!(
                "empty field fixed lost {line} in {named_again_text}\n"
            ));
        }
    }

    // voter `ann:work` and voter `ann` on rev `work` are different ballots.
    let collide_root = wire(&Op::create("proj", "proj-1", "Votes", "seat").unwrap());
    let work_yes = OpKind::Ballot {
        voter: "ann:work".to_string(),
        rev: String::new(),
        choice: "yes".to_string(),
        stamp: "[2026-01-01 Thu]".to_string(),
    };
    let ann_no = OpKind::Ballot {
        voter: "ann".to_string(),
        rev: "work".to_string(),
        choice: "no".to_string(),
        stamp: "[2026-01-02 Fri]".to_string(),
    };
    let work_line = "[2026-01-01 Thu] ann:work: yes";
    let ann_line = "[rev=work] [2026-01-02 Fri] ann: no";
    let causal_yes = wire(&spawned(&collide_root, work_yes.clone(), 1));
    let causal_no = wire(&spawned(&causal_yes, ann_no.clone(), 2));
    both_ways(&[collide_root.clone(), causal_yes, causal_no], |folded| {
        let text = drawers(folded, "proj-1");
        if !line_is(&text, work_line) || !line_is(&text, ann_line) {
            report.push_str(&format!("causal ballots {text}\n"));
        }
        if !folded.conflicts.is_empty() {
            report.push_str(&format!("causal ballots clash {:?}\n", folded.conflicts));
        }
    });
    for (wall_yes, wall_no) in [(1_i64, 2_i64), (2, 1)] {
        let yes = wire(&spawned(&collide_root, work_yes.clone(), wall_yes));
        let no = wire(&spawned(&collide_root, ann_no.clone(), wall_no));
        both_ways(&[collide_root.clone(), yes, no], |folded| {
            let text = drawers(folded, "proj-1");
            if !line_is(&text, work_line) || !line_is(&text, ann_line) {
                report.push_str(&format!("concurrent ballots {text}\n"));
            }
            if folded
                .conflicts
                .iter()
                .any(|clash| clash.field == "ballot:ann:work")
                || !folded.conflicts.is_empty()
            {
                report.push_str(&format!(
                    "concurrent ballots clash {:?}\n",
                    folded.conflicts
                ));
            }
        });
    }
    let mut genesis_votes = plain_heading("proj-1", "Genesis");
    genesis_votes.extra_drawers = vec![
        ":VOTES:\n[2026-01-01 Thu] ann:work: yes\n[rev=work] [2026-01-02 Fri] ann: no\n:END:\n"
            .to_string(),
    ];
    let votes_born = genesis_of(genesis_votes);
    let bob_vote = wire(&spawned(
        &votes_born,
        OpKind::Ballot {
            voter: "bob".to_string(),
            rev: String::new(),
            choice: "yes".to_string(),
            stamp: "[2026-01-03 Sat]".to_string(),
        },
        1,
    ));
    let votes_ops = [votes_born, bob_vote];
    both_ways(&votes_ops, |folded| {
        let text = drawers(folded, "proj-1");
        for line in [work_line, ann_line, "[2026-01-03 Sat] bob: yes"] {
            if !line_is(&text, line) {
                report.push_str(&format!("genesis ballots lost {line} in {text}\n"));
            }
        }
        if !folded.conflicts.is_empty() {
            report.push_str(&format!("genesis ballots clash {:?}\n", folded.conflicts));
        }
    });
    let votes_again = reencode(
        &heading_of(&fold(&votes_ops).expect("genesis ballots")),
        OpKind::Ballot {
            voter: "bob".to_string(),
            rev: String::new(),
            choice: "yes".to_string(),
            stamp: "[2026-01-03 Sat]".to_string(),
        },
    );
    let votes_again_text = drawers(&votes_again, "proj-1");
    for line in [work_line, ann_line, "[2026-01-03 Sat] bob: yes"] {
        if !line_is(&votes_again_text, line) {
            report.push_str(&format!(
                "genesis ballots fixed lost {line} in {votes_again_text}\n"
            ));
        }
    }

    assert!(report.is_empty(), "{report}");
}
