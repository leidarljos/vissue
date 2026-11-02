"""The one hand-written version, and every copy generated from it.

The version is written by hand in exactly one place: `[workspace.package]` in
the root Cargo.toml. The crates inherit it, the CLI reads it at build time,
and the Sphinx config reads it through this module. Every other file that has
to spell it out carries a cogapp block calling into this module, listed in
cogfiles.txt, so

    cog -r @cogfiles.txt        rewrites them from the manifest
    cog --check @cogfiles.txt   fails when one has drifted

The Claude plugin manifest is JSON, which has nowhere to put a cog block, so
this module writes its version field directly.

Run as a script, `versions.py X.Y.Z` moves the hand-written line and the
plugin manifest with it. That is the only edit a bump makes outside cog.
`versions.py --print` prints the current version for shell callers.
"""

from __future__ import annotations

import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "Cargo.toml"
PLUGIN = ROOT / ".claude-plugin" / "plugin.json"

_SEMVER = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")
_JSON_VERSION = re.compile(r'"version"\s*:\s*("(?:[^"\\]|\\.)*")')


def version() -> str:
    with MANIFEST.open("rb") as f:
        return tomllib.load(f)["workspace"]["package"]["version"]


def minor() -> str:
    return version().rpartition(".")[0]


def pins(*crates: str) -> str:
    """`[workspace.dependencies]` lines for the workspace's own crates.

    The published crates depend on each other by version as well as by path,
    so each pin has to move with the workspace or the lockfile stops resolving.
    """
    v = version()
    return "".join(
        f'{crate} = {{ version = "{v}", path = "crates/{crate}" }}\n'
        for crate in crates
    )


def set_version(new: str) -> None:
    if not _SEMVER.match(new):
        sys.exit(f"not a semantic version: {new}")
    text = MANIFEST.read_text()
    section = re.search(r"(?ms)^\[workspace\.package\]\n.*?(?=^\[|\Z)", text)
    if section is None:
        sys.exit("Cargo.toml has no [workspace.package] table")
    body, count = re.subn(
        r'(?m)^version = "[^"]*"$', f'version = "{new}"', section.group(0), count=1
    )
    if count != 1:
        sys.exit("[workspace.package] has no version line")
    MANIFEST.write_text(text[: section.start()] + body + text[section.end() :])
    if version() != new:
        sys.exit(f"Cargo.toml still reads {version()} after writing {new}")


def set_plugin_version() -> None:
    """Set the plugin manifest's top-level `version` to the workspace version.

    Only the value's characters change, so the file keeps its hand-chosen
    layout (inline arrays, key order); the result is checked by parsing it
    back against the loaded manifest with the version set.
    """
    text = PLUGIN.read_text()
    want = json.loads(text)
    if "version" not in want:
        sys.exit(f"{PLUGIN.relative_to(ROOT)} has no version field")
    want["version"] = version()
    for m in _JSON_VERSION.finditer(text):
        new = text[: m.start(1)] + json.dumps(want["version"]) + text[m.end(1) :]
        if json.loads(new) == want:
            PLUGIN.write_text(new)
            return
    sys.exit(f"could not set the version in {PLUGIN.relative_to(ROOT)}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("usage: versions.py X.Y.Z | --print")
    if sys.argv[1] == "--print":
        print(version())
    else:
        set_version(sys.argv[1])
        set_plugin_version()
