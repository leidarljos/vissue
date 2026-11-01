//! What each verb costs on a tracker of a given size, and where the cost sits.
//!
//! Rendering rows and computing the answer are different questions: `count`
//! does the parse and prints one number, `list` prints every row. Timing both
//! separates the two, and process startup is timed on its own because every
//! number below includes it.
//!
//! ```console
//! $ cargo run -q -p xtask -- bench-tracker [BINARY] [ISSUES]
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

/// Title words, repeated so search has something to match on.
const WORDS: &[&str] = &[
    "parser", "header", "overlay", "ripgrep", "review", "manifest", "record", "token",
];

/// Median milliseconds over `reps` runs.
fn timed(mut call: impl FnMut(), reps: usize) -> f64 {
    let mut times: Vec<f64> = Vec::with_capacity(reps);
    for _ in 0..reps {
        let start = Instant::now();
        call();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).expect("no nan"));
    times[times.len() / 2]
}

/// Run the tracker binary against one root and return its stdout.
fn run(binary: &Path, root: &Path, args: &[&str]) -> String {
    let out = Command::new(binary)
        .arg("--root")
        .arg(root)
        .args(args)
        .output()
        .expect("vissue");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Time the verbs on a freshly built tracker.
///
/// # Errors
///
/// Fails when the temporary tracker cannot be created.
pub fn tracker(flags: &[String]) -> Result<(), String> {
    let binary = flags
        .first()
        .map_or_else(|| PathBuf::from("target/release/vissue"), PathBuf::from);
    let total: usize = flags
        .get(1)
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(5000);

    let root = std::env::temp_dir().join(format!("vissue-bench-{}", std::process::id()));
    std::fs::create_dir_all(root.join("Software")).map_err(|e| e.to_string())?;

    let mut first = String::new();
    for index in 0..total {
        let word = WORDS[index % WORDS.len()];
        let title = format!("The {word} number {index} settled it");
        let project = format!("proj{}", index % 4);
        let id = run(&binary, &root, &["create", "-p", &project, &title, "-q"])
            .trim()
            .to_string();
        if first.is_empty() {
            first = id.clone();
        }
        // A third of the corpus waits on the first issue, so readiness has
        // something to decide rather than answering yes for everything.
        if index % 3 == 0 && !first.is_empty() && id != first {
            run(&binary, &root, &["update", &id, "--block", &first]);
        }
    }

    let rows = run(&binary, &root, &["list"]).lines().count();
    let ready = run(&binary, &root, &["ready"]).lines().count();
    println!("{total} issues, {rows} rows listed, {ready} ready");

    let startup = timed(
        || {
            Command::new(&binary)
                .arg("--version")
                .output()
                .expect("vissue");
        },
        7,
    );
    println!("  {:18} {startup:6.1} ms", "process startup");

    for (label, args) in [
        ("count", vec!["count"]),
        ("count --ready", vec!["count", "--ready"]),
        ("ready", vec!["ready"]),
        ("list", vec!["list"]),
        ("check", vec!["check"]),
        ("search", vec!["search", "parser"]),
        ("tree", vec!["tree", first.as_str()]),
        ("related", vec!["related", first.as_str()]),
    ] {
        let ms = timed(
            || {
                run(&binary, &root, &args);
            },
            7,
        );
        println!("  {label:18} {ms:6.1} ms");
    }

    std::fs::remove_dir_all(&root).ok();
    Ok(())
}
