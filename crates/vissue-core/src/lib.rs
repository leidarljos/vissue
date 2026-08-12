//! Plain-text issue tracking over per-project orgmode files.
//!
//! An issue is one top-level org heading in `<root>/<prefix>/<project>/issues.org`,
//! where `prefix` defaults to `Software`. The file is the database: every verb
//! parses it, and every mutation rewrites it under a lock. Reports never print;
//! they return `String`, so a CLI, an MCP server, and a library caller all share
//! one code path.

pub mod agent;
pub mod config;
pub mod digest;
pub mod events;
pub mod graph;
pub mod mirror;
pub mod model;
pub mod ops;
pub mod related;
pub mod report;
pub mod store;

pub use config::{Layout, VissueConfig, DEFAULT_PREFIX};
pub use model::{IssueHeading, LogEntry, READY_STATES, TODO_HEADER, TODO_KEYWORDS};
pub use ops::CreateOpts;
pub use store::IssueDoc;
