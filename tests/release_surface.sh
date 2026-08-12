#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

for path in README.md CHANGELOG.md CONTRIBUTING.md SECURITY.md CITATION.cff LICENSE; do
  test -s "$path" || {
    echo "missing release document: $path" >&2
    exit 1
  }
done

grep -q '^## Ecosystem$' README.md
grep -q 'https://github.com/HaoZeke/orgaw' README.md
grep -q 'orgaw' CHANGELOG.md
grep -q 'orgaw' CITATION.cff

test -s scripts/release-prepare.sh || {
  echo "missing dry release preparation script" >&2
  exit 1
}
test -s .github/workflows/publish-crates.yml || {
  echo "missing crates.io publication workflow" >&2
  exit 1
}
grep -q 'pr-run-mode = "upload"' dist-workspace.toml
grep -q 'aarch64-apple-darwin' dist-workspace.toml
grep -q 'x86_64-unknown-linux-gnu' dist-workspace.toml
grep -q 'cargo-dist-installer.sh' .github/workflows/release.yml
grep -q 'crates-io-auth-action@v1' .github/workflows/publish-crates.yml
grep -q 'cargo publish --locked -p vissue-core' .github/workflows/publish-crates.yml
grep -q 'cargo publish --locked -p vissue-cli' .github/workflows/publish-crates.yml
grep -q 'cargo publish --locked -p vissue-mcp' .github/workflows/publish-crates.yml
grep -q 'patch.crates-io.vissue-core.path' scripts/release-prepare.sh
grep -q 'patch.crates-io.vissue-core.path' .github/workflows/publish-crates.yml
grep -q 'dist build --artifacts=local --target="\$host_target"' scripts/release-prepare.sh
! grep -q 'dist build --artifacts=all' scripts/release-prepare.sh

for manifest in crates/vissue-core/Cargo.toml crates/vissue-cli/Cargo.toml crates/vissue-mcp/Cargo.toml; do
  grep -q '^description = ' "$manifest"
  grep -q '^readme.workspace = true$' "$manifest"
done

echo "vissue release surface: ok"
