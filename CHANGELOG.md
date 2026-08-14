# Changelog

All notable changes to vissue are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and releases use
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- towncrier release notes start -->

## [0.2.0](https://github.com/HaoZeke/vissue/releases/tag/v0.2.0) - 2026-08-14

### Added

- A documentation site at [vissue.rgoswami.me](https://vissue.rgoswami.me), written as Org under `docs/orgmode/` and rendered with Sphinx. The pages keep to one Diataxis quadrant each, and the README is a front door rather than the whole manual. The site is a managed Cloudflare Pages target, deployed with `ttech-ops site deploy vissue`.
- Logo mark: a tessellated V of issue nodes with one ready cell, plus a circular crest and wordmark lockup under `assets/`.
- MCP tools for `ancestors`, `impact`, `cycles`, `refile`, `wait`, and `whoami`. `identity` also emits `root=` / `prefix=` tokens.
- `vissue completions <shell>` and `vissue man`, generated from the CLI definition so they cannot drift, with copies committed under `completions/` and `man/`.

### Changed

- **Compatibility**: a tracker written by this version is not readable by 0.1.0, which fails on the planning line with ":ID: property missing". Upgrade every reader of a tracker together. A second tool that writes an `issues.org` also needs to place a `:LOGBOOK:` drawer below the planning line, or it breaks the file for every reader including `vissue`.
- A new project file carries `#+CATEGORY: <project>`. Org otherwise takes the agenda category from the file name, and every project's file is `issues.org`, so a multi-project agenda labelled every row `issues`.
- Dates and tags move to where Org keeps them. `DEADLINE`, `SCHEDULED`, and `CLOSED` render on the planning line under a heading, and a tag Org can hold renders in the heading's own tag run, so `org-agenda`, Org tag search, and `org-lint` all work against a tracker. Both shapes parse, so existing files read the same and settle on the Org shape when next rewritten.
- JSONL export and `show --json` carry `org_tags` and a `tags` union.
- The README is a front door rather than the manual: what vissue is, how to install it, a minute of it working, and links into the documentation site. The tutorial, how-to, reference, and explanation move to their own pages.
- The tag property is `:VISSUE_TAGS:`. `TAGS` is a name Org reserves and `org-lint` reports; a drawer written under the old name is migrated on parse.
- `tests/org_interop.sh` drives Emacs over a tracker in CI: org-lint, org-agenda, tag search, and org-id all have to agree, and Org's own edits have to survive a vissue rewrite.

### Fixed

- A `fold` that fails partway stamps the issues it did create, so rerunning does not create them twice.
- A heading whose priority cookie holds a multi-byte character no longer panics the parser.
- A prefix-scoped `issues.config.toml` overrides `vissue.toml` key by key instead of resetting the keys it does not name.
- A reader that closes the pipe, as `vissue export | head` does, exits 0 instead of panicking with status 101.
- An Org planning line no longer hides the property drawer below it. Writing a deadline, a scheduled date, or a `CLOSED` stamp from Emacs used to make every verb fail with ":ID: property missing" for the whole project file.
- An `issues.org` is flushed to the device before the rename publishes it.
- Auto-unblock to TODO releases the claim.
- Event generation and log append take a file lock.
- JSONL export and digest include CLOCK `raw` logbook lines.
- Text `ready` now uses the corpus-wide open-blocker set, matching `count --ready` and JSON ready.
- The `body-excerpt` secret screen reads credential shapes rather than three substrings, and matches vendor token prefixes on a whole word: the old form would have flagged "making" as an AWS key had that prefix been listed.
- The acyclicity check for `--block` reads the corpus inside the lock, so a peer write that landed first is part of the answer.
- `--project` folds case in `list`, `count`, `ready`, `export`, `graph`, `roadmap`, and the JSON rows, matching `claims`, `agenda`, and every verb that writes.
- `check` counts a duplicate id as an error, and reports a `:PARENT:` cycle. Every id in a parent loop resolves, so the edge checks passed it while the hierarchy stayed unwalkable and `tree` printed "(cycle, stopping)".
- `create` reports a full id space instead of panicking inside the write. `id_length = 2` is 1296 suffixes, which a project can outgrow.
- `events::tail_report` counts log lines rather than sequence numbers, so a debounced burst no longer shortens the tail.
- `graph` and `tree --format dot` escape backslashes and newlines in titles and ids, so issue text cannot become DOT syntax.
- `hygiene` matches issues by id rather than by row prefix.
- `mirror --out` writes through a flushed temporary and a rename, so a reader mid-pull cannot see a half-written projection.
- `note` adds its entry at the top of the logbook, where state transitions and claim releases already go.
- `org-set-tags` in Emacs no longer folds the tag run into the issue title.
- `refile` writes the target file before removing the heading from the source, so a failed write cannot lose the issue.
- `search` and `related` read tags from the heading as well as the drawer.


### Developer

- The Emacs interop check reports the error when Emacs itself fails, instead of exiting with a bare status and no message.


## [0.1.0] - 2026-08-12

### Added

- Org-backed issue storage, queries, graph operations, claims, events, mirrors,
  and JSONL export through the `vissue` CLI.
- MCP server exposing the same issue operations over stdio.
- Release preparation, package validation, and cross-platform archive wiring.

[0.1.0]: https://github.com/HaoZeke/vissue/releases/tag/v0.1.0
