//! `ready` and `list` narrowed by claim, parent and tag, and capped.

#![allow(missing_docs)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn run(root: &Path, agent: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vissue"))
        .env("VISSUE_NO_ROUTE", "1")
        .env("VISSUE_AGENT", agent)
        .args(["--root", root.to_str().unwrap()])
        .args(args)
        .output()
        .expect("run vissue")
}

fn out(o: &Output) -> String {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// A plan with three children, one claimed and one a bug, a blocker outside
/// the plan, and a second project.
fn tracker() -> (tempfile::TempDir, [String; 6]) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("Software")).unwrap();
    let q = |args: &[&str]| out(&run(root, "setup", args)).trim().to_string();
    let plan = q(&["q", "-p", "atlas", "plan"]);
    let free = q(&["q", "-p", "atlas", "--parent", &plan, "free child"]);
    let held = q(&["q", "-p", "atlas", "--parent", &plan, "held child"]);
    let bug = q(&[
        "q",
        "-p",
        "atlas",
        "--parent",
        &plan,
        "-t",
        "bug",
        "bug child",
    ]);
    let outside = q(&["q", "-p", "atlas", "outside blocker"]);
    let other = q(&["q", "-p", "beacon", "other project"]);
    out(&run(root, "alice", &["claim", &held]));
    out(&run(root, "setup", &["update", &bug, "--block", &outside]));
    out(&run(root, "setup", &["update", &other, "--priority", "A"]));
    (dir, [plan, free, held, bug, outside, other])
}

#[test]
fn unclaimed_drops_issues_another_identity_holds() {
    let (dir, [_, free, held, ..]) = tracker();
    let all = out(&run(dir.path(), "bob", &["ready"]));
    assert!(all.contains(&held), "{all}");
    let open = out(&run(dir.path(), "bob", &["ready", "--unclaimed"]));
    assert!(!open.contains(&held), "{open}");
    assert!(open.contains(&free), "{open}");
    let mine = out(&run(dir.path(), "bob", &["list", "--claimed-by", "alice"]));
    assert_eq!(mine.lines().count(), 1, "{mine}");
    assert!(mine.contains(&held), "{mine}");
}

#[test]
fn parent_narrows_after_blockers_are_weighed() {
    let (dir, [plan, free, held, bug, outside, _]) = tracker();
    let kids = out(&run(dir.path(), "bob", &["ready", "--parent", &plan]));
    assert!(kids.contains(&free) && kids.contains(&held), "{kids}");
    // The bug waits on an issue outside the plan, so it is still not ready.
    assert!(!kids.contains(&bug), "{kids}");
    assert!(!kids.contains(&outside), "{kids}");
    let listed = out(&run(dir.path(), "bob", &["list", "--parent", &plan]));
    assert_eq!(listed.lines().count(), 3, "{listed}");
}

#[test]
fn tag_and_limit_narrow_and_cap_in_both_shapes() {
    let (dir, [_, _, _, bug, _, other]) = tracker();
    let bugs = out(&run(dir.path(), "bob", &["list", "--tag", "bug"]));
    assert_eq!(bugs.lines().count(), 1, "{bugs}");
    assert!(bugs.contains(&bug), "{bugs}");
    let one = out(&run(
        dir.path(),
        "bob",
        &["ready", "--limit", "1", "--json"],
    ));
    let rows: Vec<serde_json::Value> = serde_json::from_str(&one).unwrap();
    assert_eq!(rows.len(), 1, "{one}");
    // Across projects the cap keeps the most urgent row.
    assert_eq!(rows[0]["id"], other.as_str(), "{one}");
    let text = out(&run(dir.path(), "bob", &["ready", "--limit", "2"]));
    assert_eq!(text.lines().count(), 2, "{text}");
}

#[test]
fn unclaimed_and_claimed_by_do_not_combine() {
    let (dir, _) = tracker();
    let both = run(
        dir.path(),
        "bob",
        &["ready", "--unclaimed", "--claimed-by", "alice"],
    );
    assert!(!both.status.success());
}
