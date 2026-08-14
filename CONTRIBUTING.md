# Contributing

Contributions should preserve the plain-text issue corpus as the source of
truth and keep the CLI, MCP server, and library behavior aligned.

## Development checks

Use Rust 1.88 or newer and run:

```console
cargo fmt --all --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
bash tests/release_surface.sh
bash tests/org_interop.sh ./target/release/vissue   # needs emacs
```

Before changing file rewrites, add a fixture that includes properties, body
text, LOGBOOK entries, and CLOCK entries. Tests must prove that data outside the
operation's ownership remains unchanged.

## Documentation

The site sources are Org under `docs/orgmode/`, exported to RST and rendered
with Sphinx. Edit the Org, never `docs/source/*.rst`, which is generated.

```console
bash docs/build.sh          # export, install deps into .venv-docs, render
```

It has to finish with no Sphinx warnings. Keep each page in its own Diataxis
quadrant: `getting-started` teaches one path, `howto` answers one task per
section, `reference` describes, `explanation` argues.

Shell completions and the manual page are generated from the CLI definition
(`vissue completions`, `vissue man`); regenerate the committed copies under
`completions/` and `man/` when an argument changes.

## Changes

Keep commits focused and use a Conventional Commit subject. Add user-visible
changes under `Unreleased` in `CHANGELOG.md`. Security reports follow
`SECURITY.md` and must not be opened publicly.

## Ecosystem compatibility

[orgaw](https://github.com/HaoZeke/orgaw) consumes the public command protocol
and writes CLOCK entries into the same `issues.org`. Changes to `identity`,
`ready`, `list`, `export`, `show`, or `claim` output, and any change to the
on-disk shape, need an integration check against orgaw before release:

```console
bash tests/orgaw_interop.sh ./target/release/vissue /path/to/orgaw
```

It skips when orgaw is absent, so it is safe to run unconditionally. Emacs is
the other client of the same file:

```console
bash tests/org_interop.sh ./target/release/vissue
```

The maintainer release sequence, including the private publication arm and
first-version crates.io bootstrap, is documented in `RELEASING.md`.
