//! The server as an agent meets it: a process on the other end of a pipe.
//!
//! Every other test here calls the tools as Rust functions, which proves what
//! they compute and nothing about whether an agent can reach them. The
//! handshake, the tool listing and the call framing all live in the protocol
//! layer, so a version bump that changed any of them would leave those tests
//! green and every client unable to connect.

#![allow(missing_docs)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixture_vault")
}

/// A running server, and the two ends of the pipe to it.
struct Server {
    child: Child,
    /// Taken when the test wants the server to see end of input.
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl Server {
    fn start(root: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_vissue-mcp"))
            .env("VISSUE_ROOT", root)
            .env("VISSUE_PREFIX", "Software")
            .env("VISSUE_AGENT", "mcp-stdio-test")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn vissue-mcp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Self {
            child,
            stdin: Some(stdin),
            stdout,
            next_id: 0,
        }
    }

    fn notify(&mut self, method: &str, params: Value) {
        let msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
        self.write(&msg);
    }

    fn write(&mut self, msg: &Value) {
        let stdin = self.stdin.as_mut().expect("stdin is still open");
        writeln!(stdin, "{msg}").expect("write");
        stdin.flush().expect("flush");
    }

    /// Let the server see end of input, and report how it exited.
    fn close_and_wait(&mut self) -> std::process::ExitStatus {
        drop(self.stdin.take());
        self.child.wait().expect("wait")
    }

    /// Send a request and read until the reply carrying its id.
    ///
    /// Anything else on the pipe is a notification the server chose to send,
    /// which is not what this call is waiting for.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.write(&msg);

        loop {
            let mut line = String::new();
            let read = self.stdout.read_line(&mut line).expect("read");
            assert!(read > 0, "{method}: the server closed the pipe");
            let value: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(err) => panic!("{method}: not JSON: {err}: {line:?}"),
            };
            if value.get("id").and_then(Value::as_i64) == Some(id) {
                return value;
            }
        }
    }

    fn handshake(&mut self) -> Value {
        let init = self.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "vissue-stdio-test", "version": "0"},
            }),
        );
        self.notify("notifications/initialized", json!({}));
        init
    }

    /// The text of a tool result, which is where every tool answers.
    fn call_text(&mut self, name: &str, arguments: Value) -> Result<String, Value> {
        let reply = self.request("tools/call", json!({"name": name, "arguments": arguments}));
        if let Some(err) = reply.get("error") {
            return Err(err.clone());
        }
        let result = &reply["result"];
        if result["isError"].as_bool().unwrap_or(false) {
            return Err(result.clone());
        }
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        Ok(text)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn the_server_completes_a_handshake_and_names_itself() {
    let mut server = Server::start(&fixture_root());
    let init = server.handshake();
    assert!(init.get("error").is_none(), "{init}");

    let info = &init["result"]["serverInfo"];
    assert_eq!(info["name"], "vissue", "{init}");
    assert_eq!(info["version"], env!("CARGO_PKG_VERSION"), "{init}");
    assert!(
        init["result"]["capabilities"].get("tools").is_some(),
        "a server with no tools capability offers an agent nothing: {init}"
    );
}

#[test]
fn every_tool_is_listed_with_a_schema_an_agent_can_fill_in() {
    let mut server = Server::start(&fixture_root());
    server.handshake();

    let listed = server.request("tools/list", json!({}));
    let tools = listed["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("no tools array: {listed}"));
    assert!(tools.len() > 20, "only {} tools listed", tools.len());

    for tool in tools {
        let name = tool["name"].as_str().unwrap_or_default();
        assert!(name.starts_with("vissue_"), "odd tool name: {name}");
        assert!(
            tool["description"].as_str().is_some_and(|d| !d.is_empty()),
            "{name} has no description for an agent to read"
        );
        // Without a schema an agent cannot know what to pass.
        assert_eq!(
            tool["inputSchema"]["type"], "object",
            "{name} has no object input schema: {tool}"
        );
    }

    // What the binary lists is what the crate implements.
    let source =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/server.rs"))
            .expect("server.rs");
    let implemented = source
        .lines()
        .filter_map(|l| l.trim().strip_prefix("async fn "))
        .filter_map(|rest| rest.split('(').next())
        .filter(|n| n.starts_with("vissue_"))
        .count();
    assert_eq!(
        tools.len(),
        implemented,
        "the wire lists {} tools, the crate defines {implemented}",
        tools.len()
    );
}

#[test]
fn a_read_tool_answers_over_the_pipe() {
    let mut server = Server::start(&fixture_root());
    server.handshake();

    let ready = server.call_text("vissue_ready", json!({})).expect("ready");
    assert!(!ready.trim().is_empty(), "ready answered nothing");

    let shown = server
        .call_text("vissue_show", json!({"issue_id": "atlas-2c3d"}))
        .expect("show");
    let detail: Value = serde_json::from_str(&shown).expect("show returns JSON");
    assert_eq!(detail["id"], "atlas-2c3d", "{detail}");
    assert!(
        detail["body"].as_str().is_some_and(|b| !b.is_empty()),
        "the detail an agent receives carries no body: {detail}"
    );

    // The whole heading, for handing the issue over as a specification.
    let org = server
        .call_text("vissue_org", json!({"issue_id": "atlas-2c3d"}))
        .expect("org");
    assert!(org.starts_with("* "), "{org}");
    assert!(org.contains(":ID:"), "{org}");
}

#[test]
fn a_write_tool_changes_the_tracker_over_the_pipe() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("Software")).expect("projects dir");
    let mut server = Server::start(dir.path());
    server.handshake();

    let made = server
        .call_text(
            "vissue_create",
            json!({"project": "atlas", "title": "Made over stdio"}),
        )
        .expect("create");
    let id = made.split_whitespace().next().expect("an id").to_string();
    assert!(id.starts_with("atlas-"), "{made}");

    // A report with markdown in it, as an agent writes back.
    server
        .call_text(
            "vissue_append",
            json!({"issue_id": id, "text": "## Done\n\n* took the pipe\n"}),
        )
        .expect("append");

    let org = server
        .call_text("vissue_org", json!({"issue_id": id}))
        .expect("org");
    assert!(org.contains("took the pipe"), "{org}");

    // Markdown in a body leaves the tracker readable.
    let check = server.call_text("vissue_check", json!({})).expect("check");
    assert!(check.contains("0 error(s)"), "{check}");
}

/// The whole handoff over the pipe an agent actually speaks: one unit of work
/// names what it produced, and the next one's working set carries the
/// accession to whoever picks it up.
#[test]
fn the_working_set_reaches_an_agent_over_the_pipe() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("Software")).expect("projects dir");
    let mut server = Server::start(dir.path());
    server.handshake();

    let id_of = |made: String| made.split_whitespace().next().expect("an id").to_string();
    let first = id_of(
        server
            .call_text(
                "vissue_create",
                json!({"project": "keys", "title": "Catalog the actions"}),
            )
            .expect("create"),
    );
    let second = id_of(
        server
            .call_text(
                "vissue_create",
                json!({"project": "keys", "title": "Write the schema"}),
            )
            .expect("create"),
    );
    server
        .call_text("vissue_update", json!({"issue_id": second, "block": first}))
        .expect("block");

    server
        .call_text(
            "vissue_deed",
            json!({"issue_id": first, "add": ["deed-file-catalog"]}),
        )
        .expect("deed");

    let recalled = server
        .call_text("vissue_recall", json!({"issue_id": second}))
        .expect("recall");
    assert!(
        recalled.contains(&first),
        "the input is missing: {recalled}"
    );
    assert!(
        recalled.contains("deed-file-catalog"),
        "the input's product is the point: {recalled}"
    );

    // And the refusal an agent needs to see rather than a citation that
    // resolves to nothing.
    let refused = server.call_text(
        "vissue_deed",
        json!({"issue_id": second, "add": ["/tmp/note.md"]}),
    );
    assert!(refused.is_err(), "{refused:?}");
}

#[test]
fn failures_come_back_as_errors_rather_than_a_dropped_pipe() {
    let mut server = Server::start(&fixture_root());
    server.handshake();

    let unknown_id = server
        .call_text("vissue_show", json!({"issue_id": "atlas-zzzz"}))
        .expect_err("an unknown id is an error");
    assert!(
        format!("{unknown_id}").contains("atlas-zzzz"),
        "{unknown_id}"
    );

    let unknown_tool = server.call_text("vissue_nope", json!({}));
    assert!(unknown_tool.is_err(), "an unknown tool answered anyway");

    // Bad arguments are refused without taking the server down.
    let bad_args = server.call_text("vissue_show", json!({}));
    assert!(bad_args.is_err(), "a call with no id was accepted");

    // Still serving after all three.
    let ready = server.call_text("vissue_ready", json!({})).expect("ready");
    assert!(!ready.trim().is_empty());
}

#[test]
fn closing_the_pipe_ends_the_server() {
    let mut server = Server::start(&fixture_root());
    server.handshake();
    server.call_text("vissue_ready", json!({})).expect("ready");

    // An agent that goes away leaves the server nothing to read. It should
    // exit rather than sit on a dead pipe holding the tracker open.
    let status = server.close_and_wait();
    assert!(status.success(), "exited with {status}");
}
