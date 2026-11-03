#!/usr/bin/env bash
# Diff what two vissue binaries print for the read verbs over one tracker.
# Prints the count and exits 0 when every output and exit code matches byte for
# byte; otherwise prints the first differing verb's diff and exits 1.
#   same_output.sh BEFORE_BIN AFTER_BIN TRACKER_ROOT
set -euo pipefail
before="${1:?before binary}"; after="${2:?after binary}"; root="${3:?tracker root}"
export VISSUE_ROOT="$root" VISSUE_PREFIX=Issues VISSUE_NO_ROUTE=1
ids=$(grep -rh "^:ID:" "$root/Issues" | awk '{print $2}')
# A limit of every issue on the tracker never cuts a hit list short.
all=$(echo "$ids" | wc -l)
verbs=("list" "list --json" "ready" "ready --json" "count" "count --ready"
       "search lease" "search --json --limit $all lease" "search --limit $all fence")
for p in $(ls "$root/Issues"); do
  verbs+=("list -p $p" "list --json -p $p" "ready -p $p" "ready --json -p $p")
done
# Every 97th id: about forty on a 4000-issue tracker, from the lead project and
# the small ones alike. A prime, so the stride never falls into step with the
# size of a project.
for id in $(echo "$ids" | awk 'NR % 97 == 1'); do
  verbs+=("show $id" "show --json $id" "show --org $id")
done
checked=0
for v in "${verbs[@]}"; do
  if ! diff <("$before" $v 2>&1; echo "exit $?") <("$after" $v 2>&1; echo "exit $?") >/tmp/same_output.diff; then
    echo "differs: vissue $v"; cat /tmp/same_output.diff; exit 1
  fi
  checked=$((checked + 1))
done
echo "identical: $checked invocations"
