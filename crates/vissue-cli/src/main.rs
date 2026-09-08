//! `vissue`: plain-text issue tracking over per-project orgmode files.

#![allow(
    missing_debug_implementations,
    missing_docs,
    rustdoc::missing_crate_level_docs,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]

use anyhow::{Context, Result, bail};
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use std::io::{Read, Write};
use std::path::PathBuf;

use vissue_core::config::Layout;
use vissue_core::mirror::{self, Format};
use vissue_core::ops::{self, CreateOpts, RejectOpts, UpdatePred};
use vissue_core::router::Router;
use vissue_core::store;
use vissue_core::{agent, events, report};

mod rofi;

/// Write to stdout, surfacing a closed pipe as an error the caller handles.
///
/// The `print!` family unwraps the write and aborts the process instead, so
/// `vissue export | head` ends in a panic and a 101 exit status rather than
/// the answer the reader asked for.
macro_rules! emit {
    ($($arg:tt)*) => {
        write_stdout(format_args!($($arg)*))?
    };
}

/// [`emit!`] with a trailing newline.
macro_rules! emitln {
    ($($arg:tt)*) => {
        write_stdout(format_args!("{}\n", format_args!($($arg)*)))?
    };
}

fn write_stdout(args: std::fmt::Arguments<'_>) -> Result<()> {
    std::io::stdout().lock().write_fmt(args)?;
    Ok(())
}

/// Whether a failure is a reader that closed the pipe, which is how `head`
/// and `less` say they have seen enough.
fn is_broken_pipe(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
        .any(|io| io.kind() == std::io::ErrorKind::BrokenPipe)
}

#[derive(Parser)]
#[command(
    name = "vissue",
    version,
    about = "Plain-text issue tracking over per-project orgmode files"
)]
struct Cli {
    /// Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory.
    #[arg(long, global = true)]
    root: Option<PathBuf>,

    /// Directory under the root holding one subdirectory per project. Falls
    /// back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`.
    #[arg(long, global = true)]
    prefix: Option<String>,

    /// Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep
    /// every verb on the process default layout.
    #[arg(long, global = true)]
    no_route: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create an issue. Pass the body with --body or --body-file (`-` reads
    /// stdin); omit both to leave the body empty for a later edit.
    Create {
        /// One-line title
        title: String,
        /// Project name. Auto-detected from .project-ctx.toml when omitted.
        #[arg(short = 'p', short_alias = 'P', long)]
        project: Option<String>,
        /// Priority cookie: A high, B mid, C low
        #[arg(long)]
        priority: Option<char>,
        /// Type tag such as feature, bug, or task
        #[arg(short = 't', long = "type")]
        issue_type: Option<String>,
        /// Org deadline like `<2026-05-15 Fri>` or `[2026-05-15]`
        #[arg(long)]
        deadline: Option<String>,
        /// Org scheduled date like `<2026-05-01 Mon>`
        #[arg(long)]
        scheduled: Option<String>,
        /// Comma- or colon-separated tags
        #[arg(long)]
        tags: Option<String>,
        /// Parent id, which must already exist
        #[arg(long)]
        parent: Option<String>,
        /// Print only the new id
        #[arg(short, long)]
        quiet: bool,
        /// Body text written under the heading
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// Read the body from a file; `-` reads stdin
        #[arg(long)]
        body_file: Option<String>,
    },
    /// Quick capture: create and print only the id.
    Q {
        /// One-line title
        title: String,
        #[arg(short = 'p', short_alias = 'P', long)]
        project: Option<String>,
        #[arg(short = 't', long = "type")]
        issue_type: Option<String>,
        #[arg(long)]
        parent: Option<String>,
    },
    /// List issues, sorted by priority then state then id.
    List {
        #[arg(short = 'p', short_alias = 'P', long)]
        project: Option<String>,
        /// Filter by state: TODO, STARTED, BLOCKED, DONE, or CANCELLED
        #[arg(short, long)]
        state: Option<String>,
        /// Emit JSON rows instead of text
        #[arg(long)]
        json: bool,
    },
    /// Show one issue: metadata, then the body.
    Show {
        id: String,
        /// Emit a JSON object instead of text
        #[arg(long)]
        json: bool,
        /// Emit the heading's org text in full, nothing else. Use this to
        /// write the issue out as the specification someone works from.
        #[arg(long, conflicts_with = "json")]
        org: bool,
    },
    /// Update state, priority, or blocker edges.
    Update {
        id: String,
        #[arg(short, long)]
        state: Option<String>,
        #[arg(long)]
        priority: Option<char>,
        /// Add a blocker edge
        #[arg(long)]
        block: Option<String>,
        /// Remove a blocker edge
        #[arg(long)]
        unblock: Option<String>,
        /// Refuse unless the heading is still this state
        #[arg(long)]
        if_state: Option<String>,
        /// Refuse unless the corpus generation is still this value
        #[arg(long)]
        if_gen: Option<u64>,
    },
    /// Pick one terminal after a sibling close.
    Resolve {
        id: String,
        #[arg(short, long)]
        state: String,
    },
    /// Reject an issue, redirecting to an existing destination or a new replacement.
    Reject {
        /// Issue id to reject
        id: String,
        /// Existing destination issue
        #[arg(long, required_unless_present = "project", conflicts_with = "project")]
        to: Option<String>,
        /// Project for a newly created replacement
        #[arg(
            short = 'p',
            short_alias = 'P',
            long,
            required_unless_present = "to",
            requires = "title"
        )]
        project: Option<String>,
        /// Title of the newly created replacement
        #[arg(required_unless_present = "to", conflicts_with = "to")]
        title: Option<String>,
        /// Why this issue is rejected
        #[arg(long)]
        reason: Option<String>,
    },
    /// Actionable issues: TODO or STARTED with no open blocker.
    Ready {
        #[arg(short = 'p', short_alias = 'P', long)]
        project: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Take an issue: move it to STARTED and stamp the claim.
    Claim {
        id: String,
        /// Take over a claim held by another identity
        #[arg(long)]
        force: bool,
    },
    /// Cast this agent's vote on an issue, or show the tally with no `--for`.
    ///
    /// One ballot per identity and a recast replaces it, so several agents can
    /// disagree on the record and a reader sees whether they agree.
    Vote {
        id: String,
        /// What to vote for. Omit to read the tally without casting.
        #[arg(long = "for", value_name = "CHOICE")]
        choice: Option<String>,
    },
    /// Cite, drop, or list the deeds this issue's work produced.
    ///
    /// A deed is deedar's record of a product: `deedar get <id>` returns it and
    /// `deedar trail <id>` walks what it was built from. The tracker stores the
    /// accession and nothing else, so the next unit opens the work instead of
    /// rereading a transcript.
    Deed {
        id: String,
        /// A deed accession this issue produced. Repeatable.
        #[arg(long = "add", value_name = "DEED")]
        add: Vec<String>,
        /// A citation to drop. Repeatable.
        #[arg(long = "remove", value_name = "DEED")]
        remove: Vec<String>,
    },
    /// The working set for an issue: its plan, its inputs' deeds, and its own.
    ///
    /// What to open before working a node, derived from `:PARENT:`,
    /// `:BLOCKED_BY:`, and `:DISCOVERED_FROM:` rather than retrieved by
    /// resemblance. `related` answers the other question.
    Recall {
        id: String,
        /// Hops of the blocker walk. One is enough when the deeds carry their
        /// own sources, which `deedar trail` walks.
        #[arg(short, long, default_value = "1")]
        depth: usize,
        /// Print only the deed accessions, one per line.
        #[arg(long, conflicts_with = "json")]
        deeds_only: bool,
        /// Include a capped excerpt of each input's heading.
        #[arg(long)]
        excerpts: bool,
        /// Emit a JSON object instead of text
        #[arg(long)]
        json: bool,
    },
    /// Weigh an issue's ballots by who the group listens to (DeGroot).
    ///
    /// `vote` counts. This averages over the trust graph in `[consensus.trust]`,
    /// reports each agent's social power, and says when there is no consensus to
    /// reach rather than reporting one that is not there.
    Consensus {
        id: String,
        /// Roll up over this issue's children instead of reading its own ballots.
        #[arg(long)]
        children: bool,
        /// Exit non-zero when there is nothing settled to act on.
        #[arg(long)]
        gate: bool,
        /// Emit a JSON object instead of text
        #[arg(long)]
        json: bool,
    },
    /// Add a dated note to the top of an issue's logbook; state and claim untouched.
    Note {
        id: String,
        /// The note. Multiple words are joined with spaces.
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
    },
    /// Append a dated report to an issue's body.
    ///
    /// For recording work that was done. The logbook holds one line per
    /// event, so a written report belongs here instead. Markdown is safe.
    Append {
        id: String,
        /// The text to append.
        #[arg(long, conflicts_with = "file")]
        text: Option<String>,
        /// Read the text from a file; `-` reads stdin.
        #[arg(long)]
        file: Option<String>,
    },
    /// Every live claim, oldest first: who holds what, and for how long.
    Claims {
        /// Only claims held by this identity
        #[arg(long)]
        by: Option<String>,
        /// Only claims in this project
        #[arg(short = 'p', short_alias = 'P', long)]
        project: Option<String>,
        /// Machine-readable output
        #[arg(long)]
        json: bool,
    },
    /// Fold an inbox org file: each unstamped `* TODO <title>` heading
    /// becomes an issue, then the heading is stamped with the id and flipped
    /// to DONE in place. Already-stamped headings are skipped.
    Fold {
        /// The inbox file to fold
        file: PathBuf,
        /// Project the folded issues are created in. Auto-detected from
        /// .project-ctx.toml when omitted.
        #[arg(short = 'p', short_alias = 'P', long)]
        project: Option<String>,
    },
    /// Dated open work: deadlines and scheduled starts inside a horizon,
    /// overdue first.
    Agenda {
        /// Days ahead to include
        #[arg(short, long, default_value = "14")]
        days: i64,
        #[arg(short = 'p', short_alias = 'P', long)]
        project: Option<String>,
        /// Emit a JSON array instead of text
        #[arg(long)]
        json: bool,
    },
    /// Checklist for agents and CI: stalled claims plus corpus validation.
    Hygiene {
        /// Days a claim may be held before it counts as stale
        #[arg(long)]
        stale_days: Option<i64>,
    },
    /// Print the identity this tracker would record on a claim.
    Whoami,
    /// Issues waiting on this one.
    #[command(name = "waiting-on")]
    WaitingOn { id: String },
    /// The first lines of an issue's file range.
    #[command(name = "body-excerpt")]
    BodyExcerpt {
        id: String,
        /// Emit a JSON object instead of text
        #[arg(long)]
        json: bool,
    },
    /// Substring search over ids, titles, properties, and bodies.
    Search {
        query: String,
        #[arg(short = 'n', long, default_value = "20")]
        limit: usize,
        /// Emit a JSON array instead of text
        #[arg(long)]
        json: bool,
    },
    /// Issues whose `:PARENT:` matches this id.
    Children {
        id: String,
        /// Emit a JSON array instead of text
        #[arg(long)]
        json: bool,
    },
    /// Blockers transitively required by this issue.
    Ancestors {
        id: String,
        #[arg(short, long, default_value = "3")]
        depth: usize,
        /// Emit a JSON array instead of text
        #[arg(long)]
        json: bool,
    },
    /// Issues transitively waiting on this issue.
    Impact {
        id: String,
        #[arg(short, long, default_value = "3")]
        depth: usize,
        /// Emit a JSON array instead of text
        #[arg(long)]
        json: bool,
    },
    /// Explain bounded Org and lexical connections around an issue.
    Related {
        id: String,
        #[arg(short, long, default_value = "2")]
        depth: usize,
        #[arg(short = 'n', long, default_value = "20")]
        limit: usize,
        /// text or org; org emits links to the source headings.
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// Open issues whose `:CREATED:` is older than N days.
    Stale {
        #[arg(short, long, default_value = "30")]
        days: i64,
        #[arg(short = 'p', short_alias = 'P', long)]
        project: Option<String>,
    },
    /// Print only the matching issue count.
    Count {
        #[arg(short = 'p', short_alias = 'P', long)]
        project: Option<String>,
        #[arg(short, long)]
        state: Option<String>,
        /// Count only actionable issues
        #[arg(short, long)]
        ready: bool,
    },
    /// One JSON object per issue per line.
    Export {
        #[arg(short = 'p', short_alias = 'P', long)]
        project: Option<String>,
    },
    /// Children and blockers below an id.
    Tree {
        id: String,
        /// ascii or dot
        #[arg(short, long, default_value = "ascii")]
        format: String,
        /// Emit the tree as JSON instead of ascii or dot
        #[arg(long, conflicts_with = "format")]
        json: bool,
    },
    /// Cycles in the blocker graph.
    Cycles,
    /// The blocker and parent graph as Graphviz DOT.
    Graph {
        #[arg(short = 'p', short_alias = 'P', long)]
        project: Option<String>,
    },
    /// Move an issue to another project's file.
    Refile {
        id: String,
        /// Target project
        #[arg(long)]
        to: String,
    },
    /// Issues referring to this id.
    Backlinks {
        id: String,
        /// Emit a JSON array instead of text
        #[arg(long)]
        json: bool,
    },
    /// A markdown roadmap of active and closed work.
    Roadmap {
        #[arg(short = 'p', short_alias = 'P', long)]
        project: Option<String>,
    },
    /// Validate the corpus. Exits non-zero on any error.
    Check,
    /// Rewrite files onto the Org / ELPA / vissue property split.
    Normalize {
        #[arg(short = 'p', short_alias = 'P', long)]
        project: Option<String>,
        /// Print what would change without writing.
        #[arg(long)]
        dry_run: bool,
    },
    /// A content digest of the corpus, for telling whether a copy is current.
    Digest {
        /// Project to include; repeat for several. Omit for every project.
        #[arg(short = 'p', short_alias = 'P', long = "project")]
        projects: Vec<String>,
        /// Emit a JSON object instead of text
        #[arg(long)]
        json: bool,
        /// Print only the combined digest
        #[arg(short, long)]
        quiet: bool,
    },
    /// Write a read-only projection of one or more projects to a file.
    Mirror {
        /// Project to include; repeat for several. Omit for every project.
        #[arg(short = 'p', short_alias = 'P', long = "project")]
        projects: Vec<String>,
        /// Destination file; `-` writes to standard output.
        #[arg(short, long, required_unless_present = "check")]
        out: Option<String>,
        /// Compare an existing mirror's stamp against the tracker instead of
        /// writing. Exits 0 when fresh, 1 when stale.
        #[arg(long, conflicts_with = "out")]
        check: Option<PathBuf>,
        /// org or markdown
        #[arg(short, long, default_value = "org")]
        format: String,
        /// Include only this state
        #[arg(short, long)]
        state: Option<String>,
    },
    /// Change events with a sequence above --since.
    Events {
        /// Only events newer than this sequence
        #[arg(long, default_value_t = 0)]
        since: u64,
        /// Maximum events returned
        #[arg(short = 'n', long, default_value_t = 50)]
        limit: usize,
    },
    /// Append a manual event, waking pollers without editing an issue.
    Ping {
        #[arg(long)]
        detail: Option<String>,
    },
    /// Block until the generation passes --last, or until an issue is terminal.
    /// Exits 2 on timeout.
    Wait {
        #[arg(long, default_value_t = 0)]
        last: u64,
        /// Issue to watch when --until-terminal is set
        #[arg(long)]
        id: Option<String>,
        /// Block until the issue is DONE or CANCELLED
        #[arg(long)]
        until_terminal: bool,
        #[arg(long, default_value_t = 200)]
        poll_ms: u64,
        #[arg(long, default_value_t = 10_000)]
        timeout_ms: u64,
    },
    /// Print the current generation counter.
    Gen,
    /// List the projects found under the layout prefix.
    Projects {
        /// Emit a JSON array instead of one name per line
        #[arg(long)]
        json: bool,
    },
    /// This binary's own surface as JSON: every subcommand, its aliases, and its
    /// long flags.
    ///
    /// Hidden because it describes the tool rather than the tracker, and a person
    /// reading `--help` is looking for the second. It exists so the checks that hold
    /// the command line to `schema/vissue.capnp` can ask the parser what it accepts
    /// rather than parse what it prints, in one process rather than one per verb.
    #[command(hide = true)]
    Surface,
    /// Print the resolved binary, root, and prefix.
    Identity,
    /// Own the per-user Unix control socket.
    ///
    /// Unix only. On Windows this command exits 1.
    Serve(ServeArgs),
    /// Interactive board over ready, list, claims, agenda, and search.
    ///
    /// First paint reads the files. Unless `--offline`, the board then
    /// attaches to `vissue serve` (starting it when the socket is free).
    /// A root or prefix mismatch stays on the files and does not mutate
    /// the wrong vault. On Windows, omit `--offline` to get a Unix-only
    /// error; `--offline` still runs.
    Tui {
        /// Never attach, never spawn serve; CatalogService plus generation poll.
        #[arg(long)]
        offline: bool,
        /// Control socket path. Falls back to VISSUE_CONTROL_SOCKET, then
        /// $XDG_RUNTIME_DIR/vissue/control.sock, then ~/.vissue/run/control.sock.
        #[arg(short = 's', long)]
        socket: Option<PathBuf>,
    },
    /// Task board. Default execs `vissue-hud`. Home is the project list.
    ///
    /// `--rofi` is the seat dmenu picker: Return opens the heading in
    /// `$EDITOR`, Alt+c claims, Alt+n notes.
    Hud {
        /// ready, list (all), claims, stale, or new. Used by `--rofi`.
        #[arg(long, default_value = "ready")]
        mode: String,
        /// Never attach, never spawn serve.
        #[arg(long)]
        offline: bool,
        /// Stay on the terminal.
        #[arg(long)]
        foreground: bool,
        /// Show or hide a running board, or dismiss a live rofi picker.
        #[arg(long, group = "summon")]
        toggle: bool,
        /// Show a running board.
        #[arg(long, group = "summon")]
        show: bool,
        /// Hide a running board, or dismiss a live rofi picker.
        #[arg(long, group = "summon")]
        hide: bool,
        /// Use the iced board. Default when `--rofi` is absent.
        #[arg(long)]
        iced: bool,
        /// Use the rofi picker instead of the iced board.
        #[arg(long)]
        rofi: bool,
        /// Control socket path. Falls back to VISSUE_CONTROL_SOCKET, then
        /// $XDG_RUNTIME_DIR/vissue/control.sock, then ~/.vissue/run/control.sock.
        #[arg(short = 's', long)]
        socket: Option<PathBuf>,
    },
    /// Write a shell completion script to stdout.
    ///
    /// Generated from this binary's own argument definitions, so it cannot
    /// drift from the commands it completes.
    Completions {
        /// Shell to generate for
        #[arg(value_enum)]
        shell: CompletionShell,
    },
    /// Write the roff manual page to stdout.
    Man,
    /// Print the HUD key catalog, or check a keys.toml overlay.
    ///
    /// Defaults live in code. `~/.config/vissue/keys.toml` or `VISSUE_KEYS`
    /// overlays diffs. A bad file keeps defaults and `--check` exits 1.
    Keys {
        /// Load the overlay and exit 1 on conflict.
        #[arg(long)]
        check: bool,
        /// Print taken chords.
        #[arg(long)]
        occupancy: bool,
    },
}

/// Flags and verbs under `vissue serve`.
#[derive(Args)]
struct ServeArgs {
    /// Detach after the socket accepts. The child is placed in its own
    /// process group (not a new session) and can still receive SIGHUP from
    /// the parent terminal.
    #[arg(short = 'd', long)]
    detach: bool,
    /// Hidden supervisor flag for the detached child. Alias: --no-detach.
    #[arg(long, hide = true, alias = "no-detach")]
    foreground: bool,
    /// Control socket path. Falls back to VISSUE_CONTROL_SOCKET, then
    /// $XDG_RUNTIME_DIR/vissue/control.sock, then ~/.vissue/run/control.sock.
    #[arg(short = 's', long, global = true)]
    socket: Option<PathBuf>,
    #[command(subcommand)]
    action: Option<ServeAction>,
}

#[derive(Subcommand)]
enum ServeAction {
    /// Signal the owner (SIGTERM, then SIGKILL) and wait.
    Stop,
    /// Stop, then start detached.
    Restart,
    /// Print a live/pid/socket snapshot. Exit 0 if live, 1 otherwise.
    Status {
        /// Machine-readable object
        #[arg(long)]
        json: bool,
    },
}

/// Shells `completions` can emit for.
#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum CompletionShell {
    Bash,
    Elvish,
    Fish,
    Powershell,
    Zsh,
}

fn main() {
    if let Err(e) = run() {
        if is_broken_pipe(&e) {
            return;
        }
        eprintln!("vissue: {e:#}");
        std::process::exit(1);
    }
}

fn build_router(cli: &Cli) -> Result<Router> {
    let default = Layout::resolve(cli.root.as_deref(), cli.prefix.as_deref())?;
    if cli.no_route {
        return Ok(Router::unrouted(default));
    }
    Ok(Router::load(default)?)
}

fn create_routed(
    router: &Router,
    project: &str,
    title: &str,
    opts: CreateOpts<'_>,
) -> Result<String> {
    let pref = router.route(project);
    // Paths rather than ids: the mint reads them under the lock it writes
    // under, so a twin create in another root cannot slip between.
    let twins = router.extra_id_paths_for(&pref.dir);
    let opts = CreateOpts {
        extra_id_paths: &twins,
        ..opts
    };
    Ok(ops::create(&pref.layout, &pref.dir, title, opts)?)
}

/// The catalog service for one layout, which is what the control socket answers from.
///
/// The `--json` modes go through this rather than through the text reports, so the two
/// surfaces are the same computation and not two that have to be kept in agreement.
/// The text reports stay as they are: they are what a person reads, and they group by
/// project because the command line can span layouts.
/// Emit a read's answer in whichever of its two shapes the caller asked for.
///
/// A read that takes `--json` answers the same question twice: the structured value a
/// caller parses, and the report a person prints. Ten subcommands spelled that choice
/// out as an `if json` around two emit calls, which is ten places for the two to come
/// apart. Both go through the same layout and the same service, so the pair is one
/// computation and this is where the shape is chosen.
///
/// # Errors
///
/// Returns whatever producing the chosen shape returns, or a write error.
fn emit_shape<T, S, E, F>(
    json: bool,
    structured: impl FnOnce() -> std::result::Result<T, E>,
    text: impl FnOnce() -> std::result::Result<S, F>,
) -> Result<()>
where
    T: serde::Serialize,
    S: std::fmt::Display,
    E: Into<anyhow::Error>,
    F: Into<anyhow::Error>,
{
    // Generic over both error types because the two halves come from different layers:
    // the structured answer through this binary's own helpers, the report straight out
    // of the core, which has its own error type.
    if json {
        let value = structured().map_err(Into::into)?;
        emitln!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        emit!("{}", text().map_err(Into::into)?);
    }
    Ok(())
}

fn with_catalog<T>(
    layout: &Layout,
    f: impl FnOnce(&vissue_core::catalog::CatalogService<'_>) -> vissue_core::Result<T>,
) -> Result<T> {
    let recs = vissue_core::catalog::load_recs(layout)?;
    let svc = vissue_core::catalog::CatalogService::from_recs(&recs);
    Ok(f(&svc)?)
}

fn layout_for_id(router: &Router, id: &str) -> Result<Layout> {
    Ok(router.find_by_id(id)?.layout)
}

/// Where a `reject` should put its successor.
struct RejectDest {
    layout: Layout,
    project: Option<String>,
    extra_id_paths: Vec<PathBuf>,
}

/// `--to` names an existing heading, so its own layout wins. Otherwise the
/// create project is routed, which is what keeps a bounce onto a routed name
/// off the source root.
fn reject_destination(
    router: &Router,
    src: &Layout,
    to: Option<&str>,
    project: Option<&str>,
) -> Result<RejectDest> {
    if let Some(to) = to {
        return Ok(RejectDest {
            layout: layout_for_id(router, to)?,
            project: project.map(str::to_string),
            extra_id_paths: Vec::new(),
        });
    }
    let Some(project) = project else {
        return Ok(RejectDest {
            layout: src.clone(),
            project: None,
            extra_id_paths: Vec::new(),
        });
    };
    let pref = router.route(project);
    let extra_id_paths = router.extra_id_paths_for(&pref.dir);
    Ok(RejectDest {
        layout: pref.layout,
        project: Some(pref.dir),
        extra_id_paths,
    })
}

/// Block until something changes, or until one issue settles.
///
/// Two waits behind one verb. `--until-terminal` watches one issue for a state nothing
/// follows, and the plain form watches the corpus generation. Both report through the
/// exit status as well as stdout, because a poller reads the status.
fn run_wait(
    router: &Router,
    layout: &Layout,
    last: u64,
    id: Option<String>,
    until_terminal: bool,
    poll_ms: u64,
    timeout_ms: u64,
) -> Result<()> {
    if until_terminal {
        let Some(id) = id else {
            bail!("--until-terminal requires --id");
        };
        let found = layout_for_id(router, &id)?;
        match events::wait_until_terminal(&found, &id, poll_ms, timeout_ms)? {
            events::TerminalWait::Done { generation } => {
                emitln!("DONE {generation}");
            }
            events::TerminalWait::Cancelled { generation } => {
                emitln!("CANCELLED {generation}");
            }
            events::TerminalWait::Timeout { generation, state } => {
                emitln!("TIMEOUT {state} {generation}");
                std::process::exit(2);
            }
        }
    } else {
        let generation = events::wait_generation(layout, last, poll_ms, timeout_ms)?;
        emitln!("{generation}");
        if generation <= last {
            // Unchanged: a polling script tells timeout from progress by
            // the exit status rather than by parsing the number.
            std::process::exit(2);
        }
    }
    Ok(())
}

/// Render a mirror of the corpus, or judge whether one on disk is still current.
///
/// `--check` answers a different question from the rest of the verb and answers it
/// through the exit status, since a stale mirror is a normal finding rather than a
/// failure to run.
fn run_mirror(
    layout: &Layout,
    projects: &[String],
    out: Option<String>,
    check: Option<PathBuf>,
    format: &str,
    state: Option<String>,
) -> Result<()> {
    if let Some(path) = check {
        let verdict = mirror::check(layout, &path, projects)?;
        emit!("{}", verdict.report);
        if !verdict.fresh {
            // A stale mirror is a normal answer, not a failure to run,
            // so it reports on stdout and signals through the status.
            std::process::exit(1);
        }
        return Ok(());
    }
    let out = out.expect("clap requires --out unless --check is given");
    let text = mirror::render(layout, projects, Format::parse(format)?, state.as_deref())?;
    if out == "-" {
        emit!("{text}");
    } else {
        let path = PathBuf::from(&out);
        store::replace_file_atomically(&path, &text)?;
        emitln!("wrote {}", path.display());
    }
    Ok(())
}

/// The heads-up display, through rofi or through the iced window.
///
/// Two front ends with different capabilities: rofi has no window to toggle, so the
/// window flags are meaningless there and are consumed rather than silently ignored.
/// What the HUD subcommand was asked for.
///
/// Grouped rather than passed one flag at a time: seven of these are booleans, and a
/// call site spelling seven booleans in a row is one transposition away from asking
/// for something else entirely.
struct HudRequest {
    mode: String,
    offline: bool,
    foreground: bool,
    toggle: bool,
    show: bool,
    hide: bool,
    iced: bool,
    rofi: bool,
    socket: Option<PathBuf>,
}

fn run_hud(layout: Layout, request: HudRequest) -> Result<()> {
    let HudRequest {
        mode,
        offline,
        foreground,
        toggle,
        show,
        hide,
        iced,
        rofi,
        socket,
    } = request;
    let use_rofi = rofi && !iced;
    if use_rofi {
        let _ = (toggle, show, hide, offline, foreground, socket);
        let mode = rofi::Mode::parse(&mode)?;
        rofi::run(rofi::RofiOpts::from_env(layout, mode)?)?;
    } else {
        if !offline && cfg!(not(unix)) {
            bail!("vissue hud is Unix-only");
        }
        exec_hud(ExecHud {
            layout,
            socket,
            offline,
            foreground,
            toggle,
            show,
            hide,
        })?;
    }
    Ok(())
}

/// What this binary is, where it reads, and every project it can route to.
///
/// Printed in both a labelled and a `key=value` form because a person and a script read
/// the same output.
fn run_identity(router: &Router, layout: &Layout) -> Result<()> {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "vissue".into());
    emitln!("vissue {}", env!("CARGO_PKG_VERSION"));
    emitln!("protocol: {}", vissue_core::org::PROTOCOL_VERSION);
    emitln!("binary: {exe}");
    emitln!("root:   {}", layout.root().display());
    emitln!("prefix: {}", layout.prefix());
    emitln!("root={}", layout.root().display());
    emitln!("prefix={}", layout.prefix());
    if router.is_routed() {
        for pref in router.visible_projects()? {
            if pref.key == pref.dir
                && pref.layout.root() == layout.root()
                && pref.layout.prefix() == layout.prefix()
            {
                continue;
            }
            emitln!(
                "route: {} -> {} {} {}",
                pref.key,
                pref.layout.root().display(),
                pref.layout.prefix(),
                pref.dir
            );
        }
    }
    Ok(())
}

/// A digest of the corpus: the whole report, one combined hash, or JSON.
fn run_digest(layout: &Layout, projects: &[String], json: bool, quiet: bool) -> Result<()> {
    let digest = vissue_core::digest::corpus_digest(layout, projects)?;
    if json {
        emitln!("{}", serde_json::to_string_pretty(&digest.to_json())?);
    } else if quiet {
        emitln!("{}", digest.combined);
    } else {
        emit!("{}", digest.render());
    }
    Ok(())
}

/// One issue, as a report, as its own org text, or as JSON.
///
/// Three shapes rather than the usual two, because the org form is the file's own text
/// and is what a person pastes back into a vault.
fn run_show(router: &Router, id: &str, json: bool, org: bool) -> Result<()> {
    let found = layout_for_id(router, id)?;
    if json {
        emitln!(
            "{}",
            serde_json::to_string_pretty(&agent::show_json(&found, id)?)?
        );
    } else if org {
        emit!("{}", agent::org_text(&found, id)?);
    } else {
        emit!("{}", report::show(&found, id)?);
    }
    Ok(())
}

/// Every project this process can see, one per line or as a JSON array.
fn run_projects(router: &Router, json: bool) -> Result<()> {
    let visible = router.visible_projects()?;
    if json {
        let names: Vec<&str> = visible.iter().map(|p| p.key.as_str()).collect();
        emitln!("{}", serde_json::to_string_pretty(&names)?);
    } else {
        for project in visible {
            emitln!("{}", project.key);
        }
    }
    Ok(())
}

/// The keymap: check an overlay, list what each chord is taken by, or print the table.
///
/// Three answers from one subcommand, and `--check` exits non-zero on a bad overlay
/// rather than printing one, which is what a shell hook wants.
fn run_keys(check: bool, occupancy: bool) -> Result<()> {
    let map = vissue_core::keys::KeyMap::load();
    if check {
        if let Some(err) = map.overlay_error {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
        emitln!("ok");
    } else if occupancy {
        for (chord, id) in map.occupancy() {
            emitln!("{chord}\t{id}");
        }
    } else {
        if let Some(err) = &map.overlay_error {
            eprintln!("error: {err}");
        }
        if let Some(leader) = map.leader {
            emitln!("leader {leader}");
        }
        for line in map.table_lines() {
            emitln!("{line}");
        }
    }
    Ok(())
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let router = build_router(&cli)?;
    let layout = router.default_layout().clone();

    match cli.command {
        Command::Create {
            title,
            project,
            priority,
            issue_type,
            deadline,
            scheduled,
            tags,
            parent,
            quiet,
            body,
            body_file,
        } => {
            let project = ops::resolve_project(&layout, project.as_deref())?;
            let body_text = match (body, body_file) {
                (Some(b), _) => Some(b),
                (None, Some(path)) => Some(read_body_file(&path)?),
                (None, None) => None,
            };
            let out = create_routed(
                &router,
                &project,
                &title,
                CreateOpts {
                    priority,
                    issue_type: issue_type.as_deref(),
                    deadline: deadline.as_deref(),
                    scheduled: scheduled.as_deref(),
                    tags: tags.as_deref(),
                    parent: parent.as_deref(),
                    quiet,
                    body: body_text.as_deref(),
                    ..Default::default()
                },
            )?;
            emit!("{out}");
        }
        Command::Q {
            title,
            project,
            issue_type,
            parent,
        } => {
            let project = ops::resolve_project(&layout, project.as_deref())?;
            let out = create_routed(
                &router,
                &project,
                &title,
                CreateOpts {
                    issue_type: issue_type.as_deref(),
                    parent: parent.as_deref(),
                    quiet: true,
                    ..Default::default()
                },
            )?;
            emit!("{out}");
        }
        Command::List {
            project,
            state,
            json,
        } => {
            if json {
                emitln!(
                    "{}",
                    serde_json::to_string_pretty(&issues_json_routed(
                        &router,
                        project.as_deref(),
                        state.as_deref(),
                        false
                    )?)?
                );
            } else {
                emit!(
                    "{}",
                    list_routed(&router, project.as_deref(), state.as_deref(), false)?
                );
            }
        }
        Command::Show { id, json, org } => {
            run_show(&router, &id, json, org)?;
        }
        Command::Update {
            id,
            state,
            priority,
            block,
            unblock,
            if_state,
            if_gen,
        } => {
            let found = layout_for_id(&router, &id)?;
            let outcome = ops::update_pred(
                &found,
                &id,
                state.as_deref(),
                priority,
                block.as_deref(),
                unblock.as_deref(),
                UpdatePred {
                    if_state: if_state.as_deref(),
                    if_gen,
                },
            )?;
            emit!("{}", outcome.report);
            for hint in outcome.hints {
                eprintln!("[hint] {hint}");
            }
        }
        Command::Resolve { id, state } => {
            let found = layout_for_id(&router, &id)?;
            emit!("{}", ops::resolve_terminal(&found, &id, &state)?)
        }
        Command::Reject {
            id,
            to,
            project,
            title,
            reason,
        } => {
            let found = layout_for_id(&router, &id)?;
            let dst = reject_destination(&router, &found, to.as_deref(), project.as_deref())?;
            emit!(
                "{}",
                ops::reject(
                    &found,
                    &id,
                    RejectOpts {
                        to: to.as_deref(),
                        project: dst.project.as_deref(),
                        title: title.as_deref(),
                        reason: reason.as_deref(),
                        dst_layout: Some(&dst.layout),
                        dst_extra_id_paths: &dst.extra_id_paths,
                    },
                )?
            )
        }
        Command::Ready { project, json } => {
            if json {
                emitln!(
                    "{}",
                    serde_json::to_string_pretty(&issues_json_routed(
                        &router,
                        project.as_deref(),
                        None,
                        true
                    )?)?
                );
            } else {
                emit!("{}", list_routed(&router, project.as_deref(), None, true)?);
            }
        }
        Command::Claim { id, force } => {
            let found = layout_for_id(&router, &id)?;
            emit!("{}", agent::claim(&found, &id, force)?)
        }
        Command::Deed { id, add, remove } => {
            let found = layout_for_id(&router, &id)?;
            emit!("{}", ops::deed(&found, &id, &add, &remove)?)
        }
        Command::Recall {
            id,
            depth,
            deeds_only,
            excerpts,
            json,
        } => {
            let found = layout_for_id(&router, &id)?;
            if deeds_only {
                emit!("{}", report::recall_deeds(&found, &id, depth)?);
            } else {
                emit_shape(
                    json,
                    || with_catalog(&found, |svc| svc.recall(&id, depth, excerpts)),
                    || report::recall(&found, &id, depth, excerpts),
                )?;
            }
        }
        Command::Consensus {
            id,
            children,
            gate,
            json,
        } => {
            let found = layout_for_id(&router, &id)?;
            // The report prints either way. A gate that swallowed the reason it
            // failed would send a reader back to run the command again without
            // it, which is what `mirror --check` already avoids.
            let settled = if children {
                let roll = vissue_core::consensus::of_plan(&found, &id)?;
                emit_shape(
                    json,
                    || vissue_core::Result::Ok(roll.clone()),
                    || report::plan_consensus(&found, &id),
                )?;
                roll.settled()
            } else {
                let outcome = vissue_core::consensus::of_issue(&found, &id)?;
                emit_shape(
                    json,
                    || vissue_core::Result::Ok(outcome.clone()),
                    || report::consensus(&found, &id),
                )?;
                outcome.settled()
            };
            if gate && !settled {
                std::process::exit(1);
            }
        }
        Command::Vote { id, choice } => {
            let found = layout_for_id(&router, &id)?;
            let who = vissue_core::config::identity(&found);
            emit!("{}", ops::vote(&found, &id, choice.as_deref(), &who)?)
        }
        Command::Append { id, text, file } => {
            let body = match (text, file) {
                (Some(t), None) => t,
                (None, Some(path)) => read_body_file(&path)?,
                (None, None) => bail!("pass --text or --file (`-` reads stdin)"),
                (Some(_), Some(_)) => unreachable!("clap rejects both"),
            };
            let found = layout_for_id(&router, &id)?;
            emit!("{}", ops::append_body(&found, &id, &body)?)
        }
        Command::Note { id, text } => {
            let found = layout_for_id(&router, &id)?;
            emit!("{}", ops::note(&found, &id, &text.join(" "))?)
        }
        Command::Claims { by, project, json } => {
            emit!(
                "{}",
                claims_routed(&router, by.as_deref(), project.as_deref(), json)?
            )
        }
        Command::Fold { file, project } => {
            let project = ops::resolve_project(&layout, project.as_deref())?;
            let pref = router.route(&project);
            emit!("{}", ops::fold(&pref.layout, &file, &pref.dir)?)
        }
        Command::Agenda {
            days,
            project,
            json,
        } => {
            emit_shape(
                json,
                || with_catalog(&layout, |svc| svc.agenda(days, project.as_deref())),
                || agenda_routed(&router, days, project.as_deref()),
            )?;
        }
        Command::Hygiene { stale_days } => emit!("{}", hygiene_routed(&router, stale_days)?),
        Command::Whoami => emitln!("{}", vissue_core::config::identity(&layout)),
        Command::WaitingOn { id } => {
            let found = layout_for_id(&router, &id)?;
            emit!("{}", agent::waiting_on(&found, &id)?)
        }
        Command::BodyExcerpt { id, json } => {
            let found = layout_for_id(&router, &id)?;
            emit_shape(
                json,
                || with_catalog(&found, |svc| svc.excerpt(&id)),
                || agent::body_excerpt(&found, &id),
            )?;
        }
        Command::Search { query, limit, json } => {
            emit_shape(
                json,
                || with_catalog(&layout, |svc| svc.search(&query, limit)),
                || search_routed(&router, &query, limit),
            )?;
        }
        Command::Children { id, json } => {
            let found = layout_for_id(&router, &id)?;
            emit_shape(
                json,
                || with_catalog(&found, |svc| svc.children(&id)),
                || report::children(&found, &id),
            )?;
        }
        Command::Ancestors { id, depth, json } => {
            let found = layout_for_id(&router, &id)?;
            emit_shape(
                json,
                || with_catalog(&found, |svc| svc.ancestors(&id, depth)),
                || report::ancestors(&found, &id, depth),
            )?;
        }
        Command::Impact { id, depth, json } => {
            let found = layout_for_id(&router, &id)?;
            emit_shape(
                json,
                || with_catalog(&found, |svc| svc.impact(&id, depth)),
                || report::impact(&found, &id, depth),
            )?;
        }
        Command::Related {
            id,
            depth,
            limit,
            format,
        } => {
            let found = layout_for_id(&router, &id)?;
            emit!("{}", report::related(&found, &id, depth, limit, &format)?)
        }
        Command::Stale { days, project } => {
            emit!("{}", stale_routed(&router, days, project.as_deref())?)
        }
        Command::Count {
            project,
            state,
            ready,
        } => emit!(
            "{}",
            count_routed(&router, project.as_deref(), state.as_deref(), ready)?
        ),
        Command::Export { project } => emit!("{}", export_routed(&router, project.as_deref())?),
        Command::Tree { id, format, json } => {
            let found = layout_for_id(&router, &id)?;
            emit_shape(
                json,
                || with_catalog(&found, |svc| svc.tree(&id)),
                || report::tree(&found, &id, &format),
            )?;
        }
        Command::Cycles => emit!("{}", cycles_routed(&router)?),
        Command::Graph { project } => emit!("{}", graph_routed(&router, project.as_deref())?),
        Command::Refile { id, to } => {
            let found = layout_for_id(&router, &id)?;
            let dest = router.route(&to);
            emit!("{}", ops::refile_to(&found, &id, &dest.layout, &dest.dir)?)
        }
        Command::Backlinks { id, json } => emit_shape(
            json,
            || backlinks_rows(&router, &id),
            || backlinks_text(&router, &id),
        )?,
        Command::Roadmap { project } => {
            emit!("{}", roadmap_routed(&router, project.as_deref())?)
        }
        Command::Check => {
            let (text, errors) = check_routed(&router)?;
            emit!("{}", text);
            if errors > 0 {
                bail!("{errors} validation error(s)");
            }
        }
        Command::Normalize { project, dry_run } => {
            emit!(
                "{}",
                normalize_routed(&router, project.as_deref(), dry_run)?
            )
        }
        Command::Digest {
            projects,
            json,
            quiet,
        } => {
            run_digest(&layout, &projects, json, quiet)?;
        }
        Command::Mirror {
            projects,
            out,
            check,
            format,
            state,
        } => {
            run_mirror(&layout, &projects, out, check, &format, state)?;
        }
        Command::Events { since, limit } => {
            emit!("{}", events::since_report(&layout, since, limit)?)
        }
        Command::Ping { detail } => {
            emit!("{}", events::ping_report(&layout, detail.as_deref())?)
        }
        Command::Wait {
            last,
            id,
            until_terminal,
            poll_ms,
            timeout_ms,
        } => {
            run_wait(
                &router,
                &layout,
                last,
                id,
                until_terminal,
                poll_ms,
                timeout_ms,
            )?;
        }
        Command::Gen => emitln!("{}", events::generation(&layout)),
        Command::Projects { json } => {
            run_projects(&router, json)?;
        }
        Command::Surface => {
            let mut cmd = Cli::command();
            cmd.build();
            let verbs: Vec<serde_json::Value> = cmd
                .get_subcommands()
                .map(|sub| {
                    let flags: Vec<&str> = sub
                        .get_arguments()
                        .filter_map(clap::Arg::get_long)
                        .collect();
                    serde_json::json!({
                        "name": sub.get_name(),
                        "hidden": sub.is_hide_set(),
                        "aliases": sub.get_all_aliases().collect::<Vec<_>>(),
                        "flags": flags,
                    })
                })
                .collect();
            emitln!("{}", serde_json::to_string_pretty(&verbs)?);
        }
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            let mut buffer: Vec<u8> = Vec::new();
            match shell {
                CompletionShell::Bash => {
                    clap_complete::generate(clap_complete::Shell::Bash, &mut cmd, name, &mut buffer)
                }
                CompletionShell::Elvish => clap_complete::generate(
                    clap_complete::Shell::Elvish,
                    &mut cmd,
                    name,
                    &mut buffer,
                ),
                CompletionShell::Fish => {
                    clap_complete::generate(clap_complete::Shell::Fish, &mut cmd, name, &mut buffer)
                }
                CompletionShell::Powershell => clap_complete::generate(
                    clap_complete::Shell::PowerShell,
                    &mut cmd,
                    name,
                    &mut buffer,
                ),
                CompletionShell::Zsh => {
                    clap_complete::generate(clap_complete::Shell::Zsh, &mut cmd, name, &mut buffer)
                }
            }
            emit!(
                "{}",
                strip_hidden_serve_flags(&String::from_utf8_lossy(&buffer))
            );
        }
        Command::Keys { check, occupancy } => {
            run_keys(check, occupancy)?;
        }
        Command::Man => {
            let mut buffer: Vec<u8> = Vec::new();
            clap_mangen::Man::new(Cli::command())
                .render(&mut buffer)
                .context("render the manual page")?;
            emit!(
                "{}",
                trim_man_trailing_space(&String::from_utf8_lossy(&buffer))
            );
        }
        Command::Identity => {
            run_identity(&router, &layout)?;
        }
        Command::Serve(args) => {
            let socket = args
                .socket
                .unwrap_or_else(vissue_control::default_socket_path);
            let cfg = vissue_serve::ServeConfig {
                layout,
                socket,
                exe: None,
            };
            let action = match args.action {
                Some(ServeAction::Stop) => vissue_serve::Action::Stop,
                Some(ServeAction::Restart) => vissue_serve::Action::Restart,
                Some(ServeAction::Status { json }) => vissue_serve::Action::Status { json },
                None if args.foreground => vissue_serve::Action::Foreground,
                None if args.detach => vissue_serve::Action::Detach,
                None => vissue_serve::Action::Foreground,
            };
            let code = vissue_serve::invoke(action, &cfg)?;
            if code != 0 {
                std::process::exit(code);
            }
        }
        Command::Tui { offline, socket } => {
            if !offline && cfg!(not(unix)) {
                bail!("vissue tui is Unix-only");
            }
            let socket = socket.unwrap_or_else(vissue_control::default_socket_path);
            let agent = vissue_core::config::identity(&layout);
            vissue_tui::run(vissue_tui::RunOpts {
                layout,
                socket,
                offline,
                agent,
            })?;
        }
        Command::Hud {
            mode,
            offline,
            foreground,
            toggle,
            show,
            hide,
            iced,
            rofi,
            socket,
        } => {
            run_hud(
                layout,
                HudRequest {
                    mode,
                    offline,
                    foreground,
                    toggle,
                    show,
                    hide,
                    iced,
                    rofi,
                    socket,
                },
            )?;
        }
    }
    // Flush here rather than at exit, so a full disk or a closed pipe reaches
    // the caller as a status instead of being dropped on the way out.
    std::io::stdout().flush()?;
    Ok(())
}

/// clap_mangen pads `.TH` with a trailing space. prek trailing-whitespace
/// rejects that, so strip per-line padding from the generated page.
fn trim_man_trailing_space(page: &str) -> String {
    let mut out = String::with_capacity(page.len());
    for line in page.lines() {
        out.push_str(line.trim_end());
        out.push('\n');
    }
    if page.ends_with('\n') || page.is_empty() {
        out
    } else {
        out.pop();
        out
    }
}

/// clap_complete still emits `hide = true` flags. Drop the supervisor
/// `--foreground` / `--no-detach` tokens so tab-complete does not advertise them.
fn strip_hidden_serve_flags(script: &str) -> String {
    let mut out = String::with_capacity(script.len());
    for line in script.lines() {
        if line_is_hidden_serve_flag(line) {
            continue;
        }
        let cleaned = line
            .replace(" --foreground", "")
            .replace("--foreground ", "")
            .replace(" --no-detach", "")
            .replace("--no-detach ", "");
        out.push_str(&cleaned);
        out.push('\n');
    }
    if script.ends_with('\n') || script.is_empty() {
        out
    } else {
        out.pop();
        out
    }
}

fn line_is_hidden_serve_flag(line: &str) -> bool {
    let trimmed = line.trim();
    let names_flag = trimmed.contains("--foreground")
        || trimmed.contains("--no-detach")
        || trimmed.contains("-l foreground")
        || trimmed.contains("-l no-detach");
    if !names_flag {
        return false;
    }
    // Keep multi-option `opts=` lines; strip the token there instead.
    !trimmed.contains("opts=")
}

/// Args forwarded to `vissue-hud`. The launcher never cargo-builds.
struct ExecHud {
    layout: Layout,
    socket: Option<PathBuf>,
    offline: bool,
    foreground: bool,
    toggle: bool,
    show: bool,
    hide: bool,
}

const HUD_BIN_ENV: &str = "VISSUE_HUD_BIN";

fn resolve_hud_bin() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var(HUD_BIN_ENV) {
        let t = raw.trim();
        if !t.is_empty() {
            return Some(PathBuf::from(t));
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join("vissue-hud");
        if sibling.is_file() {
            return Some(sibling);
        }
        #[cfg(windows)]
        {
            let exe = dir.join("vissue-hud.exe");
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join("vissue-hud");
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let exe = dir.join("vissue-hud.exe");
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    None
}

fn exec_hud(opts: ExecHud) -> Result<()> {
    let Some(bin) = resolve_hud_bin().filter(|p| p.is_file()) else {
        eprintln!("vissue-hud is not installed. Install it with:\n  cargo install vissue-hud");
        std::process::exit(127);
    };
    let mut cmd = std::process::Command::new(&bin);
    cmd.arg("--root")
        .arg(opts.layout.root())
        .arg("--prefix")
        .arg(opts.layout.prefix());
    if let Some(socket) = opts.socket {
        cmd.arg("--socket").arg(socket);
    }
    if opts.offline {
        cmd.arg("--offline");
    }
    if opts.foreground {
        cmd.arg("--foreground");
    }
    if opts.toggle {
        cmd.arg("--toggle");
    } else if opts.show {
        cmd.arg("--show");
    } else if opts.hide {
        cmd.arg("--hide");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = cmd.exec();
        bail!("exec {}: {err}", bin.display());
    }
    #[cfg(not(unix))]
    {
        let status = cmd
            .status()
            .with_context(|| format!("spawn {}", bin.display()))?;
        if let Some(code) = status.code() {
            std::process::exit(code);
        }
        bail!("{} exited without a status", bin.display());
    }
}

fn list_routed(
    router: &Router,
    project: Option<&str>,
    state: Option<&str>,
    ready_only: bool,
) -> Result<String> {
    if let Some(p) = project {
        let pref = router.route(p);
        return if ready_only {
            Ok(report::ready(&pref.layout, Some(&pref.dir))?)
        } else {
            Ok(report::list(&pref.layout, Some(&pref.dir), state, false)?)
        };
    }
    let mut out = String::new();
    for pref in router.visible_projects()? {
        let chunk = if ready_only {
            report::ready(&pref.layout, Some(&pref.dir))?
        } else {
            report::list(&pref.layout, Some(&pref.dir), state, false)?
        };
        out.push_str(&chunk);
    }
    Ok(out)
}

fn issues_json_routed(
    router: &Router,
    project: Option<&str>,
    state: Option<&str>,
    ready_only: bool,
) -> Result<serde_json::Value> {
    if let Some(p) = project {
        let pref = router.route(p);
        return Ok(agent::issues_json(
            &pref.layout,
            Some(&pref.dir),
            state,
            ready_only,
        )?);
    }
    let mut rows = Vec::new();
    for pref in router.visible_projects()? {
        let value = agent::issues_json(&pref.layout, Some(&pref.dir), state, ready_only)?;
        if let Some(arr) = value.as_array() {
            rows.extend(arr.iter().cloned());
        }
    }
    Ok(serde_json::Value::Array(rows))
}

fn count_routed(
    router: &Router,
    project: Option<&str>,
    state: Option<&str>,
    ready: bool,
) -> Result<String> {
    if let Some(p) = project {
        let pref = router.route(p);
        return Ok(report::count(&pref.layout, Some(&pref.dir), state, ready)?);
    }
    let mut n = 0usize;
    for pref in router.visible_projects()? {
        let text = report::count(&pref.layout, Some(&pref.dir), state, ready)?;
        n += text.trim().parse::<usize>().unwrap_or(0);
    }
    Ok(format!("{n}\n"))
}

/// The line a report prints when it has nothing to show. It belongs to the
/// answer, not to a project's share of it: without this, a run over six
/// projects printed "no live claims" above a list of nine claims, and
/// `agenda` printed "nothing dated in range" five times above the dated rows.
const NO_CLAIMS: &str = "no live claims\n";
const NOTHING_DATED: &str = "nothing dated in range\n";

fn claims_routed(
    router: &Router,
    by: Option<&str>,
    project: Option<&str>,
    json: bool,
) -> Result<String> {
    if json {
        return claims_json_routed(router, by, project);
    }
    concat_project_reports_with(router, project, Some(NO_CLAIMS), |layout, filter| {
        report::claims(layout, by, filter, false)
    })
}

/// The claims of every project as one JSON array.
///
/// A fragment per project is a stream of arrays, which no JSON parser takes as
/// a document: `json.load` stops at the second `[` and reports extra data.
fn claims_json_routed(router: &Router, by: Option<&str>, project: Option<&str>) -> Result<String> {
    if let Some(p) = project {
        let pref = router.route(p);
        return Ok(report::claims(&pref.layout, by, Some(&pref.dir), true)?);
    }
    let mut rows: Vec<serde_json::Value> = Vec::new();
    for pref in router.visible_projects()? {
        let part = report::claims(&pref.layout, by, Some(&pref.dir), true)?;
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&part)
            .with_context(|| format!("parsing the claims of {}", pref.dir))?;
        rows.extend(parsed);
    }
    Ok(format!("{}\n", serde_json::to_string(&rows)?))
}

fn agenda_routed(router: &Router, days: i64, project: Option<&str>) -> Result<String> {
    concat_project_reports_with(router, project, Some(NOTHING_DATED), |layout, filter| {
        report::agenda(layout, days, filter)
    })
}

fn stale_routed(router: &Router, days: i64, project: Option<&str>) -> Result<String> {
    concat_project_reports(router, project, |layout, filter| {
        report::stale(layout, days, filter)
    })
}

fn export_routed(router: &Router, project: Option<&str>) -> Result<String> {
    concat_project_reports(router, project, report::export)
}

fn graph_routed(router: &Router, project: Option<&str>) -> Result<String> {
    // One graph, so `dot` draws every project rather than the first one it
    // meets. Concatenating whole documents also leaves a cross-project edge
    // pointing at a node declared in a different `digraph` block.
    if project.is_some() {
        return concat_project_reports(router, project, report::graph);
    }
    let body = concat_project_reports(router, project, report::graph_body)?;
    Ok(format!(
        "{}{body}{}",
        report::GRAPH_HEADER,
        report::GRAPH_FOOTER
    ))
}

fn roadmap_routed(router: &Router, project: Option<&str>) -> Result<String> {
    // The title belongs to the document, so it goes above the projects rather
    // than above each one. Concatenating whole roadmaps repeats it per project.
    if project.is_some() {
        return concat_project_reports(router, project, report::roadmap);
    }
    let body = concat_project_reports(router, project, report::roadmap_body)?;
    Ok(format!("{}{body}", report::ROADMAP_HEADER))
}

fn concat_project_reports(
    router: &Router,
    project: Option<&str>,
    f: impl FnMut(&Layout, Option<&str>) -> vissue_core::Result<String>,
) -> Result<String> {
    concat_project_reports_with(router, project, None, f)
}

/// Concatenate a per-project report, collapsing the empty-set line if one is
/// named.
///
/// A report that says "nothing here" says it per project, and a run over a
/// corpus of six says it up to six times, interleaved with the projects that
/// did have rows. Asked for one project, the sentinel is the whole answer and
/// stays; asked for all of them, it is dropped from each fragment and printed
/// once when every fragment was empty.
fn concat_project_reports_with(
    router: &Router,
    project: Option<&str>,
    empty_marker: Option<&str>,
    mut f: impl FnMut(&Layout, Option<&str>) -> vissue_core::Result<String>,
) -> Result<String> {
    if let Some(p) = project {
        let pref = router.route(p);
        return Ok(f(&pref.layout, Some(&pref.dir))?);
    }
    let mut out = String::new();
    for pref in router.visible_projects()? {
        let part = f(&pref.layout, Some(&pref.dir))?;
        match empty_marker {
            Some(marker) if part == marker => {}
            _ => out.push_str(&part),
        }
    }
    if let Some(marker) = empty_marker
        && out.is_empty()
    {
        out.push_str(marker);
    }
    Ok(out)
}

fn search_routed(router: &Router, query: &str, limit: usize) -> Result<String> {
    let mut out = String::new();
    for layout in router.unique_layouts() {
        out.push_str(&report::search(layout, query, limit)?);
    }
    Ok(out)
}

fn normalize_routed(router: &Router, project: Option<&str>, dry_run: bool) -> Result<String> {
    let mut out = String::new();
    if let Some(name) = project {
        let pref = router.route(name);
        out.push_str(&ops::normalize(&pref.layout, Some(&pref.dir), dry_run)?);
    } else {
        for layout in router.unique_layouts() {
            out.push_str(&ops::normalize(layout, None, dry_run)?);
        }
    }
    Ok(out)
}

fn hygiene_routed(router: &Router, stale_days: Option<i64>) -> Result<String> {
    let mut out = String::new();
    for layout in router.unique_layouts() {
        out.push_str(&agent::hygiene(layout, stale_days)?);
    }
    Ok(out)
}

/// A known id routes to its own layout and the walk answers there, the way
/// every other walk does. An accession names a product rather than a heading,
/// so it has no layout of its own and any tracker in reach can cite it: those
/// are scanned in full.
fn backlinks_layouts(router: &Router, id: &str) -> Result<Vec<Layout>> {
    match layout_for_id(router, id) {
        Ok(found) => Ok(vec![found]),
        Err(_) if ops::is_deed_accession(id) => {
            Ok(router.unique_layouts().into_iter().cloned().collect())
        }
        Err(err) => Err(err),
    }
}

fn backlinks_rows(router: &Router, id: &str) -> Result<Vec<vissue_core::views::WalkHit>> {
    let mut out = Vec::new();
    for layout in backlinks_layouts(router, id)? {
        out.extend(with_catalog(&layout, |svc| svc.backlinks(id))?);
    }
    Ok(out)
}

fn backlinks_text(router: &Router, id: &str) -> Result<String> {
    let mut out = String::new();
    for layout in backlinks_layouts(router, id)? {
        out.push_str(&report::backlinks(&layout, id)?);
    }
    Ok(out)
}

fn cycles_routed(router: &Router) -> Result<String> {
    let mut out = String::new();
    for layout in router.unique_layouts() {
        out.push_str(&report::cycles(layout)?);
    }
    Ok(out)
}

fn check_routed(router: &Router) -> Result<(String, usize)> {
    let mut text = String::new();
    let mut errors = 0usize;
    for layout in router.unique_layouts() {
        let report = report::check(layout)?;
        text.push_str(&report.text);
        errors += report.errors;
    }
    for (id, paths) in router.duplicate_ids()? {
        errors += 1;
        let listed = paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        text.push_str(&format!(
            "[err]  duplicate id across layouts: {id} in {listed}\n"
        ));
    }
    Ok((text, errors))
}

fn read_body_file(path: &str) -> Result<String> {
    if path == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("read body from stdin")?;
        return Ok(buf);
    }
    std::fs::read_to_string(path).with_context(|| format!("read body file {path}"))
}
