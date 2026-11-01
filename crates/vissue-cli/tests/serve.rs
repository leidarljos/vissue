//! Serve lifecycle through the built binary. Unix only.

#![cfg(unix)]
#![allow(missing_docs)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_vissue")
}

fn copy_fixture(dest: &Path) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixture_vault");
    copy_dir(&src, dest);
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

struct Harness {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    socket: PathBuf,
}

impl Harness {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("vault");
        copy_fixture(&root);
        let socket = tmp.path().join("run/control.sock");
        Self {
            _tmp: tmp,
            root,
            socket,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(bin())
            .arg("--root")
            .arg(&self.root)
            .arg("--prefix")
            .arg("Software")
            .args(args)
            .arg("-s")
            .arg(&self.socket)
            .output()
            .expect("run vissue")
    }

    fn start_d(&self) -> Output {
        self.run(&["serve", "-d"])
    }

    fn stop(&self) -> Output {
        self.run(&["serve", "stop"])
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn wait_accepts(socket: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if std::os::unix::net::UnixStream::connect(socket).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    std::os::unix::net::UnixStream::connect(socket).is_ok()
}

#[test]
fn lifecycle_start_status_second_d_stop() {
    let h = Harness::new();
    let out = h.start_d();
    assert!(
        out.status.success(),
        "serve -d failed: {}{}",
        stdout(&out),
        stderr(&out)
    );
    assert!(
        wait_accepts(&h.socket, Duration::from_secs(5)),
        "socket did not accept"
    );

    let status = h.run(&["serve", "status"]);
    assert!(
        status.status.success(),
        "status exit {}: {}{}",
        status.status.code().unwrap_or(-1),
        stdout(&status),
        stderr(&status)
    );
    let text = stdout(&status);
    assert!(text.contains("live: true"), "{text}");
    assert!(
        text.contains(&format!("socket: {}", h.socket.display())),
        "{text}"
    );

    let again = h.start_d();
    assert!(again.status.success(), "{}", stderr(&again));
    let again_text = stdout(&again);
    assert!(
        again_text.contains("already running"),
        "{again_text} {}",
        stderr(&again)
    );
    assert!(again_text.contains("pid="), "{again_text}");
    assert!(
        again_text.contains(&format!("socket={}", h.socket.display())),
        "{again_text}"
    );

    let stop = h.stop();
    assert!(stop.status.success(), "{}", stderr(&stop));

    let down = h.run(&["serve", "status"]);
    assert_eq!(down.status.code(), Some(1), "{}", stdout(&down));
    assert!(stdout(&down).contains("live: false"), "{}", stdout(&down));

    // The socket goes with the owner. One left behind refuses the next bind,
    // and the daemon that finds it has to decide whether it is stale.
    assert!(
        !h.socket.exists(),
        "the socket outlived the owner: {}",
        h.socket.display()
    );
}

#[test]
fn status_json_and_socket_modes() {
    let h = Harness::new();
    assert!(h.start_d().status.success());
    assert!(wait_accepts(&h.socket, Duration::from_secs(5)));

    let dir_mode = fs::metadata(h.socket.parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(dir_mode, 0o700);
    let sock_mode = fs::metadata(&h.socket).unwrap().permissions().mode() & 0o777;
    assert_eq!(sock_mode, 0o600);

    let out = h.run(&["serve", "status", "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(value["live"], true);
    assert_eq!(value["prefix"], "Software");
    assert!(value["pid"].as_u64().is_some());
    assert!(
        value["revision"].as_u64().unwrap() >= 1,
        "live owner starts catalog revision at 1: {value}"
    );
}

#[test]
fn initialize_requires_agent_over_the_socket() {
    use vissue_control::Error;
    use vissue_control::client::Client;

    let h = Harness::new();
    assert!(h.start_d().status.success());
    assert!(wait_accepts(&h.socket, Duration::from_secs(5)));

    let mut client = Client::connect(&h.socket).unwrap();
    let err = client
        .request(
            "initialize",
            serde_json::json!({"protocolVersion": 1, "client": "cli-test"}),
        )
        .unwrap_err();
    match err {
        Error::Rpc(e) => {
            assert_eq!(e.code, -32602);
            assert_eq!(e.message, "agent is required");
        }
        other => panic!("{other:?}"),
    }

    let ok = client
        .request(
            "initialize",
            serde_json::json!({"protocolVersion": 1, "agent": "cli-test"}),
        )
        .unwrap();
    assert_eq!(ok["identity"], "cli-test");
    let ident = client
        .request("identity/get", serde_json::json!({}))
        .unwrap();
    assert_eq!(ident["identity"], "cli-test");
}

#[test]
fn default_socket_uses_xdg_runtime_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("vault");
    copy_fixture(&root);
    let runtime = tmp.path().join("xdg");
    fs::create_dir_all(&runtime).unwrap();
    let socket = runtime.join("vissue/control.sock");

    let start = Command::new(bin())
        .arg("--root")
        .arg(&root)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env_remove("VISSUE_CONTROL_SOCKET")
        .args(["serve", "-d"])
        .output()
        .unwrap();
    assert!(
        start.status.success(),
        "{}{}",
        String::from_utf8_lossy(&start.stdout),
        String::from_utf8_lossy(&start.stderr)
    );
    assert!(wait_accepts(&socket, Duration::from_secs(5)));

    let status = Command::new(bin())
        .arg("--root")
        .arg(&root)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env_remove("VISSUE_CONTROL_SOCKET")
        .args(["serve", "status"])
        .output()
        .unwrap();
    assert!(status.status.success());

    let _ = Command::new(bin())
        .arg("--root")
        .arg(&root)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env_remove("VISSUE_CONTROL_SOCKET")
        .args(["serve", "stop"])
        .output();
}

#[test]
fn serve_help_hides_foreground() {
    let out = Command::new(bin())
        .args(["serve", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(text.contains("-d"), "{text}");
    assert!(text.contains("--detach"), "{text}");
    assert!(!text.contains("--foreground"), "{text}");
    assert!(!text.contains("--no-detach"), "{text}");
    assert!(text.contains("stop"), "{text}");
    assert!(text.contains("status"), "{text}");
}

#[test]
fn completions_omit_foreground() {
    // Every shell the CLI offers, not a sample: a generator is per-shell, so
    // one that emits a hidden flag would hide behind the others.
    for shell in ["bash", "elvish", "fish", "powershell", "zsh"] {
        let out = Command::new(bin())
            .args(["completions", shell])
            .output()
            .unwrap();
        assert!(out.status.success(), "{shell}: {}", stderr(&out));
        let text = stdout(&out);
        assert!(
            text.contains("vissue"),
            "{shell} produced nothing that names the binary"
        );
        assert!(
            !text.contains("--foreground"),
            "{shell} completions advertise --foreground"
        );
        assert!(
            !text.contains("--no-detach"),
            "{shell} completions advertise --no-detach"
        );
    }
}

#[test]
fn man_page_has_no_trailing_whitespace() {
    let out = Command::new(bin()).args(["man"]).output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    for (i, line) in text.lines().enumerate() {
        assert_eq!(
            line,
            line.trim_end(),
            "man line {} has trailing space: {line:?}",
            i + 1
        );
    }
    let committed =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../man/vissue.1"))
            .unwrap();
    for (i, line) in committed.lines().enumerate() {
        assert_eq!(
            line,
            line.trim_end(),
            "committed man line {} has trailing space",
            i + 1
        );
    }
}

#[test]
fn committed_completions_omit_foreground() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in [
        "completions/vissue.bash",
        "completions/_vissue",
        "completions/vissue.fish",
    ] {
        let text = fs::read_to_string(root.join(name)).unwrap();
        assert!(
            !text.contains("--foreground"),
            "{name} still contains --foreground"
        );
        assert!(
            !text.contains("--no-detach"),
            "{name} still contains --no-detach"
        );
    }
}

#[test]
fn claim_uses_client_agent_not_process_env() {
    use vissue_control::Error;
    use vissue_control::client::Client;

    let h = Harness::new();
    let start = Command::new(bin())
        .arg("--root")
        .arg(&h.root)
        .arg("--prefix")
        .arg("Software")
        .args(["serve", "-d"])
        .arg("-s")
        .arg(&h.socket)
        .env("VISSUE_AGENT", "daemon")
        .output()
        .unwrap();
    assert!(
        start.status.success(),
        "{}{}",
        stdout(&start),
        stderr(&start)
    );
    assert!(wait_accepts(&h.socket, Duration::from_secs(5)));

    let mut client = Client::connect(&h.socket).unwrap();
    client
        .request(
            "initialize",
            serde_json::json!({"protocolVersion": 1, "agent": "tui"}),
        )
        .unwrap();
    let claimed = client
        .request("issue/claim", serde_json::json!({"id": "atlas-2c3d"}))
        .unwrap();
    assert_eq!(claimed["issue"]["claimed_by"], "tui");
    let text = fs::read_to_string(h.root.join("Software/atlas/issues.org")).unwrap();
    assert!(
        text.contains("tui"),
        "claim stamp must be the client agent: {text}"
    );
    assert!(
        !text.contains(":CLAIMED_BY: daemon") && !text.contains("CLAIMED_BY:         daemon"),
        "process VISSUE_AGENT must not stamp the claim: {text}"
    );

    let err = client
        .request(
            "issue/claim",
            serde_json::json!({"id": "atlas-2c3d", "agent": "other"}),
        )
        .unwrap_err();
    match err {
        Error::Rpc(e) => assert_eq!(e.code, -32009),
        other => panic!("{other:?}"),
    }
}

#[test]
fn restart_brings_a_stopped_owner_back() {
    let h = Harness::new();
    assert!(h.start_d().status.success());
    assert!(wait_accepts(&h.socket, Duration::from_secs(5)));
    let first = stdout(&h.run(&["serve", "status"]))
        .lines()
        .find_map(|l| l.strip_prefix("pid: "))
        .map(|p| p.trim().to_string());
    assert!(first.is_some(), "no pid while it was up");
    assert!(h.stop().status.success());
    let restart = h.run(&["serve", "restart"]);
    assert!(
        restart.status.success(),
        "{}{}",
        stdout(&restart),
        stderr(&restart)
    );
    assert!(wait_accepts(&h.socket, Duration::from_secs(5)));
    let back = h.run(&["serve", "status"]);
    assert!(back.status.success());
    // A restart that kept the old process would satisfy everything above,
    // so the pid has to have moved.
    let pid = |text: &str| -> Option<String> {
        text.lines()
            .find_map(|l| l.strip_prefix("pid: "))
            .map(|p| p.trim().to_string())
    };
    let after = pid(&stdout(&back)).expect("a pid once it is back");
    assert!(!after.is_empty(), "{}", stdout(&back));
    assert_ne!(Some(after), first, "restart kept the same process");
}

/// `ping` cannot be compared by equality, and the reason is worth its own test.
///
/// It appends to the event log, so calling it twice gives two sequence numbers: the
/// command line saw `seq=1` and the socket `seq=2`, which is the verb working rather
/// than the surfaces disagreeing. Equality is the wrong instrument for a read with a
/// side effect, so the shape is what is asserted.
#[test]
fn ping_agrees_in_shape_because_it_cannot_agree_in_bytes() {
    let h = Harness::new();
    assert!(h.start_d().status.success(), "server did not start");
    assert!(
        wait_accepts(&h.socket, Duration::from_secs(30)),
        "server never accepted"
    );

    let mut client = vissue_control::client::Client::connect(&h.socket).expect("connect");
    client
        .request(
            "initialize",
            serde_json::json!({"protocolVersion": vissue_control::rpc::PROTOCOL_VERSION,
                              "client": "ping-shape", "agent": "ping-shape"}),
        )
        .expect("initialize");

    let from_cli = Command::new(bin())
        .arg("--root")
        .arg(&h.root)
        .arg("--prefix")
        .arg("Software")
        .arg("ping")
        .output()
        .expect("run vissue");
    let cli_text = String::from_utf8_lossy(&from_cli.stdout).into_owned();

    let reply = client
        .request("events/ping", serde_json::json!({}))
        .unwrap();
    let socket_text = reply["report"].as_str().unwrap_or_default();

    for (label, text) in [("cli", cli_text.as_str()), ("socket", socket_text)] {
        let first = text.lines().next().unwrap_or_default();
        assert!(
            first.starts_with("ping seq=") && first.contains("generation="),
            "{label} ping does not report a sequence and a generation: {first:?}"
        );
    }
    assert_eq!(
        cli_text.lines().count(),
        socket_text.lines().count(),
        "the two pings report a different number of lines\n  cli: {cli_text}\n  socket: {socket_text}"
    );
}

/// The socket answers the same thing the subcommand prints.
///
/// This is the class of bug the name and type checks cannot see. `issue/mirror`
/// answered with the corpus digest for a while: the schema agreed with it, the types
/// agreed, the reference agreed, and the method still gave a caller a hash where it
/// had asked whether its mirror was current. Nothing compares content.
///
/// So for the reads that produce a report, the socket's text has to equal the
/// subcommand's. Where a read is inherently different between the two, that
/// difference belongs in the schema's note and not in a silent divergence here.
#[test]
fn the_socket_reports_match_the_subcommand() {
    let h = Harness::new();
    assert!(h.start_d().status.success(), "server did not start");
    assert!(
        wait_accepts(&h.socket, Duration::from_secs(30)),
        "server never accepted"
    );

    let mut client = vissue_control::client::Client::connect(&h.socket).expect("connect");
    client
        .request(
            "initialize",
            serde_json::json!({"protocolVersion": vissue_control::rpc::PROTOCOL_VERSION,
                              "client": "equivalence-test",
                              "agent": "equivalence-test"}),
        )
        .expect("initialize");

    // Every read that answers with a report, and the arguments that mean the same
    // thing on both sides. A default that differs between the surfaces is itself a
    // divergence, so where one exists it is passed explicitly rather than left to
    // whichever side happens to be asked.
    let cases: Vec<(Vec<&str>, &str, serde_json::Value)> = vec![
        (vec!["export"], "issue/export", serde_json::json!({})),
        (vec!["graph"], "issue/graph", serde_json::json!({})),
        (vec!["roadmap"], "issue/roadmap", serde_json::json!({})),
        (vec!["cycles"], "issue/cycles", serde_json::json!({})),
        (vec!["count"], "issue/count", serde_json::json!({})),
        (vec!["check"], "issue/check", serde_json::json!({})),
        // stale defaults to 30 days on the command line and has no default on the
        // socket, so the number is given to both.
        (
            vec!["stale", "--days", "30"],
            "issue/stale",
            serde_json::json!({"days": 30}),
        ),
        (vec!["hygiene"], "issue/hygiene", serde_json::json!({})),
        (
            vec!["waiting-on", "atlas-3e4f"],
            "issue/waiting_on",
            serde_json::json!({"id": "atlas-3e4f"}),
        ),
    ];

    // The CLI side runs without `-s`, which the harness appends for the serve verbs
    // and which the read verbs do not accept: with it they exit non-zero and print
    // nothing, and comparing against nothing passes for agreement.
    let cli = |args: &[&str]| -> String {
        let out = Command::new(bin())
            .arg("--root")
            .arg(&h.root)
            .arg("--prefix")
            .arg("Software")
            .args(args)
            .output()
            .expect("run vissue");
        // `check` exits non-zero when the corpus has validation errors, and the
        // fixture is allowed to. What matters is the text, so the status is only
        // required to be one of the two the verb documents.
        assert!(
            out.status.code().is_some_and(|c| c == 0 || c == 1),
            "{args:?} died: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };

    for (args, method, params) in cases {
        let verb = args[0];
        let from_cli = cli(&args);
        let reply = client
            .request(method, params)
            .unwrap_or_else(|e| panic!("{method} failed: {e}"));
        let from_socket = reply["report"].as_str().unwrap_or_default();
        if from_socket.trim() != from_cli.trim() {
            // The first differing line, rather than two walls of text: these
            // reports are long and the eye cannot find the divergence in a dump.
            let a: Vec<&str> = from_cli.trim().lines().collect();
            let b: Vec<&str> = from_socket.trim().lines().collect();
            let at = a
                .iter()
                .zip(b.iter())
                .position(|(x, y)| x != y)
                .unwrap_or(a.len().min(b.len()));
            panic!(
                "{verb} and {method} disagree at line {at}\n  cli:    {:?}\n  socket: {:?}\n  \
                 lines: cli {} socket {}",
                a.get(at),
                b.get(at),
                a.len(),
                b.len()
            );
        }
    }
}

/// The structured reads agree with `--json` too.
///
/// The report comparison covers the reads that answer in prose. These answer with
/// objects, and the same class of divergence applies: `issue/digest` omitted the
/// `generation` field that `digest --json` has always carried, which no name or type
/// check could see because the schema described what I wrote rather than what the
/// verb means.
///
/// The envelope is allowed to differ and the content is not. A socket reply carries
/// `revision`, which is the server's notion of how many writes it has seen and has no
/// command-line equivalent; the rows inside it have to match.
#[test]
fn the_structured_reads_match_the_json_mode() {
    let h = Harness::new();
    assert!(h.start_d().status.success(), "server did not start");
    assert!(
        wait_accepts(&h.socket, Duration::from_secs(30)),
        "server never accepted"
    );

    let mut client = vissue_control::client::Client::connect(&h.socket).expect("connect");
    client
        .request(
            "initialize",
            serde_json::json!({"protocolVersion": vissue_control::rpc::PROTOCOL_VERSION,
                              "client": "json-equivalence", "agent": "json-equivalence"}),
        )
        .expect("initialize");

    let cli_json = |args: &[&str]| -> serde_json::Value {
        let out = Command::new(bin())
            .arg("--root")
            .arg(&h.root)
            .arg("--prefix")
            .arg("Software")
            .args(args)
            .arg("--json")
            .output()
            .expect("run vissue");
        assert!(
            out.status.success(),
            "{args:?} --json failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).expect("the command line did not emit JSON")
    };

    // list: the rows have to match, whatever the envelope adds.
    let cli_rows = cli_json(&["list"]);
    let reply = client.request("issue/list", serde_json::json!({})).unwrap();
    let socket_rows = &reply["issues"];
    assert_eq!(
        socket_rows.as_array().map(Vec::len),
        cli_rows.as_array().map(Vec::len),
        "list row counts differ\n  cli: {cli_rows}\n  socket: {socket_rows}"
    );
    let ids = |v: &serde_json::Value| -> Vec<String> {
        let mut out: Vec<String> = v
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(|r| r["id"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    };
    // As sets, not sequences, and the order is where they legitimately differ. Both
    // sort by priority, then state, then id. The command line applies that within
    // each project and concatenates, because it can span layouts and a global sort
    // across them would interleave two trackers. The socket serves one layout and
    // sorts across the whole of it, which is what a client asking for a frontier
    // wants and what the terminal UI relies on.
    //
    // So the rows are the same rows and the sequence is not, which is a difference
    // worth pinning as intended rather than quietly tolerating or breaking a working
    // client to remove.
    assert_eq!(
        ids(&cli_rows),
        ids(socket_rows),
        "list returns different issues, not merely a different order"
    );

    // digest: every field the command line reports, the socket reports.
    let cli_digest = cli_json(&["digest"]);
    let socket_digest = client
        .request("issue/digest", serde_json::json!({}))
        .unwrap();
    let missing: Vec<&String> = cli_digest
        .as_object()
        .map(|o| {
            o.keys()
                .filter(|k| socket_digest.get(*k).is_none())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        missing.is_empty(),
        "digest --json reports these and the method does not: {missing:?}"
    );
    assert_eq!(
        socket_digest["combined"], cli_digest["combined"],
        "the two digests disagree about the corpus"
    );
}

/// The nine reads that had no JSON mode now have one, and it matches the socket.
///
/// They answered in prose on the command line and in structure over the socket, so
/// there was nothing to compare and they went unaudited while every other read was
/// checked. The `--json` modes go through the same `CatalogService` the socket answers
/// from, so this asserts one computation rather than reconciling two — which is the
/// point of adding them this way round.
#[test]
fn the_json_modes_match_their_socket_methods() {
    let h = Harness::new();
    assert!(h.start_d().status.success(), "server did not start");
    assert!(
        wait_accepts(&h.socket, Duration::from_secs(30)),
        "server never accepted"
    );

    let mut client = vissue_control::client::Client::connect(&h.socket).expect("connect");
    client
        .request(
            "initialize",
            serde_json::json!({"protocolVersion": vissue_control::rpc::PROTOCOL_VERSION,
                              "client": "json-modes", "agent": "json-modes"}),
        )
        .expect("initialize");

    let cli_json = |args: &[&str]| -> serde_json::Value {
        let out = Command::new(bin())
            .arg("--root")
            .arg(&h.root)
            .arg("--prefix")
            .arg("Software")
            .args(args)
            .arg("--json")
            .output()
            .expect("run vissue");
        assert!(
            out.status.success(),
            "{args:?} --json failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{args:?} --json did not emit JSON: {e}"))
    };

    // The id is one the fixture has, with a parent, a blocker and a backlink.
    const ID: &str = "atlas-1a2b";

    // (cli args, method, params, the field of the reply to compare)
    let cases: Vec<(Vec<&str>, &str, serde_json::Value, &str)> = vec![
        (
            vec!["search", "parser"],
            "issue/search",
            serde_json::json!({"query": "parser", "limit": 20}),
            "",
        ),
        (
            vec!["children", ID],
            "issue/children",
            serde_json::json!({"id": ID}),
            "hits",
        ),
        (
            vec!["ancestors", "atlas-3e4f", "--depth", "3"],
            "issue/ancestors",
            serde_json::json!({"id": "atlas-3e4f", "depth": 3}),
            "hits",
        ),
        (
            vec!["impact", ID, "--depth", "3"],
            "issue/impact",
            serde_json::json!({"id": ID, "depth": 3}),
            "hits",
        ),
        (
            vec!["backlinks", ID],
            "issue/backlinks",
            serde_json::json!({"id": ID}),
            "hits",
        ),
        (
            vec!["agenda", "--days", "14"],
            "issue/agenda",
            serde_json::json!({"days": 14}),
            "rows",
        ),
        (
            vec!["body-excerpt", ID],
            "issue/excerpt",
            serde_json::json!({"id": ID}),
            "",
        ),
        (
            vec!["tree", ID],
            "issue/tree",
            serde_json::json!({"id": ID}),
            "",
        ),
        // projects: the socket wraps its list in a revision, which the command line
        // has no notion of.
        (
            vec!["projects"],
            "project/list",
            serde_json::json!({}),
            "projects",
        ),
    ];

    for (args, method, params, field) in cases {
        let from_cli = cli_json(&args);
        let reply = client
            .request(method, params)
            .unwrap_or_else(|e| panic!("{method} failed: {e}"));
        // A socket reply may wrap its payload; the command line prints the payload.
        let from_socket = if field.is_empty() {
            &reply
        } else {
            reply.get(field).unwrap_or(&reply)
        };
        assert_eq!(
            from_socket, &from_cli,
            "{args:?} --json and {method} disagree\n  cli:    {from_cli}\n  socket: {from_socket}"
        );
    }
}
