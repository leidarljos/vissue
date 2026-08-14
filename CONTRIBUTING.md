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

It has to finish with no Sphinx warnings. Publication is not a repository
workflow: the site is a managed Cloudflare Pages target under `ttech-ops`,
deployed with `ttech-ops site deploy vissue`, which reads its credentials
from `pass` rather than from repository secrets. Keep each page in its own Diataxis
quadrant: `getting-started` teaches one path, `howto` answers one task per
section, `reference` describes, `explanation` argues.

Shell completions and the manual page are generated from the CLI definition
(`vissue completions`, `vissue man`); regenerate the committed copies under
`completions/` and `man/` when an argument changes.

## Changes

Keep commits focused and use a Conventional Commit subject. Add user-visible
changes under `Unreleased` in `CHANGELOG.md`. Security reports follow
`SECURITY.md` and must not be opened publicly.

## The file is a contract

An `issues.org` is an ordinary Org file, and other tools read and write it.
Two checks hold that promise, and a change to the command output or the
on-disk shape has to pass both:

```console
bash tests/org_interop.sh ./target/release/vissue    # needs emacs
bash tests/release_surface.sh
```

The first drives a real Emacs over a tracker: `org-lint` must find nothing,
the agenda must show the dates, tag search must match, `org-id` must resolve,
and Emacs's own edits must survive a vissue rewrite.

Before changing file rewrites, add a fixture that includes properties, body
text, LOGBOOK entries, and CLOCK entries. Tests must prove that data outside
the operation's ownership remains unchanged.

The maintainer release sequence, including the private publication arm and
first-version crates.io bootstrap, is documented in `RELEASING.md`.
