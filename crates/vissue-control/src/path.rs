//! Control-socket locations and the sibling files next to them.

use std::path::{Path, PathBuf};

/// Override for [`default_socket_path`]. Empty or unset falls through.
pub const SOCKET_ENV: &str = "VISSUE_CONTROL_SOCKET";
/// Override for [`control_log_path`]. Empty or unset falls through.
pub const SERVE_LOG_ENV: &str = "VISSUE_SERVE_LOG";
/// Override for [`hud_log_path`]. Empty or unset falls through.
pub const HUD_LOG_ENV: &str = "VISSUE_HUD_LOG";
/// Override for [`hud_socket_path`]. Empty or unset falls through.
pub const HUD_SOCKET_ENV: &str = "VISSUE_HUD_SUMMON_SOCKET";

const CONTROL_SOCK: &str = "control.sock";
const CONTROL_LOG: &str = "control.log";
const HUD_LOG: &str = "hud.log";
const HUD_SOCK: &str = "hud.sock";

/// Per-user runtime directory that holds the control socket.
pub fn runtime_dir() -> PathBuf {
    match default_socket_path().parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Default control socket: env, then `$XDG_RUNTIME_DIR/vissue/control.sock`,
/// then `~/.vissue/run/control.sock`.
pub fn default_socket_path() -> PathBuf {
    resolve_socket_path(
        vissue_core::process_env::var(SOCKET_ENV).ok().as_deref(),
        std::env::var("XDG_RUNTIME_DIR").ok().as_deref(),
        home_dir().as_deref(),
    )
}

/// Detached serve log. `VISSUE_SERVE_LOG` wins; otherwise `control.log` next
/// to the default socket.
pub fn control_log_path() -> PathBuf {
    resolve_named_path(
        vissue_core::process_env::var(SERVE_LOG_ENV).ok().as_deref(),
        &default_socket_path(),
        CONTROL_LOG,
    )
}

/// HUD stderr log. `VISSUE_HUD_LOG` wins; otherwise `hud.log` next to the
/// default socket.
pub fn hud_log_path() -> PathBuf {
    resolve_named_path(
        vissue_core::process_env::var(HUD_LOG_ENV).ok().as_deref(),
        &default_socket_path(),
        HUD_LOG,
    )
}

/// HUD summon socket. `VISSUE_HUD_SUMMON_SOCKET` wins; otherwise `hud.sock`
/// next to the default socket.
pub fn hud_socket_path() -> PathBuf {
    resolve_named_path(
        vissue_core::process_env::var(HUD_SOCKET_ENV)
            .ok()
            .as_deref(),
        &default_socket_path(),
        HUD_SOCK,
    )
}

/// `{socket}.lock`, the exclusive flock file. Never unlinked by callers.
pub fn socket_lock_path(socket: &Path) -> PathBuf {
    append_suffix(socket, ".lock")
}

/// `{socket}.pid`, the owner pid file.
pub fn socket_pid_path(socket: &Path) -> PathBuf {
    append_suffix(socket, ".pid")
}

/// `name` in the same directory as `socket`.
pub fn beside_socket(socket: &Path, name: &str) -> PathBuf {
    match socket.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(name),
        _ => PathBuf::from(name),
    }
}

fn resolve_socket_path(
    socket: Option<&str>,
    xdg_runtime: Option<&str>,
    home: Option<&Path>,
) -> PathBuf {
    if let Some(path) = nonempty(socket) {
        return PathBuf::from(path);
    }
    if let Some(runtime) = nonempty(xdg_runtime) {
        return PathBuf::from(runtime).join("vissue").join(CONTROL_SOCK);
    }
    match home {
        Some(home) => home.join(".vissue").join("run").join(CONTROL_SOCK),
        None => PathBuf::from(CONTROL_SOCK),
    }
}

fn resolve_named_path(override_path: Option<&str>, socket: &Path, name: &str) -> PathBuf {
    match nonempty(override_path) {
        Some(path) => PathBuf::from(path),
        None => beside_socket(socket, name),
    }
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut raw = path.as_os_str().to_os_string();
    raw.push(suffix);
    PathBuf::from(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn env_override_wins() {
        let path = resolve_socket_path(
            Some("/tmp/custom.sock"),
            Some("/run/user/1000"),
            Some(Path::new("/home/me")),
        );
        assert_eq!(path, PathBuf::from("/tmp/custom.sock"));
    }

    #[test]
    fn empty_env_falls_through_to_xdg() {
        let path = resolve_socket_path(
            Some(""),
            Some("/run/user/1000"),
            Some(Path::new("/home/me")),
        );
        assert_eq!(path, PathBuf::from("/run/user/1000/vissue/control.sock"));
    }

    #[test]
    fn whitespace_env_falls_through_to_xdg() {
        let path = resolve_socket_path(
            Some("   "),
            Some("/run/user/1000"),
            Some(Path::new("/home/me")),
        );
        assert_eq!(path, PathBuf::from("/run/user/1000/vissue/control.sock"));
    }

    #[test]
    fn empty_xdg_falls_through_to_home() {
        let path = resolve_socket_path(None, Some(""), Some(Path::new("/home/me")));
        assert_eq!(path, PathBuf::from("/home/me/.vissue/run/control.sock"));
    }

    #[test]
    fn missing_home_uses_cwd_name() {
        let path = resolve_socket_path(None, None, None);
        assert_eq!(path, PathBuf::from("control.sock"));
    }

    #[test]
    fn logs_and_hud_socket_sit_beside_the_socket() {
        let socket = Path::new("/run/user/1000/vissue/control.sock");
        assert_eq!(
            beside_socket(socket, CONTROL_LOG),
            PathBuf::from("/run/user/1000/vissue/control.log")
        );
        assert_eq!(
            beside_socket(socket, HUD_LOG),
            PathBuf::from("/run/user/1000/vissue/hud.log")
        );
        assert_eq!(
            beside_socket(socket, HUD_SOCK),
            PathBuf::from("/run/user/1000/vissue/hud.sock")
        );
        assert_eq!(
            socket_lock_path(socket),
            PathBuf::from("/run/user/1000/vissue/control.sock.lock")
        );
        assert_eq!(
            socket_pid_path(socket),
            PathBuf::from("/run/user/1000/vissue/control.sock.pid")
        );
    }

    #[test]
    fn named_path_override_wins() {
        let socket = Path::new("/run/user/1000/vissue/control.sock");
        assert_eq!(
            resolve_named_path(Some("/tmp/serve.log"), socket, CONTROL_LOG),
            PathBuf::from("/tmp/serve.log")
        );
        assert_eq!(
            resolve_named_path(Some(""), socket, CONTROL_LOG),
            PathBuf::from("/run/user/1000/vissue/control.log")
        );
    }

    #[test]
    fn beside_socket_on_bare_name_uses_cwd() {
        assert_eq!(
            beside_socket(Path::new("control.sock"), "control.log"),
            PathBuf::from("control.log")
        );
    }

    #[test]
    fn default_socket_path_returns_a_path() {
        let path = default_socket_path();
        assert!(path.ends_with("control.sock") || path.file_name().is_some());
        assert_eq!(runtime_dir(), path.parent().unwrap());
        assert_eq!(control_log_path().file_name().unwrap(), "control.log");
        assert_eq!(hud_log_path().file_name().unwrap(), "hud.log");
        assert_eq!(hud_socket_path().file_name().unwrap(), "hud.sock");
    }
}
