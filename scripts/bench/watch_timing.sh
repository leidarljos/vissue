#!/usr/bin/env bash
# Record runs of the watch_timing example, each with a header naming the machine, as `OUT/run-N.txt`.
#
#     scripts/bench/watch_timing.sh OUT [RUNS]
#
# `RUNS` defaults to 6, the fewest the bench notes report a median and interquartile range (IQR) over.
# Every run is timed against the 173-project tracker of read_verbs.md, so the poll tick is timed on the same tracker as the full rebuild of rebuild_timing.sh.
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
cargo build --locked --release -p vissue-serve --example watch_timing

for n in $(seq 1 "$runs"); do
  {
    bash scripts/bench/bench_header.sh release
    echo "# run: $n of $runs"
    ./target/release/examples/watch_timing --run "$n" "$tracker" Issues
  } > "$out/run-$n.txt"
  echo "run $n of $runs: $out/run-$n.txt"
done
