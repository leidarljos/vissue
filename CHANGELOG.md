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

### Fixed

- Text `ready` now uses the corpus-wide open-blocker set, matching
  `count --ready` and JSON ready.
- Auto-unblock to TODO releases the claim.
- JSONL export and digest include CLOCK `raw` logbook lines.
- Event generation and log append take a file lock.

### Changed

- README leads with the multi-agent plan graph, a worked board example,
  an audit table for citations, and the OokCite `vissue-dag` collection.

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
