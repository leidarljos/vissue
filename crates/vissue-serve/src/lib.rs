//! Control-socket owner: bind, detach, stop, catalog cache, and RPC dispatch.
//!
//! Serve is a cache. Mutations go through `vissue-core` ops; a crash loses
//! nothing. The hot path never calls `load_all`.

use std::path::PathBuf;

use serde::Serialize;
use vissue_core::config::Layout;

pub mod error;

#[cfg(not(unix))]
mod stub;
#[cfg(unix)]
mod unix;

pub use error::{Error, Result};

#[cfg(not(unix))]
pub use stub::{ensure_serve, invoke, socket_accepts};
#[cfg(unix)]
pub use unix::{OwnerHandle, ensure_serve, invoke, socket_accepts};

/// How the CLI (or a later TUI/HUD) wants to talk to the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Bind and serve until SIGINT/SIGTERM.
    Foreground,
    /// Spawn `serve --foreground` and return when the socket accepts.
    Detach,
    /// SIGTERM the owner, then SIGKILL if it stays up.
    Stop,
    /// [`Action::Stop`] then [`Action::Detach`].
    Restart,
    /// Print a snapshot. Exit 0 when live, 1 otherwise.
    Status {
        /// Pretty-print JSON instead of the line-oriented text form.
        json: bool,
    },
}

/// Layout plus the socket this process should own or probe.
#[derive(Debug, Clone)]
pub struct ServeConfig {
    /// Tracker root and project prefix the owner serves.
    pub layout: Layout,
    /// Unix socket this process owns or probes.
    pub socket: PathBuf,
    /// Binary used by [`Action::Detach`]. `None` resolves `vissue` from
    /// `current_exe()`, a sibling, or `$PATH`.
    pub exe: Option<PathBuf>,
}

/// `vissue serve status` snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    /// True when the socket currently accepts a connection.
    pub live: bool,
    /// Owner pid from the sidecar pid file, if present.
    pub pid: Option<u32>,
    /// Control socket path.
    pub socket: PathBuf,
    /// Tracker root the owner reports, or the caller's layout when down.
    pub root: PathBuf,
    /// Project prefix the owner reports, or the caller's layout when down.
    pub prefix: String,
    /// Event generation from the owner handshake, or the on-disk counter when down.
    pub generation: u64,
    /// Catalog revision from the owner, or [`SERVE_REVISION`] when down.
    pub revision: u64,
    /// Connected client count.
    pub clients: u64,
}

/// Outcome of [`ensure_serve`] / a detach start.
#[derive(Debug, Clone)]
pub struct EnsureResult {
    /// True when a live owner is available after the call.
    pub ok: bool,
    /// True when the socket already accepted and no child was started.
    pub already_running: bool,
    /// True when this call spawned a child.
    pub spawned: bool,
    /// Owner or child pid, when known.
    pub pid: Option<u32>,
    /// Socket that was probed or bound.
    pub socket: PathBuf,
    /// Failure text when [`Self::ok`] is false.
    pub error: Option<String>,
}

impl EnsureResult {
    /// True when [`Self::ok`] is set and [`socket_accepts`] still succeeds.
    #[must_use]
    pub fn live(&self) -> bool {
        self.ok && socket_accepts(&self.socket)
    }
}

/// Methods this release actually answers.
pub const LIVE_CAPABILITIES: &[&str] = vissue_control::V1_CAPABILITIES;

/// Fallback revision when no owner is live. A running owner starts at 1.
pub const SERVE_REVISION: u64 = 0;

/// How long a caller waits for a freshly started owner to accept.
///
/// A cold start binds a socket after building a multi-thread runtime and
/// loading the tracker, so a saturated machine can need well past a second.
/// `VISSUE_ACCEPT_TIMEOUT_MS` overrides it for hosts slower still.
pub const ACCEPT_TIMEOUT_MS: u64 = 15_000;

/// [`ACCEPT_TIMEOUT_MS`], or `VISSUE_ACCEPT_TIMEOUT_MS` when that parses as
/// a non-zero number of milliseconds.
#[must_use]
pub fn accept_timeout() -> std::time::Duration {
    let ms = std::env::var("VISSUE_ACCEPT_TIMEOUT_MS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .unwrap_or(ACCEPT_TIMEOUT_MS);
    std::time::Duration::from_millis(ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_capabilities_are_the_implemented_methods() {
        assert_eq!(LIVE_CAPABILITIES, vissue_control::V1_CAPABILITIES);
        assert!(LIVE_CAPABILITIES.contains(&"issue/list"));
        assert!(LIVE_CAPABILITIES.contains(&"issue/ready"));
        assert!(LIVE_CAPABILITIES.contains(&"identity/get"));
        assert_eq!(SERVE_REVISION, 0);
    }

    #[test]
    fn status_json_uses_snake_case_keys() {
        let status = Status {
            live: false,
            pid: None,
            socket: PathBuf::from("/tmp/control.sock"),
            root: PathBuf::from("/tmp/tracker"),
            prefix: "Software".into(),
            generation: 0,
            revision: 0,
            clients: 0,
        };
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["live"], false);
        assert_eq!(value["socket"], "/tmp/control.sock");
        assert!(value.get("socket_path").is_none());
        assert_eq!(value["clients"], 0);
    }
}
