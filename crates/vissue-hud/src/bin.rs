//! `vissue-hud` process entry. `vissue hud` execs this binary.

#![allow(
    missing_docs,
    rustdoc::missing_crate_level_docs,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]

fn main() {
    vissue_hud::log::install_panic_hook();
    let code = match vissue_hud::run_cli() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("vissue-hud: {err:#}");
            1
        }
    };
    std::process::exit(code);
}
