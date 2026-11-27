//! A target record, a change's target list, and a land-field clash.

#![allow(missing_docs)]

use std::fs;
use std::path::Path;
use std::process::Command;

fn vissue(root: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vissue"));
    cmd.env("VISSUE_NO_ROUTE", "1");
    cmd.env("VISSUE_AGENT", "ada");
    let mut argv = vec!["--root", root.to_str().unwrap()];
    argv.extend_from_slice(args);
    cmd.args(argv).output().expect("run vissue")
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

fn run(root: &Path, args: &[&str]) -> String {
    let out = vissue(root, args);
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

fn tracker() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("Software")).unwrap();
    dir
}

/// A target and a change's target list survive a write, a read, and a second
/// write as the same bytes. A retarget records the old id and the new one.
#[test]
fn a_target_and_a_change_target_list_round_trip_byte_for_byte() {
    let dir = tracker();
    let root = dir.path();
    let issue = run(root, &["create", "-p", "api", "Land the kit", "-q"]);
    let issue = issue.trim();

    let write = [
        "target",
        "--repo",
        "github:acme/widgets",
        "--branch",
        "develop",
        "--mode",
        "merge",
        "--lease",
        "strict-base",
        "--quorum",
        "8 seats",
        "--landers",
        "seat:reviewer",
        "--check",
        "ci:required",
        "--owned",
        "upstream",
        "--fork",
        "github:acme/fork",
        "--mirror",
        "gitlab:acme/widgets",
    ];
    let id = run(root, &write);
    let id = id.trim().to_string();
    assert!(id.starts_with("sha256:"), "{id}");

    let path = root.join("Software/targets.org");
    let first = fs::read(&path).unwrap();
    let again = run(root, &write);
    assert_eq!(again.trim(), id);
    let second = fs::read(&path).unwrap();
    assert_eq!(
        first, second,
        "a second write of the same target changed the file"
    );

    let org = run(
        root,
        &[
            "target",
            "--org",
            "--repo",
            "github:acme/widgets",
            "--branch",
            "develop",
        ],
    );
    assert_eq!(org.into_bytes(), first, "the read is not the stored file");
    let text = String::from_utf8(first.clone()).unwrap();
    for needle in [
        "github:acme/widgets",
        "develop",
        "merge",
        "strict-base",
        "8 seats",
        "seat:reviewer",
        "ci:required",
        "upstream",
        "github:acme/fork",
        "gitlab:acme/widgets",
    ] {
        assert!(
            text.contains(needle),
            "stored file missing {needle}: {text}"
        );
    }

    run(root, &["target", issue, "--add", &id]);
    let board = root.join("Software/api/issues.org");
    let listed = fs::read(&board).unwrap();
    let listed_again = {
        run(root, &["target", issue, "--add", &id]);
        fs::read(&board).unwrap()
    };
    assert_eq!(
        listed, listed_again,
        "adding the same target rewrote the issue"
    );
    let listed_text = String::from_utf8(listed).unwrap();
    assert!(
        listed_text.contains(&id),
        "the change's target list dropped {id}: {listed_text}"
    );

    let other = run(
        root,
        &[
            "target",
            "--repo",
            "github:acme/widgets",
            "--branch",
            "release",
        ],
    );
    let other = other.trim().to_string();
    let moved = run(root, &["target", issue, "--from", &id, "--to", &other]);
    assert!(moved.contains(&id) && moved.contains(&other), "{moved}");
    let after = fs::read_to_string(&board).unwrap();
    let targets_line = after
        .lines()
        .find(|line| line.contains(":TARGETS:"))
        .unwrap_or("");
    assert!(targets_line.contains(&other), "{targets_line}");
    assert!(!targets_line.contains(&id), "{targets_line}");
    assert!(
        after.contains(&format!("retarget {id} -> {other}")),
        "the retarget note is missing: {after}"
    );
}

/// `lease-rewrite` on a main ref is refused, and the same rule on another
/// branch is stored.
#[test]
fn lease_rewrite_on_a_main_ref_is_refused() {
    let dir = tracker();
    let root = dir.path();
    let path = root.join("Software/targets.org");
    for branch in ["main", "refs/heads/main"] {
        let out = vissue(
            root,
            &[
                "target",
                "--repo",
                "github:acme/widgets",
                "--branch",
                branch,
                "--lease",
                "lease-rewrite",
            ],
        );
        refused(
            &out,
            &format!("lease-rewrite is refused on a main ref: {branch}"),
        );
        assert!(!path.exists(), "a refused main ref was stored");
    }

    let id = run(
        root,
        &[
            "target",
            "--repo",
            "github:acme/widgets",
            "--branch",
            "develop",
            "--lease",
            "lease-rewrite",
        ],
    );
    assert!(id.trim().starts_with("sha256:"), "{id}");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("lease-rewrite"), "{text}");
    assert!(text.contains("develop"), "{text}");
}

/// Two writers setting a land field to different values keep both, and the
/// merge says so.
#[test]
fn a_clash_on_a_land_field_keeps_both_values() {
    let dir = tracker();
    let root = dir.path();
    let issue = run(root, &["create", "-p", "api", "Land the kit", "-q"]);
    let issue = issue.trim();
    let board = root.join("Software/api/issues.org");
    let base = dir.path().join("base.org");
    let ours = dir.path().join("ours.org");
    let theirs = dir.path().join("theirs.org");
    fs::copy(&board, &base).unwrap();

    run(root, &["target", issue, "--quorum", "seats-eight"]);
    fs::copy(&board, &ours).unwrap();
    fs::copy(&base, &board).unwrap();
    run(root, &["target", issue, "--quorum", "seats-five"]);
    fs::copy(&board, &theirs).unwrap();

    let out = vissue(
        root,
        &[
            "merge-driver",
            base.to_str().unwrap(),
            ours.to_str().unwrap(),
            theirs.to_str().unwrap(),
            "api/issues.org",
        ],
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    assert!(err.contains("clash"), "{err}");
    let merged = fs::read_to_string(&ours).unwrap();
    assert!(merged.contains("seats-eight"), "{merged}");
    assert!(merged.contains("seats-five"), "{merged}");
    assert!(merged.contains(":MERGE_CONFLICT:"), "{merged}");
    assert!(merged.contains("QUORUM"), "{merged}");
}
