//! Same attach story as the TUI: first paint is core; live serve is optional.

use std::path::Path;

use vissue_core::config::Layout;
use vissue_tui::attach::{AttachFail, AttachHooks, AttachOutcome, try_attach};
use vissue_tui::backend::BoardBackend;

/// HUD `initialize.client` name.
pub const CLIENT: &str = "vissue-hud";

/// Hooks that identify as [`CLIENT`] on connect.
pub fn hud_hooks() -> AttachHooks {
    AttachHooks {
        probe: default_probe,
        ensure: default_ensure,
        connect: hud_connect,
    }
}

fn default_probe(path: &Path) -> bool {
    vissue_serve::socket_accepts(path)
}

fn default_ensure(cfg: &vissue_serve::ServeConfig) -> Result<vissue_serve::EnsureResult, String> {
    vissue_serve::ensure_serve(cfg).map_err(|e| e.to_string())
}

fn hud_connect(
    path: &Path,
    layout: &Layout,
    agent: &str,
) -> Result<Box<dyn BoardBackend>, AttachFail> {
    #[cfg(unix)]
    {
        use vissue_tui::control::{ControlAttachError, ControlBackend};
        match ControlBackend::connect_as(path, layout, agent, CLIENT) {
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
        Err(AttachFail::Other("vissue hud attach is Unix-only".into()))
    }
}

/// Re-export so callers do not name `vissue_tui` for the post-paint step.
pub fn attach(
    layout: &Layout,
    socket: &Path,
    agent: &str,
    offline: bool,
    hooks: &AttachHooks,
) -> AttachOutcome {
    try_attach(layout, socket, agent, offline, hooks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use vissue_tui::attach::ServeStatus;

    static TOUCHED: AtomicBool = AtomicBool::new(false);

    fn panic_probe(_: &Path) -> bool {
        TOUCHED.store(true, Ordering::SeqCst);
        panic!("--offline must not probe the socket");
    }

    fn panic_ensure(_: &vissue_serve::ServeConfig) -> Result<vissue_serve::EnsureResult, String> {
        TOUCHED.store(true, Ordering::SeqCst);
        panic!("--offline must not spawn serve");
    }

    fn panic_connect(_: &Path, _: &Layout, _: &str) -> Result<Box<dyn BoardBackend>, AttachFail> {
        TOUCHED.store(true, Ordering::SeqCst);
        panic!("--offline must not connect");
    }

    #[test]
    fn offline_never_attempts_socket() {
        TOUCHED.store(false, Ordering::SeqCst);
        let layout = Layout::new("/tmp/vissue-hud-offline", "Software");
        let hooks = AttachHooks {
            probe: panic_probe,
            ensure: panic_ensure,
            connect: panic_connect,
        };
        let outcome = attach(
            &layout,
            &PathBuf::from("/tmp/vissue-hud-offline.sock"),
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

    #[test]
    fn hud_hooks_probe_a_missing_socket_as_free() {
        let hooks = hud_hooks();
        assert!(!(hooks.probe)(Path::new("/tmp/vissue-hud-no-such.sock")));
        assert_eq!(CLIENT, "vissue-hud");
    }
}
