//! `show` prints the newest logbook lines after the body.

#![allow(missing_docs)]

use std::fs;
use std::process::Command;

#[test]
fn show_prints_the_newest_notes_and_points_at_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("Software")).unwrap();
    let run = |args: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_vissue"))
            .env("VISSUE_NO_ROUTE", "1")
            .env("VISSUE_AGENT", "brio")
            .args(["--root", root.to_str().unwrap()])
            .args(args)
            .output()
            .expect("run vissue");
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    let id = run(&["q", "-p", "atlas", "noted"]).trim().to_string();
    let quiet = run(&["show", &id]);
    assert!(!quiet.contains("Log"), "no logbook, no section: {quiet}");

    run(&["note", &id, "found the cause"]);
    let one = run(&["show", &id]);
    assert!(one.contains("Log (newest first):"), "{one}");
    assert!(one.contains("found the cause"), "{one}");

    for n in 0..10 {
        run(&["note", &id, &format!("step {n}")]);
    }
    let many = run(&["show", &id]);
    assert!(many.contains("newest 8 of 11"), "{many}");
    assert!(many.contains("step 9"), "{many}");
    assert!(
        !many.contains("found the cause"),
        "the oldest waits for --json: {many}"
    );
}
