//! A target record, a change's target list, and a land-field clash.

#![allow(missing_docs)]

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

fn vissue(root: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vissue"));
    cmd.env("VISSUE_NO_ROUTE", "1");
    cmd.env("VISSUE_AGENT", "ada");
    let mut argv = vec!["--root", root.to_str().unwrap()];
    argv.extend_from_slice(args);
    cmd.args(argv).output().expect("run vissue")
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

fn run(root: &Path, args: &[&str]) -> String {
    let out = vissue(root, args);
    assert!(
        out.status.success(),
        "{}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    stdout(&out)
}

fn refused(out: &std::process::Output, needle: &str) {
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "exit 0, wanted a refusal ({needle}): {err}"
    );
    assert!(err.contains(needle), "stderr missing {needle:?}: {err}");
}

fn tracker() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("Software")).unwrap();
    dir
}

/// A target and a change's target list survive a write, a read, and a second
/// write as the same bytes. A retarget records the old id and the new one.
#[test]
fn a_target_and_a_change_target_list_round_trip_byte_for_byte() {
    let dir = tracker();
    let root = dir.path();
    let issue = run(root, &["create", "-p", "api", "Land the kit", "-q"]);
    let issue = issue.trim();

    let write = [
        "target",
        "--repo",
        "github:acme/widgets",
        "--branch",
        "develop",
        "--mode",
        "merge",
        "--lease",
        "strict-base",
        "--quorum",
        "8 seats",
        "--landers",
        "seat:reviewer",
        "--check",
        "ci:required",
        "--owned",
        "upstream",
        "--fork",
        "github:acme/fork",
        "--mirror",
        "gitlab:acme/widgets",
    ];
    let id = run(root, &write);
    let id = id.trim().to_string();
    assert!(id.starts_with("sha256:"), "{id}");

    let path = root.join("Software/targets.org");
    let first = fs::read(&path).unwrap();
    let again = run(root, &write);
    assert_eq!(again.trim(), id);
    let second = fs::read(&path).unwrap();
    assert_eq!(
        first, second,
        "a second write of the same target changed the file"
    );

    let org = run(
        root,
        &[
            "target",
            "--org",
            "--repo",
            "github:acme/widgets",
            "--branch",
            "develop",
        ],
    );
    assert_eq!(org.into_bytes(), first, "the read is not the stored file");
    let text = String::from_utf8(first.clone()).unwrap();
    for needle in [
        "github:acme/widgets",
        "develop",
        "merge",
        "strict-base",
        "8 seats",
        "seat:reviewer",
        "ci:required",
        "upstream",
        "github:acme/fork",
        "gitlab:acme/widgets",
    ] {
        assert!(
            text.contains(needle),
            "stored file missing {needle}: {text}"
        );
    }

    run(root, &["target", issue, "--add", &id]);
    let board = root.join("Software/api/issues.org");
    let listed = fs::read(&board).unwrap();
    let listed_again = {
        run(root, &["target", issue, "--add", &id]);
        fs::read(&board).unwrap()
    };
    assert_eq!(
        listed, listed_again,
        "adding the same target rewrote the issue"
    );
    let listed_text = String::from_utf8(listed).unwrap();
    assert!(
        listed_text.contains(&id),
        "the change's target list dropped {id}: {listed_text}"
    );

    let other = run(
        root,
        &[
            "target",
            "--repo",
            "github:acme/widgets",
            "--branch",
            "release",
        ],
    );
    let other = other.trim().to_string();
    let moved = run(root, &["target", issue, "--from", &id, "--to", &other]);
    assert!(moved.contains(&id) && moved.contains(&other), "{moved}");
    let after = fs::read_to_string(&board).unwrap();
    let targets_line = after
        .lines()
        .find(|line| line.contains(":TARGETS:"))
        .unwrap_or("");
    assert!(targets_line.contains(&other), "{targets_line}");
    assert!(!targets_line.contains(&id), "{targets_line}");
    assert!(
        after.contains(&format!("retarget {id} -> {other}")),
        "the retarget note is missing: {after}"
    );
}

/// `lease-rewrite` on a main ref is refused, and the same rule on another
/// branch is stored.
#[test]
fn lease_rewrite_on_a_main_ref_is_refused() {
    let dir = tracker();
    let root = dir.path();
    head_points_at_main(root);
    let path = root.join("Software/targets.org");
    let socket = root.join("control.sock");
    let _serve = Serve::start(root, &socket);
    let mut rpc = Rpc::connect(&socket);
    let mut mcp = Mcp::start(root);

    // `main` and `refs/heads/main`, plus every other spelling that is a main
    // ref or not a branch name. Symbolic `HEAD` is included while this
    // tracker's `HEAD` points at `refs/heads/main`.
    for branch in [
        "main",
        "refs/heads/main",
        "MAIN",
        "Main",
        "master",
        "refs/heads/MAIN",
        "Refs/Heads/main",
        "HEAD",
        "head",
        "@",
        "refs/HEAD",
        "refs/heads/HEAD",
        "heads/main",
    ] {
        refuse_lease(
            root,
            &mut rpc,
            &mut mcp,
            branch,
            &format!("lease-rewrite is refused on a main ref: {branch}"),
            -32010,
            "lease_rewrite_on_main",
        );
    }
    for branch in ["main/", "refs/heads//main"] {
        refuse_lease(
            root,
            &mut rpc,
            &mut mcp,
            branch,
            &format!("invalid branch: {branch}"),
            -32602,
            "invalid_branch",
        );
    }
    assert!(!path.exists(), "a refused branch was stored");

    run(
        root,
        &[
            "target",
            "--repo",
            "github:acme/widgets",
            "--branch",
            "MAIN",
            "--lease",
            "strict-base",
        ],
    );
    let edited = fs::read_to_string(&path)
        .unwrap()
        .replace("strict-base", "lease-rewrite");
    fs::write(&path, &edited).unwrap();
    refuse_read(root, &mut rpc, &mut mcp, "MAIN");
    fs::remove_file(&path).unwrap();

    for branch in ["develop", "mainline"] {
        accept_lease(root, &mut rpc, &mut mcp, branch);
    }
    let text = fs::read_to_string(&path).unwrap();
    let branches: Vec<&str> = text
        .lines()
        .filter(|line| line.contains(":BRANCH:"))
        .collect();
    assert_eq!(branches.len(), 2, "{text}");
    assert!(
        branches.iter().any(|line| line.ends_with("develop")),
        "{text}"
    );
    assert!(
        branches.iter().any(|line| line.ends_with("mainline")),
        "{text}"
    );
    assert!(text.contains("lease-rewrite"), "{text}");
}

fn head_points_at_main(root: &Path) {
    let out = Command::new("git")
        .args(["-C", root.to_str().unwrap(), "init", "-q", "-b", "main"])
        .output()
        .expect("git init");
    assert!(
        out.status.success(),
        "git init: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = Command::new("git")
        .args(["-C", root.to_str().unwrap(), "symbolic-ref", "HEAD"])
        .output()
        .expect("symbolic-ref");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "refs/heads/main",
        "symbolic HEAD is not main"
    );
}

fn refuse_lease(
    root: &Path,
    rpc: &mut Rpc,
    mcp: &mut Mcp,
    branch: &str,
    needle: &str,
    code: i64,
    data_code: &str,
) {
    let path = root.join("Software/targets.org");
    let out = vissue(
        root,
        &[
            "target",
            "--repo",
            "github:acme/widgets",
            "--branch",
            branch,
            "--lease",
            "lease-rewrite",
        ],
    );
    refused(&out, needle);
    let err = rpc
        .call(
            "issue/target",
            json!({
                "repo": "github:acme/widgets",
                "branch": branch,
                "lease": "lease-rewrite",
            }),
        )
        .expect_err("socket stored a refused branch");
    assert_eq!(err["code"].as_i64(), Some(code), "{branch} socket {err}");
    assert_eq!(err["data"]["code"].as_str(), Some(data_code), "{err}");
    assert_eq!(err["data"]["branch"].as_str(), Some(branch), "{err}");
    assert!(
        err["message"].as_str().unwrap_or("").contains(needle),
        "{err}"
    );
    let reply = mcp.call(json!({
        "repo": "github:acme/widgets",
        "branch": branch,
        "lease": "lease-rewrite",
    }));
    let err = reply
        .get("error")
        .unwrap_or_else(|| panic!("mcp stored {branch}: {reply}"));
    assert_eq!(err["code"].as_i64(), Some(code), "{branch} mcp {err}");
    assert_eq!(err["data"]["code"].as_str(), Some(data_code), "{err}");
    assert_eq!(err["data"]["branch"].as_str(), Some(branch), "{err}");
    assert!(
        err["message"].as_str().unwrap_or("").contains(needle),
        "{err}"
    );
    assert!(!path.exists(), "a refused branch was stored: {branch}");
}

fn refuse_read(root: &Path, rpc: &mut Rpc, mcp: &mut Mcp, branch: &str) {
    let needle = format!("lease-rewrite is refused on a main ref: {branch}");
    let out = vissue(
        root,
        &[
            "target",
            "--org",
            "--repo",
            "github:acme/widgets",
            "--branch",
            branch,
        ],
    );
    refused(&out, &needle);
    let err = rpc
        .call(
            "issue/target",
            json!({
                "org": true,
                "repo": "github:acme/widgets",
                "branch": branch,
            }),
        )
        .expect_err("socket read a main ref");
    assert_eq!(err["code"].as_i64(), Some(-32010), "{err}");
    assert_eq!(
        err["data"]["code"].as_str(),
        Some("lease_rewrite_on_main"),
        "{err}"
    );
    let reply = mcp.call(json!({
        "org": true,
        "repo": "github:acme/widgets",
        "branch": branch,
    }));
    let err = reply
        .get("error")
        .unwrap_or_else(|| panic!("mcp read a main ref: {reply}"));
    assert_eq!(err["code"].as_i64(), Some(-32010), "{err}");
    assert_eq!(
        err["data"]["code"].as_str(),
        Some("lease_rewrite_on_main"),
        "{err}"
    );
}

fn accept_lease(root: &Path, rpc: &mut Rpc, mcp: &mut Mcp, branch: &str) {
    let id = run(
        root,
        &[
            "target",
            "--repo",
            "github:acme/widgets",
            "--branch",
            branch,
            "--lease",
            "lease-rewrite",
        ],
    );
    let id = id.trim();
    assert!(id.starts_with("sha256:"), "{id}");
    let got = rpc
        .call(
            "issue/target",
            json!({
                "repo": "github:acme/widgets",
                "branch": branch,
                "lease": "lease-rewrite",
            }),
        )
        .unwrap_or_else(|err| panic!("socket refused {branch}: {err}"));
    assert!(
        got["report"].as_str().unwrap_or("").contains(id),
        "{branch} socket {got}"
    );
    let reply = mcp.call(json!({
        "repo": "github:acme/widgets",
        "branch": branch,
        "lease": "lease-rewrite",
    }));
    assert!(reply.get("error").is_none(), "{branch} mcp {reply}");
    let text = reply["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(text.contains(id), "{branch} mcp {reply}");
}

struct Serve {
    child: Child,
}

impl Serve {
    fn start(root: &Path, socket: &Path) -> Self {
        let err_path = root.join("serve.err");
        let err = File::create(&err_path).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_vissue"))
            .args([
                "--root",
                root.to_str().unwrap(),
                "serve",
                "--foreground",
                "-s",
                socket.to_str().unwrap(),
            ])
            .env("VISSUE_NO_ROUTE", "1")
            .env("VISSUE_AGENT", "ada")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(err))
            .spawn()
            .expect("spawn serve");
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut accepted = false;
        while Instant::now() < deadline {
            if UnixStream::connect(socket).is_ok() {
                accepted = true;
                break;
            }
            if let Some(status) = child.try_wait().expect("serve status") {
                let err = fs::read_to_string(&err_path).unwrap_or_default();
                panic!("serve exited {status}: {err}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !accepted {
            let _ = child.kill();
            let _ = child.wait();
            let err = fs::read_to_string(&err_path).unwrap_or_default();
            panic!("serve did not accept: {err}");
        }
        Self { child }
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Rpc {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: i64,
}

impl Rpc {
    fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("connect");
        let reader = BufReader::new(stream.try_clone().expect("clone"));
        Self {
            writer: stream,
            reader,
            next_id: 0,
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        self.next_id += 1;
        let id = self.next_id;
        writeln!(
            self.writer,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        )
        .expect("write");
        self.writer.flush().expect("flush");
        loop {
            let mut line = String::new();
            let read = self.reader.read_line(&mut line).expect("read");
            assert!(read > 0, "{method}: the socket closed");
            let value: Value = serde_json::from_str(&line).unwrap_or_else(|err| {
                panic!("{method}: not JSON: {err}: {line}");
            });
            if value.get("method").is_some() && value.get("id").is_none() {
                continue;
            }
            if value.get("id").and_then(Value::as_i64) != Some(id) {
                continue;
            }
            if let Some(err) = value.get("error") {
                return Err(err.clone());
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

struct Mcp {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: i64,
}

impl Mcp {
    fn start(root: &Path) -> Self {
        let bin = Path::new(env!("CARGO_BIN_EXE_vissue"))
            .parent()
            .expect("bin dir")
            .join("vissue-mcp");
        assert!(bin.is_file(), "build vissue-mcp before this test");
        let mut child = Command::new(&bin)
            .env("VISSUE_ROOT", root)
            .env("VISSUE_PREFIX", "Software")
            .env("VISSUE_NO_ROUTE", "1")
            .env("VISSUE_AGENT", "ada")
            .env_remove("VISSUE_CONFIG")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn vissue-mcp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        let mut mcp = Self {
            child,
            stdin,
            stdout,
            next_id: 0,
        };
        let init = mcp.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "target-record", "version": "0"},
            }),
        );
        assert!(init.get("error").is_none(), "{init}");
        mcp.write(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        }));
        mcp
    }

    fn write(&mut self, msg: &Value) {
        writeln!(self.stdin, "{msg}").expect("write");
        self.stdin.flush().expect("flush");
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            let read = self.stdout.read_line(&mut line).expect("read");
            assert!(read > 0, "{method}: the server closed the pipe");
            let value: Value = serde_json::from_str(&line).unwrap_or_else(|err| {
                panic!("{method}: not JSON: {err}: {line}");
            });
            if value.get("id").and_then(Value::as_i64) == Some(id) {
                return value;
            }
        }
    }

    fn call(&mut self, arguments: Value) -> Value {
        self.request(
            "tools/call",
            json!({"name": "vissue_target", "arguments": arguments}),
        )
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Two writers setting a land field to different values keep both, and the
/// merge says so.
#[test]
fn a_clash_on_a_land_field_keeps_both_values() {
    let dir = tracker();
    let root = dir.path();
    let issue = run(root, &["create", "-p", "api", "Land the kit", "-q"]);
    let issue = issue.trim();
    let board = root.join("Software/api/issues.org");
    let base = dir.path().join("base.org");
    let ours = dir.path().join("ours.org");
    let theirs = dir.path().join("theirs.org");
    fs::copy(&board, &base).unwrap();

    run(root, &["target", issue, "--quorum", "seats-eight"]);
    fs::copy(&board, &ours).unwrap();
    fs::copy(&base, &board).unwrap();
    run(root, &["target", issue, "--quorum", "seats-five"]);
    fs::copy(&board, &theirs).unwrap();

    let out = vissue(
        root,
        &[
            "merge-driver",
            base.to_str().unwrap(),
            ours.to_str().unwrap(),
            theirs.to_str().unwrap(),
            "api/issues.org",
        ],
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    assert!(err.contains("clash"), "{err}");
    let merged = fs::read_to_string(&ours).unwrap();
    assert!(merged.contains("seats-eight"), "{merged}");
    assert!(merged.contains("seats-five"), "{merged}");
    assert!(merged.contains(":MERGE_CONFLICT:"), "{merged}");
    assert!(merged.contains("QUORUM"), "{merged}");

    let carried = dir.path().join("carried.org");
    fs::copy(&ours, &carried).unwrap();
    fs::copy(&carried, &board).unwrap();
    run(root, &["target", issue, "--landers", "seat:ada"]);
    let ours2 = dir.path().join("ours2.org");
    fs::copy(&board, &ours2).unwrap();
    fs::copy(&carried, &board).unwrap();
    run(root, &["target", issue, "--landers", "seat:bob"]);
    let theirs2 = dir.path().join("theirs2.org");
    fs::copy(&board, &theirs2).unwrap();

    let out = vissue(
        root,
        &[
            "merge-driver",
            carried.to_str().unwrap(),
            ours2.to_str().unwrap(),
            theirs2.to_str().unwrap(),
            "api/issues.org",
        ],
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    assert!(err.contains("clash"), "{err}");
    let again = fs::read_to_string(&ours2).unwrap();
    let drawer = again.split(":MERGE_CONFLICT:").nth(1).unwrap_or("");
    assert!(
        drawer.contains("seats-five"),
        "the first clash value was dropped: {again}"
    );
    assert!(
        drawer.contains("seat:bob"),
        "the second clash value was dropped: {again}"
    );
    assert!(again.contains("seats-eight"), "{again}");
    assert!(again.contains("seat:ada"), "{again}");
}

/// A value that ends in `:` and a value that is `:END:` are still those
/// bytes after three merges.
#[test]
fn a_colon_clash_value_survives_three_merges() {
    let dir = tracker();
    let root = dir.path();
    let issue = run(root, &["create", "-p", "api", "Land the kit", "-q"]);
    let issue = issue.trim();
    let board = root.join("Software/api/issues.org");
    let base = dir.path().join("base.org");
    fs::copy(&board, &base).unwrap();
    run(
        root,
        &[
            "target",
            issue,
            "--quorum",
            "kept-ours",
            "--landers",
            "kept-ours",
        ],
    );
    let ours = dir.path().join("ours.org");
    fs::copy(&board, &ours).unwrap();
    fs::copy(&base, &board).unwrap();
    run(
        root,
        &["target", issue, "--quorum", "lost:", "--landers", ":END:"],
    );
    let theirs = dir.path().join("theirs.org");
    fs::copy(&board, &theirs).unwrap();

    let once = merge_sides(root, dir.path(), &base, &ours, &theirs, "1");
    let twice = merge_self(root, dir.path(), &once, "2");
    let thrice = merge_self(root, dir.path(), &twice, "3");
    assert_eq!(
        (
            clash_value(&thrice, "property QUORUM"),
            clash_value(&thrice, "property LANDERS"),
        ),
        ("lost:".to_string(), ":END:".to_string()),
        "{thrice}"
    );
}

/// The same target ids in opposite order merge to one line, either way.
#[test]
fn targets_order_is_the_same_either_way() {
    let dir = tracker();
    let root = dir.path();
    let issue = run(root, &["create", "-p", "api", "Land the kit", "-q"]);
    let issue = issue.trim();
    let first = run(
        root,
        &[
            "target",
            "--repo",
            "github:acme/widgets",
            "--branch",
            "alpha",
        ],
    );
    let second = run(
        root,
        &[
            "target",
            "--repo",
            "github:acme/widgets",
            "--branch",
            "beta",
        ],
    );
    let first = first.trim();
    let second = second.trim();
    let board = root.join("Software/api/issues.org");
    let base = dir.path().join("base.org");
    fs::copy(&board, &base).unwrap();
    run(root, &["target", issue, "--add", first]);
    run(root, &["target", issue, "--add", second]);
    let ab = dir.path().join("ab.org");
    fs::copy(&board, &ab).unwrap();
    fs::copy(&base, &board).unwrap();
    run(root, &["target", issue, "--add", second]);
    run(root, &["target", issue, "--add", first]);
    let ba = dir.path().join("ba.org");
    fs::copy(&board, &ba).unwrap();

    let into_ba = merge_sides(root, dir.path(), &base, &ba, &ab, "into-ba");
    let into_ab = merge_sides(root, dir.path(), &base, &ab, &ba, "into-ab");
    let line = |text: &str| {
        text.lines()
            .find(|line| line.contains(":TARGETS:"))
            .unwrap_or("")
            .to_string()
    };
    assert_eq!(line(&into_ba), line(&into_ab), "{into_ba}\n{into_ab}");
}

/// An unknown id still runs the field check, as `issue/target` does.
#[test]
fn an_unknown_id_is_checked_before_not_found() {
    let dir = tracker();
    let root = dir.path();
    let socket = root.join("control.sock");
    let _serve = Serve::start(root, &socket);
    let mut rpc = Rpc::connect(&socket);
    let mut mcp = Mcp::start(root);
    let out = vissue(root, &["target", "no-such-issue", "--lease", "not-a-rule"]);
    let cli = String::from_utf8(out.stderr).unwrap();
    let sock = rpc
        .call(
            "issue/target",
            json!({"id": "no-such-issue", "lease": "not-a-rule"}),
        )
        .expect_err("socket accepted an unknown lease");
    let reply = mcp.call(json!({"issue_id": "no-such-issue", "lease": "not-a-rule"}));
    let needle = "unknown lease rule: not-a-rule";
    assert!(
        cli.contains(needle)
            && sock["message"].as_str().unwrap_or("").contains(needle)
            && reply["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains(needle),
        "cli={cli} socket={sock} mcp={reply}"
    );
}

/// A drawer in the shape develop writes (bare value lines, one of them
/// ending in `:`, inside a ledger) still has every value after three merges.
#[test]
fn a_caff198_clash_drawer_survives_three_merges() {
    let dir = tracker();
    let root = dir.path();
    // `{field}:\n{value}\n`, the bytes caff198a merge.rs writes. `Their title`
    // is only in the drawer. `lost:` is a value, not a field name.
    let ledger = "\
#+TODO: TODO | DONE CANCELLED

#+VISSUE_LEDGER:
#+VISSUE_LINES: 4 22
* TODO [#B] Land the kit
:PROPERTIES:
:ID:         kit-aaaa
:END:
:MERGE_CONFLICT:
property QUORUM:
seats-five
property LANDERS:
seat:bob
property TARGET:
tgt-theirs
state:
CANCELLED
title:
Their title
ballot of ada:
[X] rev 1 ada
body:
DONE
drawer NOTES:
theirs-title-ONLY-IN-DRAWER
priority:
lost:
:END:
#+VISSUE_LEDGER_LOG:
";
    let once = merge_self(root, dir.path(), ledger, "legacy-1");
    let twice = merge_self(root, dir.path(), &once, "legacy-2");
    let thrice = merge_self(root, dir.path(), &twice, "legacy-3");
    let want = [
        "seats-five",
        "seat:bob",
        "tgt-theirs",
        "CANCELLED",
        "Their title",
        "[X] rev 1 ada",
        "DONE",
        "theirs-title-ONLY-IN-DRAWER",
        "lost:",
    ];
    let fields = [
        "property QUORUM",
        "property LANDERS",
        "property TARGET",
        "state",
        "title",
        "ballot of ada",
        "body",
        "drawer NOTES",
        "priority",
    ];
    let live = |text: &str| {
        let start = text.rfind(":MERGE_CONFLICT:").expect("clash drawer");
        let rest = &text[start..];
        let end = rest.find(":END:").expect("drawer end");
        let drawer = &rest[..end + ":END:".len()];
        let values: Vec<String> = fields
            .iter()
            .map(|field| clash_value(drawer, field))
            .collect();
        let emptied = drawer.lines().any(|line| line == "%" || line == "lost:");
        (values, emptied)
    };
    assert_eq!(
        (live(&once), live(&twice), live(&thrice)),
        (
            (vec_of(want), false),
            (vec_of(want), false),
            (vec_of(want), false),
        ),
        "{thrice}"
    );
}

fn vec_of<const N: usize>(rows: [&str; N]) -> Vec<String> {
    rows.iter().map(|row| (*row).to_string()).collect()
}

/// `--from` without `--to` checks the lease, then the pair, as the socket does.
#[test]
fn from_without_to_uses_the_socket_error() {
    let dir = tracker();
    let root = dir.path();
    let socket = root.join("control.sock");
    let _serve = Serve::start(root, &socket);
    let mut rpc = Rpc::connect(&socket);
    let mut mcp = Mcp::start(root);
    let bad = vissue(
        root,
        &[
            "target",
            "no-such-issue",
            "--from",
            "sha256:abc",
            "--lease",
            "not-a-rule",
        ],
    );
    let bad_cli = String::from_utf8(bad.stderr).unwrap();
    let bad_sock = rpc
        .call(
            "issue/target",
            json!({"id": "no-such-issue", "from": "sha256:abc", "lease": "not-a-rule"}),
        )
        .expect_err("socket accepted an unknown lease");
    let bad_mcp =
        mcp.call(json!({"issue_id": "no-such-issue", "from": "sha256:abc", "lease": "not-a-rule"}));
    let pair = vissue(
        root,
        &[
            "target",
            "no-such-issue",
            "--from",
            "sha256:abc",
            "--lease",
            "strict-base",
        ],
    );
    let pair_cli = String::from_utf8(pair.stderr).unwrap();
    let pair_sock = rpc
        .call(
            "issue/target",
            json!({"id": "no-such-issue", "from": "sha256:abc", "lease": "strict-base"}),
        )
        .expect_err("socket accepted a one-sided retarget");
    let pair_mcp = mcp
        .call(json!({"issue_id": "no-such-issue", "from": "sha256:abc", "lease": "strict-base"}));
    let lease = "unknown lease rule: not-a-rule";
    let both = "retarget needs --from and --to";
    assert!(
        bad.status.code() == Some(1)
            && bad_cli.contains(lease)
            && !bad_cli.contains(both)
            && bad_sock["code"].as_i64() == Some(-32603)
            && bad_sock["message"].as_str().unwrap_or("").contains(lease)
            && bad_mcp["error"]["code"].as_i64() == Some(-32603)
            && bad_mcp["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains(lease)
            && pair.status.code() == Some(1)
            && pair_cli.contains(both)
            && !pair_cli.contains(lease)
            && pair_sock["code"].as_i64() == Some(-32602)
            && pair_sock["data"]["code"].as_str() == Some("retarget_needs_both")
            && pair_sock["message"].as_str().unwrap_or("").contains(both)
            && pair_mcp["error"]["code"].as_i64() == Some(-32602)
            && pair_mcp["error"]["data"]["code"].as_str() == Some("retarget_needs_both")
            && pair_mcp["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains(both)
            && !root.join("Software/targets.org").exists(),
        "bad cli={bad_cli} sock={bad_sock} mcp={bad_mcp} pair cli={pair_cli} sock={pair_sock} mcp={pair_mcp}"
    );
}

/// A missing issue is `-32004` `not_found` on the socket and on MCP.
#[test]
fn a_missing_issue_is_not_found() {
    let dir = tracker();
    let root = dir.path();
    let socket = root.join("control.sock");
    let _serve = Serve::start(root, &socket);
    let mut rpc = Rpc::connect(&socket);
    let mut mcp = Mcp::start(root);
    let out = vissue(root, &["target", "no-such-issue"]);
    let cli = String::from_utf8(out.stderr).unwrap();
    let sock = rpc
        .call("issue/target", json!({"id": "no-such-issue"}))
        .expect_err("socket found a missing issue");
    let reply = mcp.call(json!({"issue_id": "no-such-issue"}));
    let needle = "issue no-such-issue not found";
    assert!(
        out.status.code() == Some(1)
            && cli.contains(needle)
            && sock["code"].as_i64() == Some(-32004)
            && sock["data"]["code"].as_str() == Some("not_found")
            && sock["data"]["id"].as_str() == Some("no-such-issue")
            && sock["message"].as_str().unwrap_or("").contains(needle)
            && reply["error"]["code"].as_i64() == Some(-32004)
            && reply["error"]["data"]["code"].as_str() == Some("not_found")
            && reply["error"]["data"]["id"].as_str() == Some("no-such-issue")
            && reply["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains(needle),
        "cli={cli} socket={sock} mcp={reply}"
    );
}

fn merge_sides(
    root: &Path,
    scratch: &Path,
    base: &Path,
    ours: &Path,
    theirs: &Path,
    tag: &str,
) -> String {
    let work = scratch.join(format!("work-{tag}.org"));
    fs::copy(ours, &work).unwrap();
    let out = vissue(
        root,
        &[
            "merge-driver",
            base.to_str().unwrap(),
            work.to_str().unwrap(),
            theirs.to_str().unwrap(),
            "api/issues.org",
        ],
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    fs::read_to_string(&work).unwrap()
}

fn merge_self(root: &Path, scratch: &Path, text: &str, tag: &str) -> String {
    let side = scratch.join(format!("side-{tag}.org"));
    fs::write(&side, text).unwrap();
    merge_sides(root, scratch, &side, &side, &side, tag)
}

fn clash_value(text: &str, field: &str) -> String {
    let mut inside = false;
    let mut current: Option<String> = None;
    let mut lines: Vec<String> = Vec::new();
    let mut found = String::new();
    for line in text.lines() {
        if !inside {
            if line.trim().eq_ignore_ascii_case(":MERGE_CONFLICT:") {
                inside = true;
            }
            continue;
        }
        if line.eq_ignore_ascii_case(":END:") {
            break;
        }
        if let Some(value) = line.strip_prefix('%') {
            if current.as_deref() == Some(field) {
                lines.push(value.to_string());
            }
            continue;
        }
        if let Some(name) = line.strip_suffix(':')
            && !name.is_empty()
            && !line.starts_with(':')
        {
            if current.as_deref() == Some(field) {
                found = lines.join("\n");
            }
            current = Some(name.to_string());
            lines.clear();
        }
    }
    if current.as_deref() == Some(field) {
        found = lines.join("\n");
    }
    found
}
