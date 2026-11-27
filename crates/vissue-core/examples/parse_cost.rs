//! Wall time to load a synthetic tracker through today's org parse and through
//! an op fold.
//!
//! ```text
//! cargo run --release -p vissue-core --example parse_cost [ISSUES]
//! ```
//!
//! Prints `issues`, `org_ms` and `fold_ms`. The tracker is generated here.
//! `org_ms` is `load_all`. `fold_ms` is decoding the op messages and folding
//! them. The default size is 4000 issues across 20 projects.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use vissue_core::config::Layout;
use vissue_core::oplog::{Op, describe, fold};
use vissue_core::store::load_all;

const WORDS: &[&str] = &[
    "alpha", "bravo", "cedar", "delta", "echo", "fable", "grain", "harbor", "inlet", "juniper",
    "kernel", "lattice",
];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let issues = env::args()
        .nth(1)
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(4000);
    let projects = 20;
    let root = env::temp_dir().join(format!("vissue-parse-cost-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    write_tracker(&root, issues, projects)?;
    let layout = Layout::new(&root, "Issues");
    let started = Instant::now();
    let loaded = load_all(&layout)?;
    let org_ms = started.elapsed().as_millis();

    let mut by_project: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for (project, heading) in &loaded {
        by_project
            .entry(project.clone())
            .or_default()
            .push(heading.clone());
    }
    let mut messages = Vec::new();
    for (project, headings) in &by_project {
        for op in describe(project, headings)? {
            messages.push(op.message_bytes()?);
        }
    }

    let started = Instant::now();
    let mut ops = Vec::with_capacity(messages.len());
    for message in &messages {
        ops.push(Op::from_message_bytes(message)?);
    }
    let folded = fold(&ops)?;
    let fold_ms = started.elapsed().as_millis();

    println!("issues={}", loaded.len());
    println!("org_ms={org_ms}");
    println!("fold_ms={fold_ms}");
    println!("folded={}", folded.issues.len());
    let _ = fs::remove_dir_all(root);
    Ok(())
}

fn write_tracker(root: &Path, issues: usize, projects: usize) -> std::io::Result<()> {
    let mut files: BTreeMap<String, String> = BTreeMap::new();
    for index in 0..issues {
        let project = format!("p{:02}", index % projects);
        let id = format!("{project}-{index:04}");
        let word = WORDS[index % WORDS.len()];
        let title = format!("The {word} number {index} settled");
        let state = ["TODO", "STARTED", "BLOCKED", "DONE", "CANCELLED"][index % 5];
        let priority = ["A", "B", "C"][index % 3];
        let mut props =
            format!(":ID:         {id}\n:CREATED:    [2026-01-15 Thu]\n:TYPE:       task");
        if index > 0 && index % 3 == 0 {
            let blocker = format!("p{:02}-{:04}", (index - 1) % projects, index - 1);
            props.push_str(&format!("\n:BLOCKED_BY: {blocker}"));
        }
        let mut body = format!("* {state} [#{priority}] {title}\n:PROPERTIES:\n{props}\n:END:\n");
        if index % 2 == 0 {
            body.push_str(&format!(
                ":LOGBOOK:\n- Note: \"note {index}\" [2026-01-16 Fri 12:00]\n:END:\n"
            ));
        }
        body.push_str(&format!("\nA note about {word} {index}.\n\n"));
        let file = files.entry(project).or_insert_with(|| {
            "\
#+TITLE: synthetic issues
#+TODO: TODO STARTED BLOCKED | DONE CANCELLED

"
            .to_string()
        });
        file.push_str(&body);
    }
    for (project, text) in files {
        let dir = root.join("Issues").join(project);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("issues.org"), text)?;
    }
    Ok(())
}
