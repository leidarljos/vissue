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
cargo test --locked --workspace --no-fail-fast
cargo clippy --locked --workspace --all-targets -- -D warnings
bash tests/release_surface.sh
dist generate --check

# Every publishable crate, in the order the release will upload them, each
# packaged against its siblings' local paths: the versions they name are not
# on the registry until the release puts them there. Naming a few crates
# here is how a local check passes while the release stops at the first
# crate whose dependency was never uploaded.
mapfile -t crates < <(cargo run -q -p xtask -- publish-order)
test "${#crates[@]}" -gt 0
echo "publishing order: ${crates[*]}"
for target in "${crates[@]}"; do
  args=()
  for dep in "${crates[@]}"; do
    [ "$dep" = "$target" ] && continue
    args+=(--config "patch.crates-io.$dep.path=\"$repo_root/crates/$dep\"")
  done
  cargo "${args[@]}" publish --locked --dry-run -p "$target"
done

host_target=$(rustc -vV | sed -n 's/^host: //p')
test -n "$host_target"
dist build --artifacts=local --target="$host_target" --output-format=json

echo "dry release artifacts: target/distrib"
