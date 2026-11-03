#!/usr/bin/env bash
# Record the three change-check tests against the watcher's comparison and against each broken one, as `OUT/none.txt`, `OUT/bytes.txt` and `OUT/no-spans.txt`.
#
#     scripts/bench/watch_mutants.sh OUT
#
# Each output starts with a header naming the machine and the commit, and a `# mutant:` line naming the diff applied on top of that commit.
# The tests are built in the dev profile, as a plain `cargo test` builds them.
set -euo pipefail

out=$1
here=$(cd "$(dirname "$0")" && pwd)
cd "$(git -C "$here" rev-parse --show-toplevel)"
mkdir -p "$out"
out=$(cd "$out" && pwd)

catalog=crates/vissue-serve/src/unix/catalog.rs
tests=(
  a_touch_that_changes_no_bytes_is_not_broadcast
  an_edit_that_parses_the_same_is_not_broadcast
  an_edit_that_only_moves_lines_is_broadcast
)
# The outputs are written outside the tree and copied at the end, so no header after the first sees an earlier output as an uncommitted change.
work=$(mktemp -d)
trap 'git checkout -q -- "$catalog"; rm -rf "$work"' EXIT

for m in none bytes no-spans; do
  {
    bash scripts/bench/bench_header.sh dev
    if [ "$m" = none ]; then
      echo "# mutant: none"
    else
      echo "# mutant: scripts/bench/data/watch_mutants/$m.diff applied to $catalog"
    fi
  } > "$work/$m.txt"
  if [ "$m" != none ]; then
    git apply "scripts/bench/data/watch_mutants/$m.diff"
  fi
  status=0
  cargo test --locked -p vissue-serve --test protocol -- "${tests[@]}" >> "$work/$m.txt" || status=$?
  echo "exit status: $status" >> "$work/$m.txt"
  git checkout -q -- "$catalog"
  echo "$m: exit status $status"
done
cp "$work"/none.txt "$work"/bytes.txt "$work"/no-spans.txt "$out"/
