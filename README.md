# vissue

<p align="center">
  <img src="assets/logo.svg" width="176" alt="vissue mark: a ready teal node with coral edges to two waiting issues">
</p>

[![CI](https://github.com/leidarljos/vissue/actions/workflows/ci_test.yml/badge.svg)](https://github.com/leidarljos/vissue/actions/workflows/ci_test.yml)
[![crates.io](https://img.shields.io/crates/v/vissue-cli.svg)](https://crates.io/crates/vissue-cli)
[![docs.rs](https://docs.rs/vissue-core/badge.svg)](https://docs.rs/vissue-core)
[![MSRV](https://img.shields.io/badge/MSRV-1.89-blue.svg)](https://www.rust-lang.org/)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![docs](https://img.shields.io/badge/docs-vissue.rgoswami.me-teal.svg)](https://vissue.rgoswami.me)

An issue tracker that is a directory of Org files. One file per project
holds the issues as headings; `:PARENT:` groups them under a plan,
`:BLOCKED_BY:` orders them, `ready` is the frontier any agent can pick
up, `claim` is the lock so two agents do not take the same one, `recall`
is what an issue stands on, `:DEEDS:` names what its work produced, and
`consensus` weighs the ballots several agents cast on it. There is no
second store: the files diff, merge and grep with the repository they
live in, Emacs reads them with nothing installed, and a CLI and a Model
Context Protocol server share one library. Agents write through those
two surfaces, never by editing an `issues.org`.

## Install

```console
$ cargo install vissue-cli
$ cargo install vissue-mcp   # the MCP server, same version
$ cargo install vissue-hud   # summonable overlay, optional
```

Tagged releases carry prebuilt archives and a shell installer on the
[releases page](https://github.com/leidarljos/vissue/releases).
`vissue completions zsh` and `vissue man` come out of the binary.

## A minute of it

```console
$ vissue create --project parser "Reject a manifest with no header"
parser-k29f  TODO  [#C]  Reject a manifest with no header

$ vissue create --project parser --priority A "Publish the release notes"
parser-3xq7  TODO  [#A]  Publish the release notes

$ vissue update parser-3xq7 --block parser-k29f
parser-3xq7: state TODO -> BLOCKED (auto on block), blocked_by += parser-k29f

$ vissue ready
parser-k29f            TODO      [#C]  Reject a manifest with no header
```

The files land under `<root>/Software/<project>/issues.org`; the prefix
is configurable and `<root>` comes from `--root`, `VISSUE_ROOT`, or a
user-level route table that sends named projects to other checkouts.

## Documentation

Full documentation is at **[vissue.rgoswami.me](https://vissue.rgoswami.me)**.

| Page | What it answers |
|---|---|
| [Getting started](https://vissue.rgoswami.me/getting-started) | An empty directory to a backlog two workers share |
| [How-to](https://vissue.rgoswami.me/howto) | One task at a time: split a plan, share, watch, fold, project, validate |
| [Reference](https://vissue.rgoswami.me/reference) | Commands, properties, config, export schema, exit statuses |
| [Control](https://vissue.rgoswami.me/control) | Unix socket protocol, `serve`, the terminal board and the HUD |
| [Explanation](https://vissue.rgoswami.me/explanation) | Why the file is the database, what `recall` and `consensus` are, and the sources |
| [Emacs](https://vissue.rgoswami.me/emacs) | The agenda, tag search, and `id:` links, with nothing installed |
| [Org syntax](https://vissue.rgoswami.me/org-syntax) | Org 9.8 mapped onto an `issues.org` |
| [Org ecosystem](https://vissue.rgoswami.me/ecosystem) | ELPA / MELPA names the tracker owns, reads, or preserves |

The sources are Org under `docs/orgmode/`; `bash docs/build.sh` renders
the site.

## Contributing

Conventional commits, `cargo fmt`, and `cargo clippy -- -D warnings`.
Run `prek install` for the hooks. The minimum supported Rust version is
1.89. See [CONTRIBUTING.md](CONTRIBUTING.md) for the interop checks a
change to the command surface or the on-disk shape has to pass.

## Citation

See [CITATION.cff](CITATION.cff).

## License

MIT. See [LICENSE](LICENSE).
