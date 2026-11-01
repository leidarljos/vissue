# Contributing

Contributions should preserve the plain-text issue corpus as the source of
truth and keep the CLI, MCP server, and library behavior aligned.

## Development checks

Use Rust 1.89 or newer and run:

```console
cargo fmt --all --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps --document-private-items
bash tests/release_surface.sh
bash tests/org_interop.sh ./target/release/vissue   # needs emacs
```

The published `rust-version` is 1.89, and the MSRV job runs
`cargo +1.89.0 check`. Clippy, fmt, and rustdoc CI use 1.97.1.
`rust-toolchain.toml` pins that for local rustup. The dtolnay rust-toolchain
action at our SHA defaults to floating stable and ignores the file, so
every non-MSRV job names `toolchain: 1.97.1` explicitly. The MSRV job
must use `cargo +1.89.0`; a bare `cargo` would follow the file and
compile 1.97.1. Bump the file and those inputs together when taking a
new clippy. `rust-version` moves when a dependency or language feature
requires it, as a minor bump; this is not a latest-stable-only project.

Before changing file rewrites, add a fixture that includes properties, body
text, LOGBOOK entries, and CLOCK entries. Tests must show that data outside the
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

Keep commits focused and use a Conventional Commit subject.

A user-visible change adds a news fragment under `docs/newsfragments/` rather
than editing `CHANGELOG.md`, which towncrier assembles at release time:

```console
$ printf 'What changed, for someone using it.\n' \
    > docs/newsfragments/+short-slug.fixed.md
$ uvx towncrier build --draft --version 0.0.0   # preview, writes nothing
```

`docs/newsfragments/README.md` covers the types and when a fragment is
required. Security reports follow `SECURITY.md` and must not be opened
publicly.

## The file is a contract

An `issues.org` is an ordinary Org file. Emacs and other Org tools may
read and write it; that is the interop the tests pin. Agents must not:
never `Write` or `StrReplace` an `issues.org`. `vissue` and the MCP
server are the writers, and every mutation already takes the file lock.
A raw agent edit is a second writer. `scripts/pre-commit-issues-org.sh`
flags a staged `issues.org` unless `VISSUE_ALLOW_ORG_EDIT=1`.
Two checks hold the Org promise, and a change to the command output or the
on-disk shape has to pass both:

```console
bash tests/org_interop.sh ./target/release/vissue    # needs emacs
bash tests/release_surface.sh
```

The first drives a real Emacs over a tracker: `org-lint` must find nothing,
the agenda must show the dates, tag search must match, `org-id` must resolve,
and Emacs's own edits must survive a vissue rewrite.

Before changing file rewrites, add a fixture that includes properties, body
text, LOGBOOK entries, and CLOCK entries. Tests must show that data outside
the operation's ownership remains unchanged.

The command output is a contract too, and it has a reader outside this
repository. [vissue.el](https://github.com/leidarljos/vissue.el) drives the
binary and parses `ready --json` plus the text of `claim`, `identity` and
`projects`. Nothing here runs its tests, so a change to any of those four is
checked by hand:

```console
VISSUE_BIN=$PWD/target/release/vissue make -C ../vissue.el test
```

Adding a field to a JSON payload is safe; renaming or removing one, or
changing the shape of those three text outputs, is not.

The maintainer release sequence is documented in `RELEASING.md`. The
workspace crates are on crates.io; a `v*` tag publishes later versions
through trusted publishing.
