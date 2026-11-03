//! Time what the owner's watcher sees, to set its two intervals.
//!
//! Watches a copy of the fixture vault the way `vissue serve` does, saves
//! `atlas/issues.org` once by each method per round, and prints one line per save:
//!
//!     save METHOD ROUND EVENTS SPAN_NS FIRST_NS GAPS_NS
//!
//! `SPAN_NS` runs from the first event to the last.
//! `GAPS_NS` lists every gap between two events of the save, comma-separated.
//! `FIRST_NS` is the delay from an in-place write returning to its first event.
//! A field that does not apply is `-`.
//! The coalescing window has to be longer than the largest gap, or one save is rebuilt twice.
//!
//! Besides the saves, given a tracker directory and its projects prefix it also times poll ticks there, the project listing, a stat per project and the generation read, and prints one `tick NS` line per tick.
//!
//!     cargo run --release -p vissue-serve --example watch_timing [--run N] [TRACKER PREFIX]
//!
//! `--run N` starts each round N - 1 methods along the list, so each method's place in the round moves from run to run.
//!
//! `scripts/bench/watch_timing_summary.py` turns the lines into the tables in
//! `scripts/bench/watch_timing.md`.

#[cfg(unix)]
fn main() {
    timing::main();
}

#[cfg(not(unix))]
fn main() {}

#[cfg(unix)]
mod timing {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
    use vissue_core::config::Layout;
    use vissue_core::events;
    use vissue_core::store::list_projects;

    // Rounds per run.
    // Enough that the largest gap is a tail, not one sample, and few enough that the editors' start-up time keeps a run short.
    const TRIALS: usize = 40;
    // A save is over once the watcher has been quiet this long. It is five
    // times the 200 ms window under test, so a gap that would split a
    // rebuild in two is still counted inside the save.
    const QUIET: Duration = Duration::from_secs(1);

    pub fn main() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("vault");
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixture_vault"),
            &root,
        );
        git(&root, &["init", "-q", "-b", "base"]);
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "a"]);
        let layout = Layout::new(&root, "Software");
        let file = root.join("Software/atlas/issues.org");
        // git-checkout switches to this commit.
        // It is made before the watch starts, so no save counts its writes.
        fs::write(&file, body(&file, 0)).unwrap();
        git(&root, &["checkout", "-qb", "b"]);
        git(&root, &["commit", "-qam", "b"]);
        git(&root, &["checkout", "-q", "base"]);

        let (tx, rx) = mpsc::channel();
        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                if let Ok(ev) = res {
                    let _ = tx.send((Instant::now(), ev));
                }
            },
            notify::Config::default(),
        )
        .unwrap();
        watcher
            .watch(&layout.projects_dir(), RecursiveMode::Recursive)
            .unwrap();

        let methods: [(&str, &dyn Fn(usize)); 6] = [
            ("write", &|i| fs::write(&file, body(&file, i)).unwrap()),
            ("rename", &|i| {
                let tmp = file.with_extension("org.tmp");
                fs::write(&tmp, body(&file, i)).unwrap();
                fs::rename(&tmp, &file).unwrap();
            }),
            ("vissue-note", &|i| {
                vissue_core::ops::note(&layout, "atlas-2c3d", &format!("trial {i}")).unwrap();
            }),
            ("vim", &|i| {
                run(Command::new("vim")
                    .args(["-N", "-u", "DEFAULTS", "-i", "NONE", "-Es"])
                    .arg(format!("+$put ='trial {i}'"))
                    .arg("+wq")
                    .arg(&file))
            }),
            ("emacs", &|i| {
                run(Command::new("emacs")
                    .args(["-Q", "--batch"])
                    .arg(&file)
                    .arg("--eval")
                    .arg(format!(
                        "(progn (goto-char (point-max)) (insert \"trial {i}\\n\") (save-buffer))"
                    )))
            }),
            // `-f`, since the other methods leave the file edited between two checkouts.
            // A plain checkout would refuse to overwrite it.
            ("git-checkout", &|i| {
                let branch = if i % 2 == 0 { "b" } else { "base" };
                git(&root, &["checkout", "-qf", branch]);
            }),
        ];
        let mut args: Vec<String> = std::env::args().skip(1).collect();
        let run = match args.first().map(String::as_str) {
            Some("--run") => {
                let run = args[1].parse::<usize>().unwrap();
                args.drain(..2);
                run
            }
            _ => 1,
        };
        let mut active = Vec::new();
        for (name, save) in methods {
            if name == "git-checkout" || available(name) {
                active.push((name, save));
            } else {
                println!("missing {name}");
            }
        }
        // One save per method per round, so a drift in the machine's load over a run reaches every method alike.
        // Run n starts each round n - 1 methods along, so each method's place in the round moves from run to run.
        let shift = run.saturating_sub(1) % active.len();
        active.rotate_left(shift);
        for i in 0..TRIALS {
            for (name, save) in &active {
                drain(&rx);
                save(i);
                let done = Instant::now();
                println!("{}", save_line(name, i, done, &collect(&rx)));
            }
        }

        let mut args = args.into_iter();
        if let (Some(tracker), Some(prefix)) = (args.next(), args.next()) {
            tick_cost(&Layout::new(PathBuf::from(tracker), &prefix));
        }
    }

    /// One `save` line for the events `seen` after a save that returned at `done`.
    fn save_line(name: &str, round: usize, done: Instant, seen: &[Instant]) -> String {
        let ns = |d: Duration| d.as_nanos().to_string();
        let (span, first) = match (seen.first(), seen.last()) {
            (Some(&first), Some(&last)) => (
                ns(last.duration_since(first)),
                if name == "write" {
                    ns(first.saturating_duration_since(done))
                } else {
                    String::from("-")
                },
            ),
            _ => (String::from("-"), String::from("-")),
        };
        let gaps: Vec<String> = seen
            .windows(2)
            .map(|w| ns(w[1].duration_since(w[0])))
            .collect();
        let gaps = if gaps.is_empty() {
            String::from("-")
        } else {
            gaps.join(",")
        };
        format!("save {name} {round} {} {span} {first} {gaps}", seen.len())
    }

    /// One `tick` line for each of 1000 poll ticks over `layout`.
    fn tick_cost(layout: &Layout) {
        for _ in 0..1000 {
            let start = Instant::now();
            let _ = events::generation(layout);
            for project in list_projects(layout).unwrap() {
                let _ =
                    fs::metadata(layout.project_issues_path(&project)).and_then(|m| m.modified());
            }
            println!("tick {}", start.elapsed().as_nanos());
        }
    }

    /// Arrival times of the events the watcher counts, until it goes quiet.
    fn collect(rx: &mpsc::Receiver<(Instant, Event)>) -> Vec<Instant> {
        let mut seen = Vec::new();
        while let Ok((at, ev)) = rx.recv_timeout(QUIET) {
            if !matches!(ev.kind, EventKind::Access(_)) {
                seen.push(at);
            }
        }
        seen
    }

    fn drain(rx: &mpsc::Receiver<(Instant, Event)>) {
        while rx.recv_timeout(Duration::from_millis(50)).is_ok() {}
    }

    fn body(file: &Path, i: usize) -> String {
        let text = fs::read_to_string(file).unwrap();
        format!("{}\n# trial {i}\n", text.trim_end())
    }

    fn available(name: &str) -> bool {
        let bin = match name {
            "vim" | "emacs" => name,
            _ => return true,
        };
        Command::new(bin)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    fn run(cmd: &mut Command) {
        let status = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "{cmd:?}");
    }

    fn git(root: &Path, args: &[&str]) {
        run(Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args));
    }

    fn copy_dir(src: &Path, dest: &Path) {
        fs::create_dir_all(dest).unwrap();
        for entry in fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let to = dest.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &to);
            } else {
                fs::copy(entry.path(), to).unwrap();
            }
        }
    }
}
