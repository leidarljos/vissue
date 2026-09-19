# Releasing vissue

Every crate in the workspace shares one version and is published in
dependency order. Git tags use `vX.Y.Z` and must match the workspace version
in `Cargo.toml`.

The order is not written down anywhere by hand: `xtask publish-order`
reads it from `cargo metadata`, so adding a crate is enough. Run it to see
what a release will do.

```console
$ cargo run -q -p xtask -- publish-order
```

## Cutting a version

`cog bump` drives it. The pre-bump hooks move every version surface together,
fold the news fragments into `CHANGELOG.md`, and run the release-surface gate,
so a bump that would produce an inconsistent tree fails before the tag exists.

```console
$ cog bump --minor          # or --patch / --major / --version X.Y.Z
```

That leaves a version commit and a `vX.Y.Z` tag. Review both before pushing;
the tag is what the release workflows key on.

To move the surfaces without bumping, `scripts/sync-version.sh X.Y.Z`. To see
the release notes without consuming the fragments, `uvx towncrier build
--draft --version X.Y.Z`.

## Private preparation

Run the complete gate on a supported build host before creating a tag:

```console
$ scripts/release-prepare.sh
```

The gate runs formatting, tests, clippy, release-surface validation,
`cargo-dist` generation validation, registry dry-runs, and a host archive
build. It does not create a tag, GitHub release, or registry version.

## How publication works

There is no registry secret in this repository. `publish-crates.yml` mints a
crates.io token from the job's own OIDC identity, and that token lives only
for the length of the run. crates.io accepts it because the registry side
pins this repository, this workflow file, and the `crates-io` environment; a
mismatch fails there rather than falling back to a long-lived credential.

A `v*` tag runs `release.yml`. That workflow builds the platform archives,
attests them, creates the GitHub release, and then calls
`publish-crates.yml` as a reusable workflow. crates.io reads the
**caller** workflow filename from the GitHub OIDC JWT, so a tag publish
presents as `release.yml`. A `workflow_dispatch` of the crates workflow
presents as `publish-crates.yml`. Each crate therefore needs both
filenames pinned (same repository, same `crates-io` environment). The
crates workflow has no tag trigger of its own, so a tag cannot start two
publishes.

## First public version

The workspace crates exist on crates.io from 0.3.0. Trusted publishing
pins repository `leidarljos/vissue` (not the HaoZeke fork), environment
`crates-io`, and both workflow filenames `release.yml` and
`publish-crates.yml`. Later `v*` tags publish through OIDC. The GitHub
environment already exists; recreate it with

```console
$ gh api --method PUT repos/leidarljos/vissue/environments/crates-io
```

Trusted publishing cannot create a crate that does not exist. If the
workspace grows a crate that has never been uploaded, that first version
still needs an API token with the **`publish-new`** scope. A per-crate
token, or one carrying only `publish-update`, is refused. Tokens are
minted on the crates.io website; the API refuses to create them.

Publish from the tagged commit, so the registry matches the GitHub release.
crates.io rate-limits new crate creation; a 429 names the retry time.
Wait for each crate to appear on the registry API before publishing the
next one that names it. `cargo info` from the workspace reads the local
path and is not that signal.

```console
$ git switch --detach vX.Y.Z
$ version=$(sed -n '0,/^version = /s/^version = "\([^"]*\)"/\1/p' Cargo.toml)
$ for crate in $(cargo run -q -p xtask -- publish-order); do
    cargo publish --locked -p "$crate"
    until curl -fsS -A 'vissue-publish' \
      "https://crates.io/api/v1/crates/${crate}/${version}" >/dev/null
    do sleep 10; done
  done
```

Then add trusted publishers to the new crate, at
`https://crates.io/crates/<name>/settings`, or via

`POST /api/v1/trusted_publishing/github_configs` with
`crate`, `repository_owner=leidarljos`, `repository_name=vissue`,
`environment=crates-io`, once for `workflow_filename=release.yml` and
once for `workflow_filename=publish-crates.yml`.

After that the token is never needed again, and it should be revoked.

## Tag-driven releases

`cog bump` moves every version surface, folds the news fragments into the
changelog, runs the release gate, and makes the signed tag. Pushing that tag
is what publishes:

```console
$ cog bump --minor          # or --patch / --major / --version X.Y.Z
$ git push origin main
$ git push origin vX.Y.Z
```

`release.yml` builds the archives (including musl for `vissue-cli` and
`vissue-mcp`; the HUD stays on glibc because iced links native graphics
stacks), attests them, creates the GitHub release, and calls
`publish-crates.yml` to publish every crate through OIDC.

There is no Homebrew tap. `GITHUB_TOKEN` cannot push a formula to another
repository, and `HaoZeke/homebrew-tap` does not exist.

To rehearse crates.io without uploading, dispatch that workflow by hand:
`dry_run` defaults to true, so it mints the token and packages everything
while uploading nothing.

```console
$ gh workflow run crates.io --ref vX.Y.Z
```

Update `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, and `CITATION.cff` together
for every later version, run the private preparation gate, and push only the
matching signed tag.
