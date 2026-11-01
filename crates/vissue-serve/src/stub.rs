//! Windows (and other non-Unix) surface: the verb exists, the owner does not.

use std::path::Path;

use crate::error::Result;

use crate::{Action, EnsureResult, ServeConfig};

/// Always false: there is no Unix owner on this target.
pub fn socket_accepts(_path: &Path) -> bool {
    false
}

/// Reject every serve verb: the owner is Unix-only.
///
/// Prints `vissue serve is Unix-only` and returns exit code 1.
///
/// # Errors
///
/// Never fails.
pub fn invoke(_action: Action, _cfg: &ServeConfig) -> Result<i32> {
    eprintln!("vissue serve is Unix-only");
    Ok(1)
}

/// Return a failed ensure: the owner is Unix-only.
///
/// # Errors
///
/// Never fails.
pub fn ensure_serve(cfg: &ServeConfig) -> Result<EnsureResult> {
    Ok(EnsureResult {
        ok: false,
        already_running: false,
        spawned: false,
        pid: None,
        socket: cfg.socket.clone(),
        error: Some("vissue serve is Unix-only".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vissue_core::config::Layout;

    #[test]
    fn serve_is_unix_only() {
        let cfg = ServeConfig {
            layout: Layout::new("/tmp", "Software"),
            socket: "/tmp/control.sock".into(),
            exe: None,
        };
        assert_eq!(invoke(Action::Foreground, &cfg).unwrap(), 1);
        assert!(!socket_accepts(&cfg.socket));
        let ensured = ensure_serve(&cfg).unwrap();
        assert!(!ensured.ok);
        assert_eq!(ensured.error.as_deref(), Some("vissue serve is Unix-only"));
    }
}
