//! A per-issue file holding an op from a later vissue still loads.

#![allow(missing_docs)]

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output};

/// A record from a later vissue, with an op this one does not know.
const FUTURE: &str = "%% deeds-add 9\ndeed-file\n";

fn vissue(root: &Path, args: &[&str]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_vissue"))
        .env("VISSUE_NO_ROUTE", "1")
        .args(["--root", root.to_str().unwrap()])
        .args(args)
        .output()
        .expect("run vissue");
    assert!(
        out.status.success(),
        "vissue {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn warnings(out: &Output) -> Vec<String> {
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|line| line.starts_with("[warn]"))
        .map(str::to_string)
        .collect()
}

fn append(path: &Path, text: &str) {
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(text.as_bytes()).unwrap();
}

#[test]
fn an_op_from_a_later_vissue_warns_once_and_stays_as_written() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let stdout = |out: &Output| String::from_utf8_lossy(&out.stdout).trim().to_string();
    let first = stdout(&vissue(root, &["q", "-p", "sample", "First"]));
    let second = stdout(&vissue(root, &["q", "-p", "sample", "Second"]));
    vissue(root, &["migrate-ledger"]);
    let issues = root.join("Software/sample/issues");
    let first_file = issues.join(format!("{first}.org"));
    let second_file = issues.join(format!("{second}.org"));
    append(&first_file, &format!("{FUTURE}{FUTURE}%% state 4\nDONE\n"));
    append(&second_file, FUTURE);
    let written = fs::read(&first_file).unwrap();

    let shown = vissue(root, &["show", &first]);
    let card = stdout(&shown);
    assert!(card.contains("State:    DONE"), "{card}");
    let said = warnings(&shown);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].contains("deeds-add"), "{said:?}");
    assert_eq!(fs::read(&first_file).unwrap(), written);

    let noted = vissue(root, &["note", &first, "after the later op"]);
    assert_eq!(warnings(&noted).len(), 1, "{:?}", warnings(&noted));
    let grown = fs::read(&first_file).unwrap();
    assert!(grown.starts_with(&written), "the file was rewritten");
    let shown = stdout(&vissue(root, &["show", &first]));
    assert!(shown.contains("after the later op"), "{shown}");
}
