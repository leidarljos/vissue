//! Concurrency, asserted rather than claimed.
//!
//! The reference says a read-modify-write cycle is serialised per file. That
//! sentence was true of `create`, which had a test, and unverified of everything
//! else. A lost append or a swallowed ballot is invisible afterwards: the file
//! parses, the numbers look plausible, and the writer that lost has already
//! reported success.
//!
//! So this runs every mutating verb at once from separate processes against one
//! file and counts what survived. Separate processes and not threads, because
//! agents are processes and the in-process mutex would carry a thread test on its
//! own while the advisory lock went unexercised.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

fn vissue_cmd() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vissue"));
    cmd.env("VISSUE_NO_ROUTE", "1");
    cmd
}

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    vissue_cmd()
        .args(["--root", root.to_str().unwrap()])
        .args(args)
        .output()
        .expect("spawn")
}

fn stdout_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Every writer lands, whatever verb it used.
#[test]
fn concurrent_writers_of_every_kind_all_land() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("Software")).unwrap();

    // Three targets, so appends, votes and notes hit different headings while
    // creates grow the same file underneath them.
    let mut targets = Vec::new();
    for i in 0..3 {
        let out = run(
            root,
            &["create", "-p", "atlas", "--quiet", &format!("target {i}")],
        );
        assert!(out.status.success(), "seed failed: {}", stdout_of(&out));
        targets.push(stdout_of(&out).trim().to_string());
    }

    const EACH: usize = 8;
    let mut kids = Vec::new();

    for i in 0..EACH {
        // create
        kids.push(
            vissue_cmd()
                .args([
                    "--root",
                    root.to_str().unwrap(),
                    "create",
                    "-p",
                    "atlas",
                    "--quiet",
                    &format!("writer {i}"),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        // append
        kids.push(
            vissue_cmd()
                .args([
                    "--root",
                    root.to_str().unwrap(),
                    "append",
                    &targets[0],
                    "--text",
                    &format!("report-{i}"),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        // vote, one distinct agent each
        kids.push(
            vissue_cmd()
                .env("VISSUE_AGENT", format!("agent-{i:02}"))
                .args([
                    "--root",
                    root.to_str().unwrap(),
                    "vote",
                    &targets[1],
                    "--for",
                    "ship",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        // note
        kids.push(
            vissue_cmd()
                .args([
                    "--root",
                    root.to_str().unwrap(),
                    "note",
                    &targets[2],
                    &format!("noted-{i}"),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }

    for (i, kid) in kids.into_iter().enumerate() {
        let out = kid.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "writer {i} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // The file still parses as one document.
    let listed = run(root, &["list", "--json"]);
    assert!(listed.status.success(), "list failed");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout_of(&listed)).expect("the corpus stopped being JSON");
    let rows = parsed.as_array().map(Vec::len).unwrap_or(0);
    assert_eq!(
        rows,
        3 + EACH,
        "headings were lost: {rows} rows for {} writers plus 3 seeds",
        EACH
    );

    // Every append survived.
    let body = run(root, &["show", &targets[0], "--org"]);
    let text = stdout_of(&body);
    for i in 0..EACH {
        assert!(
            text.contains(&format!("report-{i}")),
            "append report-{i} was lost:\n{text}"
        );
    }

    // Every ballot survived, one per agent.
    let tally = run(root, &["vote", &targets[1]]);
    let tally = stdout_of(&tally);
    assert!(
        tally.contains(&format!("{EACH} votes from 1 option")),
        "ballots were lost: {tally}"
    );
    for i in 0..EACH {
        assert!(
            tally.contains(&format!("agent-{i:02}")),
            "agent-{i:02} lost its ballot: {tally}"
        );
    }

    // Every note survived.
    let noted = run(root, &["show", &targets[2], "--org"]);
    let noted = stdout_of(&noted);
    for i in 0..EACH {
        assert!(
            noted.contains(&format!("noted-{i}")),
            "note noted-{i} was lost:\n{noted}"
        );
    }
}

/// Ids stay unique when creates race across two roots for one project name,
/// which is the case the per-file lock cannot see on its own.
///
/// The seed is pinned, and that is what gives this test power.
///
/// An earlier version left the seed to the clock. Two racing creates then drew
/// from 36^4 suffixes and never collided by luck, so it passed with the
/// reservation bug in place and against a no-op lock: it proved nothing. With
/// `VISSUE_ID_SEED` fixed, both racers probe the same suffix first, so a stale
/// reservation collides on the first attempt rather than never.
#[test]
fn creates_racing_across_two_roots_do_not_share_an_id() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    for root in [&first, &second] {
        fs::create_dir_all(root.join("Software")).unwrap();
    }
    // Each side runs on its own root and names the other as a layout, so both
    // mint into their own file while `atlas` exists on two layouts. That is the
    // twin-file case: the per-file lock does not span it, and the reservation has
    // to.
    let cfg_a = dir.path().join("a.toml");
    let cfg_b = dir.path().join("b.toml");
    fs::write(
        &cfg_a,
        format!(
            "[layouts.other]\nroot = \"{}\"\nprefix = \"Software\"\n",
            second.display()
        ),
    )
    .unwrap();
    fs::write(
        &cfg_b,
        format!(
            "[layouts.other]\nroot = \"{}\"\nprefix = \"Software\"\n",
            first.display()
        ),
    )
    .unwrap();

    const ROUNDS: usize = 12;
    let mut kids = Vec::new();
    for i in 0..ROUNDS {
        for (root, cfg) in [(&first, &cfg_a), (&second, &cfg_b)] {
            kids.push(
                Command::new(env!("CARGO_BIN_EXE_vissue"))
                    .env("VISSUE_CONFIG", cfg)
                    .env("VISSUE_ID_SEED", "424242")
                    .args([
                        "--root",
                        root.to_str().unwrap(),
                        "create",
                        "-p",
                        "atlas",
                        "--quiet",
                        &format!("racer {i}"),
                    ])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap(),
            );
        }
    }
    let mut ids = Vec::new();
    for kid in kids {
        let out = kid.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "racer failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        ids.push(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        ids.len(),
        "an id was minted twice across the two roots: {ids:?}"
    );
}

/// Sequences stay unique when separate processes emit at once.
///
/// The events log has its own lock, a process mutex plus an advisory lock, and
/// its existing test spawns threads. Threads exercise the mutex and leave the
/// advisory half unexercised, so two agent processes could have taken the same
/// sequence and nothing would have said so. Agents are processes.
#[test]
fn concurrent_processes_do_not_reuse_an_event_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("Software")).unwrap();

    // One project per subject, and that is the point rather than tidiness. With
    // every subject in one file the issues lock serialises the whole pipeline and
    // the events lock never contends: a version of this test that used one project
    // passed with the advisory lock removed, for that reason. Separate files take
    // separate issues locks, so the emissions can actually race for the sequence.
    const N: usize = 24;
    let mut ids = Vec::new();
    for i in 0..N {
        let out = run(
            root,
            &[
                "create",
                "-p",
                &format!("proj{i:02}"),
                "--quiet",
                &format!("subject {i}"),
            ],
        );
        assert!(out.status.success(), "seed failed: {}", stdout_of(&out));
        ids.push(stdout_of(&out).trim().to_string());
    }

    // Several state changes per process, so the windows genuinely overlap: one
    // event per process launched in a loop does not contend, and a version of
    // this test that did exactly that passed with the advisory lock removed.
    let mut kids = Vec::new();
    for id in &ids {
        for state in ["STARTED", "BLOCKED", "STARTED", "TODO"] {
            kids.push(
                vissue_cmd()
                    .args([
                        "--root",
                        root.to_str().unwrap(),
                        "update",
                        id,
                        "--state",
                        state,
                    ])
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap(),
            );
        }
    }
    for kid in kids {
        let out = kid.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "update failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let listed = run(root, &["events", "-n", "200"]);
    assert!(listed.status.success(), "events failed");
    let text = stdout_of(&listed);
    let seqs: Vec<&str> = text
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|tok| tok.chars().all(|c| c.is_ascii_digit()) && !tok.is_empty())
        .collect();
    assert!(
        seqs.len() >= N,
        "expected at least {N} events, parsed {}: {text}",
        seqs.len()
    );
    let mut unique = seqs.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        seqs.len(),
        "a sequence was handed out twice: {seqs:?}"
    );
}

/// Exactly one agent wins a contested claim.
///
/// This is the property several agents on one tracker actually depend on, and it
/// is not the same as the file staying intact. Claiming is how an agent says "mine,
/// nobody else start this". If two claims can both succeed, both agents do the work
/// and the file is perfectly well formed the whole time, so no integrity test would
/// ever notice.
///
/// `claim_as` does the read and the write inside one lock and returns a conflict to
/// the loser, which is the right shape; this is the test that it holds when the
/// claims are simultaneous rather than sequential.
#[test]
fn exactly_one_agent_wins_a_contested_claim() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("Software")).unwrap();

    let out = run(root, &["create", "-p", "atlas", "--quiet", "contested"]);
    assert!(out.status.success());
    let id = stdout_of(&out).trim().to_string();

    const AGENTS: usize = 12;
    let mut kids = Vec::new();
    for i in 0..AGENTS {
        kids.push(
            vissue_cmd()
                .env("VISSUE_AGENT", format!("agent-{i:02}"))
                .args(["--root", root.to_str().unwrap(), "claim", &id])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }

    let mut winners = Vec::new();
    let mut losers = 0;
    for kid in kids {
        let out = kid.wait_with_output().unwrap();
        if out.status.success() {
            winners.push(String::from_utf8_lossy(&out.stdout).to_string());
        } else {
            let err = String::from_utf8_lossy(&out.stderr).to_string();
            assert!(
                err.contains("claim") || err.contains("held"),
                "a claim failed for the wrong reason: {err}"
            );
            losers += 1;
        }
    }

    assert_eq!(
        winners.len(),
        1,
        "{} agents think they hold the same issue; {losers} were refused",
        winners.len()
    );
    assert_eq!(losers, AGENTS - 1, "some agent neither won nor was refused");

    // And the file agrees with whoever won, once.
    let claims = run(root, &["claims"]);
    let text = stdout_of(&claims);
    let holders = text.matches(&id).count();
    assert_eq!(
        holders, 1,
        "the claim list shows the issue {holders} times: {text}"
    );
}
