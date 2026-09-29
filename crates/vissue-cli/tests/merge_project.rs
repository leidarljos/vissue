//! The merge driver under a real `git merge`, and `vissue project` over a
//! repository that projects its own tracker.

#![allow(missing_docs)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn vissue_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vissue"))
        .env("VISSUE_NO_ROUTE", "1")
        .env("VISSUE_AGENT", "brio")
        .current_dir(dir)
        .args(["--root", dir.to_str().unwrap()])
        .args(args)
        .output()
        .expect("run vissue")
}

fn git(dir: &Path, args: &[&str]) -> Output {
    let out = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=brio",
            "-c",
            "user.email=brio@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// A tracker repository with two issues on `main`.
fn tracker_repo() -> (tempfile::TempDir, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("Software")).unwrap();
    let first = vissue_in(root, &["create", "-p", "alpha", "-q", "first"]);
    assert!(first.status.success(), "{}", text(&first));
    let second = vissue_in(root, &["create", "-p", "alpha", "-q", "second"]);
    assert!(second.status.success(), "{}", text(&second));
    git(root, &["init", "-q"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "two issues"]);
    (
        dir,
        String::from_utf8_lossy(&first.stdout).trim().to_string(),
        String::from_utf8_lossy(&second.stdout).trim().to_string(),
    )
}

#[test]
fn the_driver_installs_once_and_merges_two_branches_by_heading() {
    let (dir, first, second) = tracker_repo();
    let root = dir.path();

    let install = vissue_in(root, &["merge-driver", "--install"]);
    assert!(install.status.success(), "{}", text(&install));
    assert!(
        text(&install).contains("registered as"),
        "{}",
        text(&install)
    );
    let again = vissue_in(root, &["merge-driver", "--install"]);
    assert!(again.status.success(), "{}", text(&again));
    let attrs = fs::read_to_string(root.join(".gitattributes")).unwrap();
    assert_eq!(
        attrs.matches("issues.org merge=vissue").count(),
        1,
        "a second install adds no second line: {attrs}"
    );
    let driver = git(root, &["config", "--local", "merge.vissue.driver"]);
    assert!(String::from_utf8_lossy(&driver.stdout).contains("merge-driver %O %A %B %P"));
    git(root, &["add", ".gitattributes"]);
    git(root, &["commit", "-q", "-m", "merge issues.org by heading"]);

    git(root, &["checkout", "-q", "-b", "side"]);
    let note = vissue_in(root, &["note", &first, "noted on the side branch"]);
    assert!(note.status.success(), "{}", text(&note));
    git(root, &["commit", "-q", "-am", "side note"]);
    git(root, &["checkout", "-q", "main"]);
    let note = vissue_in(root, &["note", &second, "noted on main"]);
    assert!(note.status.success(), "{}", text(&note));
    git(root, &["commit", "-q", "-am", "main note"]);

    git(root, &["merge", "-q", "--no-edit", "side"]);
    let merged = fs::read_to_string(root.join("Software/alpha/issues.org")).unwrap();
    assert!(merged.contains("noted on the side branch"), "{merged}");
    assert!(merged.contains("noted on main"), "{merged}");
    assert!(!merged.contains("<<<<<<<"), "no text conflict: {merged}");
}

#[test]
fn the_driver_leaves_a_text_conflict_when_a_side_does_not_parse() {
    let (dir, _, _) = tracker_repo();
    let root = dir.path();
    let tracker = fs::read_to_string(root.join("Software/alpha/issues.org")).unwrap();
    let base = root.join("base.org");
    let ours = root.join("ours.org");
    let theirs = root.join("theirs.org");
    fs::write(&base, &tracker).unwrap();
    fs::write(&ours, &tracker).unwrap();
    fs::write(&theirs, format!("{tracker}* TODO [#A] no id here\n")).unwrap();
    let out = vissue_in(
        root,
        &[
            "merge-driver",
            base.to_str().unwrap(),
            ours.to_str().unwrap(),
            theirs.to_str().unwrap(),
            "Software/alpha/issues.org",
        ],
    );
    let said = text(&out);
    assert!(said.contains("leaving git's text conflict"), "{said}");
    assert!(
        !out.status.success(),
        "a text conflict is not a clean merge"
    );

    let missing = vissue_in(root, &["merge-driver", base.to_str().unwrap()]);
    assert!(!missing.status.success());
    assert!(
        text(&missing).contains("pass BASE OURS THEIRS"),
        "{}",
        text(&missing)
    );
}

#[test]
fn project_writes_the_mirror_and_check_then_reads_it_fresh() {
    let (dir, first, _) = tracker_repo();
    let root = dir.path();
    fs::write(
        root.join("vissue.toml"),
        "prefix = \"Software\"\n\n[[projection.board]]\nproject = \"alpha\"\nmirror = \"share/alpha-mirror.org\"\n\n[[projection.board]]\nproject = \"beta\"\nsource = \"/nonexistent/tracker\"\nmirror = \"share/beta.org\"\n",
    )
    .unwrap();

    let stale = vissue_in(root, &["project", "--check"]);
    assert!(!stale.status.success(), "a missing mirror is stale");
    assert!(
        text(&stale).contains("does not exist yet"),
        "{}",
        text(&stale)
    );

    let run = vissue_in(root, &["project"]);
    assert!(run.status.success(), "{}", text(&run));
    let said = text(&run);
    assert!(
        said.contains("commit and push: share/alpha-mirror.org"),
        "{said}"
    );
    assert!(
        said.contains("1 board(s) have no source on this seat"),
        "{said}"
    );
    let mirror = fs::read_to_string(root.join("share/alpha-mirror.org")).unwrap();
    assert!(mirror.contains(&first), "{mirror}");

    let fresh = vissue_in(root, &["project", "--check"]);
    assert!(fresh.status.success(), "{}", text(&fresh));
}
