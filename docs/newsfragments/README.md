# Towncrier news fragments

A user-visible change lands a fragment here **before** a release is cut. On
`cog bump`, `uvx towncrier build` folds the fragments into `CHANGELOG.md` and
deletes them, so the changelog is assembled from what each change said about
itself rather than reconstructed from the log afterwards.

## Naming

```
<issue-or-slug>.<type>.md
```

`type` is one of `security`, `removed`, `deprecated`, `added`, `changed`,
`fixed`, `dev` — the sections in `towncrier.toml`, in Keep a Changelog order.

Use the issue number when there is one (`42.fixed.md`). Use a `+` prefixed
slug when there is not (`+planning-line.fixed.md`); towncrier treats a leading
`+` as "no issue" and will not try to link it.

## Discipline

- One fragment per user-visible change, not one per commit.
- Write for someone using the tool, not someone maintaining it. What changed
  for them, and what they have to do about it.
- Required when a change touches the command output, the on-disk shape, the
  export schema, the MCP tools, or the configuration.
- Not required for pure documentation, CI, or formatting changes.
- A change that breaks an existing tracker or an existing reader says so in
  the fragment. That sentence is the one people actually need.

## Preview

```console
$ uvx towncrier build --draft --version 0.2.0
```

Nothing is written and no fragment is removed.
