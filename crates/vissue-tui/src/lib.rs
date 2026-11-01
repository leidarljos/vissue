//! Terminal board over ready, list, claims, agenda, and search.
//!
//! First paint always goes through [`CoreBackend`]. Live updates attach to
//! `vissue serve` unless `--offline`. A root/prefix mismatch stays on core
//! so mutations never hit the wrong vault.

pub mod app;
pub mod attach;
pub mod backend;
pub mod core_backend;
pub mod keys;
pub mod view;

#[cfg(unix)]
pub mod control;

pub use app::{App, RunOpts, run};
pub use attach::{AttachHooks, AttachOutcome, ServeStatus, try_attach};
pub use backend::{BackendKind, BoardBackend, ListPage, MutResult, SinceGate, UpdateReq};
pub use core_backend::CoreBackend;
pub use keys::{Action, ConfirmKind, DetailTab, Focus, Pane};

#[cfg(unix)]
pub use control::ControlBackend;
