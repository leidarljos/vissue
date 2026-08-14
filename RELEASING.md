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

The crates.io publication job is dormant unless the repository variable
`PUBLIC_RELEASE_ENABLED` is exactly `true`. Keep that variable absent during
private preparation.

## First public version

crates.io requires the first version of a new crate to be published with an
API token before GitHub OIDC trusted publishing can be configured. When public
release is explicitly authorized, create the signed tag locally from the
verified commit and keep it unpushed while the registry bootstrap runs:

```console
$ git tag -s v0.1.0 -m 'vissue 0.1.0'
$ git switch --detach v0.1.0
```

Publish the packages from that tag checkout in this order:

```console
$ cargo publish --locked -p vissue-core
$ cargo search vissue-core --limit 1
$ cargo publish --locked -p vissue-cli
$ cargo publish --locked -p vissue-mcp
```

Wait until `vissue-core` is visible in the registry index before publishing
the two consumers.

For each crate, add a crates.io trusted publisher with:

- repository: `HaoZeke/vissue`
- workflow: `publish-crates.yml`
- environment: `crates-io`

Create the matching GitHub environment without adding a long-lived registry
secret. Keep `PUBLIC_RELEASE_ENABLED` absent through the first GitHub release:

```console
$ gh api --method PUT repos/HaoZeke/vissue/environments/crates-io
```

## Tag-driven releases

After the first-version bootstrap and trusted-publisher configuration, push the
matching signed tag while public publication remains disarmed:

```console
$ git push origin v0.1.0
```

`release.yml` builds platform archives and creates the GitHub release. The
dormant `publish-crates.yml` job skips this manually bootstrapped version.
After the first GitHub release exists, arm OIDC publication for later versions:

```console
$ gh variable set PUBLIC_RELEASE_ENABLED --repo HaoZeke/vissue --body true
```

`publish-crates.yml` publishes later matching registry versions through OIDC.
Update `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, and `CITATION.cff` together
for every later version, run the private preparation gate, and push only the
matching signed tag.
