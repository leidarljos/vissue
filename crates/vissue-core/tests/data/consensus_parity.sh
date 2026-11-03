#!/bin/sh
# Rewrite each case's "crate" object in consensus_parity.json with what the
# pinned ljos-consensus prints for its ballots, trust rows and anchors, the
# three flags ljos passes. Inputs are left as they are.
#
#   cargo install --locked --version =0.9.0 ljos-consensus
#   sh crates/vissue-core/tests/data/consensus_parity.sh
set -eu

pin="ljos-consensus 0.9.0"
here=$(CDPATH='' cd -- "$(dirname "$0")" && pwd)
file="$here/consensus_parity.json"

have=$(ljos-consensus --version)
if [ "$have" != "$pin" ]; then
  echo "consensus_parity.sh: the expected data is from $pin, and PATH has $have" >&2
  exit 1
fi

jq -c '.cases[] | del(.crate)' "$file" | while IFS= read -r case; do
  out=$(ljos-consensus settle \
    --ballots "$(printf '%s' "$case" | jq -c .ballots)" \
    --trust "$(printf '%s' "$case" | jq -c .trust)" \
    --susceptibility-of "$(printf '%s' "$case" | jq -c .anchors)")
  printf '%s' "$case" | jq -c --argjson out "$out" '.crate = $out'
done | jq -s --arg pin "$pin" '{generator: $pin, cases: .}' >"$file.new"
mv "$file.new" "$file"
