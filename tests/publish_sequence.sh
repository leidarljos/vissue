#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

source scripts/publish-crates.sh

declare -A published=()
declare -A pending=()
calls=()

registry_has() {
  local package=$1
  calls+=("info:$package")
  test "${published[$package]:-}" = 1
}

publish_package() {
  local package=$1
  calls+=("publish:$package")
  if test "$package" != vissue-core; then
    test "${published[vissue-core]:-}" = 1
  fi
  pending[$package]=1
}

pause_for_registry() {
  calls+=(pause)
  for package in "${!pending[@]}"; do
    published[$package]=1
    unset 'pending[$package]'
  done
}

main

expected='info:vissue-core publish:vissue-core info:vissue-core pause info:vissue-core info:vissue-cli publish:vissue-cli info:vissue-mcp publish:vissue-mcp'
test "${calls[*]}" = "$expected"

published[vissue-cli]=1
published[vissue-mcp]=1
calls=()
main

test "${calls[*]}" = 'info:vissue-core info:vissue-cli info:vissue-mcp'

echo 'vissue publication sequence: ok'
