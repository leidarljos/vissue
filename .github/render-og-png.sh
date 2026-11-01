#!/usr/bin/env bash
# Draw og.png from docs/source/_static/og.svg at 1200 by 630.
# Pin: resvg 0.48.1
# https://github.com/linebender/resvg/releases/download/v0.48.1/resvg-linux-x86_64.tar.gz
# sha256 fa8c26495a187e592c501db15bf9e8a9fdc051d4b2b336b39703d5b59f912b9d
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
version="0.48.1"
url="https://github.com/linebender/resvg/releases/download/v${version}/resvg-linux-x86_64.tar.gz"
sha="fa8c26495a187e592c501db15bf9e8a9fdc051d4b2b336b39703d5b59f912b9d"
cache="${XDG_CACHE_HOME:-${HOME}/.cache}/resvg-${version}"
mkdir -p "${cache}"
if [[ ! -x "${cache}/resvg" ]]; then
  tmp="$(mktemp)"
  curl -fsSL -o "${tmp}" "${url}"
  echo "${sha}  ${tmp}" | sha256sum -c -
  tar -C "${cache}" -xzf "${tmp}"
  rm -f "${tmp}"
fi
svg="${root}/docs/source/_static/og.svg"
out="${1:-${root}/docs/source/_static/og.png}"
mkdir -p "$(dirname "${out}")"
"${cache}/resvg" --version
"${cache}/resvg" -w 1200 -h 630 \
  --serif-family Georgia \
  "${svg}" "${out}"
