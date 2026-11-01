//! Fan-out for JSON-RPC notifications. A stuck client is dropped.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vissue_control::frame::{Framing, write_message};
use vissue_control::rpc::{JsonRpcRequest, Notification};

const NOTIFY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
struct Sink {
    id: u64,
    writer: Arc<Mutex<UnixStream>>,
    framing: Framing,
}

#[derive(Debug)]
pub struct Bus {
    next_id: Mutex<u64>,
    sinks: Mutex<Vec<Sink>>,
}

impl Bus {
    pub fn new() -> Self {
        Self {
            next_id: Mutex::new(1),
            sinks: Mutex::new(Vec::new()),
        }
    }

    pub fn register(&self, writer: Arc<Mutex<UnixStream>>, framing: Framing) -> u64 {
        let id = {
            let mut next = self.next_id.lock().unwrap_or_else(|p| p.into_inner());
            let id = *next;
            *next += 1;
            id
        };
        self.sinks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(Sink {
                id,
                writer,
                framing,
            });
        id
    }

    pub fn unregister(&self, id: u64) {
        self.sinks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|sink| sink.id != id);
    }

    pub fn broadcast(&self, note: &Notification) {
        let req = JsonRpcRequest::notification(note.method(), note.to_params());
        let Ok(bytes) = serde_json::to_vec(&req) else {
            return;
        };
        let sinks = self.sinks.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let mut dead = Vec::new();
        for sink in &sinks {
            if !write_notify(&sink.writer, &bytes, sink.framing) {
                dead.push(sink.id);
            }
        }
        if !dead.is_empty() {
            self.sinks
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .retain(|sink| !dead.contains(&sink.id));
        }
    }
}

fn write_notify(writer: &Mutex<UnixStream>, bytes: &[u8], framing: Framing) -> bool {
    let mut guard = writer.lock().unwrap_or_else(|p| p.into_inner());
    let _ = guard.set_write_timeout(Some(NOTIFY_TIMEOUT));
    let ok = write_message(&mut *guard, bytes, framing)
        .and_then(|()| guard.flush())
        .is_ok();
    let _ = guard.set_write_timeout(None);
    ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::io::BufReader;
    use std::os::unix::net::UnixListener;
    use std::thread;
    use vissue_control::frame::read_message;
    use vissue_control::rpc::{NOTIFY_VAULT_CHANGED, VaultChanged};

    #[test]
    fn broadcast_reaches_a_registered_client() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("n.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let (payload, _) = read_message(&mut reader).unwrap();
            let value: Value = serde_json::from_slice(&payload).unwrap();
            assert_eq!(value["method"], NOTIFY_VAULT_CHANGED);
            assert_eq!(value["params"]["revision"], 3);
        });
        let stream = UnixStream::connect(&sock).unwrap();
        let bus = Bus::new();
        let id = bus.register(Arc::new(Mutex::new(stream)), Framing::Jsonl);
        bus.broadcast(&Notification::VaultChanged(VaultChanged {
            generation: 9,
            revision: 3,
            projects: vec!["atlas".into()],
            ids: None,
        }));
        handle.join().unwrap();
        bus.unregister(id);
    }
}
