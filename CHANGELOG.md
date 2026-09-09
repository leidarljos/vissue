# Changelog

All notable changes to vissue are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and releases use
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- towncrier release notes start -->

## [0.9.1](https://github.com/HaoZeke/vissue/releases/tag/v0.9.1) - 2026-09-09

### Fixed

- A directory that is not a tracker says so instead of answering "none". The root
  falls back to the working directory, and a reading verb run from somewhere else
  found no projects and printed `0` with a zero exit. A caller cannot tell that
  from a tracker with nothing in it, and the two mean opposite things. A guessed
  root now has to hold `vissue.toml` or the prefix directory. A root named with
  `--root` or `VISSUE_ROOT` is trusted whether or not it holds anything, and `man`
  and `completions` still work from anywhere because they describe the program
  rather than a corpus.
- `backlinks` reports a duplicated id instead of scanning it as a deed. The
  accession walk exists because an accession has no heading of its own. An id
  with a heading in two routed trackers is a corpus fault. Answering it with an
  empty citation list reported that fault as "nothing cites this". Only "no such
  heading" falls through to the walk now, and an unreadable file stays an error.

### Developer

- The schema comparison covers notes, and the committed constant was
  regenerated. It compared surface names and a field count. A note-only edit to
  `vissue.capnp` therefore left the constant carrying the previous prose, and
  every check still passed. The reader now takes each operation's note and its
  fields' notes. That is what makes the schema authoritative in fact rather than
  in the documentation alone.


## [0.9.0](https://github.com/HaoZeke/vissue/releases/tag/v0.9.0) - 2026-09-09

### Added

- Each `export` row carries a typed `deeds` array beside `properties`, in the order
  the citations were made. The socket already handed that field over typed and the
  JSONL did not, so a consumer had to split a drawer string on whichever separator
  the author used. `:DEEDS:` stays in `properties` verbatim for a reader that wants
  the drawer.
- The explanation page now names the prior art for `recall`: walking a dependency
  graph to assemble context is not a new idea, and two 2026 systems do it from a
  derived graph. What is unusual here is where the graph comes from, since it was
  written down by whoever split the work and `ready` reads the same edges, so a
  wrong edge is a planning error rather than a bad retrieval.
- The explanation page now says why a working set has no poisoning surface: nothing
  an agent reads becomes part of one. An entry is there because an issue declares
  `:BLOCKED_BY:`, `:PARENT:`, or `:DISCOVERED_FROM:` naming it, and those are
  written only by a tracker mutation under the lock, by a named identity, with a
  logbook line. `fold` ingests a file into issues carrying a title and a body and
  no edges at all.
- The terminal board and the HUD both grew a **recall** detail tab: the plan above
  the selected issue, each declared input with the deeds it produced, and the last
  thing said about it. In `vissue tui` it joins show / excerpt / tree / related; in
  `vissue hud` it joins tree / related / notes. A 360-pixel HUD overlay has room
  for three tab labels, so recall sits past the edge of the strip and Enter cycles
  to it.
- `[consensus.susceptibility_of]` sets one agent's susceptibility where it differs
  from the `consensus.susceptibility` default. Friedkin and Johnsen's
  susceptibility is a diagonal rather than one number, which is the part that
  carries the meaning: a maintainer who has read the code for years and a reviewer
  seeing it for the first time are not equally movable. Rows merge agent by agent
  like the trust rows, and where the agents differ the report puts the value on
  each row rather than on the header.
- `backlinks` takes a deed accession as well as an issue id, and answers with the
  issues standing on that product: `(cites)` for a `:DEEDS:` citation, and
  `(body mention)` for prose naming it with nothing declared. The join across the
  tracker and the deed store ran in one direction, so the question you have when a
  product turns out to be wrong had no command.

  The corpus decides which namespace the argument belongs to. A known issue id
  stays an issue whatever it looks like, so a project literally named `deed` keeps
  working. An accession nobody cited answers empty rather than failing, since the
  product may be real and simply unused. Under routing, the command line and the
  tool scan every tracker in reach: a product has no project of its own.
- `consensus.susceptibility` chooses the opinion model. At `1.0`, the default, an
  agent gives up its own starting position entirely and the group converges on one
  number: DeGroot, exactly as before. Below it, an agent moves that fraction of the
  way toward what it hears and keeps the rest of the ballot it cast, which is
  Friedkin and Johnsen's generalisation.

  Two things follow. The group settles while still disagreeing, and `consensus`
  reports where each agent landed plus the spread between them instead of naming a
  position none of them holds. And any anchor makes the step a contraction, so the
  periodic trust graph that never settles under DeGroot cannot arise.
- `d` on either board cites a deed on the selected issue. The terminal board opens
  a field like the note field; the HUD switches to the recall tab first, so the
  citation lands where you are looking. A value that is not an accession is refused
  with the message the command line gives, on the board rather than in a log, and
  the HUD keeps what was typed in the field, since a refused accession is usually a
  typo in a long hash.

  The action is `issue.deed` in the key catalog and remappable like the rest.
- `issues.expect_deeds = true` makes `hygiene` report every issue that closed
  without citing a deed. On a tracker where the next unit is expected to open the
  last one's product, work that named nothing is a hole in the handoff, and this
  makes it visible instead of leaving it to be found by whoever needed it. Off by
  default, because plenty of issues produce nothing a deed store would hold.
- `vissue consensus <id> --gate` adds an exit status a shell hook can act on, and
  prints the report either way so a failing hook leaves the reason on screen.

  On one issue it exits non-zero unless the group agreed and one choice leads: a
  plurality, a tie, a split and an oscillation are all cases where acting on the
  number would be acting on agreement that is not there. Over `--children` it exits
  non-zero when any child settled split or carries no ballots, which are the two
  rows a parent cannot decide on a child's behalf.
- `vissue consensus <id>` weighs an issue's existing ballots by how much the
  group listens to each agent, using DeGroot averaging over a trust graph in
  `[consensus.trust]`. It prints the plain count and the weighted position
  together, each agent's social power, and the two ways there is no consensus to
  report: a trust graph with more than one closed group, or one that never
  settles.

  `--json` gives the same result as structure: the choice set, each agent's limit
  and social power, the settling, and the factions when there are any.

  See `consensus.susceptibility` for the anchored variant, where the group settles
  while still disagreeing.

  Nothing new has to be cast, and nothing changes on a tracker that configures no
  trust: every agent then listens to every other equally and the consensus is the
  tally as a fraction.
- `vissue consensus <plan> --children` rolls up over a plan's children, which is
  the "can this epic close" question rather than the "what does this issue hold"
  one.

  It reports the children row by row and does not average them. No weighting over
  children can be picked without a judgement the tracker has no basis for, and an
  equal-weight one lets an epic split finely outvote one split coarsely; a child
  that settled split has no single position to fold in; and a child nobody voted on
  is absent rather than neutral, which matters because unvoted is the common case.
  So the report says how many children carry ballots, what each holds, whether the
  voted ones point the same way, which settled split, and how many carry none.
- `vissue deed <id> --add <accession>` cites the deeds a unit of work produced,
  in a `:DEEDS:` property on the heading. A deed is
  [deedar](https://github.com/indynull/deedar)'s frozen record of a product, and
  the accession is the whole handoff: the next unit runs `deedar get` on it
  instead of rereading a transcript. The tracker cites and stores no product
  bytes, so a value that is not an accession (`deed-<kind>-<slug>`, or a
  `sha256:` of the deed or of one product path) is refused rather than stored as
  a citation that resolves to nothing.
- `vissue recall <id> --excerpts` splices a capped excerpt of each input's heading
  into the working set. What an input concluded lives in its body, since `append`
  writes the report there and the deed names the product rather than the reasoning.

  Off unless asked: the common case wants the accessions, and a working set
  carrying four screens of prose is one nobody reads. The excerpt goes through the
  same path `body-excerpt` uses, so it is capped and an input whose body looks like
  credential material is suppressed rather than spliced into a model's context.
- `vissue recall <id>` prints the working set for an issue: the plan it sits in,
  the deeds produced by what blocks it, the issue it was bounced from, and what
  it has produced itself. Read it before starting work on a node.

  It walks `:PARENT:`, `:BLOCKED_BY:`, and `:DISCOVERED_FROM:` rather than
  ranking the corpus by resemblance, so it is what the plan says the work stands
  on. `related` still answers the resemblance question. `--deeds-only` prints the
  accessions one per line, for `deedar get $(vissue recall <id> --deeds-only)`.
  A `:PARENT:` that names a design document rather than an issue is named in the
  plan too, marked as a heading outside the tracker, because that document is what
  the reader should open.

  The blocker walk is one hop by default, because a deed records its own sources
  and `deedar trail` walks them; `--depth` widens it.

### Fixed

- A sentence in the explanation page had wrapped so that a number landed in column
  zero, which Org reads as an ordered list item: the paragraph rendered split in
  two around a stray list. `docs/scripts/check_org.py` now runs before the export
  and refuses that shape, along with a citation whose reference has no entry, an
  entry nobody cites, and a gap or a duplicate in the numbering. Neither class
  produces invalid output, so nothing downstream was ever going to complain.
- `ready` no longer rescans the corpus once per issue to find that issue's parent
  and siblings. It was quadratic in the corpus on any tracker whose issues have
  parents, which is every tracker with a plan in it, and `ready` is the verb an
  agent polls. On 20,000 issues across 10 projects the call drops from 617 ms to
  512 ms, and the gap widens as parents sit further from the top of their file.
- `vissue-core` now builds against `capnp` 0.27. Versions before 0.24 let safe
  Rust code trigger undefined behaviour through the generated schema readers
  (capnproto-rust#605), and the operation set this crate reads is one of those
  readers. Nothing about the schema or the verbs changed; regenerate
  `vissue_capnp.rs` with `capnpc` 0.27 if you carry a local edit to
  `schema/vissue.capnp`.


## [0.8.0](https://github.com/HaoZeke/vissue/releases/tag/v0.8.0) - 2026-08-23

### Added

- Eighteen tests for findings `check` could emit and nothing exercised: every preamble
  keyword Org needs, the noexport warning that is about disclosure rather than
  convenience, each way a blocker is spelled wrong, a type that is not on its heading,
  two tags from one exclusive group, a priority outside the declared range, work started
  out of order under an ORDERED parent, a DONE parent with an open child, a missing
  creation date, a DONE body that reads as a reject, a sibling that already settled, a
  discovery claim with no edge behind it, and a blocker ring every edge check passes.
- Every flag a verb takes has to appear in the schema, which closes the last
  one-directional check.

  The flag checks ran schema to surface only: a flag the schema names must exist, and a
  flag that exists and the schema omits went unseen. That is the same asymmetry as at the
  verb level, one level down, and it stayed open after the verb level was closed.

  Global flags are declared once as `globalFlags` and subtracted, so a per-verb row stays
  about what the verb actually takes rather than repeating `--root` forty times. A
  local-only verb is skipped: the HUD's nine flags are window management, and enumerating
  them in a schema of operations would bury the flags that describe a surface.

  Forty-one flags had to be recorded, generated from the argument structs rather than
  written out, and two checks had to stop guessing type names while that happened. The
  tool check derived `<Verb>Args` and the socket check `<Verb>Params`, which is wrong
  wherever a struct is shared: ancestors and impact both take `DepthArgs`, list and ready
  both take `IssueListParams`. Both now read the type from the code that declares it, the
  tool's signature and the handler's own body.
- Every method has a typed request and response form. Nineteen did not.

  `Request::parse` and `decode_response` answered "no typed form; send it untyped" for
  `append`, `vote`, `fold`, `check` and the rest, so a client wanting typed access to
  them could not have it. The two enums had also drifted from the method list by exactly
  the amount nobody was checking.

  Four result types carry the replies: `ReportResult` for the reads that produce prose,
  `CheckResult` where the error and warning counts travel beside the text, `DigestResult`
  and `WaitResult` where there is structure worth having. Reports share one type on
  purpose — a type per report would be a contract per report to keep in step with the
  text, and the text is the part anyone reads.

  A test drives the round-trip from the capability list, so a method reaching the wire
  without a typed form fails there rather than being discovered by whoever wanted it.
- Every mutating socket reply is asserted to have the same shape: a boolean `ok`, a
  `report` string, and the affected issue where one is named.

  `mut_result` produced that shape by convention rather than contract, so a method
  returning a bare field, or omitting `ok`, would have passed every other check. A
  client switching on `ok` and printing `report` is the normal way to use this socket,
  which makes the shape worth asserting once across all of them instead of trusting
  eleven call sites to agree.
- Nine reads gained a `--json` mode: `search`, `children`, `ancestors`, `impact`,
  `backlinks`, `agenda`, `tree`, `body-excerpt` and `projects`.

  They answered in prose on the command line and in structure over the socket, so there
  was nothing to compare between the two surfaces and they went unaudited while every
  other read was checked.

  The modes go through the same `CatalogService` the socket answers from, so the two
  surfaces are one computation rather than two that have to be kept in agreement. A test
  compares all nine, and they matched on the first run, which is what routing them this
  way round was for.
- Seven relatedness evidence labels had no test: `blocked_by`, `parent`, `source_of`,
  `pivoted_to`, `successor_of`, `shared_tags` and `same_project`. Nor did the limit or the
  tie-break that makes the ranking total. Two of the new tests assert a weight rather than
  a label, because a scorer that records the reason and scores it zero passes any test that
  only looks for the reason.
- The control socket now has a method for every verb that changes a file:
  `issue/append`, `issue/reject`, `issue/resolve`, `issue/fold` and
  `issue/normalize` join the six that were already there.

  It stays optional. The advisory lock serialises a direct write just as well, so
  this is not a correctness fix and a client mixing the two paths corrupts nothing.
  What it buys is a complete surface: `append` used to exist only on the command
  line, so a socket client had to shell out for it, those writes went behind the
  server's back, and its change stream had a hole exactly where they were.
- The operation schema names each verb's command-line flags, and every subcommand's
  help has to offer them.

  Naming a verb on each surface stops the verb going missing. It does nothing about
  the surfaces disagreeing on what to call a field, which is the next way this drifts:
  a socket method taking `body` where the subcommand takes `--text` is two spellings of
  one idea and no verb-level check sees it.

  Read out of each subcommand's own help, so the parser answers rather than a scan of
  the source that declares it. Positional arguments are excluded, and the reason is on
  the field: the first version of the list claimed `create` and `reject` took `--title`
  when both take it positionally, and the check caught that plus a `--blocked-by` that
  is really `--block` on its first run.
- The reference states what the change stream promises: unique sequences, and no
  ordering.

  An event is emitted after the file lock is released, so two changes to different
  project files can reach the log in the opposite order to the one they reached disk in.
  A consumer treating a notification as "something moved, go and look" is correct; one
  reconstructing history from the log's order is not, and nothing promised it could.

  Emitting inside the lock would order the log at the cost of holding a file lock across
  another write for every state change. Declined, and the reasoning is written down: the
  log exists to wake a poller, and a poller re-reads state.

  The cross-file cycle window is recorded the same way. Locking every file an acyclicity
  check reads would prevent it and serialise every blocker edit in the corpus against
  every other, because the check reads all of it. For blockers added by hand a few at a
  time, the after-the-fact report is the better side of that trade.
- The schema covers every subcommand, not only the mutating ones, and a test refuses a
  subcommand it does not mention.

  Until now the schema constrained the verbs it already listed. A brand-new subcommand
  failed nothing, because nothing asked whether the schema knew about it, which is
  exactly how the earlier gaps arrived: `vote` was added to the command line and no test
  anywhere had an opinion.

  Read verbs are held to the surfaces they claim rather than to all of them, because
  fourteen of them are genuinely absent from the socket today and the schema says so
  instead of pretending otherwise.

  Three things had to be modelled that the mutating-only version never met. Aliases,
  because `create` answers to `q` and the new check found it and could not tell it from
  a verb. Local-only verbs, because the terminal UI and shell completions have no
  remote surface and are not gaps. And a socket method answering for two subcommands,
  which `identity/get` does for `identity` and `whoami`, allowed when the rows say so
  and refused in silence.

  The committed generated file is now checked against the schema text as well. Editing
  `vissue.capnp` and forgetting to regenerate left every check validating the previous
  schema and passing, so the schema was authoritative in the documentation only.
- The schema names each method's socket parameters, and the param structs are checked
  against it.

  The verb check says `issue/append` answers. It does not say the method takes `text`
  rather than `body`, and the third spelling of a field is where this drifts next. Read
  off the param structs in `rpc.rs`, where serde decides the wire names, so what is
  checked is what a client has to send.

  Two facts that had never been written down anywhere are now on the fields. The issue
  being acted on is positional on the command line, `id` on the socket and `issue_id` as
  a tool argument. And `agent` exists only on the socket, because only there is there a
  connection whose identity can be overridden; the other surfaces take it from the
  environment.
- The schema records each field's Rust type per surface, and both checks verify it.

  A name check cannot see a type change: a field going from a number to a string keeps
  its name. `update --if-gen` is a `u64` on both remote surfaces and nothing said the
  two agreed.

  Recorded per surface rather than once, because two of them genuinely differ and
  forcing one type over both would be a lie. `priority` is `Option<String>` as a tool
  argument and `Option<char>` on the socket, `force` is `Option<bool>` and `bool`. What
  this catches is either side changing.

  Generated from the structs rather than written out, after two earlier rounds of
  schema data written from memory turned out wrong.
- Thirteen read verbs gained control-socket methods: `issue/check`, `issue/count`,
  `issue/cycles`, `issue/digest`, `issue/export`, `issue/graph`, `issue/roadmap`,
  `issue/stale`, `issue/hygiene`, `issue/waiting_on`, `issue/mirror`, `events/ping`
  and `events/wait`.

  A socket client used to shell out for all of them. That cost a subprocess rather
  than correctness, which is why it outlived the write gap, but `events/wait` is the
  case that made it worth closing: a verb whose whole job is to block until something
  changes is exactly what a connection is good at and a subprocess is bad at.

  Reads that produce a report answer with `report`, the same text the subcommand
  prints, because inventing a structure per report would be a second contract to keep
  in step with the first. `issue/check` carries `errors` and `warnings` beside it,
  since the subcommand exits non-zero on an error count and a client needs that signal
  without parsing prose. `issue/digest` and `events/wait` answer with fields.
- Two checks now run surface to schema, not only schema to surface: every tool the
  server exposes and every method it dispatches must appear in a schema row.

  Every existing check ran one way, catching a verb the schema names and the surface
  lacks. Nothing caught the reverse, so a tool that existed and the schema omitted was
  invisible to all of them — the same asymmetry that let verbs drift in the first place.

  It had already bitten three times. The schema claimed `normalize` had no tool while
  `vissue_normalize` sat in the server, and `vissue_org` and `vissue_mirror_check` were
  in no row at all. Every check passed throughout.

  The last two needed a shape the schema could not express: a tool whose command-line
  form is a flag on another verb rather than a verb of its own, `show --org` and
  `mirror --check`. Such a row carries no subcommand and says so, and the uniqueness
  checks were treating two absent names as a collision.
- `clippy::cognitive_complexity` is enabled with the threshold held at today's ceiling in
  `clippy.toml`. A ratchet rather than a target: it blocks a function growing past the
  worst one already here, and lowering it is the work of splitting those.

  The comment there records what the measure does not see. The control socket dispatches
  thirty-seven methods from one match and scores under clippy's default, because the
  metric counts nesting and a wide flat match has none. Drift between the surfaces lives
  in exactly that breadth, which is why the schema checks catch it and this never would.
- `schema/regen.sh` refuses to run when the `capnpc-rust` plugin's version does not
  match the `capnp` runtime crate the generated file has to compile against, and will
  run the plugin over ssh when `VISSUE_CAPNPC_SSH` names a host. The compiler is a
  distro package and the plugin is a cargo install, so they land on different machines
  often enough that the round trip is the common case.
- `schema/regen.sh` regenerates the schema's Rust in one command.

  The step needs the Cap'n Proto compiler and the `capnpc-rust` plugin, and neither is
  needed to build or test. Where both are present the script is one command; where only
  the compiler is, it writes the request file and prints what to run on a machine with
  the plugin, because the two halves can live on different machines and the request
  moves between them.

  It finds the generated file rather than assuming its name, since the plugin lays its
  output out by the source path recorded in the request and can put it in a `schema/`
  subdirectory rather than flat.
- `schema/vissue.capnp` states the verb set once, and the command line, the control
  socket and the MCP tool list are each tested against it.

  Every surface used to declare its own verbs in its own idiom, so a verb could exist
  on one and not the others. That kept happening and nothing caught it: `vote` shipped
  on the command line alone, `append` had no socket method for as long as the socket
  existed, and a test asserting `issue/fold` was an unknown method became wrong the day
  fold got one. The reference-completeness tests stayed green through all of it, because
  they checked that the docs listed what existed rather than whether what should exist
  did.

  Three tests now read the schema and fail by name until their surface satisfies it.
  Adding a verb to the schema that nothing implements fails all three at once, each
  naming the gap on its own surface.

  A verb that should not reach a surface leaves that field empty and says why in `note`,
  so a deliberate omission reads differently from a forgotten one.

  The schema's constant is compiled into a committed Rust file, so the checks read the
  schema's own bytes rather than a list maintained beside it. `capnp` is not needed to
  build or test; regenerating after a schema edit is a maintainer step, written down in
  `schema/README.md`.
- `vissue surface` is documented: a row in the reference's command table, its JSON shape
  field by field, why it is hidden and why hidden is not secret, and a how-to for reading
  it from a wrapper instead of parsing help text.

  The check that holds the reference to the command list now reads the parser's own list
  rather than help output, so it covers hidden subcommands too. `--help` omitting a verb
  is a choice about what a person reading help wants, and says nothing about whether the
  verb needs writing down.
- `vissue vote ID --for CHOICE` records one ballot per agent identity, and
  `vissue vote ID` prints the tally.

  Several agents working one tracker had no way to disagree on the record. Each
  could append prose saying what it concluded, and a reader had to read every
  append and count by hand. Ballots live in a `:VOTES:` drawer on the heading, one
  line per identity, so the file shows who thinks what.

  One ballot per identity, and casting again replaces it rather than adding a
  second, so an agent that reconsiders does not appear twice. The tally separates
  consensus from a plurality and from a tie, because a count that calls all three
  agreement is worth nothing.
- `vote` is reachable from all three surfaces: `vissue vote` on the command line,
  `vissue_vote` over MCP, and `issue/vote` on the control socket.

  The feature exists for agents rather than for a person at a prompt, and agents
  reach the tracker through MCP and the socket. A vote only on the command line
  would have been a tally with nothing able to cast into it.

  Socket and MCP ballots name the calling agent rather than the server process, so
  one server serving several agents records which of them voted.

### Changed

- An id's starting suffix is now derived from the project *and the issue's title*,
  so minting is a function of its inputs rather than of the clock.

  The same project, title and seed ask for the same id every time, which makes a
  mint replayable. It also keeps two agents apart without coordination in the
  ordinary case: two creates with different titles start from different points in the
  space, so neither has to see the other's write to avoid it. Two agents asking for
  the same title at the same moment do want one suffix, and the reservation settles
  that, the first taking it and the second walking one along.
- Id suffixes come from xxh3 over the project name, keyed by a seed, then a
  one-at-a-time walk through the base-36 space. `VISSUE_ID_SEED` pins the seed and
  makes a minting sequence reproducible.

  The previous derivation was a single multiply by Knuth's constant over a
  nanosecond clock, taking base-36 digits off the low bits and placing the next
  probe 17 away from the last. Successive probes correlated, and two processes
  reading the same coarse clock walked the same short arithmetic sequence. This
  crate already depended on xxh3 for the export digest, so the better start cost no
  new dependency.

  The walk matters as much as the hash. A first version hashed each probe
  independently, which draws with replacement and can miss a free suffix that
  exists: with 1295 of 1296 taken, 2592 draws find the survivor about six times in
  seven, and a test that had to find it failed one run in seven. Stepping visits
  each suffix once.

  Pinning the seed is what gives a racing-creates test power. With a clock seed two
  racers draw from 36^4 and never collide by luck, so such a test passes whether or
  not the id reservation is read under the lock; pinned, it fails on every id when
  the reservation is stale.
- The HUD task board only mounts the rows on screen. j/k keeps the selected card in view.
- The checks that hold the MCP tools to `schema/vissue.capnp` ask the server over
  stdio instead of reading `server.rs` as text. One `tools/list` handshake returns the
  same answer an agent gets, so an argument is checked by the name and JSON type it is
  advertised under, along with whether a caller may omit it.

  This catches a class the source scan could not. A `#[serde(rename)]` on an argument
  leaves the Rust field name in place, so scanning the struct for it passed while
  agents saw a different name.
- The checks that hold the command line to `schema/vissue.capnp` ask the parser what
  it accepts instead of parsing what it prints. A hidden `vissue surface` walks the
  built `clap::Command` and emits every subcommand with its aliases and long flags as
  JSON, so a flag reaches a check because the parser takes it rather than because a
  line of help spelled it in a recognisable way.
- The checks that hold the control socket to `schema/vissue.capnp` ask a running owner
  instead of reading `dispatch.rs` and `rpc.rs` as text. A parameter is present because
  the method refuses a value of the wrong type for it, and required because the method
  refuses the request without it. Every request is built to fail at decode, so a
  mutating method can be asked this without writing anything.
- The command line's dispatch is one call per verb. Eight reads that answer in two shapes
  went through a single `emit_shape`, which is where the choice between the structured
  value and the report is now made, and the eight arms carrying real nesting are functions
  with names: the keymap, the two waits, the mirror, the HUD, the identity report, the
  digest, one issue, and the project list.
- The corpus validator is a sequence of named checks rather than one four-hundred-line
  function: one project's preamble, one project's headings, one issue, one provenance
  link, one parent walk. Findings accumulate through a small type carrying the text and
  the two counts together, so a finding cannot be written without being counted, which
  is how the exit code and the report could have disagreed.
- The relatedness scorer is four named scorers over one candidate set: shared words by
  inverse document frequency, distance along declared edges, the relations between the
  target and one other issue, and what the pair have in common. The six copies of the
  same `entry().or_insert_with()` incantation are one `bump` call each.
- `clippy::cognitive_complexity` runs at clippy's own default of 25 rather than the
  ratchet's 40. Nothing in the workspace sits above it.

### Fixed

- A read-modify-write cycle over several files now counts a file once however its
  path was spelled, and four correctness fixes land in `vote`.

  The lock helper deduplicated by the path as written while keying its mutex on the
  resolved path. Two names for one file therefore took the same non-reentrant mutex
  twice and hung, and took a second advisory lock on the same file besides. Latent
  while callers passed one or two known-distinct files; reachable now that a mint
  locks every twin file for a project, where two configured roots can be links to
  one tree.

  In `vote`:

  - A single ballot is no longer called a consensus. One agent agreeing with itself
    is not agreement, and reporting it as such is how one unreviewed opinion gets
    acted on as though it had been checked.
  - An identity containing a colon and a space is refused. A choice may contain one,
    which is why the ballot line splits on the first, so such an identity would have
    been read back as a shorter name with the rest of itself attached to the choice:
    the ballot filed under an agent that never voted, silently.
  - Lines the parser does not recognise are kept. The drawer is org a person can
    edit, and a rewrite keeping only recognised lines ate a comment left there.
  - Two hand-written lines for one agent collapse to the last, so a duplicate cannot
    make one voter count twice.

  The votes drawer is also rewritten in place rather than removed and appended, so a
  vote no longer reorders the other drawers on the heading.
- Concurrency is now covered by tests that were each checked against the lock they
  guard, rather than by a paragraph in the reference.

  Three cross-process tests: every mutating verb at once against one file, creates
  racing across two roots for one project name, and separate processes emitting
  change events. Each was run with its lock removed to confirm it fails. Without the
  file lock, three of eleven headings vanish. Without the advisory half of the events
  lock, event sequences duplicate heavily.

  The events test needs one project per subject to have any power. With every subject
  in one file the issues lock already serialises the pipeline and the events lock
  never contends, so a first version using one project passed with the advisory lock
  removed and proved nothing.

  Two of these are labelled in the source as smoke tests rather than guards. The
  two-root create test cannot detect a duplicate id at the default id length, because
  the suffix space is 36^4 and two racing creates almost never collide by luck; the
  guard with power for that is the deterministic reservation test in `vissue-core`.
- Every read that answers with a report is compared against its subcommand: `export`,
  `graph`, `roadmap`, `cycles`, `count`, `check`, `stale`, `hygiene` and `waiting-on`.

  Reading the handlers was not a mechanism. `issue/mirror` answered with the corpus
  digest while every name and type check agreed with it, and it was found by eye.

  Nine agree byte for byte. `ping` cannot: it appends to the event log, so two calls
  report two sequence numbers, and equality is the wrong instrument for a read with a
  side effect. Its shape is asserted instead, and the schema row says why it changes
  something while remaining a non-mutating verb: it changes no issue.

  `stale` defaults to thirty days on the command line and has no default on the socket,
  so the test passes the number to both. A default that differs between surfaces is
  itself a divergence, and leaving it implicit would have hidden one.
- The `vissue_create` tool takes `deadline` and `scheduled`, which the `create`
  subcommand has always taken.

  An agent could not set a date a person could. Nothing was wrong on either side in
  isolation: the subcommand had the flags, the tool had a coherent argument list, and
  both were documented. Only a check across the pair finds a field one surface takes
  and the other does not, and that check now exists.
- The closed-pipe test builds its corpus by byte count rather than issue count. It
  needs more output than a pipe buffer holds, which twenty-four large bodies reach in
  a fraction of the work four hundred short titles took, and it now asserts the
  corpus really does exceed the buffer so the test cannot quietly stop testing
  anything.
- The schema records the parameter each of six methods is actually about. `issue/create`
  took `title`, `issue/search` took `query`, and `issue/tree`, `issue/ancestors`,
  `issue/impact` and `issue/related` took an id, none of which appeared in any row:
  `Field` was written around flags, and these arrive as positional arguments on the
  command line. The tool spells the id `issue_id` and the socket spells it `id`, which
  is the kind of divergence the schema exists to record and did not.

  Fields also carry `omittable`, for a parameter a caller may leave out whatever the
  Rust type says. `force`, `dry_run`, `last` and `projects` are plain types behind
  serde's `default`, so they read as required while every caller omits them.
- Two concurrent `create`s for one project in two roots can no longer mint the same
  id, and neither can two concurrent `reject`s minting a successor.

  The reservation that stops a twin file sharing a suffix was read by the caller
  and used by the mint. Locks are per file, so two creates in different roots took
  different locks, each read the other before either had written, and both could
  choose the same suffix. A duplicate across layouts is not cosmetic: `find_by_id`
  reports `DuplicateId` for it and the issue stops being reachable by id.

  The reservation is now a list of paths rather than a list of ids. Those files are
  locked alongside the one being written and read after the locks are held, so a
  peer's create lands wholly before or wholly after. The file being written
  appearing in its own reservation list is ordinary, because the routed lookup
  returns every layout for the project including this one, and the lock helper
  sorts and dedups.
- `check` reports a heading whose `:ID:` belongs to an org-gcal event. It counted those
  among the parsed headings, and the loader skips them precisely because a calendar sync
  owns them, so the count was always zero. Counting the file instead reports what is
  actually there, which matters because such a heading sits in the tracker and vissue
  will not touch it.
- `claims` and `agenda` no longer print their empty-set line once per project.

  "Nothing here" is an answer about the corpus, not about a project's share of it.
  Over six projects, `claims` printed "no live claims" above a list of nine claims,
  and `agenda` printed "nothing dated in range" five times above the dated rows.
  Asked about one project the line is the whole answer and stays; asked about all
  of them it appears once, and only when every project came back empty.
- `initialize` advertises every method the server answers. It was nineteen behind.

  The capability list is the fourth place the method set is written down, after the
  dispatch table, the schema and the reference, and it is the one a client reads to
  decide what it may call. While the other three agreed with each other this one fell
  behind, so a client inspecting capabilities would have concluded that `append`,
  `vote`, `fold` and every read added beside them did not exist.

  A test now holds it to the schema in both directions, since either way round is a lie:
  advertising a method that is not dispatched sends a client at a method-not-found, and
  dispatching one that is not advertised hides it from anyone who asks first.

  The `Method` enum grew the same nineteen. Two exhaustive matches then refused to
  compile, which is the arrangement working: the typed request and response helpers have
  no form for these, and they now say so per method rather than through a wildcard,
  because a wildcard would swallow the next one silently.

  `control.org` said `issue/check`, `issue/export`, `issue/graph`, `issue/mirror`,
  `issue/roadmap`, `issue/hygiene` and `issue/fold` were "not in v1" and stayed on the
  command line and MCP. All seven are methods now, and the page lists every one.
- `issue/digest` reports the event-log generation, which `digest --json` has always
  carried and the method omitted.

  A client comparing digests across time needs to know which generation each was taken
  at, and had no way to see the field was missing. Found by comparing the structured
  reads against the command line's `--json` mode, the same way the report-producing reads
  are compared.

  One divergence is intended and is now pinned as such. `issue/list` sorts by priority,
  then state, then id across every project in the layout, while the `list` subcommand
  applies that order within each project and concatenates, because it can span several
  layouts and a global sort across them would interleave two trackers. Same rows,
  different sequence, and the terminal UI relies on the socket's. The test compares the
  rows as sets and says why.
- `issue/graph` and `issue/roadmap` group their output the way the subcommands do, and a
  test now compares the socket's report against the subcommand's text.

  This is the class of bug the name and type checks cannot see. `issue/mirror` answered
  with the corpus digest while the schema, the types and the reference all agreed with
  each other, because none of them compares content.

  The graph divergence was the same shape and cosmetic: `report::graph` over a whole
  layout emits every node and then every edge, while the command line emits each
  project's nodes and edges together, because it builds one document out of per-project
  bodies. Fourteen identical lines in a different order. Two surfaces answering one
  question differently is the kind of thing nobody notices until they diff it.
- `issue/mirror_check` reports whether a mirror file's stamp is still fresh. It briefly
  answered with the corpus digest instead, under the name `issue/mirror`.

  Two mistakes in one method. It answered a different question — a hash rather than a
  verdict — so a caller asking whether its mirror was current had no way to tell it had
  been told something else. And it conflated two operations that the tool list has always
  kept apart: `vissue_mirror` renders a mirror, `vissue_mirror_check` judges one.

  Rendering has no socket method, and the reason is on the schema row: it writes a file,
  and a file the server writes lands on the server's disk rather than the caller's, so a
  remote client would get a success and no file.

  The reply carries `fresh` beside the report, because the subcommand exits non-zero on a
  stale mirror and a client needs that signal without reading prose.
- `q` has its own schema row instead of being recorded as an alias of `create`. It is
  a subcommand of its own taking three of create's fields, so an alias row answered
  for flags it rejects: `--body` and `--priority` among them. Rows now carry
  `shorthandFor`, and a shorthand has to name a verb that exists, reaches the socket
  if the shorthand mutates, and takes every field the shorthand takes.
- `vissue check` no longer reads the word "rejected" anywhere in a body as an
  issue that was rejected, and no longer asks for a `DISCOVERED_FROM` edge between
  two issues a `:PARENT:` or blocker edge already connects.

  Both were substring rules that do not match how issues get written. Every bug
  about input validation says "rejected" — "silently corrupted rather than
  rejected", "a hand-written parser is rejected as strictly dominated" — and three
  worked-and-closed issues in one corpus were flagged for exactly those sentences.
  A rejection is now recognised by the shapes one is written in: the tool's own
  phrasing, "superseded by", "rejected in favour of", or a heading that names the
  outcome. And a parent naming its child is a stated relation the tracker already
  holds, so it is no longer a warning that can only be answered with a wrong edge.

  The mention warning is narrower on both sides now: the edge may be `:PARENT:` or
  `:BLOCKED_BY:` as well, and the prose near the link has to claim a discovery or a
  pivot. A body links other issues for every reason there is, and in one corpus
  twenty-two such warnings stood and not one of them was a discovery.


## [0.7.0](https://github.com/HaoZeke/vissue/releases/tag/v0.7.0) - 2026-08-18

### Changed

- The HUD board opens on the project list. Inside a project the selected
  issue stays on screen, with properties above a wrapping body and tree,
  related, and notes as tabs. Search is a field. List titles wrap inside
  the pane. The tree tab expands or collapses the outline. Related cards
  and tree double-click open that issue. Escape on the project list
  unmaps the overlay; `vissue hud --toggle` shows it again. Closing the
  mapped window quits.

### Fixed

- A HUD Tree pick is the issue keys act on. Confirm remembers that
  issue through a reject on Ready. Hide then show remaps the overlay.
  Add from home search writes into the selected hit's project.
- The HUD list checkbox only flips TODO and DONE and selects that
  issue. Cancelled and blocked stay as they are. The overlay no longer
  stalls after a file write.


## [0.6.0](https://github.com/HaoZeke/vissue/releases/tag/v0.6.0) - 2026-08-18

### Added

- Every `issues.org` carries `#+VISSUE: 1`, the on-disk protocol
  number. It is independent of the crate version and of the
  control-socket `protocolVersion`. `normalize` writes or upgrades it.
  `check` names a missing stamp and errors on a future number.
  `identity` prints `protocol: 1`.
- The Org ecosystem page lists GNU ELPA, org-contrib, and MELPA
  names a rewrite must own, read, preserve, or ignore. Protocol 1
  is the tracker contract, not parity with org-noter. Collisions
  called out: org-gcal `ID`, org-journal `CREATED`, org-gtd
  `TRIGGER` / `CATEGORY`.
- The Org syntax map states protocol 1 is the tracker contract, not
  parity with every ELPA package. org-noter (`NOTER_DOCUMENT`,
  `NOTER_PAGE`), Interleave's older names, and org-roam
  (`ROAM_REFS`, `ROAM_ALIASES`) are preserved on rewrite.
- `#+SETUPFILE:` (local files only) merges TODO keywords, tags, and
  `#+PRIORITIES:` into the file that names it. A URL is not fetched.
  A missing or cyclic setup file is skipped so the tracker still
  parses.
- `vissue normalize` rewrites a tracker onto the Org and ELPA
  property names the other tools already read: `#+CATEGORY:`, type
  as a heading tag, typos (`BLOCKEDBY`, drawer `TAGS`) folded, and
  a bare `:BLOCKER:` id list moved to `:BLOCKED_BY:`. `--dry-run`
  prints the files that would change. An org-edna condition stays.
  vissue does not mint `:BLOCKER: ids(...)`.

### Changed

- A fresh tracker declares Org's tag and publish keywords:
  `#+FILETAGS:` includes `noexport`, `#+TAGS:` has the type group
  plus `docs` / `perf` / `ignore` / `ARCHIVE`, and
  `#+EXCLUDE_TAGS:` / `#+SELECT_TAGS:` are written. A rewrite appends
  `noexport` to an existing FILETAGS line that lacks it. `search`
  matches inherited FILETAGS and a group tag. An Org-format `mirror`
  drops a heading tagged `noexport`. `check` names a file that still
  lacks those lines.
- A parent with `:ORDERED:` holds later children with the same
  `:PARENT:` out of `ready` until every earlier sibling is `DONE` or
  `CANCELLED`. `:NOBLOCKING:` on a child skips that wait. `check`
  names a heading that started or closed early, and a `DONE` that
  still has open children.
- Type is an Org heading tag when the character class allows it
  (`:bug:`, `:feature:`, `:task:`). A write inserts a missing
  `#+CATEGORY:` and `#+FILETAGS:` so the agenda does not label every
  row `issues`. `vissue check` names a file still in the drawer-only
  shape.
- `#+PRIORITIES: highest lowest default` sets the cookie range and the
  value used when a heading has no `[#X]`. A fresh file writes
  `#+PRIORITIES: A C C`. `create` without `--priority` uses that
  default; when the file has no such line it uses
  `issues.default_priority`. A cookie outside the range is refused.
- `:BLOCKER:` is GNU ELPA org-edna (and org-depend), not a typo
  for `:BLOCKED_BY:`. A bare id list still feeds `ready` and is
  rewritten to `:BLOCKED_BY:`; an edna condition stays. `Effort`
  is recognised. `check` names computed specials written in a
  drawer, `:BLOCKEDBY:`, and an Effort value Org will not parse.

### Fixed

- An org-gcal `:ID:` of the form `<event>/<calendar>` is not an issue
  id. The heading stays in the file around the real issues.
  `find_org_ids` and `collect_org_ids` skip slash ids. `check` errors
  if a parsed issue still carries one.
- Org Babel syntax is recognised without being evaluated. A
  `#+RESULTS:` payload, a `#+CALL:` line, affiliated `#+NAME:` /
  `#+HEADER:` keywords, inline `src_lang{}` / `call_name()`, and
  noweb `<<name>>` no longer split an issue or define a ghost id.
- The `issues.org` parser now follows Org 9.8 on the constructs that
  used to fail a file or split an issue: greater and dynamic blocks are
  literal, a planning line accepts timestamp ranges and repeaters,
  drawers may arrive in any order, `COMMENT` and notes headings stay
  Org rather than becoming a missing `:ID:`, and file-local `#+TODO:`
  keywords are recognised. See `docs/orgmode/org-syntax.org`.


## [0.5.0](https://github.com/HaoZeke/vissue/releases/tag/v0.5.0) - 2026-08-18

### Added

- A user-level `~/.config/vissue/config.toml` can send named projects to
  another checkout. A route wins over `--root` and `VISSUE_ROOT`, so a
  process whose default root is one tracker can still create and show
  issues that live on another. `--no-route` and `VISSUE_NO_ROUTE=1` keep
  every verb on the process default. `projects` and `identity` list the
  routed names; `show` and `claim` find an id on any configured layout.
  `vissue.el` still parses the original `identity` and `projects` lines;
  route lines are appended.

  `refile` and `reject` route their destination as well, so a bounce onto a
  routed name lands on that name's tracker. `serve`, and the TUI and HUD
  that talk to it, stay on the single layout the server was started with.
- `vissue hud` is an overlay. On Sway it floats and centers itself over
  the compositor IPC socket. No Sway include or `for_window` rule.
- `vissue reject` closes a heading as CANCELLED and writes a successor
  edge (`PIVOTED_TO` / `DISCOVERED_FROM`) in one edit, so a bounce is a
  walkable wire rather than a body mention. `vissue wait --id --until-terminal`
  blocks until that id is DONE or CANCELLED (exit 2 on timeout). State
  changes emit a `state_change` event that names the id.
- `vissue update --if-state` / `--if-gen` refuse a write when the heading
  or corpus moved since the caller last read it. Disagreeing terminals
  keep the first close and record `:SIBLING_TERMINAL:`; `vissue resolve`
  picks one. `vissue check` warns on a DONE that reads as a reject and on
  a body `[[id:]]` with no successor edge.

### Fixed

- A freshly started `serve` owner is now waited on for 15 seconds rather than
  5, and `VISSUE_ACCEPT_TIMEOUT_MS` overrides that. A cold start binds its
  socket only after building a runtime and loading the tracker, so a loaded
  machine could reach the old deadline and be reported as a spawn failure.
  The HUD's detach path reads the same deadline.


## [0.4.1](https://github.com/HaoZeke/vissue/releases/tag/v0.4.1) - 2026-08-16

### Fixed

- `vissue-serve` compiles on Windows again. The `notify` crate is a
  Unix-only dependency; converting `notify::Error` is now gated the
  same way.


## [0.4.0](https://github.com/HaoZeke/vissue/releases/tag/v0.4.0) - 2026-08-16

### Changed

- Library crates return typed errors instead of `anyhow::Result`.
  `vissue-core` exposes `Error` and `Result`; `vissue-tui` and
  `vissue-serve` do the same. The CLI, MCP server, and HUD still
  use anyhow at the process edge. Match on the enum; do not parse
  the display text.
- The workspace uses Rust edition 2024. The published MSRV is still 1.89.


## [0.3.0](https://github.com/HaoZeke/vissue/releases/tag/v0.3.0) - 2026-08-15

### Added

- `vissue append <id>` records a dated, attributed report under an issue's
  heading, from `--text`, a `--file`, or stdin. A body could previously only be
  set at `create`, and `note` folds its text to a single line by design, so
  there was no way to write down the result of work. Markdown is safe. The MCP
  tool is `vissue_append`.
- `vissue hud` is a summonable task board. It opens on the project list;
  entering a project shows that project's ready forest, then List / Claims /
  Agenda / Search within it, with show / excerpt / tree / related / notes on
  the selected row. The window execs a separate `vissue-hud` binary, so the
  CLI does not carry the GUI dependencies.

  First paint reads the files. Unless `--offline`, the board attaches to
  `vissue serve`, starting it when the socket is free, and falls back to the
  files when serve is down or bound to another root. `--toggle`, `--show`
  and `--hide` talk to a summon socket, so a keybinding can raise the board
  that is already running rather than starting a second one.

  `--rofi` gives the seat dmenu picker instead, over the set named by
  `--mode`: Return opens the heading in `$EDITOR`, Alt+c claims, Alt+n
  notes. Keys come from a catalog that `~/.config/vissue/keys.toml` or
  `VISSUE_KEYS` remaps; `vissue keys` prints it and `--check` validates an
  overlay.
- `vissue serve` now caches the issue catalog and answers the v1 control
  methods (`issue/list`, `issue/ready`, `issue/claim`, and the rest).
  Attached clients receive `vault/changed` after a rebuild. The files stay
  the store: stopping serve loses nothing.
- `vissue show` prints the body, and `show --org` writes the heading out whole
  (property drawer, logbook, and body) for handing an issue to someone as the
  thing to work from. `IssueDetail` carries the body, so `--json`, the MCP
  tools, and the control protocol answer with what the issue asks for rather
  than a file path and a line range. Prefer `show --org` over `body-excerpt`
  when the text is the specification: the excerpt is a preview and stops at 40
  lines. The MCP tool is `vissue_org`.
- `vissue tui` is an interactive board over ready, list, claims, agenda,
  and search. First paint reads the files. Unless `--offline`, the board
  attaches to `vissue serve` (starting it when the socket is free) and
  falls back to the files when serve is down or bound to another root.

### Changed

- Publication uses crates.io trusted publishing on a `v*` tag, with no registry secret in the repository.
- The minimum supported Rust version is 1.89, which is what the iced board's
  dependencies require.
- `check` only searches the wider tree for `:PARENT:` ids that are not
  issues. It validates parents against any Org id, which includes design
  documents and notes, so on a tracker sharing a root with a notes vault it
  read every `.org` file to answer a question the issues had usually
  already answered. Where every parent is another issue the scan is now
  skipped entirely: 58ms to 23ms on a 35MB corpus. A tracker that does
  point at a design document still pays for the search, which now stops as
  soon as the ids it wants have been found.

### Fixed

- A body containing a line that starts with `* `, which any markdown bullet
  list does, no longer cuts the issue in two. The parser splits issues on that
  line, so such a body used to leave a heading with no `:ID:`, stop the file
  parsing, and drop every issue in that project out of `list`. Those lines are
  now indented by one on the way out; `** Scope` and deeper are left alone,
  being children of the issue rather than the end of it.
- A client of `vissue serve` sees its own write. The catalog was rebuilt only
  by the file watcher, so for up to about 450ms after a mutation the writer was
  answered from the catalog as it stood before: `issue/list` came back one
  short, and `issue/get` on the id `issue/create` had just returned reported
  the issue as missing. The mutation path refreshes before it answers and
  reports the resulting revision.
- An idle tracker no longer rebuilds its catalog and broadcasts `vault/changed`
  several times a second. The generation poll opens the projects directory on
  every tick, inotify reports that open against the watched tree, and a read
  was counted as a change, so the rebuild's own reads raised the next event.
- `check` now reads `:ID:` only from a property drawer under a heading, where
  org defines one. A report appended to an issue body quotes the heading it
  describes, and a quoted id used to count: a `:PARENT:` pointing at nothing
  resolved against the mention, and the check that exists to catch broken links
  went quiet.
- `digest` and `mirror --check` read the corpus once rather than once per
  project. Each project's digest came from a whole-corpus export filtered
  down to that project, which is quadratic in the project count: on a
  tracker with 115 projects and 4781 issues those commands took 6.2s, and
  now take 0.16s. The digest values are unchanged, so mirrors stamped by an
  earlier version still read as fresh.

### Developer

- The catalog query surface and the typed error enum are covered by tests.
- The seams between the parts have tests of their own. The client and the
  server now talk to each other over a real socket, which is where
  a mutation the server accepted but did not make visible to the next read
  used to hide; the Model Context Protocol (MCP) server is driven the way
  an agent meets it, a process on the other end of a pipe; `serve -d` runs
  as a real daemon; and Org decides whether a markdown body stayed body,
  rather than the parser deciding that about itself.

  Concurrent writers are covered: with the advisory lock removed, twelve
  overlapping `create` calls lose between two and six of themselves while
  every writer reports success, and nothing in the suite noticed before.

  Three drift guards fail rather than rot: every command and MCP tool must
  appear in the reference, every advertised control capability must be
  routed, and the release must cover every publishable crate in an order
  taken from the manifests.

  `cargo test` runs with `--no-fail-fast`, so a red run names every broken
  target instead of stopping at the first.


## [0.2.0](https://github.com/HaoZeke/vissue/releases/tag/v0.2.0) - 2026-08-14

### Added

- A documentation site at [vissue.rgoswami.me](https://vissue.rgoswami.me), written as Org under `docs/orgmode/` and rendered with Sphinx. The pages keep to one Diataxis quadrant each, and the README is a front door rather than the whole manual. The site is a managed Cloudflare Pages target, deployed with `ttech-ops site deploy vissue`.
- Logo mark: a tessellated V of issue nodes with one ready cell, plus a circular crest and wordmark lockup under `assets/`.
- MCP tools for `ancestors`, `impact`, `cycles`, `refile`, `wait`, and `whoami`. `identity` also emits `root=` / `prefix=` tokens.
- `vissue completions <shell>` and `vissue man`, generated from the CLI definition so they cannot drift, with copies committed under `completions/` and `man/`.

### Changed

- **Compatibility**: a tracker written by this version is not readable by 0.1.0, which fails on the planning line with ":ID: property missing". Upgrade every reader of a tracker together. A second tool that writes an `issues.org` also needs to place a `:LOGBOOK:` drawer below the planning line, or it breaks the file for every reader including `vissue`.
- A new project file carries `#+CATEGORY: <project>`. Org otherwise takes the agenda category from the file name, and every project's file is `issues.org`, so a multi-project agenda labelled every row `issues`.
- Dates and tags move to where Org keeps them. `DEADLINE`, `SCHEDULED`, and `CLOSED` render on the planning line under a heading, and a tag Org can hold renders in the heading's own tag run, so `org-agenda`, Org tag search, and `org-lint` all work against a tracker. Both shapes parse, so existing files read the same and settle on the Org shape when next rewritten.
- JSONL export and `show --json` carry `org_tags` and a `tags` union.
- The README is a front door rather than the manual: what vissue is, how to install it, a minute of it working, and links into the documentation site. The tutorial, how-to, reference, and explanation move to their own pages.
- The tag property is `:VISSUE_TAGS:`. `TAGS` is a name Org reserves and `org-lint` reports; a drawer written under the old name is migrated on parse.
- `tests/org_interop.sh` drives Emacs over a tracker in CI: org-lint, org-agenda, tag search, and org-id all have to agree, and Org's own edits have to survive a vissue rewrite.

### Fixed

- A `fold` that fails partway stamps the issues it did create, so rerunning does not create them twice.
- A heading whose priority cookie holds a multi-byte character no longer panics the parser.
- A prefix-scoped `issues.config.toml` overrides `vissue.toml` key by key instead of resetting the keys it does not name.
- A reader that closes the pipe, as `vissue export | head` does, exits 0 instead of panicking with status 101.
- An Org planning line no longer hides the property drawer below it. Writing a deadline, a scheduled date, or a `CLOSED` stamp from Emacs used to make every verb fail with ":ID: property missing" for the whole project file.
- An `issues.org` is flushed to the device before the rename publishes it.
- Auto-unblock to TODO releases the claim.
- Event generation and log append take a file lock.
- JSONL export and digest include CLOCK `raw` logbook lines.
- Text `ready` now uses the corpus-wide open-blocker set, matching `count --ready` and JSON ready.
- The `body-excerpt` secret screen reads credential shapes rather than three substrings, and matches vendor token prefixes on a whole word: the old form would have flagged "making" as an AWS key had that prefix been listed.
- The acyclicity check for `--block` reads the corpus inside the lock, so a peer write that landed first is part of the answer.
- `--project` folds case in `list`, `count`, `ready`, `export`, `graph`, `roadmap`, and the JSON rows, matching `claims`, `agenda`, and every verb that writes.
- `check` counts a duplicate id as an error, and reports a `:PARENT:` cycle. Every id in a parent loop resolves, so the edge checks passed it while the hierarchy stayed unwalkable and `tree` printed "(cycle, stopping)".
- `create` reports a full id space instead of panicking inside the write. `id_length = 2` is 1296 suffixes, which a project can outgrow.
- `events::tail_report` counts log lines rather than sequence numbers, so a debounced burst no longer shortens the tail.
- `graph` and `tree --format dot` escape backslashes and newlines in titles and ids, so issue text cannot become DOT syntax.
- `hygiene` matches issues by id rather than by row prefix.
- `mirror --out` writes through a flushed temporary and a rename, so a reader mid-pull cannot see a half-written projection.
- `note` adds its entry at the top of the logbook, where state transitions and claim releases already go.
- `org-set-tags` in Emacs no longer folds the tag run into the issue title.
- `refile` writes the target file before removing the heading from the source, so a failed write cannot lose the issue.
- `search` and `related` read tags from the heading as well as the drawer.


### Developer

- The Emacs interop check reports the error when Emacs itself fails, instead of exiting with a bare status and no message.


## [0.1.0] - 2026-08-12

### Added

- Org-backed issue storage, queries, graph operations, claims, events, mirrors,
  and JSONL export through the `vissue` CLI.
- MCP server exposing the same issue operations over stdio.
- Release preparation, package validation, and cross-platform archive wiring.

[0.1.0]: https://github.com/HaoZeke/vissue/releases/tag/v0.1.0
