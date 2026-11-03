#!/usr/bin/env bash
# Record runs of the watcher's full rebuild, each with a header naming the machine, as `OUT/run-N.txt`.
#
#     scripts/bench/rebuild_timing.sh OUT [RUNS]
#
# `RUNS` defaults to 6, as in watch_timing.sh.
# Every run rebuilds the 173-project tracker of read_verbs.md, the tracker the poll tick in watch_timing.md is timed on.
set -euo pipefail

out=$1
runs=${2:-6}
here=$(cd "$(dirname "$0")" && pwd)
cd "$(git -C "$here" rev-parse --show-toplevel)"
mkdir -p "$out"
out=$(cd "$out" && pwd)

tracker=$(mktemp -d)/synth4k-173
trap 'rm -rf "$(dirname "$tracker")"' EXIT
python3 scripts/bench/synth_tracker.py "$tracker" --issues 4000 --projects 173 --lead ljos=3800 > /dev/null
cargo test --locked --release -p vissue-serve --lib --no-run

for n in $(seq 1 "$runs"); do
  {
    bash scripts/bench/bench_header.sh release
    echo "# run: $n of $runs"
    VISSUE_REBUILD_TRACKER="$tracker" cargo test --locked --release -q -p vissue-serve --lib \
      -- --ignored --exact unix::catalog::tests::full_rebuild_timing --nocapture --test-threads=1 |
      grep '^rebuild '
  } > "$out/run-$n.txt"
  echo "run $n of $runs: $out/run-$n.txt"
done
