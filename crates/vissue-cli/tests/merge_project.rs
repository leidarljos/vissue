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

/// A tracker split into per-issue files, the driver installed before the
/// first commit, and one issue to contend over.
fn ledger_repo() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("Software")).unwrap();
    let made = vissue_in(root, &["create", "-p", "alpha", "-q", "contested"]);
    assert!(made.status.success(), "{}", text(&made));
    let split = vissue_in(root, &["migrate-ledger"]);
    assert!(split.status.success(), "{}", text(&split));
    git(root, &["init", "-q"]);
    let install = vissue_in(root, &["merge-driver", "--install"]);
    assert!(install.status.success(), "{}", text(&install));
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "one issue, split"]);
    (
        dir,
        String::from_utf8_lossy(&made.stdout).trim().to_string(),
    )
}

fn vissue_as(dir: &Path, agent: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vissue"))
        .env("VISSUE_NO_ROUTE", "1")
        .env("VISSUE_AGENT", agent)
        .current_dir(dir)
        .args(["--root", dir.to_str().unwrap()])
        .args(args)
        .output()
        .expect("run vissue")
}

#[test]
fn install_names_the_per_issue_files_and_keeps_the_event_log_out_of_git() {
    let (dir, _) = ledger_repo();
    let root = dir.path();
    let attrs = fs::read_to_string(root.join(".gitattributes")).unwrap();
    assert!(attrs.contains("**/issues/*.org merge=vissue"), "{attrs}");
    let ignore = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert!(ignore.contains(".vault-events.*"), "{ignore}");
    let tracked = git(root, &["ls-files"]);
    assert!(
        !String::from_utf8_lossy(&tracked.stdout).contains(".vault-events"),
        "{}",
        String::from_utf8_lossy(&tracked.stdout)
    );
    let again = vissue_in(root, &["merge-driver", "--install"]);
    assert!(again.status.success(), "{}", text(&again));
    let attrs = fs::read_to_string(root.join(".gitattributes")).unwrap();
    assert_eq!(
        attrs.matches("**/issues/*.org merge=vissue").count(),
        1,
        "{attrs}"
    );
}

#[test]
fn two_branches_that_note_and_vote_on_one_split_issue_merge_both() {
    let (dir, id) = ledger_repo();
    let root = dir.path();

    git(root, &["checkout", "-q", "-b", "side"]);
    for args in [
        vec!["note", id.as_str(), "noted by alice"],
        vec!["vote", id.as_str(), "--for", "ship"],
    ] {
        let out = vissue_as(root, "alice", &args);
        assert!(out.status.success(), "{}", text(&out));
    }
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "alice"]);
    git(root, &["checkout", "-q", "main"]);
    for args in [
        vec!["note", id.as_str(), "noted by bob"],
        vec!["vote", id.as_str(), "--for", "hold"],
        vec!["update", id.as_str(), "--state", "DONE"],
    ] {
        let out = vissue_as(root, "bob", &args);
        assert!(out.status.success(), "{}", text(&out));
    }
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "bob"]);

    git(root, &["merge", "-q", "--no-edit", "side"]);
    let file = fs::read_to_string(root.join(format!("Software/alpha/issues/{id}.org"))).unwrap();
    assert!(!file.contains("<<<<<<<"), "no text conflict: {file}");

    let tally = text(&vissue_in(root, &["vote", &id]));
    assert!(tally.contains("2 votes"), "both ballots kept: {tally}");
    assert!(tally.contains("alice") && tally.contains("bob"), "{tally}");
    let shown = text(&vissue_in(root, &["show", &id, "--json"]));
    assert!(shown.contains("noted by alice"), "{shown}");
    assert!(shown.contains("noted by bob"), "{shown}");
    assert!(shown.contains("\"state\": \"DONE\""), "{shown}");
    let check = vissue_in(root, &["check"]);
    assert!(check.status.success(), "{}", text(&check));
}

#[test]
fn a_split_issue_left_with_conflict_markers_is_an_error_not_an_older_issue() {
    let (dir, id) = ledger_repo();
    let root = dir.path();
    let note = vissue_as(root, "alice", &["note", &id, "kept before the clash"]);
    assert!(note.status.success(), "{}", text(&note));
    let path = root.join(format!("Software/alpha/issues/{id}.org"));
    let mut file = fs::read_to_string(&path).unwrap();
    file.push_str("<<<<<<< HEAD\n%% log 10\n- Note: x\n=======\n>>>>>>> side\n");
    fs::write(&path, file).unwrap();

    let shown = vissue_in(root, &["show", &id]);
    assert!(!shown.status.success(), "{}", text(&shown));
    assert!(text(&shown).contains("conflict marker"), "{}", text(&shown));
    let check = vissue_in(root, &["check"]);
    assert!(!check.status.success(), "{}", text(&check));
}

#[test]
fn check_names_conflict_markers_left_in_a_board() {
    let (dir, _, _) = tracker_repo();
    let root = dir.path();
    let path = root.join("Software/alpha/issues.org");
    let mut board = fs::read_to_string(&path).unwrap();
    board.push_str("<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> side\n");
    fs::write(&path, board).unwrap();
    let check = vissue_in(root, &["check"]);
    assert!(!check.status.success(), "{}", text(&check));
    assert!(
        text(&check).contains("git conflict marker"),
        "{}",
        text(&check)
    );
}
