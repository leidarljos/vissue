//! Parse, render, parse over the shapes vissue has put on disk.
//!
//! A rewrite of a tracker comes back byte for byte and reads the same, and a
//! note on an issue is the only line it adds: a stamp nobody changed is never
//! rewritten. vissue's drawer and heading writers have not changed shape since
//! 0.6.0 and its logbook writer since 0.3.0, so the fixture vault and the
//! house form cover every release.

#![allow(missing_docs)]

use std::fs;
use std::path::{Path, PathBuf};

use vissue_core::config::{DEFAULT_PREFIX, Layout};
use vissue_core::model::align_tags;
use vissue_core::store::{self, IssueDoc};
use vissue_core::{CreateOpts, LogEntry, TODO_KEYWORDS, ops};

/// Logbook lines found in trackers vissue rewrites.
const STAMPS: &[&str] = &[
    // vissue: a state change, with and without the state it left.
    "- State \"STARTED\" from \"TODO\" [2026-01-14 Wed 09:12]",
    "- State \"TODO\" [2026-01-14 Wed 09:12]",
    // vissue: `note`, a claim release with and without CLAIMED_AT, a bulk
    // release, a merge conflict, and an entry with no state and no note.
    "- Note: \"two consumers still read it\" [2026-01-28 Wed 13:20]",
    "- Note: \"claim released: inky held since [2026-10-11 Sun 05:00]\" [2026-10-12 Mon 05:00]",
    "- Note: \"claim released: inky held since \" [2026-10-12 Mon 05:00]",
    "- Note: \"released by alice: bulk release of holder inky\" [2026-10-12 Mon 05:10]",
    "- Note: \"merge conflict: title changed on both sides; kept ours, theirs is in the MERGE_CONFLICT drawer\" [2026-10-12 Mon 05:20]",
    "- [2026-10-12 Mon 05:30]",
    // Org clock rows, closed and running.
    "CLOCK: [2026-01-14 Wed 09:12]--[2026-01-14 Wed 10:42] =>  1:30",
    "   CLOCK: [2026-07-26 Sun 17:40]",
    // ljos seat stamps: a bare seat, SEAT-TAG, SEAT@TAG, and one with no time.
    "- Claimed by inky [2026-10-18 Sun 07:00]",
    "- Claimed by inky-n9o [2026-10-18 Sun 07:00]",
    "- Finished by saddle@main [2026-10-18 Sun 07:10]",
    "- claimed by worker-1",
    // The same stamps after vissue 0.21 or older folded them into notes.
    "- Note: \"Claimed by inky-n9o\" [2026-10-18 Sun 07:00]",
    "- Note: \"Finished by saddle@main\" [2026-10-18 Sun 07:10]",
];

/// Where vissue starts a drawer value: one column past `:CLAIMED_BY:`, the
/// column every release from 0.3.0 on has written.
const DRAWER_VALUE_COLUMN: usize = 13;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixture_vault")
}

fn first_difference(want: &str, got: &str) -> String {
    let (want_lines, got_lines): (Vec<_>, Vec<_>) = (want.lines().collect(), got.lines().collect());
    let at = want_lines
        .iter()
        .zip(&got_lines)
        .position(|(a, b)| a != b)
        .unwrap_or(want_lines.len().min(got_lines.len()));
    format!(
        "line {}: on disk {:?}, rewritten {:?}",
        at + 1,
        want_lines.get(at),
        got_lines.get(at)
    )
}

/// The whole contract on one tracker text.
fn assert_round_trip(label: &str, text: &str) {
    let first = IssueDoc::parse("rt", PathBuf::from("issues.org"), text).unwrap();
    let rendered = first.render_string();
    assert!(
        rendered == text,
        "{label}: a rewrite changed the file, {}",
        first_difference(text, &rendered)
    );
    let second = IssueDoc::parse("rt", PathBuf::from("issues.org"), &rendered).unwrap();
    assert!(
        second.headings == first.headings,
        "{label}: the rewrite reads differently"
    );

    let note = LogEntry {
        timestamp: "[2026-11-03 Tue 00:00]".into(),
        from_state: None,
        to_state: None,
        note: Some("re-rendered".into()),
        raw: None,
    };
    let mut noted = first.clone();
    for heading in &mut noted.headings {
        heading.logbook.insert(0, note.clone());
    }
    let out = noted.render_string();
    let without_note = out
        .replace(&format!("{}\n", note.render()), "")
        .replace(":LOGBOOK:\n:END:\n", "");
    assert!(
        without_note == text,
        "{label}: a note rewrote another line, {}",
        first_difference(text, &without_note)
    );
}

#[test]
fn fixture_trackers_with_every_stamp_form_round_trip() {
    let layout = Layout::new(fixture_root(), DEFAULT_PREFIX);
    let projects = store::list_projects(&layout).unwrap();
    assert!(!projects.is_empty());
    for project in projects {
        let text = fs::read_to_string(layout.project_issues_path(&project)).unwrap();
        assert_round_trip(&project, &text);
        for stamp in STAMPS {
            let stamped = text.replace(":LOGBOOK:\n", &format!(":LOGBOOK:\n{stamp}\n"));
            assert_round_trip(&format!("{project} with {stamp:?}"), &stamped);
        }
        let all = text.replace(
            ":LOGBOOK:\n",
            &format!(":LOGBOOK:\n{}\n", STAMPS.join("\n")),
        );
        assert_round_trip(&format!("{project} with every stamp"), &all);
    }
}

const PRIORITIES: &[char] = &['A', 'B', 'C'];
const TITLES: &[&str] = &[
    "Parse the manifest header",
    "Scope: the header block",
    "Rename the key :needs-review:",
    "[#not a cookie] stays",
    "Carry the manifest parser through every malformed record the fuzzer turned up",
];
const STATISTICS: &[Option<&str>] = &[None, Some("[1/2]"), Some("[50%]")];
const TAGS: &[&[&str]] = &[&[], &["bug"], &["docs", "retry"]];
const PLANNING: &[Option<&str>] = &[
    None,
    Some("DEADLINE: <2026-03-01 Sun>"),
    Some("SCHEDULED: <2026-02-10 Tue> DEADLINE: <2026-03-01 Sun +1w -2d>"),
    Some("CLOSED: [2026-01-08 Thu 16:40] SCHEDULED: <2026-02-10 Tue>--<2026-02-12 Thu>"),
];
const PROPERTIES: &[&[(&str, &str)]] = &[
    &[("CREATED", "[2026-01-12 Mon]")],
    &[
        ("CREATED", "[2026-01-12 Mon]"),
        ("CLAIMED_BY", "inky-n9o"),
        ("CLAIMED_AT", "[2026-10-11 Sun 05:00]"),
    ],
    &[
        ("CLAIMED_BY", "saddle@main"),
        ("CLAIMED_AT", "[2026-10-11 Sun 05:00]"),
        ("BLOCKED_BY", "gen-0001,gen-0002"),
        ("VISSUE_TAGS", "needs-review"),
        ("DEEDS", "deed-patch-config-key"),
        ("Effort", "1:30"),
    ],
];
const LOGBOOKS: &[Option<usize>] = &[None, Some(1), Some(STAMPS.len())];
const EXTRA_DRAWERS: &[&str] = &["", ":VOTES:\n[2026-01-01 Thu] inky: ship\n:END:\n"];
const BODIES: &[&str] = &[
    "",
    "Scope: one row per parsed record, counts only.",
    "Done-when: a malformed header names the line.\n - a list item\n * a bullet a write indented\n** A child heading written by hand",
];

/// The n-th combination of a run of choices, one mixed-radix digit each.
struct Digits(usize);

impl Digits {
    fn of<'a, T>(&mut self, choices: &'a [T]) -> &'a T {
        let pick = &choices[self.0 % choices.len()];
        self.0 /= choices.len();
        pick
    }
}

fn drawer_line(key: &str, value: &str) -> String {
    let key = format!(":{key}:");
    let pad = DRAWER_VALUE_COLUMN.saturating_sub(key.len()).max(1);
    format!("{key}{}{value}\n", " ".repeat(pad))
}

/// Heading `i` in the house form: the i-th combination of the heading-line
/// choices and, wrapping, the i-th combination of the choices below it.
fn generated_heading(i: usize) -> String {
    let mut line = Digits(i);
    let state = line.of(TODO_KEYWORDS);
    let priority = line.of(PRIORITIES);
    let title = line.of(TITLES);
    let statistics = line.of(STATISTICS);
    let tags: Vec<String> = line.of(TAGS).iter().map(|t| t.to_string()).collect();

    let mut below = Digits(i);
    let planning = below.of(PLANNING);
    let properties = below.of(PROPERTIES);
    let logbook = below.of(LOGBOOKS);
    let extra = below.of(EXTRA_DRAWERS);
    let body = below.of(BODIES);

    let mut stem = format!("* {state} [#{priority}] {title}");
    if let Some(cookie) = statistics {
        stem.push(' ');
        stem.push_str(cookie);
    }
    let mut out = format!("{}\n", align_tags(&stem, &tags));
    if let Some(planning) = planning {
        out.push_str(planning);
        out.push('\n');
    }
    out.push_str(":PROPERTIES:\n");
    out.push_str(&drawer_line("ID", &format!("gen-{i:04}")));
    // A type Org can carry is a heading tag as well, which is where every
    // rewrite since 0.6.0 has put it.
    if let Some(kind) = tags.first() {
        out.push_str(&drawer_line("TYPE", kind));
    }
    for (key, value) in *properties {
        out.push_str(&drawer_line(key, value));
    }
    out.push_str(":END:\n");
    if let Some(count) = logbook {
        out.push_str(":LOGBOOK:\n");
        for k in 0..*count {
            out.push_str(STAMPS[(i + k) % STAMPS.len()]);
            out.push('\n');
        }
        out.push_str(":END:\n");
    }
    out.push_str(extra);
    if !body.is_empty() {
        out.push('\n');
        out.push_str(body);
        out.push('\n');
    }
    out.push('\n');
    out
}

#[test]
fn generated_headings_round_trip() {
    let line_choices =
        TODO_KEYWORDS.len() * PRIORITIES.len() * TITLES.len() * STATISTICS.len() * TAGS.len();
    let below_choices =
        PLANNING.len() * PROPERTIES.len() * LOGBOOKS.len() * EXTRA_DRAWERS.len() * BODIES.len();
    let mut text = format!("{}\n\n", store::default_preamble("gen").trim_end());
    for i in 0..line_choices.max(below_choices) {
        text.push_str(&generated_heading(i));
    }
    assert_round_trip("generated tracker", &text);
}

/// An issue a seat claimed, as a tracker carries it after a stamp from ljos
/// and a rewrite by vissue 0.21: the holder in its SEAT-TAG form on the
/// drawer, the seat's own stamp, and the earlier stamp folded into a note.
const HELD: &str = "\
* STARTED [#B] Held by a seat
:PROPERTIES:
:ID:         demo-aaaa
:CLAIMED_BY: inky-n9o
:CLAIMED_AT: [2026-10-11 Sun 05:00]
:END:
:LOGBOOK:
- Claimed by inky-n9o [2026-10-11 Sun 05:00]
- Note: \"Claimed by inky\" [2026-10-10 Sat 04:00]
- Note: \"claim released: brio held since [2026-10-09 Fri 03:00]\" [2026-10-10 Sat 03:59]
- State \"STARTED\" from \"TODO\" [2026-10-09 Fri 03:00]
:END:

Scope: the holder's stamps stay as they were written.

";

#[test]
fn an_old_claimed_by_stamp_survives_a_claim_a_create_and_a_note() {
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
    let path = layout.project_issues_path("demo");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let open = "* TODO [#C] Unclaimed\n:PROPERTIES:\n:ID:         demo-bbbb\n:END:\n\n";
    fs::write(
        &path,
        format!(
            "{}\n\n{HELD}{open}",
            store::default_preamble("demo").trim_end()
        ),
    )
    .unwrap();

    ops::claim_as(&layout, "demo-bbbb", false, "brio").unwrap();
    ops::create(&layout, "demo", "a second issue", CreateOpts::default()).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains(HELD), "a rewrite moved a stamp:\n{text}");

    ops::note(&layout, "demo-aaaa", "picked it up").unwrap();
    let text = fs::read_to_string(&path).unwrap();
    let noted = text
        .lines()
        .find(|line| line.starts_with("- Note: \"picked it up\" ["))
        .expect("the note is written");
    let held_after_note = HELD.replacen(":LOGBOOK:\n", &format!(":LOGBOOK:\n{noted}\n"), 1);
    assert!(
        text.contains(&held_after_note),
        "the note rewrote a stamp:\n{text}"
    );

    let doc = IssueDoc::parse_file("demo", &path).unwrap();
    let held = doc.headings.iter().find(|h| h.id == "demo-aaaa").unwrap();
    assert_eq!(held.claimed_by(), Some("inky-n9o"));
    assert_eq!(held.claimed_at(), Some("[2026-10-11 Sun 05:00]"));
    assert_eq!(
        held.logbook[1].raw.as_deref(),
        Some("- Claimed by inky-n9o [2026-10-11 Sun 05:00]")
    );
    assert_eq!(held.logbook[2].note.as_deref(), Some("Claimed by inky"));
}
