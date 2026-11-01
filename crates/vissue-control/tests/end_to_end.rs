//! A client and a server actually talking over a Unix socket.
//!
//! Every other test in this crate exercises one half in isolation: framing
//! against a buffer, a method against a JSON value. That leaves the question
//! this file answers, which is whether the two halves agree once a real
//! socket is between them.
//!
//! The server here is deliberately minimal and lives in the test rather than
//! the crate, because the crate ships no server. It reads one framed request,
//! dispatches it against a corpus, and writes one framed response.

#![cfg(unix)]
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::io::{BufReader, BufWriter};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::thread;

use serde_json::{Value, json};
use vissue_control::client::Client;
use vissue_control::frame::{Framing, read_message, write_message};
use vissue_control::rpc::{
    IssueGetResult, JsonRpcRequest, JsonRpcResponse, Method, error_from_core, method_not_found,
};
use vissue_core::catalog::{CatalogService, issues_rows_from};
use vissue_core::model::IssueHeading;
use vissue_core::views::{IssueRec, ListQuery};

fn issue(project: &str, id: &str, state: &str, title: &str) -> IssueRec {
    let mut properties = BTreeMap::new();
    properties.insert("ID".to_string(), id.to_string());
    IssueRec {
        project: project.to_string(),
        heading: IssueHeading {
            id: id.to_string(),
            title: title.to_string(),
            state: state.to_string(),
            priority: 'B',
            properties,
            org_tags: Vec::new(),
            statistics: None,
            property_order: vec!["ID".to_string()],
            extra_drawers: Vec::new(),
            body: String::new(),
            logbook: Vec::new(),
            line_start: 1,
            line_end: 5,
        },
        path: PathBuf::from("/tmp/none/issues.org"),
        tag_settings: vissue_core::org::TagSettings::default(),
    }
}

fn corpus() -> Vec<IssueRec> {
    vec![
        issue("atlas", "atlas-1a2b", "STARTED", "Parse the header"),
        issue("atlas", "atlas-2c3d", "TODO", "Emit a summary table"),
        issue("beacon", "beacon-5j6k", "DONE", "Document the retry policy"),
    ]
}

/// Serve exactly `count` requests, then return.
///
/// This is the piece the workspace does not have: something that owns the
/// socket and maps a method name onto the catalog.
fn serve(listener: UnixListener, count: usize) {
    for _ in 0..count {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let mut writer = BufWriter::new(stream);

        let (payload, framing) = match read_message(&mut reader) {
            Ok(pair) => pair,
            Err(_) => return,
        };
        let request: JsonRpcRequest = serde_json::from_slice(&payload).expect("a request");
        let recs = corpus();
        let service = CatalogService::from_recs(&recs);

        let response = match request.method.as_str() {
            m if m == Method::IssueList.as_str() => {
                let rows = issues_rows_from(&recs, ListQuery::default()).expect("rows");
                JsonRpcResponse::ok(request.id.clone(), json!({ "issues": rows }))
            }
            m if m == Method::IssueReady.as_str() => {
                let rows = service.ready(None).expect("ready");
                JsonRpcResponse::ok(request.id.clone(), json!({ "issues": rows }))
            }
            m if m == Method::IssueGet.as_str() => {
                let id = request
                    .params
                    .as_ref()
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let looked_up = service.detail(id);
                match looked_up {
                    // The real server flattens the detail into the result, so
                    // a stand-in that nests it would let a client pass here
                    // and fail against `vissue serve`.
                    Ok(detail) => {
                        let body = IssueGetResult {
                            issue: detail,
                            revision: 1,
                        };
                        match serde_json::to_value(body) {
                            Ok(value) => JsonRpcResponse::ok(request.id.clone(), value),
                            Err(err) => panic!("encode detail: {err}"),
                        }
                    }
                    Err(err) => JsonRpcResponse::err(request.id.clone(), error_from_core(&err)),
                }
            }
            other => JsonRpcResponse::err(request.id.clone(), method_not_found(other)),
        };

        let body = serde_json::to_vec(&response).expect("encode");
        write_message(&mut writer, &body, framing).expect("write");
    }
}

fn start() -> (PathBuf, tempfile::TempDir, thread::JoinHandle<()>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("control.sock");
    let listener = UnixListener::bind(&path).expect("bind");
    let handle = thread::spawn(move || serve(listener, 4));
    (path, dir, handle)
}

#[test]
fn a_request_crosses_the_socket_and_comes_back_answered() {
    let (path, _dir, handle) = start();

    let mut client = Client::connect(&path).expect("connect");
    let value = client
        .request(Method::IssueList.as_str(), json!({}))
        .expect("issue/list");
    let issues = value["issues"].as_array().expect("an issues array");
    assert_eq!(issues.len(), 3, "{value}");
    assert!(issues.iter().any(|i| i["id"] == "atlas-1a2b"), "{value}");
    drop(client);

    let mut client = Client::connect(&path).expect("reconnect");
    let ready = client
        .request("issue/ready", json!({}))
        .expect("issue/ready");
    let ids: Vec<&str> = ready["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"atlas-1a2b"), "{ids:?}");
    assert!(
        !ids.contains(&"beacon-5j6k"),
        "closed work is not ready: {ids:?}"
    );
    drop(client);

    let mut client = Client::connect(&path).expect("reconnect");
    let one = client
        .request(Method::IssueGet.as_str(), json!({ "id": "atlas-2c3d" }))
        .expect("issue/get");
    assert_eq!(one["title"], "Emit a summary table");
    drop(client);

    // A core error has to arrive as a JSON-RPC error, not as a broken pipe.
    let mut client = Client::connect(&path).expect("reconnect");
    let err = client
        .request(Method::IssueGet.as_str(), json!({ "id": "atlas-zzzz" }))
        .expect_err("an unknown id is an error");
    assert!(
        err.to_string().contains("atlas-zzzz") || err.to_string().contains("not found"),
        "{err}"
    );

    handle.join().expect("server thread");
}

#[test]
fn the_client_and_the_server_agree_on_framing() {
    // The client picks a framing; whatever it picked has to be what the
    // server reads and what it accepts back.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("framing.sock");
    let listener = UnixListener::bind(&path).expect("bind");

    let observed = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let mut writer = BufWriter::new(stream);
        let (payload, framing) = read_message(&mut reader).expect("read");
        let request: JsonRpcRequest = serde_json::from_slice(&payload).expect("request");
        let response = JsonRpcResponse::ok(request.id, json!({"ok": true}));
        let body = serde_json::to_vec(&response).expect("encode");
        write_message(&mut writer, &body, framing).expect("write");
        framing
    });

    let mut client = Client::connect_with_framing(&path, Framing::Jsonl).expect("connect");
    assert_eq!(client.framing(), Framing::Jsonl);
    let value = client.request("anything", json!({})).expect("round trip");
    assert_eq!(value["ok"], true);
    assert_eq!(observed.join().expect("thread"), Framing::Jsonl);
}

#[test]
fn a_socket_nobody_is_listening_on_is_an_error_not_a_hang() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("absent.sock");
    let err = match Client::connect(&path) {
        Ok(_) => panic!("connected to a socket nobody is serving"),
        Err(err) => err,
    };
    assert!(!err.to_string().is_empty());
}

#[test]
fn an_unknown_method_comes_back_as_method_not_found() {
    let (path, _dir, handle) = start();
    let mut client = Client::connect(&path).expect("connect");
    let err = client
        .request("issue/nope", json!({}))
        .expect_err("unknown method");
    let text = err.to_string();
    assert!(
        text.contains("issue/nope") || text.to_lowercase().contains("method"),
        "{text}"
    );
    drop(client);
    // Drain the remaining accepts so the server thread finishes.
    for _ in 0..3 {
        let connected = Client::connect(&path);
        if let Ok(mut c) = connected {
            let _ = c.request(Method::IssueList.as_str(), json!({}));
        }
    }
    handle.join().expect("server thread");
}
