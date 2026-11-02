#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

for path in README.md CHANGELOG.md CONTRIBUTING.md RELEASING.md SECURITY.md CITATION.cff LICENSE; do
  test -s "$path" || {
    echo "missing release document: $path" >&2
    exit 1
  }
done

# The tracker's promise is that an issues.org is an ordinary Org file, so the
# documented Org behaviour is what the release surface pins.
grep -q '^## Documentation$' README.md
grep -q 'vissue.rgoswami.me' README.md

test -s scripts/release-prepare.sh || {
  echo "missing dry release preparation script" >&2
  exit 1
}
test -s .github/workflows/publish-crates.yml || {
  echo "missing crates.io publication workflow" >&2
  exit 1
}
grep -q 'pr-run-mode = "upload"' dist-workspace.toml
grep -q 'aarch64-apple-darwin' dist-workspace.toml
grep -q 'x86_64-unknown-linux-gnu' dist-workspace.toml
grep -q 'cargo-dist-installer.sh' .github/workflows/release.yml
# Publication is trusted publishing: an OIDC identity exchanged for a token
# that lives only as long as the run, with no long-lived registry secret in
# the repository at all.
# Trusted publishing, its action pinned by commit like every action.
grep -Eq 'rust-lang/crates-io-auth-action@[0-9a-f]{40}' .github/workflows/publish-crates.yml
grep -q 'id-token: write' .github/workflows/publish-crates.yml
grep -q 'name: crates-io' .github/workflows/publish-crates.yml
! grep -q 'secrets.CARGO_REGISTRY_TOKEN' .github/workflows/publish-crates.yml
# The upload order comes from the manifests, so a crate added to the
# workspace is released without anyone remembering to list it. Naming the
# crates here instead would put the stale list in the gate meant to catch it.
test -s xtask/src/main.rs
grep -q 'xtask -- publish-order' .github/workflows/publish-crates.yml
grep -q 'xtask -- publish-order' scripts/release-prepare.sh
grep -q 'cargo publish --locked -p "$crate"' .github/workflows/publish-crates.yml
grep -q 'patch.crates-io.$dep.path' scripts/release-prepare.sh
grep -q 'patch.crates-io.$dep.path' .github/workflows/publish-crates.yml
# One tag workflow owns both arms: cargo-dist calls the crates.io
# workflow as a reusable job. The filename stays publish-crates.yml
# because that is what trusted publishing pins.
grep -q 'workflow_call' .github/workflows/publish-crates.yml
grep -q 'inputs:' .github/workflows/publish-crates.yml
grep -q 'plan:' .github/workflows/publish-crates.yml
# A tag must not start a second, independent publish.
if grep -q "tags:" .github/workflows/publish-crates.yml; then
  echo "publish-crates.yml still has a tag trigger; the Release workflow owns tags" >&2
  exit 1
fi
grep -q './publish-crates' dist-workspace.toml
grep -q 'publish-jobs' dist-workspace.toml
grep -q 'github-custom-job-permissions' dist-workspace.toml
grep -q 'contents = "read"' dist-workspace.toml
# musl archives for the CLI and MCP; the HUD stays off musl (iced).
grep -q 'x86_64-unknown-linux-musl' dist-workspace.toml
grep -q 'aarch64-unknown-linux-musl' dist-workspace.toml
grep -q 'x86_64-unknown-linux-musl' crates/vissue-hud/Cargo.toml && {
  echo "vissue-hud must not list a musl target" >&2
  exit 1
}
grep -q '\[package.metadata.dist\]' crates/vissue-hud/Cargo.toml
grep -q 'github-attestations = true' dist-workspace.toml

# Every publishable member has to appear in that order, or the release skips
# it in silence.
declared=$(sed -n 's|^    "crates/\(.*\)",|\1|p' Cargo.toml | sort)
test -n "$declared" || {
  echo "no crates/* workspace members" >&2
  exit 1
}
grep -q 'publish-order' RELEASING.md
grep -q 'dist build --artifacts=local --target="\$host_target"' scripts/release-prepare.sh
! grep -q 'dist build --artifacts=all' scripts/release-prepare.sh

# The bootstrap has to read in the order it must be performed: the crates
# exist before a trusted publisher can name them, and the tag-driven flow
# comes after both.
bootstrap_line=$(grep -n '^## First public version$' RELEASING.md | head -n 1 | cut -d: -f1)
publish_line=$(grep -n 'publish-order' RELEASING.md | tail -n 1 | cut -d: -f1)
tagflow_line=$(grep -n '^## Tag-driven releases$' RELEASING.md | head -n 1 | cut -d: -f1)
test -n "$bootstrap_line"
test -n "$publish_line"
test -n "$tagflow_line"
test "$bootstrap_line" -lt "$publish_line"
test "$publish_line" -lt "$tagflow_line"
# The scope that the bootstrap actually needs, which is what bit us.
grep -q 'publish-new' RELEASING.md

# Public story is a cloneable Org ledger, not a seat vault.
! grep -qi host-tree README.md
! grep -q WorkGraph README.md
! grep -q 'example/host' README.md docs/orgmode/getting-started.org
grep -q -- '--if-gen' README.md
grep -q 'vissue gen' README.md
grep -q 'vissue events' README.md

# Agents are not a second writer. The skill, the CLI docs, and hygiene
# all say so, because a raw issues.org edit races the lock.
grep -q 'never `Write` or `StrReplace`' README.md
grep -q 'never `Write` or `StrReplace`' CONTRIBUTING.md
grep -q 'never =Write=s or' docs/orgmode/reference.org
grep -q 'pre-commit-issues-org.sh' CONTRIBUTING.md
test -s scripts/pre-commit-issues-org.sh
grep -q 'VISSUE_ALLOW_ORG_EDIT' scripts/pre-commit-issues-org.sh

# The documentation site ships as Org sources plus a reproducible build, and
# the generated CLI assets have to exist for a packager to install them.
for path in docs/build.sh docs/orgmode/index.org docs/orgmode/getting-started.org \
    docs/orgmode/howto.org docs/orgmode/harness.org docs/orgmode/reference.org \
    docs/orgmode/explanation.org \
    docs/orgmode/emacs.org docs/orgmode/org-syntax.org docs/orgmode/ecosystem.org \
    docs/source/conf.py man/vissue.1 \
    completions/vissue.bash completions/_vissue completions/vissue.fish; do
  test -s "$path" || {
    echo "missing documentation asset: $path" >&2
    exit 1
  }
done
grep -q 'vissue.rgoswami.me' README.md
grep -q 'vissue.rgoswami.me' docs/source/CNAME
# shields.io/docs.rs/<crate>/badge.svg is a 404. docs.rs serves the badge.
! grep -q 'img.shields.io/docs.rs/' README.md
grep -q 'https://docs.rs/vissue-core/badge.svg' README.md

for manifest in crates/vissue-core/Cargo.toml crates/vissue-cli/Cargo.toml crates/vissue-mcp/Cargo.toml; do
  grep -q '^description = ' "$manifest"
  grep -q '^readme.workspace = true$' "$manifest"
done

# Every surface that names a version has to name the same one. A release
# whose citation metadata or documentation site disagrees with the crates is
# a release that is wrong somewhere. The version is written by hand only in
# Cargo.toml; cogapp generates the other copies, and this is its drift check.
cargo_version=$(python3 scripts/versions.py --print)
test -n "$cargo_version"
if python3 -c 'import cogapp' 2>/dev/null; then
  cogapp=(python3 -m cogapp)
else
  cogapp=(uvx --from cogapp cog)
fi
"${cogapp[@]}" --check --diff --check-fail-msg='run cog -r @cogfiles.txt' @cogfiles.txt
# JSON has nowhere to hold a cog block, so versions.py writes this one.
plugin_version=$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["version"])' \
  .claude-plugin/plugin.json)
if [ "$plugin_version" != "$cargo_version" ]; then
  echo "version mismatch: Cargo.toml is $cargo_version but .claude-plugin/plugin.json is $plugin_version" >&2
  echo "run python3 scripts/versions.py $cargo_version" >&2
  exit 1
fi
# A pin added below the generated block is one cog never moves.
stale=$(grep -E '^vissue-[a-z-]+ = \{ version = ' Cargo.toml | grep -v "\"$cargo_version\"" || true)
if [ -n "$stale" ]; then
  echo "internal pins at another version than $cargo_version:" >&2
  echo "$stale" >&2
  exit 1
fi
# The lockfile records each member's version too; this is the question
# `--locked` asks in CI, answered before a tag rather than after.
cargo metadata --quiet --locked --format-version 1 >/dev/null || {
  echo "Cargo.lock does not match the manifests: cargo update --workspace" >&2
  exit 1
}

# Towncrier owns the changelog from its marker down.
grep -q '<!-- towncrier release notes start -->' CHANGELOG.md
test -d docs/newsfragments

echo "vissue release surface: ok (version $cargo_version)"
