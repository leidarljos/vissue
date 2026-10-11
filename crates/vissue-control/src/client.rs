//! Unix-socket JSON-RPC client. Clients never bind the control socket.

use std::io::{BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use crate::frame::{Framing, read_message, write_message};
use crate::rpc::{
    Error, JsonRpcError, JsonRpcId, JsonRpcRequest, JsonRpcResponse, Notification, Request,
    Response, invalid_request,
};

/// Connected client. One stream; notifications arriving during [`Self::request`]
/// are handed to the optional callback.
pub struct Client {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    framing: Framing,
    next_id: i64,
    on_notification: Option<Box<dyn FnMut(Notification) + Send>>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("framing", &self.framing)
            .field("next_id", &self.next_id)
            .field(
                "on_notification",
                &self.on_notification.as_ref().map(|_| "set"),
            )
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Connect to `path`. Uses JSONL framing.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket cannot be opened or cloned.
    pub fn connect(path: impl AsRef<Path>) -> Result<Self, Error> {
        Self::connect_with_framing(path, Framing::Jsonl)
    }

    /// Connect and send every request in `framing`. The owner replies in kind.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket cannot be opened or cloned.
    pub fn connect_with_framing(path: impl AsRef<Path>, framing: Framing) -> Result<Self, Error> {
        let stream = UnixStream::connect(path.as_ref())?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            writer: stream,
            reader,
            framing,
            next_id: 1,
            on_notification: None,
        })
    }

    /// Framing style this client writes.
    pub fn framing(&self) -> Framing {
        self.framing
    }

    /// Called for each JSON-RPC notification read while waiting on a response.
    pub fn on_notification<F>(&mut self, callback: F)
    where
        F: FnMut(Notification) + Send + 'static,
    {
        self.on_notification = Some(Box::new(callback));
    }

    /// Send `method`/`params` and return the result object.
    ///
    /// # Errors
    ///
    /// Returns an error when the write or read fails, the payload is not JSON,
    /// the response id does not match, or the server returns a JSON-RPC error.
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, Error> {
        let id = JsonRpcId::Number(self.next_id);
        self.next_id += 1;
        let msg = JsonRpcRequest::call(id.clone(), method, params);
        self.write_rpc(&msg)?;
        loop {
            let (payload, _) = read_message(&mut self.reader)?;
            let value: Value = serde_json::from_slice(&payload)?;
            if is_notification(&value) {
                let method = value
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let params = value.get("params").cloned().unwrap_or(Value::Null);
                let note = Notification::parse(&method, params);
                if let Some(cb) = &mut self.on_notification {
                    cb(note);
                }
                continue;
            }
            let resp: JsonRpcResponse = serde_json::from_value(value)?;
            if resp.id.as_ref() != Some(&id) {
                return Err(Error::Rpc(id_mismatch(resp.error)));
            }
            if let Some(err) = resp.error {
                return Err(Error::Rpc(err));
            }
            return Ok(resp.result.unwrap_or(Value::Null));
        }
    }

    /// Typed request helper. Result is the raw JSON body (caller decodes).
    ///
    /// # Errors
    ///
    /// Returns an error when the write or read fails, the payload is not JSON,
    /// the response id does not match, or the server returns a JSON-RPC error.
    pub fn request_typed(&mut self, req: &Request) -> Result<Value, Error> {
        self.request(req.method().as_str(), req.to_params())
    }

    /// Send a notification (no `id`). Does not wait.
    ///
    /// # Errors
    ///
    /// Returns an error when the notification cannot be encoded or written.
    pub fn notify(&mut self, method: &str, params: Value) -> Result<(), Error> {
        self.write_rpc(&JsonRpcRequest::notification(method, params))
    }

    /// Block until the next JSON-RPC notification, or `timeout`.
    ///
    /// # Errors
    ///
    /// Returns an error when the wait times out, the read or decode fails, or
    /// the next message is a response rather than a notification.
    pub fn wait_notification(&mut self, timeout: Duration) -> Result<Notification, Error> {
        self.reader.get_ref().set_read_timeout(Some(timeout))?;
        let result = read_next_notification(&mut self.reader);
        let _ = self.reader.get_ref().set_read_timeout(None);
        result
    }

    fn write_rpc(&mut self, msg: &JsonRpcRequest) -> Result<(), Error> {
        let bytes = serde_json::to_vec(msg)?;
        write_message(&mut self.writer, &bytes, self.framing)?;
        self.writer.flush()?;
        Ok(())
    }
}

fn read_next_notification(reader: &mut BufReader<UnixStream>) -> Result<Notification, Error> {
    loop {
        let (payload, _) = read_message(reader)?;
        if payload.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let value: Value = serde_json::from_slice(&payload)?;
        if is_notification(&value) {
            let method = value
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let params = value.get("params").cloned().unwrap_or(Value::Null);
            return Ok(Notification::parse(&method, params));
        }
        let mut err = invalid_request();
        err.message = "expected notification".into();
        return Err(Error::Rpc(err));
    }
}

/// True only when the object has `method` and no `id` member. `"id": null`
/// is a response, not a notification.
fn is_notification(value: &Value) -> bool {
    let obj = match value.as_object() {
        Some(o) => o,
        None => return false,
    };
    obj.contains_key("method") && !obj.contains_key("id") && !obj.contains_key("result")
}

fn id_mismatch(server_error: Option<JsonRpcError>) -> JsonRpcError {
    match server_error {
        Some(err) => err,
        None => {
            let mut err = invalid_request();
            err.message = "response id does not match request".into();
            err
        }
    }
}

/// Decode a typed [`Response`] from a raw result when the method is known.
///
/// # Errors
///
/// Returns an error when `method` is not a v1 method or `value` does not match
/// that method's result shape.
pub fn decode_response(method: &str, value: Value) -> Result<Response, Error> {
    match crate::rpc::Method::parse(method).map_err(Error::Rpc)? {
        crate::rpc::Method::Initialize => Ok(Response::Initialize(serde_json::from_value(value)?)),
        crate::rpc::Method::IdentityGet => {
            Ok(Response::IdentityGet(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueList => Ok(Response::IssueList(serde_json::from_value(value)?)),
        crate::rpc::Method::IssueGet => Ok(Response::IssueGet(serde_json::from_value(value)?)),
        crate::rpc::Method::IssueReady => Ok(Response::IssueReady(serde_json::from_value(value)?)),
        crate::rpc::Method::IssueSearch => {
            Ok(Response::IssueSearch(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueClaims => {
            Ok(Response::IssueClaims(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueAgenda => {
            Ok(Response::IssueAgenda(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueShow => Ok(Response::IssueShow(serde_json::from_value(value)?)),
        crate::rpc::Method::IssueExcerpt => {
            Ok(Response::IssueExcerpt(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueTree => Ok(Response::IssueTree(serde_json::from_value(value)?)),
        crate::rpc::Method::IssueRelated => {
            Ok(Response::IssueRelated(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueChildren => {
            Ok(Response::IssueChildren(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueAncestors => {
            Ok(Response::IssueAncestors(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueImpact => {
            Ok(Response::IssueImpact(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueBacklinks => {
            Ok(Response::IssueBacklinks(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueOpen => Ok(Response::IssueOpen(serde_json::from_value(value)?)),
        crate::rpc::Method::IssueCreate => {
            Ok(Response::IssueCreate(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueUpdate => {
            Ok(Response::IssueUpdate(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueClaim => Ok(Response::IssueClaim(serde_json::from_value(value)?)),
        crate::rpc::Method::IssueRelease => {
            Ok(Response::IssueRelease(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueNote => Ok(Response::IssueNote(serde_json::from_value(value)?)),
        crate::rpc::Method::IssueRefile => {
            Ok(Response::IssueRefile(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueDeed => Ok(Response::IssueDeed(serde_json::from_value(value)?)),
        crate::rpc::Method::IssueRecall => {
            Ok(Response::IssueRecall(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueConsensus => {
            Ok(Response::IssueConsensus(serde_json::from_value(value)?))
        }
        crate::rpc::Method::ProjectList => {
            Ok(Response::ProjectList(serde_json::from_value(value)?))
        }
        crate::rpc::Method::EventsSince => {
            Ok(Response::EventsSince(serde_json::from_value(value)?))
        }
        crate::rpc::Method::EventsGen => Ok(Response::EventsGen(serde_json::from_value(value)?)),
        crate::rpc::Method::IssueAppend => {
            Ok(Response::IssueAppend(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueReject => {
            Ok(Response::IssueReject(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueResolve => {
            Ok(Response::IssueResolve(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueVote => Ok(Response::IssueVote(serde_json::from_value(value)?)),
        crate::rpc::Method::IssueFold => Ok(Response::IssueFold(serde_json::from_value(value)?)),
        crate::rpc::Method::IssueNormalize => {
            Ok(Response::IssueNormalize(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueCheck => Ok(Response::IssueCheck(serde_json::from_value(value)?)),
        crate::rpc::Method::IssueCount => Ok(Response::IssueCount(serde_json::from_value(value)?)),
        crate::rpc::Method::IssueCycles => {
            Ok(Response::IssueCycles(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueDigest => {
            Ok(Response::IssueDigest(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueExport => {
            Ok(Response::IssueExport(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueGraph => Ok(Response::IssueGraph(serde_json::from_value(value)?)),
        crate::rpc::Method::IssueRoadmap => {
            Ok(Response::IssueRoadmap(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueStale => Ok(Response::IssueStale(serde_json::from_value(value)?)),
        crate::rpc::Method::IssueHygiene => {
            Ok(Response::IssueHygiene(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueWaitingOn => {
            Ok(Response::IssueWaitingOn(serde_json::from_value(value)?))
        }
        crate::rpc::Method::IssueMirror => {
            Ok(Response::IssueMirror(serde_json::from_value(value)?))
        }
        crate::rpc::Method::EventsPing => Ok(Response::EventsPing(serde_json::from_value(value)?)),
        crate::rpc::Method::EventsWait => Ok(Response::EventsWait(serde_json::from_value(value)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Framing, read_message, write_message};
    use crate::rpc::{JsonRpcRequest, NOTIFY_VAULT_CHANGED};
    use serde_json::json;
    use std::io::{BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    fn serve_one(
        path: &Path,
        framing: Framing,
        reply: impl FnOnce(JsonRpcRequest) -> Value + Send + 'static,
    ) {
        let listener = UnixListener::bind(path).unwrap();
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let (payload, got) = read_message(&mut reader).unwrap();
            assert_eq!(got, framing);
            let req: JsonRpcRequest = serde_json::from_slice(&payload).unwrap();
            let body = reply(req);
            let bytes = serde_json::to_vec(&body).unwrap();
            write_message(&mut writer, &bytes, framing).unwrap();
            writer.flush().unwrap();
        });
    }

    #[test]
    fn request_roundtrip_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("control.sock");
        serve_one(
            &sock,
            Framing::Jsonl,
            |req| json!({"jsonrpc":"2.0","id":req.id,"result":{"identity":"rg"}}),
        );
        let mut client = Client::connect(&sock).unwrap();
        assert_eq!(client.framing(), Framing::Jsonl);
        let result = client.request("identity/get", json!({})).unwrap();
        assert_eq!(result["identity"], "rg");
    }

    #[test]
    fn request_roundtrip_headers() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("control.sock");
        serve_one(
            &sock,
            Framing::Headers,
            |req| json!({"jsonrpc":"2.0","id":req.id,"result":{"ok":true}}),
        );
        let mut client = Client::connect_with_framing(&sock, Framing::Headers).unwrap();
        let result = client.request_typed(&Request::IdentityGet).unwrap();
        assert_eq!(result["ok"], true);
    }

    #[test]
    fn notification_callback_fires_before_result() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("control.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let (payload, framing) = read_message(&mut reader).unwrap();
            let req: JsonRpcRequest = serde_json::from_slice(&payload).unwrap();
            let note = json!({
                "jsonrpc":"2.0",
                "method": NOTIFY_VAULT_CHANGED,
                "params": {"generation": 9, "revision": 3, "projects": ["atlas"]}
            });
            write_message(&mut writer, &serde_json::to_vec(&note).unwrap(), framing).unwrap();
            let result = json!({"jsonrpc":"2.0","id":req.id,"result":{"ok":true}});
            write_message(&mut writer, &serde_json::to_vec(&result).unwrap(), framing).unwrap();
            writer.flush().unwrap();
        });

        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_cb = Arc::clone(&seen);
        let mut client = Client::connect(&sock).unwrap();
        client.on_notification(move |n| seen_cb.lock().unwrap().push(n.method().to_string()));
        let result = client.request("events/gen", json!({})).unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(seen.lock().unwrap().as_slice(), [NOTIFY_VAULT_CHANGED]);
    }

    #[test]
    fn null_response_id_is_rpc_error() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("control.sock");
        serve_one(&sock, Framing::Jsonl, |_req| {
            json!({
                "jsonrpc":"2.0",
                "id": null,
                "error": {"code": -32600, "message": "invalid request"}
            })
        });
        let mut client = Client::connect(&sock).unwrap();
        let err = client.request("identity/get", json!({})).unwrap_err();
        match err {
            Error::Rpc(e) => {
                assert_eq!(e.code, -32600);
                assert_eq!(e.message, "invalid request");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unmatched_response_id_is_rpc_error() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("control.sock");
        serve_one(
            &sock,
            Framing::Jsonl,
            |_req| json!({"jsonrpc":"2.0","id": 99, "result":{"ok":true}}),
        );
        let mut client = Client::connect(&sock).unwrap();
        let err = client.request("identity/get", json!({})).unwrap_err();
        match err {
            Error::Rpc(e) => {
                assert_eq!(e.code, -32600);
                assert_eq!(e.message, "response id does not match request");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rpc_error_is_returned() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("control.sock");
        serve_one(&sock, Framing::Jsonl, |req| {
            json!({
                "jsonrpc":"2.0",
                "id": req.id,
                "error": {"code": -32601, "message": "method not found", "data": {"method": "nope"}}
            })
        });
        let mut client = Client::connect(&sock).unwrap();
        let err = client.request("nope", json!({})).unwrap_err();
        match err {
            Error::Rpc(e) => assert_eq!(e.code, -32601),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn wait_notification_reads_a_push() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("control.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream;
            let note = json!({
                "jsonrpc":"2.0",
                "method": NOTIFY_VAULT_CHANGED,
                "params": {"generation": 2, "revision": 4, "projects": []}
            });
            write_message(
                &mut writer,
                &serde_json::to_vec(&note).unwrap(),
                Framing::Jsonl,
            )
            .unwrap();
            writer.flush().unwrap();
            thread::sleep(std::time::Duration::from_millis(50));
        });
        let mut client = Client::connect(&sock).unwrap();
        let note = client
            .wait_notification(std::time::Duration::from_secs(2))
            .unwrap();
        assert_eq!(note.method(), NOTIFY_VAULT_CHANGED);
    }

    #[test]
    fn notify_writes_without_id() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("control.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let (payload, _) = read_message(&mut reader).unwrap();
            let req: JsonRpcRequest = serde_json::from_slice(&payload).unwrap();
            assert!(req.is_notification());
            assert_eq!(req.method, "serve/shutting_down");
        });
        let mut client = Client::connect(&sock).unwrap();
        client.notify("serve/shutting_down", json!({})).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn decode_response_covers_methods() {
        let value = json!({"protocolVersion":1,"capabilities":[],"root":"/","prefix":"Software","generation":1,"revision":1,"identity":"a"});
        match decode_response("initialize", value).unwrap() {
            Response::Initialize(r) => assert_eq!(r.protocol_version, 1),
            other => panic!("{other:?}"),
        }
        let list = json!({"issues":[],"revision":1});
        assert!(matches!(
            decode_response("issue/list", list.clone()).unwrap(),
            Response::IssueList(_)
        ));
        assert!(matches!(
            decode_response("issue/ready", list).unwrap(),
            Response::IssueReady(_)
        ));
        assert!(matches!(
            decode_response("events/gen", json!({"generation":1,"revision":1})).unwrap(),
            Response::EventsGen(_)
        ));
        assert!(decode_response("issue/fold", json!({})).is_err());

        let detail = json!({
            "id":"atlas-1a2b","project":"atlas","title":"t","state":"TODO","priority":"B",
            "properties":{},"org_tags":[],"tags":[],"blocked_by":[],"parent":null,
            "claimed_by":null,"claimed_at":null,"file":"f","line_start":1,"line_end":2,
            "revision":1
        });
        for method in [
            "issue/get",
            "issue/show",
            "issue/open",
            "issue/excerpt",
            "issue/search",
            "issue/claims",
            "issue/agenda",
            "issue/tree",
            "issue/related",
            "issue/children",
            "issue/ancestors",
            "issue/impact",
            "issue/backlinks",
            "issue/create",
            "issue/update",
            "issue/claim",
            "issue/release",
            "issue/note",
            "issue/refile",
            "project/list",
            "events/since",
            "identity/get",
        ] {
            let value = match method {
                "issue/get" | "issue/show" | "issue/open" => detail.clone(),
                "issue/excerpt" => json!({
                    "id":"atlas-1a2b","file":"f","line_start":1,"line_end":2,
                    "text":"","suppressed":false
                }),
                "issue/search" | "issue/claims" | "issue/agenda" | "issue/related"
                | "issue/children" | "issue/ancestors" | "issue/impact" | "issue/backlinks" => {
                    json!([])
                }
                "issue/tree" => json!({"text": "* a"}),
                "issue/create" | "issue/update" | "issue/claim" | "issue/release"
                | "issue/note" | "issue/refile" => {
                    json!({"ok":true,"report":"","issue":null,"revision":1,"generation":1})
                }
                "project/list" => json!({"projects":[],"revision":1}),
                "events/since" => json!({"events":[],"generation":1}),
                "identity/get" => {
                    json!({"identity":"a","root":"/","prefix":"Software","version":"0.2.0"})
                }
                _ => json!({}),
            };
            decode_response(method, value).expect(method);
        }
    }
}
