#!/usr/bin/env bash
# CONTRIBUTING.md: changes to identity, ready, list, export, show, or claim
# output need an integration check against orgaw. This is that check. orgaw
# consumes the command protocol *and* writes CLOCK entries into the same
# issues.org, so both directions have to hold.
#
# Pass the vissue binary as $1 (or VISSUE_BIN) and orgaw as $2 (or ORGAW_BIN).
# Skips with status 0 when orgaw is not installed, so it is safe in CI that
# does not build it.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
vissue=${1:-${VISSUE_BIN:-$repo_root/target/release/vissue}}
orgaw=${2:-${ORGAW_BIN:-$(command -v orgaw || true)}}

test -x "$vissue" || {
  echo "no vissue binary at $vissue" >&2
  exit 1
}
if [ -z "$orgaw" ] || [ ! -x "$orgaw" ]; then
  echo "orgaw interop: skipped (no orgaw binary; set ORGAW_BIN)"
  exit 0
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
root=$work/tracker
issues=$root/Software/atlas/issues.org
export ISSUE_ROOT=$root VISSUE_ROOT=$root

# orgaw resolves its provider as `vissue` from PATH. Without this it would
# reach whatever copy is installed on the machine, and the test would report
# on that binary rather than the one being checked.
vissue_dir=$(cd "$(dirname "$vissue")" && pwd)
if [ "$(basename "$vissue")" != "vissue" ]; then
  ln -sf "$(cd "$(dirname "$vissue")" && pwd)/$(basename "$vissue")" "$work/vissue"
  vissue_dir=$work
fi
export PATH="$vissue_dir:$PATH"

fail() {
  echo "orgaw interop: $*" >&2
  exit 1
}

echo "1. vissue writes an issue Org owns the dates and tags of"
"$vissue" --root "$root" create -p atlas --deadline "<2026-09-01 Tue>" \
  --tags "parser,core" "Parse the manifest header" >/dev/null
"$vissue" --root "$root" create -p atlas "Plain issue with no dates" >/dev/null
id=$("$vissue" --root "$root" export |
  python3 -c 'import json,sys
for line in sys.stdin:
    row = json.loads(line)
    if row["properties"].get("DEADLINE"):
        print(row["id"]); break')
test -n "$id" || fail "could not find the dated issue"

echo "2. orgaw reads the protocol"
"$orgaw" issues | grep -q "$id" || fail "orgaw issues did not list $id"

echo "3. orgaw clocks in and out through the same file"
"$orgaw" in "$id" >/dev/null || fail "orgaw in failed"
"$orgaw" out >/dev/null || fail "orgaw out failed"

echo "4. the planning line still belongs to its heading"
# orgaw creating a LOGBOOK drawer must not push itself above the planning
# line: a DEADLINE that no longer touches its heading is not a deadline.
heading=$(grep -n "^\* .*Parse the manifest header" "$issues" | cut -d: -f1)
next=$((heading + 1))
sed -n "${next}p" "$issues" | grep -q "^DEADLINE:" ||
  fail "the planning line left its heading:
$(sed -n "${heading},$((heading + 6))p" "$issues")"

echo "5. vissue still reads the file orgaw wrote"
test "$("$vissue" --root "$root" count)" = "2" ||
  fail "an orgaw clock cost the tracker an issue"
"$vissue" --root "$root" check >/dev/null || fail "check failed after orgaw wrote"

echo "6. a vissue rewrite keeps the CLOCK orgaw recorded"
"$vissue" --root "$root" note "$id" "cli touched it" >/dev/null
grep -q "CLOCK:.*=>" "$issues" || fail "the closed CLOCK did not survive a rewrite"
"$vissue" --root "$root" export |
  grep -q '"raw":"[^"]*CLOCK:' || fail "the CLOCK is missing from the export"

echo "7. Org still reads the entry, if Emacs is here to ask"
if command -v emacs >/dev/null; then
  out=$(emacs --batch -Q --eval "(progn
      (require 'org-lint)
      (find-file \"$issues\")
      (org-mode)
      (let ((r (org-lint)))
        (if r
            (dolist (x r) (princ (format \"%s\\n\" (aref (cadr x) 2))))
          (princ \"\"))))" 2>/dev/null)
  test -z "$out" || fail "org-lint on the file orgaw wrote: $out"
else
  echo "   (no emacs; skipped the lint)"
fi

echo "vissue/orgaw interop: ok"
