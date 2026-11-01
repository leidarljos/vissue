//! `vissue-mcp`: a Model Context Protocol server over vissue-core.

#![allow(
    missing_debug_implementations,
    missing_docs,
    rustdoc::missing_crate_level_docs,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]

use rmcp::ServiceExt;

mod prompts;
mod server;
mod tools;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let version_arg = args.next();
    if version_arg
        .as_deref()
        .is_some_and(|arg| arg == "--version" || arg == "-V")
        && args.next().is_none()
    {
        println!("vissue-mcp {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let server = server::VissueServer::from_env()?;
    let service = server.serve(rmcp::transport::io::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
