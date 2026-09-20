//! Summonable icedtea task board. Home is the project list.
//!
//! First paint always goes through [`vissue_tui::CoreBackend`]. Live updates
//! attach to `vissue serve` unless `--offline`. A root/prefix mismatch stays
//! on core so mutations never hit the wrong vault.

pub mod attach;
pub mod cli;
pub(crate) mod dates;
pub mod desktop;
pub mod detach;
pub mod install_desktop;
pub mod fuzzy;
pub mod keys;
pub mod log;
pub mod palette;
pub(crate) mod place;
pub mod summon;
pub mod theme;
pub mod tray;
pub mod wire;

pub mod app;
pub mod view;

pub use cli::{HudCli, run_cli};
pub use palette::Palette;
pub use summon::{SummonAction, SummonRequest, parse_request, sanitize_token};

#[cfg(test)]
pub(crate) fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
