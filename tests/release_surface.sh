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
grep -q 'crates-io-auth-action@v1' .github/workflows/publish-crates.yml
grep -Fq "vars.PUBLIC_RELEASE_ENABLED == 'true'" .github/workflows/publish-crates.yml
grep -q 'cargo publish --locked -p vissue-core' .github/workflows/publish-crates.yml
grep -q 'cargo publish --locked -p vissue-cli' .github/workflows/publish-crates.yml
grep -q 'cargo publish --locked -p vissue-mcp' .github/workflows/publish-crates.yml
grep -q 'patch.crates-io.vissue-core.path' scripts/release-prepare.sh
grep -q 'patch.crates-io.vissue-core.path' .github/workflows/publish-crates.yml
grep -q 'dist build --artifacts=local --target="\$host_target"' scripts/release-prepare.sh
! grep -q 'dist build --artifacts=all' scripts/release-prepare.sh

tag_create_line=$(grep -n '^\$ git tag -s v' RELEASING.md | head -n 1 | cut -d: -f1)
publish_line=$(grep -n '^\$ cargo publish --locked -p vissue-core' RELEASING.md | head -n 1 | cut -d: -f1)
tag_line=$(grep -n '^\$ git push origin v' RELEASING.md | head -n 1 | cut -d: -f1)
arm_line=$(grep -n '^\$ gh variable set PUBLIC_RELEASE_ENABLED' RELEASING.md | head -n 1 | cut -d: -f1)
test -n "$tag_create_line"
test -n "$publish_line"
test -n "$tag_line"
test -n "$arm_line"
test "$tag_create_line" -lt "$publish_line"
test "$publish_line" -lt "$tag_line"
test "$tag_line" -lt "$arm_line"

# The documentation site ships as Org sources plus a reproducible build, and
# the generated CLI assets have to exist for a packager to install them.
for path in docs/build.sh docs/orgmode/index.org docs/orgmode/getting-started.org \
    docs/orgmode/howto.org docs/orgmode/reference.org docs/orgmode/explanation.org \
    docs/orgmode/emacs.org docs/source/conf.py man/vissue.1 \
    completions/vissue.bash completions/_vissue completions/vissue.fish; do
  test -s "$path" || {
    echo "missing documentation asset: $path" >&2
    exit 1
  }
done
grep -q 'vissue.rgoswami.me' README.md
grep -q 'vissue.rgoswami.me' docs/source/CNAME

for manifest in crates/vissue-core/Cargo.toml crates/vissue-cli/Cargo.toml crates/vissue-mcp/Cargo.toml; do
  grep -q '^description = ' "$manifest"
  grep -q '^readme.workspace = true$' "$manifest"
done

# Every surface that names a version has to name the same one. A release
# whose citation metadata or documentation site disagrees with the crates is
# a release that is wrong somewhere.
cargo_version=$(sed -n '0,/^version = /s/^version = "\([^"]*\)"/\1/p' Cargo.toml)
test -n "$cargo_version"
citation_version=$(sed -n 's/^version: *//p' CITATION.cff | head -n 1)
docs_release=$(sed -n 's/^release = "\([^"]*\)"/\1/p' docs/source/conf.py | head -n 1)
news_version=$(sed -n '0,/^version = /s/^version = "\([^"]*\)"/\1/p' towncrier.toml)
for pair in "CITATION.cff:$citation_version" "docs/source/conf.py:$docs_release" \
    "towncrier.toml:$news_version"; do
  name=${pair%%:*}
  value=${pair#*:}
  if [ "$value" != "$cargo_version" ]; then
    echo "version mismatch: Cargo.toml is $cargo_version but $name is $value" >&2
    echo "run scripts/sync-version.sh $cargo_version" >&2
    exit 1
  fi
done

# Towncrier owns the changelog from its marker down.
grep -q '<!-- towncrier release notes start -->' CHANGELOG.md
test -d docs/newsfragments

echo "vissue release surface: ok (version $cargo_version)"
