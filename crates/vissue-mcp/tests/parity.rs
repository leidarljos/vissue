//! The read tools against the command they mirror.
//!
//! One side lists the tools and calls one. The other runs `vissue <verb> --json`.
//! For list, show, ready, tree, search, children, claims, and recall those two
//! answers have to be the same filters and the same rows. The fixture vault is
//! the tracker both are pointed at.

#![allow(missing_docs)]

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

const VERBS: &[&str] = &[
    "list", "show", "ready", "tree", "search", "children", "claims", "recall",
];

/// Flags that choose a rendering, not which rows come back.
///
/// `--json` is how the command asks for the rows the tool returns already.
/// `--org` is the separate `vissue_org` tool. `--all` prints every issue from
/// the command line. `--deeds-only` prints one field of the recall object.
/// `--help` is clap's.
const RENDERING: &[&str] = &["json", "help", "org", "all", "deeds-only"];

/// Flags that choose the tracker, not the rows inside it.
///
/// They are global on every subcommand. The server is already pointed at one
/// tracker by `VISSUE_ROOT`, `VISSUE_PREFIX`, and `VISSUE_NO_ROUTE`, so the
/// tool does not take them again.
const TRACKER: &[&str] = &["root", "prefix", "no-route"];

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixture_vault")
        .canonicalize()
        .expect("fixture vault")
}

fn vissue_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_vissue") {
        return PathBuf::from(path);
    }
    // This test binary lives in target/<profile>/deps. `cargo test --workspace`
    // has already built the sibling `vissue` before any test runs.
    let exe = std::env::current_exe().expect("test executable");
    let built = exe
        .parent()
        .and_then(|dir| dir.parent())
        .expect("target profile dir")
        .join("vissue");
    if built.is_file() {
        return built;
    }
    // A test run of this crate alone does not build the cli. Compile it into
    // its own target dir so the build does not wait on the lock this `cargo
    // test` already holds.
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out_dir = workspace.join("target/parity-cli");
    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", "vissue-cli", "--bin", "vissue"])
        .current_dir(&workspace)
        .env("CARGO_TARGET_DIR", &out_dir)
        .status()
        .expect("cargo build vissue");
    assert!(status.success(), "cargo build -p vissue-cli failed");
    let bin = out_dir.join("debug/vissue");
    assert!(bin.is_file(), "no vissue binary at {}", bin.display());
    bin
}

fn help_text(verb: &str) -> String {
    let out = Command::new(vissue_bin())
        .args([verb, "--help"])
        .env("NO_COLOR", "1")
        .output()
        .expect("run vissue --help");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "vissue {verb} --help failed\n{stdout}{stderr}"
    );
    if stdout.contains("Usage:") {
        stdout.into_owned()
    } else {
        stderr.into_owned()
    }
}

fn usage_line(help: &str) -> String {
    let mut out = String::new();
    let mut taking = false;
    for line in help.lines() {
        if line.starts_with("Usage:") {
            taking = true;
        } else if taking && (line.is_empty() || !line.starts_with(' ')) {
            break;
        }
        if taking {
            out.push(' ');
            out.push_str(line.trim());
        }
    }
    out
}

fn long_flags(help: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in help.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('-') {
            continue;
        }
        for word in trimmed.split_whitespace() {
            let Some(name) = word.strip_prefix("--") else {
                continue;
            };
            let name = name.trim_end_matches(',');
            if !name.is_empty() {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// `--by` is the holder. The tool spells that argument `holder`. Every other
/// flag becomes the tool property by turning hyphens into underscores.
fn property_name(flag: &str) -> String {
    if flag == "by" {
        return "holder".to_string();
    }
    flag.replace('-', "_")
}

fn cli_filters(help: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for flag in long_flags(help) {
        if RENDERING.contains(&flag.as_str()) || TRACKER.contains(&flag.as_str()) {
            continue;
        }
        out.insert(property_name(&flag));
    }
    let usage = usage_line(help);
    if usage.contains("<ID>") || usage.contains("[ID]") {
        out.insert("issue_id".to_string());
    }
    if usage.contains("<QUERY>") || usage.contains("[QUERY]") {
        out.insert("query".to_string());
    }
    out
}

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl Server {
    fn start(root: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_vissue-mcp"))
            .env("VISSUE_ROOT", root)
            .env("VISSUE_PREFIX", "Software")
            .env("VISSUE_NO_ROUTE", "1")
            .env_remove("VISSUE_CONFIG")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
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

    fn write(&mut self, msg: &Value) {
        let stdin = self.stdin.as_mut().expect("stdin is still open");
        writeln!(stdin, "{msg}").expect("write");
        stdin.flush().expect("flush");
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            let read = self.stdout.read_line(&mut line).expect("read");
            assert!(read > 0, "{method}: the server closed the pipe");
            let value: Value = serde_json::from_str(&line)
                .unwrap_or_else(|err| panic!("{method}: not JSON: {err}: {line}"));
            if value.get("id").and_then(Value::as_i64) == Some(id) {
                return value;
            }
        }
    }

    fn handshake(&mut self) {
        let init = self.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "parity", "version": "0"},
            }),
        );
        assert!(init.get("error").is_none(), "{init}");
        self.write(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }));
    }

    fn tools(&mut self) -> Vec<Tool> {
        let listed = self.request("tools/list", json!({}));
        serde_json::from_value(listed["result"]["tools"].clone())
            .unwrap_or_else(|err| panic!("tools/list: {err}: {listed}"))
    }

    fn call(&mut self, name: &str, arguments: Value) -> Result<String, String> {
        let reply = self.request("tools/call", json!({"name": name, "arguments": arguments}));
        if let Some(err) = reply.get("error") {
            return Err(err.to_string());
        }
        let result = &reply["result"];
        if result["isError"].as_bool().unwrap_or(false) {
            return Err(result.to_string());
        }
        Ok(result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(serde::Deserialize)]
struct Tool {
    name: String,
    #[serde(rename = "inputSchema")]
    input_schema: InputSchema,
}

#[derive(serde::Deserialize)]
struct InputSchema {
    #[serde(default)]
    properties: std::collections::BTreeMap<String, Value>,
    #[serde(default)]
    required: Vec<String>,
}

impl Tool {
    /// Filter names, without `json`, which is the rendering flag the command
    /// spells `--json`.
    fn filters(&self) -> BTreeSet<String> {
        self.input_schema
            .properties
            .keys()
            .filter(|name| name.as_str() != "json")
            .cloned()
            .collect()
    }
}

#[test]
fn each_read_verb_has_a_tool_with_the_same_filters() {
    let mut server = Server::start(&fixture_root());
    server.handshake();
    let tools = server.tools();
    assert!(tools.len() > 20, "only {} tools listed", tools.len());

    let mut wrong = Vec::new();
    for verb in VERBS {
        let help = help_text(verb);
        let expected = cli_filters(&help);
        assert!(
            !expected.is_empty(),
            "vissue {verb} --help named no filters:\n{help}"
        );
        let Some(tool) = tools
            .iter()
            .find(|tool| tool.name == format!("vissue_{verb}"))
        else {
            wrong.push(format!("no tool vissue_{verb}"));
            continue;
        };
        let actual = tool.filters();
        if actual != expected {
            wrong.push(format!(
                "{verb}: command filters {expected:?}, tool filters {actual:?}"
            ));
        }
        for name in &expected {
            let required = tool.input_schema.required.iter().any(|item| item == name);
            if name == "issue_id" || name == "query" {
                if !required {
                    wrong.push(format!(
                        "{verb}.{name} is the id or the query and is optional"
                    ));
                }
            } else if required {
                wrong.push(format!(
                    "{verb}.{name} is a filter the command lets a caller omit, and the tool requires it"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

struct Case {
    label: &'static str,
    /// Command arguments, including the verb. `--json` is added unless `text`.
    cli: &'static [&'static str],
    tool: &'static str,
    args: Value,
    /// Compare the rendered text. The row cases compare JSON.
    text: bool,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            label: "list",
            cli: &["list"],
            tool: "vissue_list",
            args: json!({}),
            text: false,
        },
        Case {
            label: "list project and state",
            cli: &["list", "--project", "atlas", "--state", "TODO"],
            tool: "vissue_list",
            args: json!({"project": "atlas", "state": "TODO"}),
            text: false,
        },
        Case {
            label: "list unclaimed",
            cli: &["list", "--unclaimed"],
            tool: "vissue_list",
            args: json!({"unclaimed": true}),
            text: false,
        },
        Case {
            label: "list claimed-by",
            cli: &["list", "--claimed-by", "fixture-agent"],
            tool: "vissue_list",
            args: json!({"claimed_by": "fixture-agent"}),
            text: false,
        },
        Case {
            label: "list parent",
            cli: &["list", "--parent", "atlas-1a2b"],
            tool: "vissue_list",
            args: json!({"parent": "atlas-1a2b"}),
            text: false,
        },
        Case {
            label: "list one tag",
            cli: &["list", "--tag", "parser"],
            tool: "vissue_list",
            args: json!({"tag": ["parser"]}),
            text: false,
        },
        Case {
            label: "list two tags",
            cli: &["list", "--tag", "feature", "--tag", "parser"],
            tool: "vissue_list",
            args: json!({"tag": ["feature", "parser"]}),
            text: false,
        },
        Case {
            label: "list file tag is not a heading tag",
            cli: &["list", "--tag", "issues"],
            tool: "vissue_list",
            args: json!({"tag": ["issues"]}),
            text: false,
        },
        Case {
            label: "list limit",
            cli: &["list", "--limit", "4"],
            tool: "vissue_list",
            args: json!({"limit": 4}),
            text: false,
        },
        Case {
            label: "list several filters",
            cli: &[
                "list",
                "--project",
                "atlas",
                "--state",
                "TODO",
                "--unclaimed",
                "--limit",
                "1",
            ],
            tool: "vissue_list",
            args: json!({"project": "atlas", "state": "TODO", "unclaimed": true, "limit": 1}),
            text: false,
        },
        Case {
            label: "ready",
            cli: &["ready"],
            tool: "vissue_ready",
            args: json!({}),
            text: false,
        },
        Case {
            label: "ready project",
            cli: &["ready", "--project", "atlas"],
            tool: "vissue_ready",
            args: json!({"project": "atlas"}),
            text: false,
        },
        Case {
            label: "ready unclaimed",
            cli: &["ready", "--unclaimed"],
            tool: "vissue_ready",
            args: json!({"unclaimed": true}),
            text: false,
        },
        Case {
            label: "ready claimed-by",
            cli: &["ready", "--claimed-by", "fixture-agent"],
            tool: "vissue_ready",
            args: json!({"claimed_by": "fixture-agent"}),
            text: false,
        },
        Case {
            label: "ready parent",
            cli: &["ready", "--parent", "atlas-1a2b"],
            tool: "vissue_ready",
            args: json!({"parent": "atlas-1a2b"}),
            text: false,
        },
        Case {
            label: "ready tag",
            cli: &["ready", "--tag", "feature"],
            tool: "vissue_ready",
            args: json!({"tag": ["feature"]}),
            text: false,
        },
        Case {
            label: "ready limit",
            cli: &["ready", "--limit", "1"],
            tool: "vissue_ready",
            args: json!({"limit": 1}),
            text: false,
        },
        Case {
            label: "show",
            cli: &["show", "atlas-2c3d"],
            tool: "vissue_show",
            args: json!({"issue_id": "atlas-2c3d"}),
            text: false,
        },
        Case {
            label: "show with a logbook and a deed",
            cli: &["show", "atlas-4g5h"],
            tool: "vissue_show",
            args: json!({"issue_id": "atlas-4g5h"}),
            text: false,
        },
        Case {
            label: "tree",
            cli: &["tree", "atlas-1a2b"],
            tool: "vissue_tree",
            args: json!({"issue_id": "atlas-1a2b"}),
            text: false,
        },
        Case {
            label: "tree ascii",
            cli: &["tree", "atlas-1a2b", "--format", "ascii"],
            tool: "vissue_tree",
            args: json!({"issue_id": "atlas-1a2b", "format": "ascii"}),
            text: true,
        },
        Case {
            label: "search",
            cli: &["search", "header"],
            tool: "vissue_search",
            args: json!({"query": "header"}),
            text: false,
        },
        Case {
            label: "search project and limit",
            cli: &["search", "the", "--project", "beacon", "--limit", "1"],
            tool: "vissue_search",
            args: json!({"query": "the", "project": "beacon", "limit": 1}),
            text: false,
        },
        Case {
            label: "children",
            cli: &["children", "atlas-1a2b"],
            tool: "vissue_children",
            args: json!({"issue_id": "atlas-1a2b"}),
            text: false,
        },
        Case {
            label: "children of an issue with none",
            cli: &["children", "atlas-4g5h"],
            tool: "vissue_children",
            args: json!({"issue_id": "atlas-4g5h"}),
            text: false,
        },
        Case {
            label: "claims",
            cli: &["claims"],
            tool: "vissue_claims",
            args: json!({}),
            text: false,
        },
        Case {
            label: "claims by holder and project",
            cli: &["claims", "--by", "fixture-agent", "--project", "atlas"],
            tool: "vissue_claims",
            args: json!({"holder": "fixture-agent", "project": "atlas"}),
            text: false,
        },
        Case {
            label: "claims in a project with none",
            cli: &["claims", "--project", "beacon"],
            tool: "vissue_claims",
            args: json!({"project": "beacon"}),
            text: false,
        },
        Case {
            label: "recall",
            cli: &["recall", "atlas-3e4f"],
            tool: "vissue_recall",
            args: json!({"issue_id": "atlas-3e4f"}),
            text: false,
        },
        Case {
            label: "recall depth and excerpts",
            cli: &["recall", "atlas-3e4f", "--depth", "2", "--excerpts"],
            tool: "vissue_recall",
            args: json!({"issue_id": "atlas-3e4f", "depth": 2, "excerpts": true}),
            text: false,
        },
    ]
}

fn command_output(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(vissue_bin())
        .arg("--root")
        .arg(root)
        .arg("--prefix")
        .arg("Software")
        .args(args)
        .env("VISSUE_ROOT", root)
        .env("VISSUE_PREFIX", "Software")
        .env("VISSUE_NO_ROUTE", "1")
        .env_remove("VISSUE_CONFIG")
        .env("NO_COLOR", "1")
        .output()
        .expect("run vissue")
}

#[test]
fn each_read_verb_returns_the_same_rows_as_json() {
    let root = fixture_root();
    let mut server = Server::start(&root);
    server.handshake();

    let mut wrong = Vec::new();
    for case in cases() {
        let mut args: Vec<&str> = case.cli.to_vec();
        if !case.text {
            args.push("--json");
        }
        let out = command_output(&root, &args);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !out.status.success() {
            wrong.push(format!("{}: command failed\n{stdout}{stderr}", case.label));
            continue;
        }
        let tool = match server.call(case.tool, case.args) {
            Ok(text) => text,
            Err(err) => {
                wrong.push(format!("{}: tool failed: {err}", case.label));
                continue;
            }
        };
        if case.text {
            if tool != stdout {
                wrong.push(format!(
                    "{}:\ncommand: {stdout:?}\ntool:    {tool:?}",
                    case.label
                ));
            }
            continue;
        }
        let command_rows: Value = match serde_json::from_str(&stdout) {
            Ok(value) => value,
            Err(err) => {
                wrong.push(format!(
                    "{}: command output is not JSON: {err}: {stdout}",
                    case.label
                ));
                continue;
            }
        };
        let tool_rows: Value = match serde_json::from_str(&tool) {
            Ok(value) => value,
            Err(err) => {
                wrong.push(format!(
                    "{}: tool output is not JSON: {err}: {tool}",
                    case.label
                ));
                continue;
            }
        };
        if command_rows != tool_rows {
            wrong.push(format!(
                "{}:\ncommand: {command_rows}\ntool:    {tool_rows}",
                case.label
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
}
