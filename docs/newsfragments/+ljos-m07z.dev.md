Every CI job that installs Rust 1.97.1 now installs rustfmt and clippy with it.
No cargo call adds them later in a second install, which used to stop on `detected conflict: 'bin/cargo-fmt'`.
