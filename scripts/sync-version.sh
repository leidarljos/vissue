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

# The workspace pins vissue-core by version as well as by path, because the
# published crates depend on each other by version. Missing this one is how a
# lockfile refuses to resolve halfway through a bump.
sed -i "s|^vissue-core = { version = \".*\", path = |vissue-core = { version = \"$version\", path = |" Cargo.toml

sed -i "s/^version: .*/version: $version/" CITATION.cff
sed -i "s/^release = \".*\"/release = \"$version\"/" docs/source/conf.py
sed -i "s/^version = \".*\"/version = \"$minor\"/" docs/source/conf.py
sed -i "0,/^version = /s/^version = \".*\"/version = \"$version\"/" towncrier.toml

# The lockfile records the workspace members' own versions.
cargo generate-lockfile --offline >/dev/null 2>&1 || cargo generate-lockfile

echo "version surfaces now at $version:"
grep -m1 '^version = ' Cargo.toml
grep -m1 '^version:' CITATION.cff
grep -m1 '^release = ' docs/source/conf.py
grep -m1 '^version = ' towncrier.toml
