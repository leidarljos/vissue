//! `vissue`: plain-text issue tracking over per-project orgmode files.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::io::{Read, Write};
use std::path::PathBuf;

use vissue_core::config::Layout;
use vissue_core::mirror::{self, Format};
use vissue_core::ops::{self, CreateOpts};
use vissue_core::store;
use vissue_core::{agent, events, report};

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
    /// Show one issue's metadata and file range.
    Show {
        id: String,
        /// Emit a JSON object instead of text
        #[arg(long)]
        json: bool,
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
    /// Add a dated note to the top of an issue's logbook; state and claim untouched.
    Note {
        id: String,
        /// The note. Multiple words are joined with spaces.
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
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
    BodyExcerpt { id: String },
    /// Substring search over ids, titles, properties, and bodies.
    Search {
        query: String,
        #[arg(short = 'n', long, default_value = "20")]
        limit: usize,
    },
    /// Issues whose `:PARENT:` matches this id.
    Children { id: String },
    /// Blockers transitively required by this issue.
    Ancestors {
        id: String,
        #[arg(short, long, default_value = "3")]
        depth: usize,
    },
    /// Issues transitively waiting on this issue.
    Impact {
        id: String,
        #[arg(short, long, default_value = "3")]
        depth: usize,
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
    Backlinks { id: String },
    /// A markdown roadmap of active and closed work.
    Roadmap {
        #[arg(short = 'p', short_alias = 'P', long)]
        project: Option<String>,
    },
    /// Validate the corpus. Exits non-zero on any error.
    Check,
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
    /// Block until the generation passes --last. Exits 2 on timeout.
    Wait {
        #[arg(long, default_value_t = 0)]
        last: u64,
        #[arg(long, default_value_t = 200)]
        poll_ms: u64,
        #[arg(long, default_value_t = 10_000)]
        timeout_ms: u64,
    },
    /// Print the current generation counter.
    Gen,
    /// List the projects found under the layout prefix.
    Projects,
    /// Print the resolved binary, root, and prefix.
    Identity,
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

fn run() -> Result<()> {
    let cli = Cli::parse();
    let layout = Layout::resolve(cli.root.as_deref(), cli.prefix.as_deref())?;

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
            let out = ops::create(
                &layout,
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
            let out = ops::create(
                &layout,
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
                let rows =
                    agent::issues_json(&layout, project.as_deref(), state.as_deref(), false)?;
                emitln!("{}", serde_json::to_string_pretty(&rows)?);
            } else {
                emit!(
                    "{}",
                    report::list(&layout, project.as_deref(), state.as_deref(), false)?
                );
            }
        }
        Command::Show { id, json } => {
            if json {
                emitln!(
                    "{}",
                    serde_json::to_string_pretty(&agent::show_json(&layout, &id)?)?
                );
            } else {
                emit!("{}", report::show(&layout, &id)?);
            }
        }
        Command::Update {
            id,
            state,
            priority,
            block,
            unblock,
        } => {
            let outcome = ops::update(
                &layout,
                &id,
                state.as_deref(),
                priority,
                block.as_deref(),
                unblock.as_deref(),
            )?;
            emit!("{}", outcome.report);
            for hint in outcome.hints {
                eprintln!("[hint] {hint}");
            }
        }
        Command::Ready { project, json } => {
            if json {
                let rows = agent::issues_json(&layout, project.as_deref(), None, true)?;
                emitln!("{}", serde_json::to_string_pretty(&rows)?);
            } else {
                emit!("{}", report::ready(&layout, project.as_deref())?);
            }
        }
        Command::Claim { id, force } => emit!("{}", agent::claim(&layout, &id, force)?),
        Command::Note { id, text } => {
            emit!("{}", ops::note(&layout, &id, &text.join(" "))?)
        }
        Command::Claims { by, project, json } => emit!(
            "{}",
            report::claims(&layout, by.as_deref(), project.as_deref(), json)?
        ),
        Command::Fold { file, project } => {
            let project = ops::resolve_project(&layout, project.as_deref())?;
            emit!("{}", ops::fold(&layout, &file, &project)?)
        }
        Command::Agenda { days, project } => {
            emit!("{}", report::agenda(&layout, days, project.as_deref())?)
        }
        Command::Hygiene { stale_days } => emit!("{}", agent::hygiene(&layout, stale_days)?),
        Command::Whoami => emitln!("{}", vissue_core::config::identity(&layout)),
        Command::WaitingOn { id } => emit!("{}", agent::waiting_on(&layout, &id)?),
        Command::BodyExcerpt { id } => emit!("{}", agent::body_excerpt(&layout, &id)?),
        Command::Search { query, limit } => {
            emit!("{}", report::search(&layout, &query, limit)?)
        }
        Command::Children { id } => emit!("{}", report::children(&layout, &id)?),
        Command::Ancestors { id, depth } => emit!("{}", report::ancestors(&layout, &id, depth)?),
        Command::Impact { id, depth } => emit!("{}", report::impact(&layout, &id, depth)?),
        Command::Related {
            id,
            depth,
            limit,
            format,
        } => emit!("{}", report::related(&layout, &id, depth, limit, &format)?),
        Command::Stale { days, project } => {
            emit!("{}", report::stale(&layout, days, project.as_deref())?)
        }
        Command::Count {
            project,
            state,
            ready,
        } => emit!(
            "{}",
            report::count(&layout, project.as_deref(), state.as_deref(), ready)?
        ),
        Command::Export { project } => emit!("{}", report::export(&layout, project.as_deref())?),
        Command::Tree { id, format } => emit!("{}", report::tree(&layout, &id, &format)?),
        Command::Cycles => emit!("{}", report::cycles(&layout)?),
        Command::Graph { project } => emit!("{}", report::graph(&layout, project.as_deref())?),
        Command::Refile { id, to } => emit!("{}", ops::refile(&layout, &id, &to)?),
        Command::Backlinks { id } => emit!("{}", report::backlinks(&layout, &id)?),
        Command::Roadmap { project } => emit!("{}", report::roadmap(&layout, project.as_deref())?),
        Command::Check => {
            let out = report::check(&layout)?;
            emit!("{}", out.text);
            if out.errors > 0 {
                bail!("{} validation error(s)", out.errors);
            }
        }
        Command::Digest {
            projects,
            json,
            quiet,
        } => {
            let digest = vissue_core::digest::corpus_digest(&layout, &projects)?;
            if json {
                emitln!("{}", serde_json::to_string_pretty(&digest.to_json())?);
            } else if quiet {
                emitln!("{}", digest.combined);
            } else {
                emit!("{}", digest.render());
            }
        }
        Command::Mirror {
            projects,
            out,
            check,
            format,
            state,
        } => {
            if let Some(path) = check {
                let verdict = mirror::check(&layout, &path, &projects)?;
                emit!("{}", verdict.report);
                if !verdict.fresh {
                    // A stale mirror is a normal answer, not a failure to run,
                    // so it reports on stdout and signals through the status.
                    std::process::exit(1);
                }
                return Ok(());
            }
            let out = out.expect("clap requires --out unless --check is given");
            let text = mirror::render(
                &layout,
                &projects,
                Format::parse(&format)?,
                state.as_deref(),
            )?;
            if out == "-" {
                emit!("{text}");
            } else {
                let path = PathBuf::from(&out);
                store::replace_file_atomically(&path, &text)?;
                emitln!("wrote {}", path.display());
            }
        }
        Command::Events { since, limit } => {
            emit!("{}", events::since_report(&layout, since, limit)?)
        }
        Command::Ping { detail } => {
            emit!("{}", events::ping_report(&layout, detail.as_deref())?)
        }
        Command::Wait {
            last,
            poll_ms,
            timeout_ms,
        } => {
            let generation = events::wait_generation(&layout, last, poll_ms, timeout_ms)?;
            emitln!("{generation}");
            if generation <= last {
                // Unchanged: a polling script tells timeout from progress by
                // the exit status rather than by parsing the number.
                std::process::exit(2);
            }
        }
        Command::Gen => emitln!("{}", events::generation(&layout)),
        Command::Projects => {
            for project in store::list_projects(&layout)? {
                emitln!("{project}");
            }
        }
        Command::Identity => {
            let exe = std::env::current_exe()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "vissue".into());
            emitln!("vissue {}", env!("CARGO_PKG_VERSION"));
            emitln!("binary: {exe}");
            emitln!("root:   {}", layout.root().display());
            emitln!("prefix: {}", layout.prefix());
            emitln!("root={}", layout.root().display());
            emitln!("prefix={}", layout.prefix());
        }
    }
    // Flush here rather than at exit, so a full disk or a closed pipe reaches
    // the caller as a status instead of being dropped on the way out.
    std::io::stdout().flush()?;
    Ok(())
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
