#!/usr/bin/env bash
# Time the read verbs over a tracker. Reports the minimum of R runs, which is
# the number least moved by whatever else the machine is doing.
#   time_cli.sh TRACKER_ROOT [RUNS]
set -euo pipefail
root="${1:?tracker root}"; runs="${2:-7}"
export VISSUE_ROOT="$root" VISSUE_PREFIX=Issues
some_id=$(grep -rh "^:ID:" "$root/Issues" | awk 'NR==1{print $2}')
verbs=("list" "ready" "count" "check" "search lease" "show $some_id" "tree $some_id" "cycles" "digest" "agenda")
printf "%-22s %10s %10s\n" verb min_ms median_ms
for v in "${verbs[@]}"; do
  ts=()
  for _ in $(seq "$runs"); do
    s=$(date +%s%N); vissue $v >/dev/null 2>&1 || true; e=$(date +%s%N)
    ts+=($(( (e - s) / 1000000 )))
  done
  sorted=$(printf "%s\n" "${ts[@]}" | sort -n)
  min=$(echo "$sorted" | head -1); med=$(echo "$sorted" | sed -n "$(( (runs + 1) / 2 ))p")
  printf "%-22s %10s %10s\n" "$v" "$min" "$med"
done
