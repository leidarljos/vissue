#!/usr/bin/env bash
# Move every surface that names a version to the same one.
#
# Five files carry the version, and a release where they disagree is a release
# that lies somewhere: the crates, the citation metadata, the documentation
# site, and towncrier's own idea of what it is building. Run from `cog bump`
# via cog.toml, or by hand as `scripts/sync-version.sh X.Y.Z`.
set -euo pipefail

version=${1:?usage: sync-version.sh X.Y.Z}
if ! [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "not a semantic version: $version" >&2
  exit 1
fi
minor=${version%.*}

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

# Workspace version: the first `version = ` in the manifest, under
# [workspace.package]. The member crates inherit it.
sed -i "0,/^version = /s/^version = \".*\"/version = \"$version\"/" Cargo.toml

# The workspace pins its own crates by version as well as by path, because
# the published crates depend on each other by version. Missing one is how a
# lockfile refuses to resolve halfway through a bump, so every pin moves
# rather than a named few: the list grew from one to four without this line
# noticing.
sed -i -E "s|^(vissue-[a-z-]+) = \{ version = \"[^\"]*\", path = |\1 = { version = \"$version\", path = |" Cargo.toml

sed -i "s/^version: .*/version: $version/" CITATION.cff
sed -i "s/^release = \".*\"/release = \"$version\"/" docs/source/conf.py
sed -i "s/^version = \".*\"/version = \"$minor\"/" docs/source/conf.py
sed -i "0,/^version = /s/^version = \".*\"/version = \"$version\"/" towncrier.toml

# The lockfile records the workspace members' own versions. `update
# --workspace` moves only those; `generate-lockfile` re-resolves every
# dependency, offline to the newest cached release, so a bump would ship
# upgrades no test ran against.
cargo update --workspace --offline >/dev/null 2>&1 || cargo update --workspace

# And check it took. A member whose name does not match the `vissue-` pattern
# the rest of this script keys on is exactly the one that gets left behind, and
# the failure lands later as `--locked` refusing to resolve in CI rather than
# here. `cargo metadata --locked` is the same question CI asks.
if ! cargo metadata --locked --format-version 1 >/dev/null 2>&1; then
  echo "error: the lockfile does not match the manifests after the bump." >&2
  echo "       every workspace member inherits the workspace version, so a" >&2
  echo "       member left at the old one is the usual cause:" >&2
  cargo metadata --format-version 1 2>/dev/null \
    | grep -o "\"name\":\"[^\"]*\",\"version\":\"[^\"]*\"" \
    | grep -v "\"$version\"" | head -5 >&2 || true
  exit 1
fi

echo "version surfaces now at $version:"
grep -m1 '^version = ' Cargo.toml
# Any internal pin left behind would fail to resolve at publish time, so it
# is worth saying out loud rather than discovering on the tag.
stale=$(grep -E '^vissue-[a-z-]+ = \{ version = ' Cargo.toml | grep -v "\"$version\"" || true)
if [ -n "$stale" ]; then
  echo "error: internal pins left at another version:" >&2
  echo "$stale" >&2
  exit 1
fi
grep -m1 '^version:' CITATION.cff
grep -m1 '^release = ' docs/source/conf.py
grep -m1 '^version = ' towncrier.toml
