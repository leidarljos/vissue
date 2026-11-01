//! Weaker detach: `process_group(0)` only. No `setsid`, no `pre_exec`.

use std::fs::OpenOptions;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use vissue_serve::accept_timeout;

use crate::cli::HudCli;
use crate::summon;

/// Child argv for a detached owner: always `--foreground`, never a summon verb
/// that would bounce off a live socket.
pub fn child_args(cli: &HudCli) -> Vec<String> {
    let mut args = vec!["--foreground".to_string()];
    if let Some(root) = &cli.root {
        args.push("--root".into());
        args.push(root.display().to_string());
    }
    if let Some(prefix) = &cli.prefix {
        args.push("--prefix".into());
        args.push(prefix.clone());
    }
    if let Some(socket) = &cli.socket {
        args.push("--socket".into());
        args.push(socket.display().to_string());
    }
    if cli.offline {
        args.push("--offline".into());
    }
    if cli.hide && !cli.show && !cli.toggle {
        args.push("--hide".into());
    }
    args
}

/// Spawn this executable with [`child_args`] and wait until the summon socket
/// accepts. The child is placed in its own process group and can still see
/// SIGHUP from the parent terminal.
///
/// # Errors
///
/// Returns an error if the HUD log directory cannot be created, the log
/// cannot be opened or cloned onto the child, this executable cannot be
/// found, or the child process cannot be spawned.
pub fn start_detached(cli: &HudCli) -> anyhow::Result<i32> {
    if summon::already_running() {
        return Ok(0);
    }
    let log_path = crate::log::path();
    if let Some(parent) = log_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let _ = writeln!(
        log_file,
        "\n--- vissue-hud --foreground spawn parent={} ---",
        std::process::id()
    );
    let exe = std::env::current_exe()?;
    let stdout = log_file.try_clone()?;
    let mut cmd = Command::new(&exe);
    cmd.args(child_args(cli))
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(log_file));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let _child = cmd.spawn()?;
    wait_until_accepts(&summon::default_socket_path(), accept_timeout());
    Ok(0)
}

fn wait_until_accepts(path: &std::path::Path, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if summon::socket_accepts(path) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[cfg(test)]
#[allow(deprecated_safe_2024)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn base() -> HudCli {
        HudCli {
            root: Some(PathBuf::from("/tmp/vault")),
            prefix: Some("Software".into()),
            socket: Some(PathBuf::from("/tmp/control.sock")),
            offline: false,
            foreground: false,
            toggle: false,
            show: false,
            hide: false,
            install_desktop: false,
        }
    }

    #[test]
    fn child_always_foreground_and_drops_toggle() {
        let mut cli = base();
        cli.toggle = true;
        let args = child_args(&cli);
        assert!(args.contains(&"--foreground".into()));
        assert!(!args.iter().any(|a| a == "--toggle"));
        assert!(args.contains(&"--root".into()));
        assert!(args.contains(&"/tmp/vault".into()));
    }

    #[test]
    fn child_keeps_offline_and_hide() {
        let mut cli = base();
        cli.offline = true;
        cli.hide = true;
        let args = child_args(&cli);
        assert!(args.contains(&"--offline".into()));
        assert!(args.contains(&"--hide".into()));
    }

    #[cfg(unix)]
    #[test]
    fn start_detached_is_noop_when_summon_accepts() {
        use std::os::unix::net::UnixListener;

        let _guard = crate::env_lock();
        let dir = std::env::temp_dir().join(format!("vissue-hud-detach-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("hud.sock");
        let _ = std::fs::remove_file(&path);
        let _listener = UnixListener::bind(&path).unwrap();
        vissue_core::process_env::override_var(crate::summon::SOCKET_ENV, path.to_str());
        let code = start_detached(&base()).unwrap();
        assert_eq!(code, 0);
        vissue_core::process_env::clear_override(crate::summon::SOCKET_ENV);
        drop(_listener);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
