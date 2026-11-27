//! A ballot names the revision it judged, and a property can be looked up.

#![allow(missing_docs)]

use std::fs;
use std::process::Command;

fn vissue(root: &std::path::Path, agent: &str, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vissue"));
    cmd.env("VISSUE_NO_ROUTE", "1");
    cmd.env("VISSUE_AGENT", agent);
    let mut argv = vec!["--root", root.to_str().unwrap()];
    argv.extend_from_slice(args);
    cmd.args(argv).output().expect("run vissue")
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

fn run(root: &std::path::Path, agent: &str, args: &[&str]) -> String {
    let out = vissue(root, agent, args);
    assert!(
        out.status.success(),
        "{}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    stdout(&out)
}

fn refused(out: &std::process::Output, needle: &str) {
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "exit 0, wanted a refusal ({needle}): {err}"
    );
    assert!(err.contains(needle), "stderr missing {needle:?}: {err}");
}

/// A ballot cast with `--rev r1` counts for that revision only, and a recast
/// with `--rev r2` replaces it. The read after `migrate-ledger` is the ledger.
#[test]
fn a_ballot_cast_for_a_rev_is_counted_only_for_that_rev() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("Software")).unwrap();
    let root = dir.path();

    let id = run(root, "alice", &["create", "-p", "api", "Ship it?", "-q"]);
    let id = id.trim();

    let cast = run(root, "alice", &["vote", id, "--for", "ship", "--rev", "r1"]);
    assert!(cast.contains("alice voted ship"), "{cast}");
    let board = fs::read_to_string(root.join("Software/api/issues.org")).unwrap();
    assert!(
        board.lines().any(|line| {
            line.starts_with("[rev=r1] ")
                && line.contains("alice: ship")
                && !line.contains("ship rev=")
        }),
        "the org line dropped the revision: {board}"
    );

    let tally = run(root, "alice", &["vote", id]);
    assert!(tally.contains("ship"), "{tally}");
    assert!(tally.contains("one ballot only"), "{tally}");

    let for_r1: serde_json::Value =
        serde_json::from_str(&run(root, "alice", &["vote", id, "--json", "--rev", "r1"])).unwrap();
    let for_r1 = for_r1.as_array().unwrap();
    assert_eq!(for_r1.len(), 1, "{for_r1:?}");
    assert_eq!(for_r1[0]["agent"], "alice");
    assert_eq!(for_r1[0]["choice"], "ship");
    assert_eq!(for_r1[0]["rev"], "r1");

    let for_r2: serde_json::Value =
        serde_json::from_str(&run(root, "alice", &["vote", id, "--json", "--rev", "r2"])).unwrap();
    assert!(for_r2.as_array().unwrap().is_empty(), "{for_r2}");
    let text_r2 = run(root, "alice", &["vote", id, "--rev", "r2"]);
    assert!(text_r2.contains("no votes"), "{text_r2}");

    run(root, "alice", &["migrate-ledger"]);
    let ledger = fs::read_to_string(root.join(format!("Software/api/issues/{id}.org"))).unwrap();
    assert!(
        ledger.lines().any(|line| {
            line.starts_with("[rev=r1] ")
                && line.contains("alice: ship")
                && !line.contains("ship rev=")
        }),
        "the ledger dropped the revision: {ledger}"
    );
    let again: serde_json::Value =
        serde_json::from_str(&run(root, "alice", &["vote", id, "--json", "--rev", "r1"])).unwrap();
    assert_eq!(again.as_array().unwrap().len(), 1);

    let recast = run(root, "alice", &["vote", id, "--for", "hold", "--rev", "r2"]);
    assert!(recast.contains("changed ship to hold"), "{recast}");
    let dropped: serde_json::Value =
        serde_json::from_str(&run(root, "alice", &["vote", id, "--json", "--rev", "r1"])).unwrap();
    assert!(dropped.as_array().unwrap().is_empty(), "{dropped}");
    let kept: serde_json::Value =
        serde_json::from_str(&run(root, "alice", &["vote", id, "--json", "--rev", "r2"])).unwrap();
    let kept = kept.as_array().unwrap();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0]["choice"], "hold");
    assert_eq!(kept[0]["rev"], "r2");
    let all: serde_json::Value =
        serde_json::from_str(&run(root, "alice", &["vote", id, "--json"])).unwrap();
    assert_eq!(all.as_array().unwrap().len(), 1, "{all}");
}

/// `find --prop KEY=VALUE` returns the issue whose property equals the value.
#[test]
fn find_returns_the_issue_whose_property_equals_the_value() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("Software")).unwrap();
    let root = dir.path();

    let id = run(root, "alice", &["create", "-p", "api", "The one", "-q"]);
    let id = id.trim().to_string();
    let other = run(root, "alice", &["create", "-p", "api", "abc", "-q"]);
    let other = other.trim().to_string();
    let longer = run(root, "alice", &["create", "-p", "api", "The longer", "-q"]);
    let longer = longer.trim().to_string();

    let path = root.join("Software/api/issues.org");
    let text = fs::read_to_string(&path).unwrap();
    let text = text
        .replacen(
            &format!(":ID:         {id}\n"),
            &format!(":ID:         {id}\n:CHANGE:     abc\n"),
            1,
        )
        .replacen(
            &format!(":ID:         {longer}\n"),
            &format!(":ID:         {longer}\n:CHANGE:     abcdef\n"),
            1,
        );
    fs::write(&path, text).unwrap();

    assert_eq!(
        run(root, "alice", &["find", "--prop", "CHANGE=abc"]),
        format!("{id}\n")
    );
    assert_eq!(
        run(root, "alice", &["find", "--prop", "CHANGE=abcdef"]),
        format!("{longer}\n")
    );
    assert_eq!(run(root, "alice", &["find", "--prop", "CHANGE=nope"]), "");
    assert!(!other.is_empty());
    let bad = vissue(root, "alice", &["find", "--prop", "CHANGE"]);
    assert!(!bad.status.success(), "a bare key is not KEY=VALUE");
}

/// A ledger written before revisions keeps choice text that contains `rev=`,
/// including two spaces before it. The tally and the consensus use that text,
/// `--rev r1` does not count it, and a later save leaves the lines unchanged.
#[test]
fn a_base_ledger_keeps_a_choice_that_contains_rev() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("Software")).unwrap();
    let root = dir.path();

    let id = run(root, "alice", &["create", "-p", "api", "Ship it?", "-q"]);
    let id = id.trim();
    let path = root.join("Software/api/issues.org");
    let text = fs::read_to_string(&path).unwrap();
    let id_at = text.find(&format!(":ID:         {id}\n")).expect("id line");
    let end_rel = text[id_at..].find(":END:\n").expect("properties end");
    let insert_at = id_at + end_rel + ":END:\n".len();
    let old = "\
:VOTES:\n\
[2026-01-01 Thu] alice: ship rev=r1\n\
[2026-01-02 Fri] bob: ship  rev=r1\n\
:END:\n";
    let mut next = String::new();
    next.push_str(&text[..insert_at]);
    next.push_str(old);
    next.push_str(&text[insert_at..]);
    fs::write(&path, &next).unwrap();

    let tally = run(root, "carol", &["vote", id]);
    assert!(tally.contains("ship rev=r1"), "{tally}");
    assert!(tally.contains("ship  rev=r1"), "{tally}");
    assert!(
        !tally
            .lines()
            .any(|line| line.contains("holds") && line.contains("ship (")),
        "{tally}"
    );

    let consensus = run(root, "carol", &["consensus", id]);
    assert!(consensus.contains("ship rev=r1"), "{consensus}");
    assert!(consensus.contains("ship  rev=r1"), "{consensus}");
    assert!(!consensus.contains("holds: ship ("), "{consensus}");

    let filtered = run(root, "carol", &["vote", id, "--rev", "r1"]);
    assert!(filtered.contains("no votes"), "{filtered}");
    let json: serde_json::Value =
        serde_json::from_str(&run(root, "carol", &["vote", id, "--json", "--rev", "r1"])).unwrap();
    assert!(json.as_array().unwrap().is_empty(), "{json}");

    let all: serde_json::Value =
        serde_json::from_str(&run(root, "carol", &["vote", id, "--json"])).unwrap();
    let all = all.as_array().unwrap();
    assert_eq!(all.len(), 2, "{all:?}");
    let mut choices: Vec<&str> = all
        .iter()
        .map(|row| row["choice"].as_str().unwrap())
        .collect();
    choices.sort_unstable();
    assert_eq!(choices, ["ship  rev=r1", "ship rev=r1"]);
    assert!(all.iter().all(|row| row["rev"].is_null()), "{all:?}");

    run(root, "carol", &["vote", id, "--for", "later"]);
    let saved = fs::read_to_string(&path).unwrap();
    assert!(
        saved.contains("[2026-01-01 Thu] alice: ship rev=r1\n"),
        "{saved}"
    );
    assert!(
        saved.contains("[2026-01-02 Fri] bob: ship  rev=r1\n"),
        "{saved}"
    );
}

/// `--rev r1` counts the ballot cast for that revision. The same words typed
/// as the choice, with no `--rev`, stay choice text and are not counted.
#[test]
fn a_rev_flag_counts_and_the_same_words_in_the_choice_do_not() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("Software")).unwrap();
    let root = dir.path();

    let id = run(root, "alice", &["create", "-p", "api", "Ship it?", "-q"]);
    let id = id.trim();

    run(root, "alice", &["vote", id, "--for", "ship", "--rev", "r1"]);
    run(root, "bob", &["vote", id, "--for", "ship rev=r1"]);

    let counted: serde_json::Value =
        serde_json::from_str(&run(root, "carol", &["vote", id, "--json", "--rev", "r1"])).unwrap();
    let counted = counted.as_array().unwrap();
    assert_eq!(counted.len(), 1, "{counted:?}");
    assert_eq!(counted[0]["agent"], "alice");
    assert_eq!(counted[0]["choice"], "ship");
    assert_eq!(counted[0]["rev"], "r1");

    let all: serde_json::Value =
        serde_json::from_str(&run(root, "carol", &["vote", id, "--json"])).unwrap();
    let all = all.as_array().unwrap();
    assert_eq!(all.len(), 2, "{all:?}");
    let bob = all.iter().find(|row| row["agent"] == "bob").unwrap();
    assert_eq!(bob["choice"], "ship rev=r1");
    assert!(bob["rev"].is_null(), "{bob}");

    let board = fs::read_to_string(root.join("Software/api/issues.org")).unwrap();
    assert!(
        board
            .lines()
            .any(|line| line.starts_with("[rev=r1] ") && line.contains("alice: ship")),
        "{board}"
    );
    assert!(
        board.lines().any(|line| line.contains("bob: ship rev=r1")),
        "{board}"
    );
}

/// An empty, blank, or control-character `--rev` is an error. A padded token
/// is trimmed the same way on a cast and on `--json`.
#[test]
fn a_blank_or_control_rev_is_refused_and_a_padded_rev_matches() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("Software")).unwrap();
    let root = dir.path();

    let id = run(root, "alice", &["create", "-p", "api", "Ship it?", "-q"]);
    let id = id.trim();

    refused(
        &vissue(root, "alice", &["vote", id, "--for", "ship", "--rev", ""]),
        "--rev is empty",
    );
    refused(
        &vissue(
            root,
            "alice",
            &["vote", id, "--for", "ship", "--rev", "   "],
        ),
        "--rev is empty",
    );
    refused(
        &vissue(
            root,
            "alice",
            &["vote", id, "--for", "ship", "--rev", "a\n"],
        ),
        "control character",
    );
    refused(
        &vissue(
            root,
            "alice",
            &["vote", id, "--for", "ship", "--rev", "\n:END:\n"],
        ),
        "control character",
    );
    refused(
        &vissue(root, "alice", &["vote", id, "--json", "--rev", ""]),
        "--rev is empty",
    );
    refused(
        &vissue(root, "alice", &["vote", id, "--json", "--rev", "a\n"]),
        "control character",
    );

    run(
        root,
        "alice",
        &["vote", id, "--for", "ship", "--rev", "  r1  "],
    );
    let padded: serde_json::Value = serde_json::from_str(&run(
        root,
        "bob",
        &["vote", id, "--json", "--rev", "  r1  "],
    ))
    .unwrap();
    let padded = padded.as_array().unwrap();
    assert_eq!(padded.len(), 1, "{padded:?}");
    assert_eq!(padded[0]["agent"], "alice");
    assert_eq!(padded[0]["rev"], "r1");
    let exact: serde_json::Value =
        serde_json::from_str(&run(root, "bob", &["vote", id, "--json", "--rev", "r1"])).unwrap();
    assert_eq!(exact.as_array().unwrap().len(), 1);
}
