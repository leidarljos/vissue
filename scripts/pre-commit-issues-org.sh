#!/usr/bin/env bash
# Flag a staged issues.org so an agent cannot commit a raw edit by accident.
#
# Emacs and other Org tools may edit the file; that is the interop contract.
# Set VISSUE_ALLOW_ORG_EDIT=1 when a human means to stage one. Agents use
# `vissue` or the MCP server, which already take the file lock.
set -euo pipefail

if [ "${VISSUE_ALLOW_ORG_EDIT:-}" = 1 ]; then
  exit 0
fi

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  exit 0
fi

staged=$(git diff --cached --name-only --diff-filter=ACMR | grep -E '(^|/)issues\.org$' || true)
if [ -z "$staged" ]; then
  exit 0
fi

echo "staged issues.org is a second writer:" >&2
echo "$staged" >&2
echo "use vissue or the MCP issue tools. a human who meant this: VISSUE_ALLOW_ORG_EDIT=1" >&2
exit 1
