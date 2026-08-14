# Changelog

All notable changes to vissue are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and releases use
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Logo mark: a tessellated V of issue nodes with one ready cell, plus a
  circular crest and wordmark lockup under `assets/`.
- MCP tools for `ancestors`, `impact`, `cycles`, `refile`, `wait`, and
  `whoami`. `identity` also emits `root=` / `prefix=` tokens.

### Changed

- Dates and tags move to where Org keeps them. `DEADLINE`, `SCHEDULED`, and
  `CLOSED` render on the planning line under a heading, and a tag Org can
  hold renders in the heading's own tag run, so `org-agenda`, Org tag search,
  and `org-lint` all work against a tracker. Both shapes parse, so existing
  files read the same and settle on the Org shape when next rewritten.
- The tag property is `:VISSUE_TAGS:`. `TAGS` is a name Org reserves and
  `org-lint` reports; a drawer written under the old name is migrated on
  parse.
- JSONL export and `show --json` carry `org_tags` and a `tags` union.


- README leads with the multi-agent plan graph, a worked board example,
  an audit table for citations, and the OokCite `vissue-dag` collection.

### Fixed

- Text `ready` now uses the corpus-wide open-blocker set, matching
  `count --ready` and JSON ready.
- Auto-unblock to TODO releases the claim.
- JSONL export and digest include CLOCK `raw` logbook lines.
- Event generation and log append take a file lock.
- A heading whose priority cookie holds a multi-byte character no longer
  panics the parser.
- `graph` and `tree --format dot` escape backslashes and newlines in titles
  and ids, so issue text cannot become DOT syntax.
- `--project` folds case in `list`, `count`, `ready`, `export`, `graph`,
  `roadmap`, and the JSON rows, matching `claims`, `agenda`, and every verb
  that writes.
- A prefix-scoped `issues.config.toml` overrides `vissue.toml` key by key
  instead of resetting the keys it does not name.
- `refile` writes the target file before removing the heading from the
  source, so a failed write cannot lose the issue.
- `note` adds its entry at the top of the logbook, where state transitions
  and claim releases already go.
- `check` counts a duplicate id as an error.
- `hygiene` matches issues by id rather than by row prefix.
- An `issues.org` is flushed to the device before the rename publishes it.
- A reader that closes the pipe, as `vissue export | head` does, exits 0
  instead of panicking with status 101.
- The acyclicity check for `--block` reads the corpus inside the lock, so a
  peer write that landed first is part of the answer.
- A `fold` that fails partway stamps the issues it did create, so rerunning
  does not create them twice.
- `events::tail_report` counts log lines rather than sequence numbers, so a
  debounced burst no longer shortens the tail.


- An Org planning line no longer hides the property drawer below it. Writing
  a deadline, a scheduled date, or a `CLOSED` stamp from Emacs used to make
  every verb fail with ":ID: property missing" for the whole project file.
- `org-set-tags` in Emacs no longer folds the tag run into the issue title.
- `search` and `related` read tags from the heading as well as the drawer.

## [0.1.0] - 2026-08-12

### Added

- Org-backed issue storage, queries, graph operations, claims, events, mirrors,
  and JSONL export through the `vissue` CLI.
- MCP server exposing the same issue operations over stdio.
- Release preparation, package validation, and cross-platform archive wiring.
- Ecosystem documentation for using vissue as the issue provider for
  [orgaw](https://github.com/HaoZeke/orgaw).

[Unreleased]: https://github.com/HaoZeke/vissue/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/HaoZeke/vissue/releases/tag/v0.1.0
