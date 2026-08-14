//! `vissue`: plain-text issue tracking over per-project orgmode files.

use anyhow::{bail, Context, Result};
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use std::io::{Read, Write};
use std::path::PathBuf;

use vissue_core::config::Layout;
use vissue_core::mirror::{self, Format};
use vissue_core::ops::{self, CreateOpts};
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
    /// Task board. Default execs `vissue-hud` (Ready / Mine / Upcoming / All).
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
        Command::Show { id, json, org } => {
            if json {
                emitln!(
                    "{}",
                    serde_json::to_string_pretty(&agent::show_json(&layout, &id)?)?
                );
            } else if org {
                emit!("{}", agent::org_text(&layout, &id)?);
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
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
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
