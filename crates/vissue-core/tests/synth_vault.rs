//! `list` and `ready` over the synthetic tracker in `tests/synth_vault`, as the
//! CLI prints them when no project is named: one corpus, asked once per project
//! in name order.
//!
//! The tracker is written by
//! `python3 scripts/bench/synth_tracker.py tests/synth_vault --issues 160 --projects 21 --lead ljos=120`,
//! small enough to commit, with every rule `ready` applies acting on some issue.
//! The files under `expected/` are what `vissue list`, `vissue ready` and
//! `vissue ready --json` print over it with `VISSUE_NO_ROUTE=1`, claim ages
//! masked.

#![allow(missing_docs)]

use std::fs;
use std::path::{Path, PathBuf};

use vissue_core::catalog::{issues_rows_from, load_recs};
use vissue_core::config::Layout;
use vissue_core::report;
use vissue_core::store;
use vissue_core::views::{IssueRec, ListQuery};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/synth_vault")
}

fn expected(name: &str) -> String {
    fs::read_to_string(root().join("expected").join(name)).unwrap()
}

/// Each project's answer over the whole corpus, in the order the CLI prints them.
fn every_project<T>(each: impl Fn(&[IssueRec], &str) -> T) -> Vec<T> {
    let layout = Layout::new(root(), "Issues");
    let recs = load_recs(&layout).unwrap();
    let mut projects = store::list_projects(&layout).unwrap();
    projects.sort_by_key(|p| p.to_lowercase());
    projects.iter().map(|p| each(&recs, p)).collect()
}

/// A claim suffix counts the days since the claim, which grows with the date
/// the test runs on; the expected files hold `?d` there.
fn mask_claim_age(text: &str) -> String {
    const AT: &str = "(claimed ";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(AT) {
        let (head, tail) = rest.split_at(at + AT.len());
        out.push_str(head);
        let digits = tail.len() - tail.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits > 0 && tail[digits..].starts_with("d by ") {
            out.push('?');
        }
        rest = &tail[digits..];
    }
    out.push_str(rest);
    out
}

#[test]
fn list_prints_every_project_as_expected_on_the_synthetic_tracker() {
    let text =
        every_project(|recs, p| report::list_in(recs, Some(p), None, false).unwrap()).concat();
    assert_eq!(mask_claim_age(&text), expected("list.txt"));
}

#[test]
fn ready_prints_every_project_as_expected_on_the_synthetic_tracker() {
    let text = every_project(|recs, p| report::ready_in(recs, Some(p)).unwrap()).concat();
    assert_eq!(mask_claim_age(&text), expected("ready.txt"));
}

#[test]
fn ready_json_prints_every_project_as_expected_on_the_synthetic_tracker() {
    let rows = every_project(|recs, p| {
        let q = ListQuery {
            project: Some(p.to_string()),
            ready: true,
            ..ListQuery::default()
        };
        issues_rows_from(recs, q).unwrap()
    })
    .concat();
    // Through a `Value`, as the CLI goes, which is what sorts the keys.
    let value = serde_json::to_value(&rows).unwrap();
    let text = format!("{}\n", serde_json::to_string_pretty(&value).unwrap());
    assert_eq!(text, expected("ready.json"));
}
