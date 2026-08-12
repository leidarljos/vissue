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
```

Before changing file rewrites, add a fixture that includes properties, body
text, LOGBOOK entries, and CLOCK entries. Tests must prove that data outside the
operation's ownership remains unchanged.

## Changes

Keep commits focused and use a Conventional Commit subject. Add user-visible
changes under `Unreleased` in `CHANGELOG.md`. Security reports follow
`SECURITY.md` and must not be opened publicly.

## Ecosystem compatibility

[orgaw](https://github.com/HaoZeke/orgaw) consumes the public command protocol.
Changes to `identity`, `ready`, `list`, `export`, `show`, or `claim` output need
an integration test against orgaw before release.
