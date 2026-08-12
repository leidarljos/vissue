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

cargo publish --locked --dry-run -p vissue-core
core_patch="patch.crates-io.vissue-core.path=\"$repo_root/crates/vissue-core\""
for package in vissue-cli vissue-mcp; do
  cargo --config "$core_patch" publish --locked --dry-run -p "$package"
done

host_target=$(rustc -vV | sed -n 's/^host: //p')
test -n "$host_target"
dist build --artifacts=local --target="$host_target" --output-format=json

echo "dry release artifacts: target/distrib"
