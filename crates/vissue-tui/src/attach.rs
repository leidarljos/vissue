//! Attach story: first paint is core; live serve is optional.

use std::path::Path;

use vissue_core::config::Layout;
use vissue_serve::ServeConfig;

use crate::backend::BoardBackend;
use crate::core_backend::CoreBackend;

/// How the status line labels the current store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServeStatus {
    /// Attached to a matching `vissue serve`.
    Live,
    /// Core only: `--offline`, no socket, or spawn/connect failed.
    Offline,
    /// Socket answered for a different root/prefix.
    Mismatch,
}

/// Returns true when the control socket already accepts connections.
pub type ProbeFn = fn(&Path) -> bool;
/// Spawn or reuse `vissue serve`. The `String` is a spawn failure message.
pub type EnsureFn = fn(&ServeConfig) -> Result<vissue_serve::EnsureResult, String>;
/// Connect a live [`BoardBackend`], or classify why attach must stay on core.
pub type ConnectFn = fn(&Path, &Layout, &str) -> Result<Box<dyn BoardBackend>, AttachFail>;

/// Hooks so `--offline` can be tested with a connector that panics.
#[derive(Debug)]
pub struct AttachHooks {
    /// Whether `socket` already accepts connections.
    pub probe: ProbeFn,
    /// Spawn serve when the socket is free.
    pub ensure: EnsureFn,
    /// Dial the socket after probe or a successful ensure.
    pub connect: ConnectFn,
}

/// Why a live connect was refused. [`try_attach`] maps this onto [`AttachOutcome`].
#[derive(Debug)]
pub enum AttachFail {
    /// Serve root/prefix does not match the board layout.
    Mismatch(String),
    /// Socket, RPC, or platform failure.
    Other(String),
}

impl Default for AttachHooks {
    fn default() -> Self {
        Self {
            probe: default_probe,
            ensure: default_ensure,
            connect: default_connect,
        }
    }
}

fn default_probe(path: &Path) -> bool {
    vissue_serve::socket_accepts(path)
}

fn default_ensure(cfg: &ServeConfig) -> Result<vissue_serve::EnsureResult, String> {
    vissue_serve::ensure_serve(cfg).map_err(|e| e.to_string())
}

fn default_connect(
    path: &Path,
    layout: &Layout,
    agent: &str,
) -> Result<Box<dyn BoardBackend>, AttachFail> {
    #[cfg(unix)]
    {
        use crate::control::{ControlAttachError, ControlBackend};
        match ControlBackend::connect(path, layout, agent) {
            Ok(backend) => Ok(Box::new(backend)),
            Err(ControlAttachError::Mismatch {
                want_root,
                want_prefix,
                got_root,
                got_prefix,
            }) => Err(AttachFail::Mismatch(format!(
                "want {want_root}/{want_prefix} got {got_root}/{got_prefix}"
            ))),
            Err(err) => Err(AttachFail::Other(err.to_string())),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (path, layout, agent);
        Err(AttachFail::Other("vissue tui attach is Unix-only".into()))
    }
}

/// Result of the post-paint attach attempt. First paint always used
/// [`CoreBackend`] already.
#[derive(Debug)]
pub enum AttachOutcome {
    /// Keep [`CoreBackend`]. `message` is empty on a clean `--offline`.
    Stay {
        /// Status-line label after the attempt.
        status: ServeStatus,
        /// Reason shown on the status line.
        message: String,
    },
    /// Replace the board store with a live client.
    Switch {
        /// Connected control backend.
        backend: Box<dyn BoardBackend>,
        /// Always [`ServeStatus::Live`] on this arm.
        status: ServeStatus,
    },
}

/// Never probes the socket when `offline`. Otherwise: accept and initialize;
/// on a free socket, `ensure_serve` then attach; on spawn failure stay core.
pub fn try_attach(
    layout: &Layout,
    socket: &Path,
    agent: &str,
    offline: bool,
    hooks: &AttachHooks,
) -> AttachOutcome {
    if offline {
        return AttachOutcome::Stay {
            status: ServeStatus::Offline,
            message: String::new(),
        };
    }

    if (hooks.probe)(socket) {
        return finish_connect(socket, layout, agent, hooks);
    }

    let cfg = ServeConfig {
        layout: layout.clone(),
        socket: socket.to_path_buf(),
        exe: None,
    };
    match (hooks.ensure)(&cfg) {
        Ok(ensured) if ensured.ok && (hooks.probe)(socket) => {
            finish_connect(socket, layout, agent, hooks)
        }
        Ok(ensured) => AttachOutcome::Stay {
            status: ServeStatus::Offline,
            message: ensured.error.unwrap_or_else(|| "serve spawn failed".into()),
        },
        Err(err) => AttachOutcome::Stay {
            status: ServeStatus::Offline,
            message: err,
        },
    }
}

fn finish_connect(
    socket: &Path,
    layout: &Layout,
    agent: &str,
    hooks: &AttachHooks,
) -> AttachOutcome {
    match (hooks.connect)(socket, layout, agent) {
        Ok(backend) => AttachOutcome::Switch {
            backend,
            status: ServeStatus::Live,
        },
        Err(AttachFail::Mismatch(message)) => AttachOutcome::Stay {
            status: ServeStatus::Mismatch,
            message,
        },
        Err(AttachFail::Other(message)) => AttachOutcome::Stay {
            status: ServeStatus::Offline,
            message,
        },
    }
}

/// First paint: core catalog, revision 0, no socket.
///
/// # Errors
///
/// Returns an error if a project file cannot be read or parsed.
pub fn open_core(layout: Layout, agent: String) -> Result<CoreBackend, vissue_core::error::Error> {
    CoreBackend::open(layout, agent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BoardBackend;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};

    static TOUCHED: AtomicBool = AtomicBool::new(false);

    fn panic_probe(_: &Path) -> bool {
        TOUCHED.store(true, Ordering::SeqCst);
        panic!("--offline must not probe the socket");
    }

    fn panic_ensure(_: &ServeConfig) -> Result<vissue_serve::EnsureResult, String> {
        TOUCHED.store(true, Ordering::SeqCst);
        panic!("--offline must not spawn serve");
    }

    fn panic_connect(_: &Path, _: &Layout, _: &str) -> Result<Box<dyn BoardBackend>, AttachFail> {
        TOUCHED.store(true, Ordering::SeqCst);
        panic!("--offline must not connect");
    }

    fn no_probe(_: &Path) -> bool {
        false
    }

    fn ensure_fails(_: &ServeConfig) -> Result<vissue_serve::EnsureResult, String> {
        Err("spawn failed".into())
    }

    fn connect_mismatch(
        _: &Path,
        _: &Layout,
        _: &str,
    ) -> Result<Box<dyn BoardBackend>, AttachFail> {
        Err(AttachFail::Mismatch("other root".into()))
    }

    fn yes_probe(_: &Path) -> bool {
        true
    }

    #[test]
    fn spawn_failure_stays_core() {
        let layout = Layout::new("/tmp/vissue-spawn", "Software");
        let hooks = AttachHooks {
            probe: no_probe,
            ensure: ensure_fails,
            connect: panic_connect,
        };
        match try_attach(
            &layout,
            &PathBuf::from("/tmp/vissue-spawn.sock"),
            "agent",
            false,
            &hooks,
        ) {
            AttachOutcome::Stay {
                status: ServeStatus::Offline,
                message,
            } => assert!(message.contains("spawn failed"), "{message}"),
            _ => panic!("expected offline stay"),
        }
    }

    #[test]
    fn mismatch_stays_core() {
        let layout = Layout::new("/tmp/vissue-mis", "Software");
        let hooks = AttachHooks {
            probe: yes_probe,
            ensure: panic_ensure,
            connect: connect_mismatch,
        };
        match try_attach(
            &layout,
            &PathBuf::from("/tmp/vissue-mis.sock"),
            "agent",
            false,
            &hooks,
        ) {
            AttachOutcome::Stay {
                status: ServeStatus::Mismatch,
                message,
            } => assert!(message.contains("other root"), "{message}"),
            _ => panic!("expected mismatch stay"),
        }
    }

    #[test]
    fn offline_never_connects() {
        TOUCHED.store(false, Ordering::SeqCst);
        let layout = Layout::new("/tmp/vissue-offline", "Software");
        let hooks = AttachHooks {
            probe: panic_probe,
            ensure: panic_ensure,
            connect: panic_connect,
        };
        let outcome = try_attach(
            &layout,
            &PathBuf::from("/tmp/vissue-offline.sock"),
            "agent",
            true,
            &hooks,
        );
        match outcome {
            AttachOutcome::Stay {
                status: ServeStatus::Offline,
                ..
            } => {}
            _ => panic!("offline must stay on CoreBackend"),
        }
        assert!(!TOUCHED.load(Ordering::SeqCst));
    }
}
