#!/usr/bin/env bash
# Time the read verbs over a tracker. Reports the minimum of R runs, which is
# the number least moved by whatever else the machine is doing, and the median.
#   time_cli.sh TRACKER_ROOT [RUNS]
# VISSUE_BIN names the binary to time; it defaults to the vissue on PATH.
set -euo pipefail
root="${1:?tracker root}"; runs="${2:-7}"; bin="${VISSUE_BIN:-vissue}"
export VISSUE_ROOT="$root" VISSUE_PREFIX=Issues VISSUE_NO_ROUTE=1
# The middle issue of the largest project: a lookup by id reads that whole
# file, and a lookup that stopped at the issue would be timed on a typical one.
largest=$(ls -S "$root"/Issues/*/issues.org | head -1)
some_id=$(grep "^:ID:" "$largest" | awk '{print $2}' | sed -n "$(( $(grep -c '^:ID:' "$largest") / 2 ))p")
verbs=("list" "list --json" "ready" "ready --json" "search lease" "show $some_id" "show --json $some_id"
       "count" "check" "tree $some_id" "cycles" "digest" "agenda")
printf "%-26s %10s %10s\n" verb min_ms median_ms
for v in "${verbs[@]}"; do
  ts=()
  for _ in $(seq "$runs"); do
    s=$(date +%s%N); "$bin" $v >/dev/null 2>&1 || true; e=$(date +%s%N)
    ts+=($(( (e - s) / 1000000 )))
  done
  sorted=$(printf "%s\n" "${ts[@]}" | sort -n)
  min=$(echo "$sorted" | head -1); med=$(echo "$sorted" | sed -n "$(( (runs + 1) / 2 ))p")
  printf "%-26s %10s %10s\n" "$v" "$min" "$med"
done
