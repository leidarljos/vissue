//! Plain-text issue tracking over per-project orgmode files.
//!
//! An issue is one top-level org heading in `<root>/<prefix>/<project>/issues.org`,
//! where `prefix` defaults to `Software`. The file is the database: every verb
//! parses it, and every mutation rewrites it under a lock. Reports never print;
//! they return `String`, so a CLI, an MCP server, and a library caller all share
//! one code path.

pub mod agent;
pub mod catalog;
pub mod config;
pub mod consensus;
pub mod digest;
pub mod error;
pub mod events;
pub mod graph;
pub mod keys;
pub mod mirror;
pub mod model;
pub mod ops;
pub mod org;
pub mod process_env;
pub mod projection;
pub mod props;
pub mod related;
pub mod report;
pub mod router;
pub mod satchel;
pub mod store;
pub mod surface;
/// Generated from `schema/vissue.capnp`; the operation set is encoded in here.
///
/// Committed rather than built, because `capnp` the compiler is not on the
/// machines that build this. Regenerating is a maintainer step.
#[allow(
    clippy::all,
    clippy::pedantic,
    missing_docs,
    missing_debug_implementations,
    unused_qualifications
)]
#[rustfmt::skip]
pub mod vissue_capnp {
    include!("schema/vissue_capnp.rs");
}
pub mod views;

pub use config::{ConsensusSection, DEFAULT_PREFIX, Layout, VissueConfig};
pub use error::{Error, Result};
pub use model::{IssueHeading, LogEntry, READY_STATES, TODO_HEADER, TODO_KEYWORDS};
pub use ops::{CreateOpts, RejectOpts, UpdatePred};
pub use router::{ProjectRef, RouteHit, Router};
pub use store::IssueDoc;
