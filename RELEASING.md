# Releasing vissue

`vissue-core`, `vissue-cli`, and `vissue-mcp` share one version and are
published in dependency order. Git tags use `vX.Y.Z` and must match the
workspace version in `Cargo.toml`.

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

A `v*` tag runs it. The workflow packages all three crates first, then
publishes them in dependency order, waiting for `vissue-core` to appear in
the index before the two consumers that name it.

## First public version

Trusted publishing cannot create a crate that does not exist: crates.io
resolves the crate before it will accept a publisher configuration, so the
first version of each crate has to be uploaded with an API token.

That token needs the **`publish-new`** scope. A per-crate token, or one
carrying only `publish-update`, is refused with "this token does not have the
required permissions to perform this action". Tokens are minted on the
crates.io website; the API refuses to create them.

Publish from the tagged commit, so the registry matches the GitHub release:

```console
$ git switch --detach v0.2.0
$ cargo publish --locked -p vissue-core
$ cargo publish --locked -p vissue-cli
$ cargo publish --locked -p vissue-mcp
```

Wait until `vissue-core` is visible in the index before the two consumers.

Then add a trusted publisher to each of the three crates, at
`https://crates.io/crates/<name>/settings`:

- repository owner: `HaoZeke`
- repository name: `vissue`
- workflow filename: `publish-crates.yml`
- environment: `crates-io`

The GitHub environment already exists; recreate it with

```console
$ gh api --method PUT repos/HaoZeke/vissue/environments/crates-io
```

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

`release.yml` builds the platform archives and creates the GitHub release.
`publish-crates.yml` publishes the three crates through OIDC.

To rehearse without uploading, run the workflow by hand: `dry_run` defaults
to true, so it mints the token and packages everything while uploading
nothing.

```console
$ gh workflow run crates.io --ref vX.Y.Z
```

Update `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, and `CITATION.cff` together
for every later version, run the private preparation gate, and push only the
matching signed tag.
