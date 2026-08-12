#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

command -v dist >/dev/null || {
  echo "cargo-dist 0.30.3 is required as the 'dist' executable" >&2
  exit 1
}

test "$(dist --version | awk '{print $2}')" = "0.30.3" || {
  echo "release preparation requires cargo-dist 0.30.3" >&2
  exit 1
}

cargo fmt --all --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
bash tests/release_surface.sh
dist generate --check

for package in vissue-core vissue-cli vissue-mcp; do
  cargo publish --locked --dry-run -p "$package"
done

dist build --artifacts=all --output-format=json

echo "dry release artifacts: target/distrib"
