#!/usr/bin/env bash
# Emacs is the other client of an issues.org. This proves the two agree:
# what vissue writes, Org reads with its own agenda, tag search, and linter;
# what Org writes, vissue parses and rewrites without losing it.
#
# Needs `emacs` on PATH. Pass the vissue binary as $1, or set VISSUE_BIN.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
bin=${1:-${VISSUE_BIN:-$repo_root/target/release/vissue}}
# A user-level route table would send `demo` at the live vault. This script
# owns a scratch tracker and must stay on --root.
export VISSUE_NO_ROUTE=1

command -v emacs >/dev/null || {
  echo "org interop needs emacs on PATH" >&2
  exit 1
}
test -x "$bin" || {
  echo "no vissue binary at $bin; build it or pass one" >&2
  exit 1
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
root=$work/tracker
issues=$root/Software/demo/issues.org
# A create refuses a named root that does not exist, so the tracker directory
# is made first, as a person starting one would.
mkdir -p "$root"

fail() {
  echo "org interop: $*" >&2
  exit 1
}

# Run org-lint and require it to find nothing. A file vissue wrote that Org
# lints is a file Org will not fight the next time someone edits it.
lint_clean() {
  local file=$1 label=$2 out
  out=$(emacs --batch -Q --eval "(progn
      (require 'org-lint)
      (find-file \"$file\")
      (org-mode)
      (let ((r (org-lint)))
        (if r
            (dolist (x r) (princ (format \"%s\\n\" (aref (cadr x) 2))))
          (princ \"\"))))" 2>/dev/null)
  test -z "$out" || fail "org-lint on $label: $out"
}

echo "1. vissue writes a tracker"
"$bin" --root "$root" create -p demo --deadline "<2026-09-01 Tue>" \
  --tags "parser,core,needs-review" "Ship the parser" >/dev/null
"$bin" --root "$root" create -p demo --scheduled "<2026-08-20 Thu>" \
  --body "Scope: notes." "Draft the notes" >/dev/null
"$bin" --root "$root" mirror --out "$work/mirror.org" >/dev/null

lint_clean "$issues" "issues.org"
lint_clean "$work/mirror.org" "the mirror"

echo "2. Org reads the dates vissue wrote"
agenda=$(emacs --batch -Q --eval "(progn
    (require 'org-agenda)
    (setq org-agenda-files (list \"$issues\")
          org-agenda-span 90
          org-agenda-start-day \"2026-08-01\")
    (org-agenda-list)
    (with-current-buffer org-agenda-buffer-name
      (princ (buffer-substring-no-properties (point-min) (point-max)))))" 2>/dev/null)
grep -q "Deadline:.*Ship the parser" <<<"$agenda" ||
  fail "org-agenda did not see the deadline"
# Every project file is named issues.org, so without #+CATEGORY: the agenda
# labels every row with the file name instead of the project.
grep -q "demo:.*Ship the parser" <<<"$agenda" ||
  fail "org-agenda labelled the row by file name, not by project"
grep -q "Scheduled:.*Draft the notes" <<<"$agenda" ||
  fail "org-agenda did not see the scheduled date"

echo "3. Org tag search finds the tags vissue wrote"
tagged=$(emacs --batch -Q --eval "(progn
    (require 'org)
    (find-file \"$issues\")
    (org-mode)
    (princ (format \"%S\" (org-map-entries (lambda () (org-get-heading t t)) \"parser&core\" nil))))" 2>/dev/null)
grep -q "Ship the parser" <<<"$tagged" || fail "org tag search missed the issue: $tagged"

echo "4. Org ids resolve, so [[id:...]] links work"
first_id=$("$bin" --root "$root" export | head -1 |
  python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
# Keep stderr: an Emacs that errors here exits non-zero with nothing on
# stdout, and swallowing it turns a diagnosable failure into a bare 255.
if ! found=$(emacs --batch -Q --eval "(progn
    (require 'org)
    (require 'org-id)
    (setq org-id-locations-file \"$work/id-locations.el\"
          org-id-track-globally t
          org-agenda-files (list \"$issues\"))
    (org-id-update-id-locations (list \"$issues\"))
    (princ (format \"%S\" (org-id-find \"$first_id\"))))" 2>&1); then
  fail "emacs failed resolving $first_id:
$found"
fi
grep -q "issues.org" <<<"$found" || fail "org-id could not resolve $first_id: $found"

echo "5. Emacs edits the tracker the way a person would"
emacs --batch -Q "$issues" --eval "(progn
    (setq org-log-done 'time org-log-into-drawer t)
    (org-mode)
    (goto-char (point-min))
    (re-search-forward \"^\\\\* TODO .*Draft the notes\")
    (org-deadline nil \"2026-09-15\")
    (org-set-tags (list \"docs\" \"urgent\"))
    (org-todo \"DONE\")
    (save-buffer))" >/dev/null 2>&1

echo "6. vissue still reads every issue after that edit"
test "$("$bin" --root "$root" count)" = "2" ||
  fail "an Emacs edit cost the tracker an issue"
"$bin" --root "$root" check >/dev/null || fail "check failed after an Emacs edit"
"$bin" --root "$root" export | grep -q '"docs"' ||
  fail "the tags Emacs wrote did not reach the export"
"$bin" --root "$root" export |
  grep -q '"CLOSED"' || fail "the CLOSED stamp Emacs wrote was dropped"

echo "7. a vissue rewrite keeps what Emacs wrote, and stays lint clean"
"$bin" --root "$root" note "$first_id" "touched by the cli" >/dev/null
grep -q ":docs:urgent:" "$issues" || fail "vissue dropped the Org tags on rewrite"
grep -q "^CLOSED:.*DEADLINE:" "$issues" || fail "vissue dropped the planning line"
lint_clean "$issues" "issues.org after a round trip"

echo "7b. a deed citation is an ordinary property, and survives Emacs"
"$bin" --root "$root" deed "$first_id" --add deed-patch-parser >/dev/null
grep -q ":DEEDS:.*deed-patch-parser" "$issues" || fail "the citation did not land"
lint_clean "$issues" "issues.org with a citation"
# Org is the authority on whether the drawer still reads as a property. Asking
# vissue would only prove vissue can read what vissue wrote.
cited=$(emacs --batch -Q --eval "(progn
    (require 'org)
    (find-file \"$issues\")
    (org-mode)
    (goto-char (point-min))
    (re-search-forward \"$first_id\")
    (princ (or (org-entry-get (point) \"DEEDS\") \"\")))" 2>/dev/null)
test "$cited" = "deed-patch-parser" ||
  fail "org-entry-get read the citation as '$cited'"
# And an Emacs-written one reads back, which is the direction that matters for
# a person naming a product by hand.
emacs --batch -Q --eval "(progn
    (require 'org)
    (find-file \"$issues\")
    (org-mode)
    (goto-char (point-min))
    (re-search-forward \"$first_id\")
    (org-entry-put (point) \"DEEDS\" \"deed-patch-parser deed-file-notes\")
    (save-buffer))" 2>/dev/null
"$bin" --root "$root" recall "$first_id" >/dev/null ||
  fail "vissue could not read a citation Emacs wrote"
"$bin" --root "$root" deed "$first_id" | grep -q "deed-file-notes" ||
  fail "vissue dropped the citation Emacs added"

echo "8. a second rewrite changes nothing"
before=$(cat "$issues")
"$bin" --root "$root" list >/dev/null
test "$before" = "$(cat "$issues")" || fail "a read changed the file"

echo "9. a markdown body stays body, and Org agrees"
# `* ` in the first column opens a headline. A body carrying a bullet list
# would otherwise end the issue there and leave the rest as a heading with no
# :ID:, so vissue indents that line on the way out. Org is the authority on
# whether it worked, which is why this asks Org rather than vissue.
printf 'Findings:\n\n* first bullet\n* second bullet\n\n** nested stays nested\n' \
  > "$work/body.md"
"$bin" --root "$root" create -p demo --quiet --body-file "$work/body.md" \
  "Carries a bullet list" >/dev/null

headings=$(emacs --batch -Q --eval "(progn
    (require 'org)
    (find-file \"$issues\")
    (org-mode)
    (princ (format \"%d\" (length (org-map-entries (lambda () t) \"LEVEL=1\")))))" 2>/dev/null)
test "$headings" = "3" ||
  fail "Org sees $headings top-level headings, not the 3 issues there are"

test "$("$bin" --root "$root" count)" = "3" ||
  fail "a markdown body cost the tracker an issue"
"$bin" --root "$root" check >/dev/null ||
  fail "check failed after a markdown body"
lint_clean "$issues" "issues.org with a markdown body"

# The text survives, and the deeper heading is left as written.
"$bin" --root "$root" export | grep -q "first bullet" ||
  fail "the bullet list did not survive the write"
grep -q "^ \* first bullet" "$issues" ||
  fail "the bullet was not indented out of headline position"
grep -q "^\*\* nested stays nested" "$issues" ||
  fail "a deeper heading was indented needlessly"

echo "vissue org interop: ok"
