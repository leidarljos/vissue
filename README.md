# vissue

<p align="center">
  <img src="assets/logo.svg" width="176" alt="vissue mark: a ready teal node with coral edges to two waiting issues">
</p>

[![CI](https://github.com/HaoZeke/vissue/actions/workflows/ci_test.yml/badge.svg)](https://github.com/HaoZeke/vissue/actions/workflows/ci_test.yml)
[![crates.io](https://img.shields.io/crates/v/vissue-cli.svg)](https://crates.io/crates/vissue-cli)
[![docs.rs](https://img.shields.io/docs.rs/vissue-core/badge.svg)](https://docs.rs/vissue-core)
[![MSRV](https://img.shields.io/badge/MSRV-1.88-blue.svg)](https://www.rust-lang.org/)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A plan is a directed acyclic graph of org headings. One file per project
stores the nodes. `:PARENT:` groups work under a plan. `:BLOCKED_BY:` is
the partial order. `ready` is the frontier any agent can pick up. `claim`
is the lock so two agents do not take the same node.

An issue is a top-level org heading. The file is the database: no daemon, no
SQLite, no server. Every command parses the files it needs and every mutation
rewrites one file under a lock, so the tracker diffs, merges, and greps like the
rest of the repository it lives in. A CLI and a Model Context Protocol server
share the same library.

The store is an Org file [1]. The graph has several justifications, and
they are not interchangeable. `:PARENT:` plus `:TYPE:` and tags
store the output of hierarchical task-network decomposition [3], [4]:
the agent is the planner, the file is the network. `:BLOCKED_BY:` is a
least-commitment partial order [2]. `ready` and `claim` are a
work-stealing ready deque [9] over that order, which is also how a
rebuild DAG exposes dirty sources [8]. `related` is a named
neighborhood over declared edges, the same discipline as a citation
graph [12], [14], [17] and the opposite of an extracted memory graph
[24], [25], [26]. OokCite is how those papers were found and checked,
not a second reading list to copy.

```
<root>/Software/<project>/issues.org
```

`Software` is the default prefix and is configurable. `<root>` comes from
`--root`, the `VISSUE_ROOT` environment variable, or the current directory.

## A plan on the board

A feature becomes one parent issue and five tagged children. This is the
board an orchestrator shows after that split. Only the catalog is
workable. Everything else waits on an explicit blocker, so two agents
cannot start the terminal UI and the example file at the same time.

| Id | State | What |
|---|---|---|
| `keys-e0pl` | TODO | Epic: Colemak leader sequence |
| `keys-cata` | TODO, ready | Catalog of bindable actions |
| `keys-toml` | BLOCKED on catalog | `keys.toml` schema and key names |
| `keys-tuih` | BLOCKED on overlay | Terminal UI `set_keymap` and overlay `on_key` |
| `keys-lead` | BLOCKED on wire | Leader plus `examples/keys/colemak.toml` |

```mermaid
flowchart LR
    epic["keys-e0pl epic"]
    catalog["keys-cata catalog"]
    schema["keys-toml keys.toml"]
    overlay["keys-ovly overlay"]
    wire["keys-wire wire"]
    tui["keys-tuih terminal UI"]
    example["keys-lead colemak example"]
    epic --> catalog
    epic --> schema
    epic --> overlay
    epic --> wire
    epic --> tui
    epic --> example
    catalog -->|"blocks"| schema
    schema -->|"blocks"| overlay
    overlay -->|"blocks"| tui
    overlay -->|"blocks"| wire
    wire -->|"blocks"| example
```

Solid parent edges are containment. The `blocks` edges are `:BLOCKED_BY:`
and are what `ready` reads. A topological sort is the order the graph
would fire in if every node ran; `ready` is the cheaper question of
which sources are still open. Adding a blocker that would close a
cycle is rejected.

Build that board from an empty directory:

```console
$ mkdir -p /tmp/keys && cd /tmp/keys
$ vissue create --project keys --type plan "Epic: Colemak leader sequence"
keys-e0pl  TODO  [#C]  Epic: Colemak leader sequence
$ vissue create --project keys --type task --parent keys-e0pl --tags catalog \
    "Catalog of bindable actions"
keys-cata  TODO  [#C]  Catalog of bindable actions
$ vissue create --project keys --type task --parent keys-e0pl --tags config \
    "keys.toml schema and key names"
keys-toml  TODO  [#C]  keys.toml schema and key names
$ vissue update keys-toml --block keys-cata
keys-toml: state TODO -> BLOCKED (auto on block), blocked_by += keys-cata
```

Repeat for the overlay, the terminal UI, the wire, and the example file, each
`--parent keys-e0pl` and each `--block` on the node it cannot start
without. Then:

```console
$ vissue ready --project keys
keys-cata              TODO      [#C]  Catalog of bindable actions
$ vissue tree keys-e0pl
keys-e0pl TODO      [#C]  Epic: Colemak leader sequence
  keys-cata TODO      [#C]  Catalog of bindable actions
  keys-toml BLOCKED   [#C]  keys.toml schema and key names
    * blocked-by keys-cata
```

Two agents share that frontier by claiming, not by editing a checklist.
A common pairing is one implementer and one reviewer per node: the
reviewer is a child issue blocked on the implementation issue, so it
becomes ready only when the implementer closes. Identities are opaque
strings; set `VISSUE_AGENT` to something stable.

```console
$ VISSUE_AGENT=impl vissue claim keys-cata
claimed keys-cata by impl (TODO -> STARTED)
$ vissue create --project keys --type task --parent keys-cata --tags review \
    "Review the bindable-action catalog"
$ vissue update keys-revw --block keys-cata
$ VISSUE_AGENT=review  vissue ready --project keys
# empty: the only open work is claimed or blocked
$ vissue claims --project keys
keys-cata              STARTED   [#C]    0d  impl  Catalog of bindable actions (keys)
```

The tracker does not invent the children. An agent splits the plan
[2], [3], [4]; vissue stores the resulting directed graph, names the
ready set [8], [9], and refuses a cyclic edit [5], [6].

`related` asks what else in the corpus is a neighbor, and why.
Explicit `:PARENT:`, `:BLOCKED_BY:`, `:DISCOVERED_FROM:`, and Org body
links outrank shared tags and rare terms [22]. The command prints the
evidence (`blocked_by`, `org_link`, `term:keymap`) and writes nothing
back [24], [25], [26].

## Install

```console
$ cargo install vissue-cli
```

The MCP server is a second binary in the same workspace:

```console
$ cargo install vissue-mcp
```

Both crates track one version. To build the unreleased `main` instead, name
the repository:

```console
$ cargo install --git https://github.com/HaoZeke/vissue vissue-cli
$ cargo install --git https://github.com/HaoZeke/vissue vissue-mcp
```

Tagged releases also carry prebuilt archives and a shell installer; see the
[releases page](https://github.com/HaoZeke/vissue/releases).

## Tutorial: from an empty directory to a tracked backlog

Everything below runs in a scratch directory and touches nothing else.

**1. Make a tracker and create the first issue.**

```console
$ mkdir -p /tmp/demo && cd /tmp/demo
$ vissue create --project parser "Reject a manifest with no header"
parser-k29f  TODO  [#C]  Reject a manifest with no header
file: /tmp/demo/Software/parser/issues.org
```

The file now exists, with a preamble and one heading:

```console
$ cat Software/parser/issues.org
#+TITLE: parser issues
#+FILETAGS: :issues:parser:
#+DATE: [2026-08-03 Mon]
#+DESCRIPTION: Issue tracking file for parser specs, plans, and implementation tasks.
#+STATUS: Active
#+TODO: TODO STARTED BLOCKED | DONE CANCELLED

* TODO [#C] Reject a manifest with no header
:PROPERTIES:
:ID:         parser-k29f
:CREATED:    [2026-08-03 Mon]
:END:
```

**2. Add a second issue that depends on the first.**

```console
$ vissue create --project parser --priority A \
    --body "Scope: the error message quoted in the release notes." \
    "Publish the release notes"
parser-3xq7  TODO  [#A]  Publish the release notes
file: /tmp/demo/Software/parser/issues.org

$ vissue update parser-3xq7 --block parser-k29f
parser-3xq7: state TODO -> BLOCKED (auto on block), blocked_by += parser-k29f
```

Adding a blocker moved the issue to BLOCKED on its own, and the transition is in
the logbook.

**3. Ask what is actually workable.**

```console
$ vissue ready
parser-k29f            TODO      [#C]  Reject a manifest with no header
```

The blocked issue is gone from the list, which is the whole point of `ready`.

**4. Work the issue and close it.**

```console
$ vissue claim parser-k29f
claimed parser-k29f by you@yourhost (TODO -> STARTED)
ID:       parser-k29f
...
$ vissue update parser-k29f --state DONE
parser-k29f: state STARTED -> DONE, claim released (you@yourhost)
[hint] parser-3xq7 (in parser) lists this as a blocker; clear with `vissue update parser-3xq7 --unblock parser-k29f`
```

Closing a blocker gives up the claim and names every issue still waiting on
it. The hint goes to stderr, so a pipeline reading stdout is unaffected.

**5. Clear the edge and see the backlog open up.**

```console
$ vissue update parser-3xq7 --unblock parser-k29f
parser-3xq7: state BLOCKED -> TODO (auto on unblock), blocked_by -= parser-k29f
$ vissue ready
parser-3xq7            TODO      [#A]  Publish the release notes
```

**6. Share the backlog with someone who cannot read your tracker.**

```console
$ vissue mirror --project parser --out /tmp/demo/backlog-mirror.org
wrote /tmp/demo/backlog-mirror.org
```

That file is a read-only projection with a banner saying so. Regenerate it after
changes; it is output, not a second source of truth.

## How to

**Filter and count.** `--project` (also `-p` or `-P`) and `--state` apply to
`list`, `count`, `ready`, `graph`, `roadmap`, and `export`. A project name
matches the directory on disk without regard to case, so `-p Parser` and
`-p parser` select the same tracker whichever verb reads it.

```console
$ vissue list --project parser --state TODO
$ vissue count --ready
```

**Feed another tool.** `export` writes one JSON object per line, carrying every
property, the logbook, the body, and the file line range.

```console
$ vissue export --project parser | jq -r '.id + " " + .state'
```

**Follow the graph.** `tree` walks children and blockers below an id; `graph`
emits the whole thing as Graphviz DOT; `backlinks` finds everything pointing at
an id; `cycles` reports a blocker loop; `ancestors` and `impact` bound the walk
by hop depth.

```console
$ vissue tree parser-3xq7
$ vissue graph --project parser | dot -Tsvg > backlog.svg
$ vissue ancestors parser-3xq7 --depth 3
$ vissue impact parser-k29f --depth 3
```

`related` provides a bounded, derived neighborhood for an issue. Explicit
`:PARENT:`, `:BLOCKED_BY:`, `:DISCOVERED_FROM:`, and Org body links rank above
shared tags and rare terms. The result names its evidence and does not write
inferred links into the source files. Use `--format org` for pasteable Org
links back to the headings:

```console
$ vissue related parser-k29f --depth 2 --limit 20 --format org
```

**Keep the corpus honest.** `check` validates every parent and blocker edge,
every date, and the uniqueness of ids, and exits non-zero on an error.
`hygiene` adds the claims that are not actually workable.

```console
$ vissue check
$ vissue hygiene
```

**Tell whether a copy of the backlog is current.** `digest` hashes the corpus,
combined and per project, so a consumer can compare two points in time without
reading every issue:

```console
$ vissue digest -P atlas -P beacon
combined=7f91ad67512010d0 issues=109 generation=3167 projects=2
6cdab6af46e1c979      12  atlas
671d99c6181c1494      97  beacon
$ vissue digest -P atlas --quiet
d2ee07c7f585330b
```

The per-project lines are the point: a changed combined digest says something
moved, and the sub-digests say which project. The hash is xxh3 over the JSONL
export, so it tracks issue content and ignores formatting that changes nothing.
`--json` gives the same data as an object.

Every mirror carries that digest in its header as a SYNC stamp:

```
# SYNC: digest=d2ee07c7f585330b generation=3167 issues=12 at=2026-08-03T09:53 projects=atlas:6cdab6af46e1c979
```

Which makes freshness a single command, exiting 0 when current and 1 when not:

```console
$ vissue mirror --check Software/atlas/issues-mirror.org
fresh: digest=d2ee07c7f585330b issues=12 generation=3167 (stamped 2026-08-03T09:53)

$ vissue mirror --check stale-copy.org
stale: stale-copy.org
  stamped digest=0000000000000000 at=2026-08-03T09:53 issues=12
  current digest=d2ee07c7f585330b issues=12 generation=3167
  moved: atlas 1111111111111111 -> 6cdab6af46e1c979
```

The check reads the projects from the stamp, so a caller need not repeat them,
and a mirror with no stamp reads as stale. This is what lets a collaborator who
cannot reach the tracker still know whether the copy they hold is behind: run
the check where the tracker lives and share the verdict, or compare stamps
across pulls on their side.

**See who is holding what.** Claiming an issue stamps an identity and a
timestamp onto it, so a backlog worked by several agents shows who took what
and when:

```console
$ vissue whoami
rgoswami@workstation
$ VISSUE_AGENT=grind-worker-3 vissue claim parser-k29f
claimed parser-k29f by grind-worker-3 (TODO -> STARTED)
$ vissue list --state STARTED
parser-k29f            STARTED   [#C]  Reject a manifest...  (claimed 0d by grind-worker-3)
```

The identity comes from `VISSUE_AGENT`, then `agent` in `vissue.toml`, then
`user@host`. It is an opaque string: an agent should set `VISSUE_AGENT` to
something stable enough to recognise across sessions, such as a model and
session tag.

Moving to STARTED by any route takes the claim if no one holds it, so
`vissue update <id> --state STARTED` stamps it too. BLOCKED keeps the claim,
because the holder is still on the issue. Returning to TODO, or closing as
DONE or CANCELLED, gives it up and writes a logbook note naming who held it
and since when, so the history outlives the properties. A claim held by
another identity is refused unless you pass `--force`, which records the
takeover rather than losing it.

`claims` is the standing answer to "who is on what": every live claim, oldest
first, with the holder and the age in days. `--by` and `--project` narrow it,
and `--json` emits the rows as an array for an orchestrator polling agent
state:

```console
$ vissue claims
parser-k29f            STARTED   [#C]    0d  grind-worker-3  Reject a manifest... (parser)
$ vissue claims --by grind-worker-3 --json | jq -r '.[0].claimed_at'
[2026-08-03 Mon 12:52]
```

`hygiene` reports claims held longer than `stale_claim_days` (default 7,
settable in `vissue.toml` or per run with `--stale-days`), and STARTED issues
that nobody has claimed.

**Report progress without taking over.** `note` adds a dated entry to an
issue's logbook and touches nothing else, so an agent can leave a trail on an
issue someone else holds:

```console
$ vissue note parser-k29f "grammar table regenerated; fuzz corpus next"
parser-k29f: noted
```

**Fold discovered work in from outside the tracker.** An agent without write
access to the tracker appends plain `* TODO <title>` headings (body prose
below) to an inbox org file on whatever shared surface it can reach. `fold`
turns each unstamped heading into a tracked issue, then flips the heading to
DONE and stamps it with the assigned id in place, so the inbox doubles as its
own receipt and folding twice creates nothing:

```console
$ vissue fold inbox.org --project parser
folded 2: parser-x1a2 parser-y3b4
```

Like `create`, `fold` auto-detects the project from `.project-ctx.toml`
when `--project` is omitted.

**See what is due.** `agenda` lists open and blocked issues whose
`DEADLINE` or `SCHEDULED` falls inside a horizon (default 14 days),
overdue first; a blocked issue still appears, because its date does not
stop mattering while it waits:

```console
$ vissue agenda -d 30
2026-07-23  deadline  12d overdue parser-k29f  STARTED  [#A]  Reject a manifest... (parser)
```

**Watch for changes without re-reading everything.** A write advances a
generation counter and appends to a log, both beside the project directories.
A poller compares the counter, then reads only what is new:

```console
$ vissue gen
3167
$ vissue events --since 3155 -n 5
$ vissue wait --last 3167 --timeout-ms 30000   # exits 2 on timeout
$ vissue ping --detail "external change"       # wake pollers by hand
```

Set `VISSUE_EVENTS=0` to suppress emission when the tracker must stay
untouched.

**Point at a different layout.** Use `--prefix`, `VISSUE_PREFIX`, or a
`vissue.toml` at the root:

```toml
prefix = "projects"

[issues]
default_priority = "B"
id_length = 5
```

**Detect the project from the working directory.** With a `.project-ctx.toml`
carrying `[project] name = "..."` in or above the current directory, `--project`
may be omitted.

**Know which tracker you are writing to.** `create`, `update`, `claim`, and
`refile` all write, and claiming counts as a write because it stamps the
holder onto the issue. When a wrapper or the environment sets `VISSUE_ROOT`,
a bare `vissue` writes to that tracker from any directory. Pass `--root`
explicitly for a scratch tracker, and `vissue identity` reports which binary
and which root are in play before you commit to a mutation.

## Reference

### Commands

| Command | Purpose |
|---|---|
| `create`, `q` | Add an issue; `q` prints only the new id |
| `list`, `show` | Rows of issues; one issue's metadata and file range |
| `update`, `claim`, `refile` | Change state, priority, or blockers; take an issue; move it between projects |
| `note` | Add a dated logbook entry; state and claim untouched |
| `claims` | Every live claim, oldest first: who holds what, for how long |
| `fold` | Turn an inbox org file's unstamped `* TODO` headings into issues, stamping in place |
| `whoami` | The identity a claim would record |
| `ready`, `count`, `search`, `children`, `ancestors`, `impact`, `related`, `stale` | Query the corpus; bounded dependency and related traversal |
| `agenda` | Deadlines and scheduled starts inside a horizon, overdue first |
| `export` | JSONL, one object per issue |
| `tree`, `graph`, `cycles`, `backlinks` | Relationships |
| `roadmap`, `mirror` | Markdown roadmap; read-only org or markdown projection |
| `digest`, `mirror --check` | Corpus digest; whether a mirror is still current |
| `check`, `hygiene` | Validation |
| `gen`, `events`, `wait`, `ping` | Change stream for pollers |
| `projects`, `identity` | Layout introspection |

### States and priorities

States are `TODO`, `STARTED`, `BLOCKED`, `DONE`, and `CANCELLED`. An issue is
*ready* when it is `TODO` or `STARTED` and no id in its `:BLOCKED_BY:` is still
open. Priorities are the org cookies `[#A]`, `[#B]`, and `[#C]`, defaulting
to `C`.

### Properties

`:ID:` is required and generated. `:CREATED:` is set on create. `:PARENT:`,
`:BLOCKED_BY:`, `:VISSUE_TAGS:`, `:TYPE:`,
`:CLAIMED_BY:`, `:CLAIMED_AT:`, and `:DISCOVERED_FROM:` are read by the query
verbs; any other property is preserved
untouched, in the order it appears on disk. `:BLOCKED_BY:` accepts commas,
whitespace, or both. `:PARENT:` may name another issue or any org heading with
an `:ID:` under the tracker prefix, so a design document can head a work
hierarchy.

### What Org owns

Three fields live where Org keeps them rather than in the drawer, because Org
reads them from there and nowhere else:

| Field | Written as | What it buys |
|---|---|---|
| Deadline, scheduled, closed | The planning line under the heading: `CLOSED: [...] SCHEDULED: <...> DEADLINE: <...>` | `org-agenda` shows the issue |
| Tags Org can hold | The heading's own `:tag:tag:` run | `org-agenda` tag search and `C-c \` match |
| Identity | `:ID:` | `[[id:...]]` links resolve through `org-id` |

A tag Org will not accept in a heading, `needs-review` say, stays in
`:VISSUE_TAGS:` and still answers `search` and `related`. `vissue create
--tags` splits a list between the two on that rule, and `show`, `export`, and
the markdown mirror report the union.

Both shapes are read, so a tracker written before this and one edited in Emacs
parse the same; the next rewrite settles on the Org shape. `:TAGS:`,
`:DEADLINE:`, and `:SCHEDULED:` in a drawer are names Org reserves, which
`org-lint` reports, so they are migrated rather than kept.

Because Org owns them, `C-c C-d`, `C-c C-s`, `C-c C-q`, and marking an issue
DONE with `org-log-done` are all safe to run on a tracker; `tests/org_interop.sh`
is Emacs doing exactly that and vissue reading the result back.

### JSONL export schema

Each line is an object with `id`, `project`, `title`, `state`, `priority`,
`properties`, `org_tags`, `tags`, `logbook`, `body`, `line_start`, and
`line_end`. `org_tags` is the heading's Org tag run and `tags` is the union
with `:VISSUE_TAGS:`, which is the field to filter on. Logbook entries
carry `timestamp`, `from`, `to`, and `note`. CLOCK and other opaque drawer
lines also carry `raw`, the verbatim Org line.

### MCP server

`vissue-mcp` speaks MCP over stdio and calls the library in process. It resolves
its root from `VISSUE_ROOT` and `VISSUE_PREFIX`. Tools mirror the CLI verbs:
`vissue_list`, `vissue_ready`, `vissue_show`, `vissue_create`, `vissue_update`,
`vissue_claim`, `vissue_count`, `vissue_search`, `vissue_children`,
`vissue_backlinks`, `vissue_related`, `vissue_waiting_on`, `vissue_body_excerpt`, `vissue_tree`,
`vissue_graph`, `vissue_ancestors`, `vissue_impact`, `vissue_cycles`,
`vissue_refile`, `vissue_wait`, `vissue_whoami`, `vissue_roadmap`,
`vissue_export`, `vissue_check`, `vissue_hygiene`, `vissue_mirror`,
`vissue_projects`, and `vissue_identity`.

## Ecosystem

`vissue` manages intentional work items; [orgaw](https://github.com/HaoZeke/orgaw)
records intentional time and evidence against them. The projects remain
independently installable and communicate through the `vissue` command-line
protocol, with Org issue headings and their CLOCK entries as the shared data.

```mermaid
flowchart LR
    corpus["Org issues and CLOCK entries"]
    vissue["vissue: work items"]
    protocol["CLI provider protocol"]
    orgaw["orgaw: time and evidence"]
    emacs["Emacs: agenda, tags, id links"]
    corpus <--> vissue
    corpus <--> emacs
    vissue --> protocol --> orgaw
    orgaw --> corpus
```

Install both tools for the integrated path:

```console
$ cargo install vissue-cli
$ cargo install orgaw
$ export ISSUE_ROOT=/path/to/notes
$ vissue ready
$ orgaw issues
$ orgaw in project-id
$ orgaw out
```

Emacs is a client of the corpus rather than of the command: a tracker is an
ordinary Org file, so the agenda, tag search, and `id:` links work against it
with nothing installed.
[vissue.el](https://github.com/HaoZeke/vissue.el) is the optional convenience
layer. `M-x vissue-add-to-agenda` puts the tracker files on `org-agenda-files`
and refreshes `org-id` locations; `M-x vissue-list` works the ready set from a
buffer.

## Explanation

### How to audit a citation

A paper earns a slot only if it maps onto a verb or property in this
repository. The relation is one of four:

| Relation | Meaning |
|---|---|
| implements | The library runs this algorithm or formula. |
| stores | The file is the persistent form of this object's output. The agent, not vissue, produces the object. |
| analogizes | Same interface, different domain. The mapping must name the issue property. |
| refuses | Cited so the opposite choice is checkable. |

A vibe match does not count. SPECTER does not justify `BLOCKED_BY`
because vissue does not embed papers. Zep does not justify `related`
because `related` does not extract a temporal knowledge graph. Both
still belong, under analogizes and refuses. The working bibliography
lives in the OokCite collection `vissue-dag`; every DOI below was
resolved there before it was pasted.

| Claim in the tracker | Site in the code | Paper | Relation |
|---|---|---|---|
| The file is the store | `issues.org`, Org drawers | Schulte et al. [1] | implements |
| A plan splits into tagged children | `--parent`, `--type`, `--tags` | Erol et al. [3]; Nau et al. [4] | stores |
| Order is a partial order, not a list | `:BLOCKED_BY:` | Weld [2] | stores |
| The graph stays acyclic | `DependencyGraph`, `cycles` | Kahn [5]; Tarjan [6] | implements |
| Several nodes can run at once | `ready` | Coffman and Graham [7]; Mokhov et al. [8] | analogizes |
| Two workers do not take the same node | `claim`, `VISSUE_AGENT` | Blumofe and Leiserson [9]; Anvik et al. [11] | analogizes |
| Logbook order is happened-before | `:LOGBOOK:` | Lamport [10] | analogizes |
| Edges are declared influence | `:BLOCKED_BY:`, `:PARENT:` | Garfield [12]; Pinski and Narin [13] | analogizes |
| Neighborhood walks declared edges first | `related`, `ancestors`, `impact` | Brin and Page [14]; Kleinberg [15]; Haveliwala [16]; Gleich [17] | analogizes |
| Cite-edge as a positive neighbor | OokCite encoder research, not this binary | Cohan et al. [18]; Ostendorff et al. [19]; Singh et al. [20]; Reimers and Gurevych [21] | analogizes |
| Leftover term overlap | `related` idf | Sparck Jones [22] | implements |
| Issues are the entities | headings with `:ID:` | Hogan et al. [23] | analogizes |
| Do not extract a second graph | `related` writes nothing | Rasmussen et al. [24]; Edge et al. [25]; Gutierrez et al. [26] | refuses |

### Why the file is the database

A tracker that owns its own store forces a sync problem on everyone who also
wants the data in git. Keeping the org file authoritative removes that problem:
review happens in the diff, history comes from the log, and any editor that
speaks orgmode is a client [1]. Each command pays a linear scan of the
files it needs, which stays cheap until the issue count grows large enough
to notice.

### Why HTN, work stealing, and citation graphs all fit

A markdown task list is a total order dressed as a document. Hierarchical
task-network planning [3], [4] is how a plan becomes a network of
actions; vissue does not search that network, it stores it. Least-commitment
planning [2] is why the stored order is a partial order: the terminal UI
cannot start before the overlay exists, but the catalog and an unrelated
docs pass can run together. Work stealing [9] and rebuild DAGs [8] are why
the query that matters is the set of open sources, not a serial schedule
[7]. Claiming a source is the same interface as assigning a bug [11].

Citation graphs [12], [13], [14], [17] are the other declared-edge
discipline. OokCite is where that stack was searched, collected, and
run: identity and cite edges are truth; PageRank and text are ranking
features; a neighborhood query does not mint a new citation. `related`
is that split on issue headings. Extracted memory graphs [24], [25],
[26] are the refusal: they build a second store from prose. Hogan's
knowledge graph [23] is the headings a human already wrote.

### Why `related` does not write edges

Walk `:PARENT:`, `:BLOCKED_BY:`, `:DISCOVERED_FROM:`, and Org links
first. Score leftover term overlap second [22]. Print the evidence.
Write nothing back. Promoting a derived neighbor into `:BLOCKED_BY:`
would be the same mistake as treating a PageRank score as a new
citation [14], [17].

### Why `show` does not print the body

`show` returns metadata and `file:line_start-line_end`. An editor or an agent
opens that range when it wants the prose. Keeping prose out of the command
output is what stops a status check from turning into a wall of text.

### Concurrency

Every read-modify-write cycle takes a process-local mutex and an advisory lock
on `issues.org.lock`, then writes through a temporary that is flushed to the
device, uniquely named, and renamed into place. Concurrent creates from several
processes therefore neither lose headings nor collide on the temporary file,
and a crash mid-write leaves the previous file rather than a truncated one.

The lock covers one project file. Adding a blocker reads the whole corpus for
the acyclicity check inside that lock, so it sees every write that has landed;
two blockers added at the same moment in *different* project files can still
close a cycle between them. `check` and `cycles` report one if it happens, and
neither is expensive to run from CI.

## References

Resolved and collected through OokCite into `vissue-dag`. Each entry
has a relation in the audit table above.

1. E. Schulte, D. Davison, T. Dye, and C. Dominik, "A Multi-Language Computing Environment for Literate Programming and Reproducible Research," *Journal of Statistical Software*, 2012, doi: [10.18637/jss.v046.i03](https://doi.org/10.18637/jss.v046.i03).
2. D. S. Weld, "An Introduction to Least Commitment Planning," *AI Magazine*, 1994, doi: [10.1609/aimag.v15i4.1109](https://doi.org/10.1609/aimag.v15i4.1109).
3. K. Erol, J. Hendler, and D. S. Nau, "Complexity results for HTN planning," *Annals of Mathematics and Artificial Intelligence*, 1996, doi: [10.1007/bf02136175](https://doi.org/10.1007/bf02136175).
4. D. S. Nau, T.-C. Au, O. Ilghami, U. Kuter, J. W. Murdock, D. Wu, and F. Yaman, "SHOP2: An HTN Planning System," *Journal of Artificial Intelligence Research*, 2003, doi: [10.1613/jair.1141](https://doi.org/10.1613/jair.1141).
5. A. B. Kahn, "Topological sorting of large networks," *Communications of the ACM*, 1962, doi: [10.1145/368996.369025](https://doi.org/10.1145/368996.369025).
6. R. Tarjan, "Depth-First Search and Linear Graph Algorithms," *SIAM Journal on Computing*, 1972, doi: [10.1137/0201010](https://doi.org/10.1137/0201010).
7. E. G. Coffman and R. L. Graham, "Optimal scheduling for two-processor systems," *Acta Informatica*, 1972, doi: [10.1007/bf00288685](https://doi.org/10.1007/bf00288685).
8. A. Mokhov, N. Mitchell, and S. Peyton Jones, "Build systems a la carte," *Proceedings of the ACM on Programming Languages*, 2018, doi: [10.1145/3236774](https://doi.org/10.1145/3236774).
9. R. D. Blumofe and C. E. Leiserson, "Scheduling multithreaded computations by work stealing," *Journal of the ACM*, 1999, doi: [10.1145/324133.324234](https://doi.org/10.1145/324133.324234).
10. L. Lamport, "Time, clocks, and the ordering of events in a distributed system," *Communications of the ACM*, 1978, doi: [10.1145/359545.359563](https://doi.org/10.1145/359545.359563).
11. J. Anvik, L. Hiew, and G. C. Murphy, "Who should fix this bug?," 2006, doi: [10.1145/1134285.1134336](https://doi.org/10.1145/1134285.1134336).
12. E. Garfield, "Citation Indexes for Science," *Science*, 1955, doi: [10.1126/science.122.3159.108](https://doi.org/10.1126/science.122.3159.108).
13. G. Pinski and F. Narin, "Citation influence for journal aggregates of scientific publications: Theory, with application to the literature of physics," *Information Processing & Management*, 1976, doi: [10.1016/0306-4573(76)90048-0](https://doi.org/10.1016/0306-4573(76)90048-0).
14. S. Brin and L. Page, "The anatomy of a large-scale hypertextual Web search engine," *Computer Networks and ISDN Systems*, 1998, doi: [10.1016/s0169-7552(98)00110-x](https://doi.org/10.1016/s0169-7552(98)00110-x).
15. J. M. Kleinberg, "Authoritative sources in a hyperlinked environment," *Journal of the ACM*, 1999, doi: [10.1145/324133.324140](https://doi.org/10.1145/324133.324140).
16. T. H. Haveliwala, "Topic-sensitive PageRank," 2002, doi: [10.1145/511446.511513](https://doi.org/10.1145/511446.511513).
17. D. F. Gleich, "PageRank Beyond the Web," *SIAM Review*, 2015, doi: [10.1137/140976649](https://doi.org/10.1137/140976649).
18. A. Cohan, S. Feldman, I. Beltagy, D. Downey, and D. Weld, "SPECTER: Document-level Representation Learning using Citation-informed Transformers," 2020, doi: [10.18653/v1/2020.acl-main.207](https://doi.org/10.18653/v1/2020.acl-main.207).
19. M. Ostendorff, N. Rethmeier, I. Augenstein, B. Gipp, and G. Rehm, "Neighborhood Contrastive Learning for Scientific Document Representations with Citation Embeddings," 2022, doi: [10.48550/arXiv.2202.06671](https://doi.org/10.48550/arXiv.2202.06671).
20. A. Singh, M. D'Arcy, A. Cohan, D. Downey, and S. Feldman, "SciRepEval: A Multi-Format Benchmark for Scientific Document Representations," 2023, doi: [10.18653/v1/2023.emnlp-main.338](https://doi.org/10.18653/v1/2023.emnlp-main.338).
21. N. Reimers and I. Gurevych, "Sentence-BERT: Sentence Embeddings using Siamese BERT-Networks," 2019, doi: [10.48550/arXiv.1908.10084](https://doi.org/10.48550/arXiv.1908.10084).
22. K. S. Jones, "A statistical interpretation of term specificity and its application in retrieval," *Journal of Documentation*, 1972, doi: [10.1108/eb026526](https://doi.org/10.1108/eb026526).
23. A. Hogan et al., "Knowledge Graphs," *ACM Computing Surveys*, 2022, doi: [10.1145/3447772](https://doi.org/10.1145/3447772).
24. P. Rasmussen, P. Paliychuk, T. Beauvais, J. Ryan, and D. Chalef, "Zep: A Temporal Knowledge Graph Architecture for Agent Memory," 2025, doi: [10.48550/arXiv.2501.13956](https://doi.org/10.48550/arXiv.2501.13956).
25. D. Edge et al., "From Local to Global: A Graph RAG Approach to Query-Focused Summarization," 2024, doi: [10.48550/arXiv.2404.16130](https://doi.org/10.48550/arXiv.2404.16130).
26. B. J. Gutierrez, Y. Shu, Y. Gu, M. Yasunaga, and Y. Su, "HippoRAG: Neurobiologically Inspired Long-Term Memory," 2024, doi: [10.48550/arXiv.2405.14831](https://doi.org/10.48550/arXiv.2405.14831).

See also the [Org mode manual](https://orgmode.org/manual/), the
[Model Context Protocol](https://modelcontextprotocol.io/),
[petgraph](https://github.com/petgraph/petgraph), and
[daggy](https://github.com/mitchmindtree/daggy).

## Contributing

Conventional commits, `cargo fmt`, and `cargo clippy -- -D warnings`. Run
`prek install` for the hooks. The minimum supported Rust version is 1.88.

## Citation

See [CITATION.cff](CITATION.cff).

## License

MIT. See [LICENSE](LICENSE).
