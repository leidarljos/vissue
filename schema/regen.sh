#!/usr/bin/env bash
# Regenerate crates/vissue-core/src/schema/vissue_capnp.rs from vissue.capnp.
#
# One command where both tools are present, which is the normal case. Where they
# are not, it says which is missing and what to do instead, because the two halves
# can live on different machines and the request file moves between them.
#
# Nothing in the build runs this. The generated file is committed, and
# `the_generated_constant_matches_the_schema_text` fails if it is stale, so
# forgetting to run this is caught by the suite rather than shipped.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
out="$here/../crates/vissue-core/src/schema/vissue_capnp.rs"

have() { command -v "$1" >/dev/null 2>&1; }

if ! have capnp; then
  echo "capnp is not on PATH: it compiles the schema." >&2
  echo "Install Cap'n Proto, or run this on a machine that has it." >&2
  exit 1
fi

# The plugin writes code against a particular version of the `capnp` runtime crate,
# and nothing in the generated file records which. A newer plugin against an older
# runtime is the quiet version of the drift this schema exists to prevent, so refuse
# rather than emit a file whose failure mode is a compile error three commits later.
pin="$(sed -n 's/^capnp = "\(.*\)"/\1/p' "$here/../crates/vissue-core/Cargo.toml")"
if have cargo && [ -n "$pin" ]; then
  plugin="$(cargo install --list 2>/dev/null | sed -n 's/^capnpc v\([0-9][0-9.]*\).*/\1/p' | head -1)"
  if [ -z "$plugin" ]; then
    echo "note: capnpc-rust is not a cargo install, so its version is unchecked against the" >&2
    echo "      capnp = \"$pin\" runtime that the generated file has to compile against." >&2
  elif [ "${plugin%.*}" != "$pin" ]; then
    echo "capnpc-rust is $plugin and crates/vissue-core depends on capnp = \"$pin\"." >&2
    echo "Generating with a mismatched plugin writes code for the wrong runtime." >&2
    echo "  cargo install capnpc --version $pin" >&2
    exit 1
  fi
fi

if have capnpc-rust; then
  # Into a scratch directory, and then found rather than assumed. The plugin lays
  # its output out by the source path recorded in the request, so it can land in a
  # `schema/` subdirectory rather than flat, and a `mv` of a guessed name fails on
  # whichever of the two it did not guess.
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  capnp compile -orust:"$tmp" "$here/vissue.capnp"
  generated="$(find "$tmp" -name 'vissue_capnp.rs' -print -quit)"
  if [ -z "$generated" ]; then
    echo "the plugin wrote no vissue_capnp.rs under $tmp" >&2
    exit 1
  fi
  mv "$generated" "$out"
  echo "regenerated $out"
  exit 0
fi

# Only the compiler is here. The request is the whole input the plugin needs, so it
# can run on another machine and send the Rust back.
request="${TMPDIR:-/tmp}/vissue-capnp-request.bin"
capnp compile -o- "$here/vissue.capnp" > "$request"

# Name a host in VISSUE_CAPNPC_SSH and the round trip is this one command. The two
# tools land on different machines often enough -- the compiler is a distro package
# and the plugin is a cargo install -- that doing it by hand is the common case
# rather than the exception.
if [ -n "${VISSUE_CAPNPC_SSH:-}" ]; then
  remote="${VISSUE_CAPNPC_SSH}"
  echo "running the plugin on $remote" >&2
  scp -q "$request" "$remote:/tmp/vissue-capnp-request.bin"
  # shellcheck disable=SC2029  # the remote script is meant to expand there
  ssh "$remote" 'set -eu; . "$HOME/.cargo/env" 2>/dev/null || true
    d="$(mktemp -d)"; cd "$d"
    capnpc-rust < /tmp/vissue-capnp-request.bin
    f="$(find . -name vissue_capnp.rs -print -quit)"
    [ -n "$f" ] || { echo "the plugin wrote no vissue_capnp.rs" >&2; exit 1; }
    cat "$f"' > "$out.new"
  if [ ! -s "$out.new" ]; then
    rm -f "$out.new"
    echo "the remote plugin returned nothing" >&2
    exit 1
  fi
  mv "$out.new" "$out"
  echo "regenerated $out via $remote"
  exit 0
fi

cat >&2 <<EOF
capnpc-rust is not on PATH: it turns the schema into Rust.

Wrote the compiler's request to:
  $request

On a machine with the plugin:
  capnpc-rust < $request
and put the resulting schema/vissue_capnp.rs at:
  crates/vissue-core/src/schema/vissue_capnp.rs

Or install the plugin here with: cargo install capnpc
EOF
exit 1
