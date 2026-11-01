# Security Policy

vissue reads and writes orgmode files under a tracker root. The CLI parses local
files and rewrites them in place. The MCP server exposes the same operations to
an agent over stdio.

## Supported versions

| Version | Supported |
|---------|-----------|
| 0.7.x   | Yes       |
| 0.6.x   | Yes       |
| < 0.6   | No        |

Security fixes apply to the latest `0.7.x` release and to `main`.

## Trust boundaries

- Treat every issue file as untrusted text. Titles, properties, and bodies are
  written by whoever can commit to the tracker, and they end up in Graphviz DOT,
  markdown, and JSON output that other tools consume.
- `vissue-mcp` grants an agent write access to every project under its root.
  Point it at a tracker root you are willing to let that agent edit, not at a
  home directory.
- `body-excerpt` returns file content. It screens for a few obvious secret
  markers, which is a guard against accident and not a redaction guarantee. Do
  not put credentials in issue bodies.
- `mirror` output is meant to be shared. Whatever is in the selected projects
  ends up in the projection, so select projects rather than filtering by hand
  afterwards.
- The advisory lock protects concurrent vissue processes. It does not protect a
  file being edited by hand in an editor at the same time.

## Reporting a vulnerability

Email the maintainer at [rgoswami@ieee.org](mailto:rgoswami@ieee.org) with a
description, a reproducer, the affected command, and the expected impact. Do not
open a public issue for security matters.

Relevant reports include path traversal when resolving a root, prefix, or mirror
destination, unintended writes outside the tracker root, secret disclosure
through excerpt or export paths, and supply-chain problems in release artifacts.
