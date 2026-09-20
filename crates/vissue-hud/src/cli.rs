//! Process flags for `vissue-hud` and the `vissue hud` launcher.

use std::path::PathBuf;

use clap::Parser;
use vissue_core::config::Layout;

use crate::detach;
use crate::summon::{self, SummonAction};

/// Env override for the HUD binary the launcher execs.
pub const HUD_BIN_ENV: &str = "VISSUE_HUD_BIN";

/// Summonable overlay. Home is the project list.
#[derive(Debug, Clone, Parser)]
#[command(name = "vissue-hud", version, about = "Summonable vissue task board")]
pub struct HudCli {
    /// Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then cwd.
    #[arg(long, global = true)]
    pub root: Option<PathBuf>,
    /// Directory under the root holding one subdirectory per project.
    #[arg(long, global = true)]
    pub prefix: Option<String>,
    /// Control socket path.
    #[arg(short = 's', long)]
    pub socket: Option<PathBuf>,
    /// Never attach, never spawn serve; CatalogService plus generation poll.
    #[arg(long)]
    pub offline: bool,
    /// Stay on the terminal. Default detach uses process_group(0) only.
    #[arg(long)]
    pub foreground: bool,
    /// Show or hide a running HUD (compositor bind target).
    #[arg(long, group = "summon")]
    pub toggle: bool,
    /// Show a running HUD.
    #[arg(long, group = "summon")]
    pub show: bool,
    /// Hide a running HUD.
    #[arg(long, group = "summon")]
    pub hide: bool,
    /// Write a user-local .desktop launcher and Sway overlay include.
    #[arg(long)]
    pub install_desktop: bool,
}

impl HudCli {
    /// Summon verb from flags, if any.
    pub fn summon_action(&self) -> Option<SummonAction> {
        if self.toggle {
            Some(SummonAction::Toggle)
        } else if self.show {
            Some(SummonAction::Show)
        } else if self.hide {
            Some(SummonAction::Hide)
        } else {
            None
        }
    }

    /// Whether the overlay starts mapped. `--hide` starts hidden.
    pub fn initial_visible(&self) -> bool {
        !self.hide || self.show || self.toggle
    }
}

/// Parse args and run: summon bounce, detach, or the iced loop.
///
/// # Errors
///
/// Returns an error if a summon send fails, detach cannot spawn, the vault
/// cannot be resolved, the summon socket cannot be bound, or the iced loop
/// cannot start.
pub fn run_cli() -> anyhow::Result<i32> {
    let cli = HudCli::parse();
    run_with(cli)
}

/// Run already-parsed flags: bounce off a live HUD, detach, or own the loop.
///
/// # Errors
///
/// Returns an error if a summon send fails, detach cannot spawn, the vault
/// cannot be resolved, the summon socket cannot be bound, or the iced loop
/// cannot start.
pub fn run_with(cli: HudCli) -> anyhow::Result<i32> {
    if cli.install_desktop {
        crate::install_desktop::run_cli()?;
        return Ok(0);
    }
    if summon::already_running() {
        if let Some(action) = cli.summon_action() {
            summon::send_command(action)?;
        }
        return Ok(0);
    }
    if !cli.foreground {
        return detach::start_detached(&cli);
    }
    run_owner(cli)
}

pub(crate) fn run_owner(cli: HudCli) -> anyhow::Result<i32> {
    let layout = Layout::resolve(cli.root.as_deref(), cli.prefix.as_deref())?;
    let socket = cli
        .socket
        .clone()
        .unwrap_or_else(vissue_control::default_socket_path);
    let agent = vissue_core::config::identity(&layout);
    let _summon = match summon::install() {
        Ok(server) => Some(server),
        Err(summon::SummonError::AlreadyRunning(_)) => {
            if let Some(action) = cli.summon_action() {
                let _ = summon::send_command(action);
            }
            return Ok(0);
        }
        Err(summon::SummonError::Unsupported) => None,
        Err(err) => {
            crate::log::error(&format!("summon install: {err}"));
            return Err(err.into());
        }
    };
    crate::app::run(crate::app::BootOpts {
        layout,
        socket,
        offline: cli.offline,
        agent,
        visible: cli.initial_visible(),
    })?;
    Ok(0)
}

/// Locate `vissue-hud` for the `vissue hud` launcher. Missing means 127.
pub fn resolve_hud_bin() -> Option<PathBuf> {
    if let Ok(raw) = vissue_core::process_env::var(HUD_BIN_ENV) {
        let t = raw.trim();
        if !t.is_empty() {
            return Some(PathBuf::from(t));
        }
    }
    which("vissue-hud")
}

/// True when `path` names an executable file.
pub fn bin_is_present(path: &std::path::Path) -> bool {
    path.is_file()
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let exe = dir.join(format!("{name}.exe"));
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    None
}

/// Hint printed when the launcher cannot find the HUD binary.
pub fn missing_bin_message() -> &'static str {
    "vissue-hud is not installed. Install it with:\n  cargo install vissue-hud"
}

#[cfg(test)]
#[allow(deprecated_safe_2024)]
mod tests {
    use super::*;

    #[test]
    fn parse_offline_and_summon_flags() {
        let cli = HudCli::parse_from(["vissue-hud", "--offline", "--toggle"]);
        assert!(cli.offline);
        assert_eq!(cli.summon_action(), Some(SummonAction::Toggle));
        assert!(cli.initial_visible());
    }

    #[test]
    fn hide_starts_hidden() {
        let cli = HudCli::parse_from(["vissue-hud", "--hide", "--foreground"]);
        assert!(!cli.initial_visible());
        assert_eq!(cli.summon_action(), Some(SummonAction::Hide));
    }

    #[test]
    fn resolve_hud_bin_honors_override() {
        let _guard = crate::env_lock();
        let path = PathBuf::from("/tmp/custom-vissue-hud");
        vissue_core::process_env::override_var(HUD_BIN_ENV, path.to_str());
        assert_eq!(resolve_hud_bin(), Some(path.clone()));
        assert!(!bin_is_present(&path));
        vissue_core::process_env::clear_override(HUD_BIN_ENV);
        assert!(missing_bin_message().contains("cargo install vissue-hud"));
    }

    #[cfg(unix)]
    #[test]
    fn run_with_bounces_off_a_live_summon_socket() {
        use std::io::Read;
        use std::os::unix::net::UnixListener;

        let _guard = crate::env_lock();
        let dir =
            std::env::temp_dir().join(format!("vissue-hud-cli-bounce-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("hud.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        vissue_core::process_env::override_var(crate::summon::SOCKET_ENV, path.to_str());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            loop {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut buf = String::new();
                let _ = stream.read_to_string(&mut buf);
                if !buf.is_empty() {
                    let _ = tx.send(buf);
                    break;
                }
            }
        });
        let code = run_with(HudCli {
            root: None,
            prefix: None,
            socket: None,
            offline: true,
            foreground: true,
            toggle: true,
            show: false,
            hide: false,
        })
        .unwrap();
        assert_eq!(code, 0);
        let line = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("summon line");
        assert!(line.starts_with("toggle"), "{line}");
        vissue_core::process_env::clear_override(crate::summon::SOCKET_ENV);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn run_owner_is_noop_when_install_loses_the_bind() {
        use std::os::unix::net::UnixListener;

        let _guard = crate::env_lock();
        let dir =
            std::env::temp_dir().join(format!("vissue-hud-owner-already-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("hud.sock");
        let _ = std::fs::remove_file(&path);
        let _listener = UnixListener::bind(&path).unwrap();
        vissue_core::process_env::override_var(crate::summon::SOCKET_ENV, path.to_str());
        let code = run_owner(HudCli {
            root: Some(PathBuf::from("/tmp/vissue-hud-owner-already")),
            prefix: Some("Software".into()),
            socket: None,
            offline: true,
            foreground: true,
            toggle: false,
            show: false,
            hide: false,
        })
        .unwrap();
        assert_eq!(code, 0);
        vissue_core::process_env::clear_override(crate::summon::SOCKET_ENV);
        drop(_listener);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
