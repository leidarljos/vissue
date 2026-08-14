#!/usr/bin/env bash
# Export docs/orgmode/*.org → docs/source/*.rst
# Prefer pandoc (available on many hosts); optional: VISSUE_DOC_EXPORTER=emacs
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
ORG_DIR="docs/orgmode"
OUT_DIR="docs/source"
mkdir -p "$OUT_DIR"

exporter="${VISSUE_DOC_EXPORTER:-auto}"
if [[ "$exporter" == "auto" ]]; then
  if command -v pandoc >/dev/null 2>&1; then
    exporter=pandoc
  elif command -v emacs >/dev/null 2>&1; then
    exporter=emacs
  else
    echo "error: need pandoc or emacs to export org → rst" >&2
    exit 1
  fi
fi

echo "export_org_to_rst: using $exporter"
if [[ "$exporter" == "emacs" ]]; then
  emacs --batch -l docs/export.el
else
  shopt -s nullglob
  files=("$ORG_DIR"/*.org)
  if ((${#files[@]} == 0)); then
    echo "error: no org files in $ORG_DIR" >&2
    exit 1
  fi
  for org in "${files[@]}"; do
    base="$(basename "$org" .org)"
    pandoc -f org -t rst --wrap=preserve -o "$OUT_DIR/${base}.rst" "$org"
    echo "  wrote $OUT_DIR/${base}.rst"
  done
fi

python3 docs/scripts/fix_doc_links.py

# Restore the toctree pandoc drops, and refuse an org line that would turn
# into an RST footnote and eat the sentence around it.
python3 docs/scripts/finish_export.py
