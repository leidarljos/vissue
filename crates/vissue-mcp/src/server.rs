//! The MCP tool surface, calling vissue-core in process.

use rmcp::{
    ErrorData as McpError, handler::server::ServerHandler, handler::server::wrapper::Parameters,
    model::*, tool, tool_handler, tool_router,
};

use vissue_core::config::Layout;
use vissue_core::mirror::{self, Format};
use vissue_core::ops::{self, CreateOpts, RejectOpts, UpdatePred};
use vissue_core::router::Router;
use vissue_core::{agent, events, report};

use crate::tools::*;
use std::path::PathBuf;

/// The tool router is built by `#[tool_handler]` through `Self::tool_router()`,
/// so the server carries the default layout and the user-level project router.
#[derive(Clone)]
pub struct VissueServer {
    layout: Layout,
    router: Router,
}

/// Where a `reject` should put its successor.
struct RejectDest {
    layout: Layout,
    project: Option<String>,
    extra_id_paths: Vec<PathBuf>,
}

fn text<E: std::fmt::Display>(result: Result<String, E>) -> Result<CallToolResult, McpError> {
    match result {
        Ok(s) => Ok(CallToolResult::success(vec![Content::text(s)])),
        Err(e) => Err(McpError::internal_error(format!("{e}"), None)),
    }
}

fn json<E: std::fmt::Display>(
    result: Result<serde_json::Value, E>,
) -> Result<CallToolResult, McpError> {
    match result {
        Ok(v) => {
            let rendered = serde_json::to_string_pretty(&v).unwrap_or_else(|_| "null".to_string());
            Ok(CallToolResult::success(vec![Content::text(rendered)]))
        }
        Err(e) => Err(McpError::internal_error(format!("{e}"), None)),
    }
}

#[tool_router]
impl VissueServer {
    /// Resolve the layout from `VISSUE_ROOT` and `VISSUE_PREFIX`, or the
    /// current directory, then load the user-level route table.
    pub fn from_env() -> anyhow::Result<Self> {
        let layout = Layout::resolve(None, None)?;
        let router = Router::load(layout.clone())?;
        Ok(Self { layout, router })
    }

    #[cfg(test)]
    pub fn with_layout(layout: Layout) -> Self {
        Self {
            router: Router::unrouted(layout.clone()),
            layout,
        }
    }

    fn layout_for_id(&self, id: &str) -> vissue_core::Result<Layout> {
        Ok(self.router.find_by_id(id)?.layout)
    }

    /// A known id routes to its own layout. An accession names a product
    /// rather than a heading, so it has no layout of its own and every tracker
    /// in reach can cite it.
    fn backlinks_text(&self, id: &str) -> vissue_core::Result<String> {
        match self.layout_for_id(id) {
            Ok(layout) => report::backlinks(&layout, id),
            Err(_) if ops::is_deed_accession(id) => {
                let mut out = String::new();
                for layout in self.router.unique_layouts() {
                    out.push_str(&report::backlinks(layout, id)?);
                }
                Ok(out)
            }
            Err(err) => Err(err),
        }
    }

    /// `to` names an existing heading, so its own layout wins. Otherwise the
    /// create project is routed, which keeps a bounce onto a routed name off
    /// the server's own root.
    fn reject_destination(
        &self,
        src: &Layout,
        to: Option<&str>,
        project: Option<&str>,
    ) -> vissue_core::Result<RejectDest> {
        if let Some(to) = to {
            return Ok(RejectDest {
                layout: self.layout_for_id(to)?,
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
        let pref = self.router.route(project);
        let extra_id_paths = self.router.extra_id_paths_for(&pref.dir);
        Ok(RejectDest {
            layout: pref.layout,
            project: Some(pref.dir),
            extra_id_paths,
        })
    }

    #[tool(description = "List the projects that hold an issues.org under the tracker root.")]
    async fn vissue_projects(&self) -> Result<CallToolResult, McpError> {
        text(self.router.visible_projects().map(|ps| {
            format!(
                "{}\n",
                ps.into_iter().map(|p| p.key).collect::<Vec<_>>().join("\n")
            )
        }))
    }

    #[tool(description = "List issues, optionally filtered by project and state.")]
    async fn vissue_list(
        &self,
        Parameters(args): Parameters<ListArgs>,
    ) -> Result<CallToolResult, McpError> {
        json(issues_json_routed(
            &self.router,
            args.project.as_deref(),
            args.state.as_deref(),
            false,
        ))
    }

    #[tool(description = "List actionable issues: TODO or STARTED with no open blocker.")]
    async fn vissue_ready(
        &self,
        Parameters(args): Parameters<ProjectArgs>,
    ) -> Result<CallToolResult, McpError> {
        json(issues_json_routed(
            &self.router,
            args.project.as_deref(),
            None,
            true,
        ))
    }

    #[tool(description = "Show one issue's metadata and file range. Never returns body prose.")]
    async fn vissue_show(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        json(
            self.layout_for_id(&args.issue_id)
                .and_then(|layout| agent::show_json(&layout, &args.issue_id)),
        )
    }

    #[tool(description = "Create an issue in a project's issues.org.")]
    async fn vissue_create(
        &self,
        Parameters(args): Parameters<CreateArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(create_routed(
            &self.router,
            &args.project,
            &args.title,
            CreateOpts {
                priority: priority_char(args.priority.as_ref()),
                issue_type: args.issue_type.as_deref(),
                tags: args.tags.as_deref(),
                parent: args.parent.as_deref(),
                body: args.body.as_deref(),
                deadline: args.deadline.as_deref(),
                scheduled: args.scheduled.as_deref(),
                ..Default::default()
            },
        ))
    }

    #[tool(
        description = "Reject an issue by redirecting it to an existing destination (`to`) or a newly created replacement (`project` + `title`)."
    )]
    async fn vissue_reject(
        &self,
        Parameters(args): Parameters<RejectArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(self.layout_for_id(&args.issue_id).and_then(|layout| {
            let dest =
                self.reject_destination(&layout, args.to.as_deref(), args.project.as_deref())?;
            ops::reject(
                &layout,
                &args.issue_id,
                RejectOpts {
                    to: args.to.as_deref(),
                    project: dest.project.as_deref(),
                    title: args.title.as_deref(),
                    reason: args.reason.as_deref(),
                    dst_layout: Some(&dest.layout),
                    dst_extra_id_paths: &dest.extra_id_paths,
                },
            )
        }))
    }

    #[tool(description = "Pick one terminal after a sibling close (DONE or CANCELLED).")]
    async fn vissue_resolve(
        &self,
        Parameters(args): Parameters<ResolveArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(
            self.layout_for_id(&args.issue_id)
                .and_then(|layout| ops::resolve_terminal(&layout, &args.issue_id, &args.state)),
        )
    }

    #[tool(description = "Update an issue's state, priority, or blocker edges.")]
    async fn vissue_update(
        &self,
        Parameters(args): Parameters<UpdateArgs>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = self.layout_for_id(&args.issue_id).and_then(|layout| {
            ops::update_pred(
                &layout,
                &args.issue_id,
                args.state.as_deref(),
                priority_char(args.priority.as_ref()),
                args.block.as_deref(),
                args.unblock.as_deref(),
                UpdatePred {
                    if_state: args.if_state.as_deref(),
                    if_gen: args.if_gen,
                },
            )
        });
        text(outcome.map(|o| {
            let mut s = o.report;
            for hint in o.hints {
                s.push_str(&format!("[hint] {hint}\n"));
            }
            s
        }))
    }

    #[tool(description = "Claim an issue: move it to STARTED and stamp the claiming identity.")]
    async fn vissue_claim(
        &self,
        Parameters(args): Parameters<ClaimArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(
            self.layout_for_id(&args.issue_id).and_then(|layout| {
                agent::claim(&layout, &args.issue_id, args.force.unwrap_or(false))
            }),
        )
    }

    #[tool(
        description = "Append a dated report to an issue's body. Use this to record work that was done: the logbook holds one line per event, so a written report belongs in the body. Markdown is safe."
    )]
    async fn vissue_append(
        &self,
        Parameters(args): Parameters<AppendArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(
            self.layout_for_id(&args.issue_id)
                .and_then(|layout| ops::append_body(&layout, &args.issue_id, &args.text)),
        )
    }

    #[tool(description = "Add a dated note to an issue's logbook without touching state or claim.")]
    async fn vissue_note(
        &self,
        Parameters(args): Parameters<NoteArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(
            self.layout_for_id(&args.issue_id)
                .and_then(|layout| ops::note(&layout, &args.issue_id, &args.text)),
        )
    }

    #[tool(
        description = "Cast this agent's vote on an issue, or read the tally when no choice is given. One ballot per identity: voting again replaces your own ballot and never another agent's. The tally separates a majority from a plurality and from a tie, so consult it before acting on what looks like agreement."
    )]
    async fn vissue_vote(
        &self,
        Parameters(args): Parameters<VoteArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(self.layout_for_id(&args.issue_id).and_then(|layout| {
            let who = vissue_core::config::identity(&layout);
            ops::vote(&layout, &args.issue_id, args.choice.as_deref(), &who)
        }))
    }

    #[tool(
        description = "Cite, drop, or list the deeds this issue's work produced. A deed is deedar's frozen record of a product: name the accession here when work finishes, and the next unit opens it with `deedar get` instead of rereading a transcript. Omit both lists to read the citations. Accessions are `deed-<kind>-<slug>`, or a `sha256:` of the deed or of one product path."
    )]
    async fn vissue_deed(
        &self,
        Parameters(args): Parameters<DeedArgs>,
    ) -> Result<CallToolResult, McpError> {
        let add = args.add.unwrap_or_default();
        let remove = args.remove.unwrap_or_default();
        text(
            self.layout_for_id(&args.issue_id)
                .and_then(|layout| ops::deed(&layout, &args.issue_id, &add, &remove)),
        )
    }

    #[tool(
        description = "The working set for an issue: the plan it sits in, the deeds produced by what blocks it, the issue it was bounced from, and what it has produced itself. Read this before starting work on a node. Assembled from the declared edges rather than by resemblance, so it is what the plan says the work stands on and not a ranked guess; `vissue_related` answers the resemblance question. Set `excerpts` to splice in what each input concluded, which is in its body rather than in the deed it named."
    )]
    async fn vissue_recall(
        &self,
        Parameters(args): Parameters<RecallArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(self.layout_for_id(&args.issue_id).and_then(|layout| {
            report::recall(
                &layout,
                &args.issue_id,
                args.depth.unwrap_or(1),
                args.excerpts.unwrap_or(false),
            )
        }))
    }

    #[tool(
        description = "Weigh an issue's ballots by who the group listens to (DeGroot averaging over the configured trust graph). Reports the count and the weighted position side by side, each agent's social power, and the two ways there is no consensus to report: a trust graph with more than one closed group, or one that never settles. Use it before acting on what a plurality looks like. Set `children` to roll up over a plan's children instead: that answers whether an epic can close, and it reports the children row by row rather than averaging them, because a split child has no position to fold in and an unvoted child is absent rather than neutral."
    )]
    async fn vissue_consensus(
        &self,
        Parameters(args): Parameters<ConsensusArgs>,
    ) -> Result<CallToolResult, McpError> {
        let children = args.children.unwrap_or(false);
        text(self.layout_for_id(&args.issue_id).and_then(|layout| {
            if children {
                report::plan_consensus(&layout, &args.issue_id)
            } else {
                report::consensus(&layout, &args.issue_id)
            }
        }))
    }

    #[tool(description = "Every live claim, oldest first: who holds what issue, and for how long.")]
    async fn vissue_claims(
        &self,
        Parameters(args): Parameters<ClaimsArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(report::claims(
            &self.layout,
            args.holder.as_deref(),
            args.project.as_deref(),
            args.json.unwrap_or(false),
        ))
    }

    #[tool(
        description = "Dated open work: deadlines and scheduled starts inside a horizon, overdue first."
    )]
    async fn vissue_agenda(
        &self,
        Parameters(args): Parameters<AgendaArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(report::agenda(
            &self.layout,
            args.days.unwrap_or(14),
            args.project.as_deref(),
        ))
    }

    #[tool(
        description = "Fold an inbox org file: each unstamped `* TODO` heading becomes an issue and the heading is stamped with the id in place."
    )]
    async fn vissue_fold(
        &self,
        Parameters(args): Parameters<FoldArgs>,
    ) -> Result<CallToolResult, McpError> {
        text({
            let pref = self.router.route(&args.project);
            ops::fold(&pref.layout, std::path::Path::new(&args.file), &pref.dir)
        })
    }

    #[tool(description = "Count issues, optionally filtered by project, state, or readiness.")]
    async fn vissue_count(
        &self,
        Parameters(args): Parameters<CountArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(report::count(
            &self.layout,
            args.project.as_deref(),
            args.state.as_deref(),
            args.ready.unwrap_or(false),
        ))
    }

    #[tool(description = "Substring search over ids, titles, properties, and bodies.")]
    async fn vissue_search(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(report::search(
            &self.layout,
            &args.query,
            args.limit.unwrap_or(20),
        ))
    }

    #[tool(description = "Explain bounded Org and lexical connections around an issue.")]
    async fn vissue_related(
        &self,
        Parameters(args): Parameters<RelatedArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(self.layout_for_id(&args.issue_id).and_then(|layout| {
            report::related(
                &layout,
                &args.issue_id,
                args.depth.unwrap_or(2),
                args.limit.unwrap_or(20),
                args.format.as_deref().unwrap_or("text"),
            )
        }))
    }

    #[tool(description = "List issues whose PARENT property matches this id.")]
    async fn vissue_children(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(
            self.layout_for_id(&args.issue_id)
                .and_then(|layout| report::children(&layout, &args.issue_id)),
        )
    }

    #[tool(
        description = "List issues that refer to this id through any relation, or that cite this deed accession."
    )]
    async fn vissue_backlinks(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(self.backlinks_text(&args.issue_id))
    }

    #[tool(description = "Issues waiting on this id. Dependency hygiene alias for backlinks.")]
    async fn vissue_waiting_on(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(
            self.layout_for_id(&args.issue_id)
                .and_then(|layout| agent::waiting_on(&layout, &args.issue_id)),
        )
    }

    #[tool(description = "The first lines of an issue's file range, screened for secrets.")]
    async fn vissue_body_excerpt(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(
            self.layout_for_id(&args.issue_id)
                .and_then(|layout| agent::body_excerpt(&layout, &args.issue_id)),
        )
    }

    #[tool(
        description = "One issue's org text in full, untruncated, screened for secrets. Use this when handing an issue to someone as the thing to work from; body_excerpt is a capped preview."
    )]
    async fn vissue_org(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(
            self.layout_for_id(&args.issue_id)
                .and_then(|layout| agent::org_text(&layout, &args.issue_id)),
        )
    }

    #[tool(description = "Children and blockers below an id, as ascii indent or Graphviz DOT.")]
    async fn vissue_tree(
        &self,
        Parameters(args): Parameters<TreeArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(self.layout_for_id(&args.issue_id).and_then(|layout| {
            report::tree(
                &layout,
                &args.issue_id,
                args.format.as_deref().unwrap_or("ascii"),
            )
        }))
    }

    #[tool(description = "The blocker and parent graph as Graphviz DOT.")]
    async fn vissue_graph(
        &self,
        Parameters(args): Parameters<ProjectArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(report::graph(&self.layout, args.project.as_deref()))
    }

    #[tool(description = "A markdown roadmap of active and closed work.")]
    async fn vissue_roadmap(
        &self,
        Parameters(args): Parameters<ProjectArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(report::roadmap(&self.layout, args.project.as_deref()))
    }

    #[tool(description = "One JSON object per issue per line.")]
    async fn vissue_export(
        &self,
        Parameters(args): Parameters<ProjectArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(report::export(&self.layout, args.project.as_deref()))
    }

    #[tool(description = "Validate the corpus: dangling edges, bad dates, duplicate ids.")]
    async fn vissue_check(&self) -> Result<CallToolResult, McpError> {
        text(report::check(&self.layout).map(|r| r.text))
    }

    #[tool(
        description = "Rewrite files onto the Org / ELPA / vissue property split. Dry-run by default when dry_run is true."
    )]
    async fn vissue_normalize(
        &self,
        Parameters(args): Parameters<NormalizeArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(ops::normalize(
            &self.layout,
            args.project.as_deref(),
            args.dry_run.unwrap_or(false),
        ))
    }

    #[tool(description = "Checklist for agents and CI: stalled claims plus corpus validation.")]
    async fn vissue_hygiene(
        &self,
        Parameters(args): Parameters<HygieneArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(agent::hygiene(&self.layout, args.stale_days))
    }

    #[tool(
        description = "Content digest of the corpus: combined, per-project, issue count, generation."
    )]
    async fn vissue_digest(
        &self,
        Parameters(args): Parameters<DigestArgs>,
    ) -> Result<CallToolResult, McpError> {
        json(
            vissue_core::digest::corpus_digest(&self.layout, &args.projects.unwrap_or_default())
                .map(|d| d.to_json()),
        )
    }

    #[tool(description = "Check whether a mirror file's SYNC stamp still matches the tracker.")]
    async fn vissue_mirror_check(
        &self,
        Parameters(args): Parameters<MirrorCheckArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(
            mirror::check(
                &self.layout,
                std::path::Path::new(&args.path),
                &args.projects.unwrap_or_default(),
            )
            .map(|v| v.report),
        )
    }

    #[tool(description = "Render a read-only projection of selected projects.")]
    async fn vissue_mirror(
        &self,
        Parameters(args): Parameters<MirrorArgs>,
    ) -> Result<CallToolResult, McpError> {
        let format = match Format::parse(args.format.as_deref().unwrap_or("org")) {
            Ok(f) => f,
            Err(e) => return Err(McpError::invalid_params(format!("{e:#}"), None)),
        };
        text(mirror::render(
            &self.layout,
            &args.projects.unwrap_or_default(),
            format,
            args.state.as_deref(),
        ))
    }

    #[tool(
        description = "Change events with a sequence above `since`, plus the current generation."
    )]
    async fn vissue_events(
        &self,
        Parameters(args): Parameters<EventsArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(events::since_report(
            &self.layout,
            args.since.unwrap_or(0),
            args.limit.unwrap_or(50),
        ))
    }

    #[tool(description = "Append a manual event, waking pollers without editing an issue.")]
    async fn vissue_ping(
        &self,
        Parameters(args): Parameters<PingArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(events::ping_report(&self.layout, args.detail.as_deref()))
    }

    #[tool(description = "The generation counter. Compare against the last value seen.")]
    async fn vissue_gen(&self) -> Result<CallToolResult, McpError> {
        text(Ok::<_, vissue_core::error::Error>(format!(
            "{}\n",
            events::generation(&self.layout)
        )))
    }

    #[tool(description = "Report the server version and the resolved root and prefix.")]
    async fn vissue_identity(&self) -> Result<CallToolResult, McpError> {
        text(Ok::<_, vissue_core::error::Error>(identity_report(
            &self.layout,
            &self.router,
        )))
    }

    #[tool(description = "Transitive blocker ancestors, bounded by hop depth.")]
    async fn vissue_ancestors(
        &self,
        Parameters(args): Parameters<DepthArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(
            self.layout_for_id(&args.issue_id).and_then(|layout| {
                report::ancestors(&layout, &args.issue_id, args.depth.unwrap_or(3))
            }),
        )
    }

    #[tool(description = "Issues transitively waiting on this id, bounded by hop depth.")]
    async fn vissue_impact(
        &self,
        Parameters(args): Parameters<DepthArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(
            self.layout_for_id(&args.issue_id).and_then(|layout| {
                report::impact(&layout, &args.issue_id, args.depth.unwrap_or(3))
            }),
        )
    }

    #[tool(description = "Cycles in the blocker graph, or a line saying there are none.")]
    async fn vissue_cycles(&self) -> Result<CallToolResult, McpError> {
        text(report::cycles(&self.layout))
    }

    #[tool(description = "Move an issue heading to another project file.")]
    async fn vissue_refile(
        &self,
        Parameters(args): Parameters<RefileArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(self.layout_for_id(&args.issue_id).and_then(|layout| {
            let dest = self.router.route(&args.to);
            ops::refile_to(&layout, &args.issue_id, &dest.layout, &dest.dir)
        }))
    }

    #[tool(
        description = "Block until the generation counter passes last, or until an issue is DONE or CANCELLED when until_terminal and id are set."
    )]
    async fn vissue_wait(
        &self,
        Parameters(args): Parameters<WaitArgs>,
    ) -> Result<CallToolResult, McpError> {
        if args.until_terminal.unwrap_or(false) {
            let Some(id) = args.id.as_deref() else {
                return Err(McpError::invalid_params(
                    "--until-terminal requires id",
                    None,
                ));
            };
            return text(
                events::wait_until_terminal(
                    &match self.layout_for_id(id) {
                        Ok(layout) => layout,
                        Err(e) => return text(Err(e)),
                    },
                    id,
                    args.poll_ms.unwrap_or(200),
                    args.timeout_ms.unwrap_or(10_000),
                )
                .map(|outcome| match outcome {
                    events::TerminalWait::Done { generation } => {
                        format!("DONE {generation}\n")
                    }
                    events::TerminalWait::Cancelled { generation } => {
                        format!("CANCELLED {generation}\n")
                    }
                    events::TerminalWait::Timeout { generation, state } => {
                        format!("TIMEOUT {state} {generation}\n")
                    }
                }),
            );
        }
        let last = args.last.unwrap_or(0);
        text(
            events::wait_generation(
                &self.layout,
                last,
                args.poll_ms.unwrap_or(200),
                args.timeout_ms.unwrap_or(10_000),
            )
            .map(|generation| {
                let timed_out = if generation <= last { " timeout" } else { "" };
                format!("{generation}{timed_out}\n")
            }),
        )
    }

    #[tool(description = "The identity a claim would record.")]
    async fn vissue_whoami(&self) -> Result<CallToolResult, McpError> {
        text(Ok::<_, vissue_core::error::Error>(format!(
            "{}\n",
            vissue_core::config::identity(&self.layout)
        )))
    }
}

fn create_routed(
    router: &Router,
    project: &str,
    title: &str,
    opts: CreateOpts<'_>,
) -> vissue_core::Result<String> {
    let pref = router.route(project);
    let twins = router.extra_id_paths_for(&pref.dir);
    let opts = CreateOpts {
        extra_id_paths: &twins,
        ..opts
    };
    ops::create(&pref.layout, &pref.dir, title, opts)
}

fn issues_json_routed(
    router: &Router,
    project: Option<&str>,
    state: Option<&str>,
    ready_only: bool,
) -> vissue_core::Result<serde_json::Value> {
    if let Some(p) = project {
        let pref = router.route(p);
        return agent::issues_json(&pref.layout, Some(&pref.dir), state, ready_only);
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

fn identity_report(layout: &Layout, router: &Router) -> String {
    let mut out = format!(
        "vissue-mcp {}\nprotocol: {}\nroot:   {}\nprefix: {}\nroot={}\nprefix={}\n",
        env!("CARGO_PKG_VERSION"),
        vissue_core::org::PROTOCOL_VERSION,
        layout.root().display(),
        layout.prefix(),
        layout.root().display(),
        layout.prefix()
    );
    if let Ok(prefs) = router.visible_projects() {
        for pref in prefs {
            if pref.key == pref.dir
                && pref.layout.root() == layout.root()
                && pref.layout.prefix() == layout.prefix()
            {
                continue;
            }
            out.push_str(&format!(
                "route: {} -> {} {} {}\n",
                pref.key,
                pref.layout.root().display(),
                pref.layout.prefix(),
                pref.dir
            ));
        }
    }
    out
}

#[tool_handler]
impl ServerHandler for VissueServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("vissue", env!("CARGO_PKG_VERSION")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vissue_core::config::DEFAULT_PREFIX;

    /// The tools an agent uses to work a node: recall before, deed after.
    ///
    /// Over the tool surface rather than the library, because this is the one an
    /// agent actually reaches for, and a working set it cannot ask for is a
    /// working set it will not use.
    #[tokio::test]
    async fn the_working_set_reaches_the_tool_surface() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        std::fs::create_dir_all(layout.projects_dir()).unwrap();
        ops::create(
            &layout,
            "keys",
            "catalog the actions",
            CreateOpts::default(),
        )
        .unwrap();
        ops::create(&layout, "keys", "write the schema", CreateOpts::default()).unwrap();
        let id_of = |title: &str| -> String {
            vissue_core::store::load_all(&layout)
                .unwrap()
                .into_iter()
                .find(|(_, h)| h.title == title)
                .map(|(_, h)| h.id)
                .expect("issue")
        };
        let first = id_of("catalog the actions");
        let second = id_of("write the schema");
        ops::update(&layout, &second, None, None, Some(&first), None).unwrap();

        let server = VissueServer::with_layout(layout.clone());
        let cited = server
            .vissue_deed(Parameters(DeedArgs {
                issue_id: first.clone(),
                add: Some(vec!["deed-file-catalog".into()]),
                remove: None,
            }))
            .await
            .unwrap();
        assert_eq!(cited.is_error, Some(false));

        let recalled = server
            .vissue_recall(Parameters(RecallArgs {
                issue_id: second.clone(),
                depth: None,
                excerpts: None,
            }))
            .await
            .unwrap();
        assert_eq!(recalled.is_error, Some(false));
        let rendered = format!("{:?}", recalled.content);
        assert!(
            rendered.contains("deed-file-catalog"),
            "the input's product is what the next unit opens: {rendered}"
        );

        // A citation nothing can resolve is an error the agent sees, not a
        // value the heading quietly keeps.
        let refused = server
            .vissue_deed(Parameters(DeedArgs {
                issue_id: first,
                add: Some(vec!["/tmp/note.md".into()]),
                remove: None,
            }))
            .await;
        let message = refused.expect_err("a path is not an accession").message;
        assert!(message.contains("not a deed accession"), "{message}");
    }

    /// The consensus tool answers on an unconfigured tracker, where it is the
    /// tally as shares, and says so.
    #[tokio::test]
    async fn the_consensus_tool_answers_without_trust_configured() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        std::fs::create_dir_all(layout.projects_dir()).unwrap();
        ops::create(&layout, "api", "ship it?", CreateOpts::default()).unwrap();
        let id = vissue_core::store::load_all(&layout).unwrap()[0]
            .1
            .id
            .clone();
        for (agent, choice) in [("a", "ship"), ("b", "ship"), ("c", "hold")] {
            ops::vote(&layout, &id, Some(choice), agent).unwrap();
        }

        let server = VissueServer::with_layout(layout);
        let weighed = server
            .vissue_consensus(Parameters(ConsensusArgs {
                issue_id: id,
                children: None,
            }))
            .await
            .unwrap();
        assert_eq!(weighed.is_error, Some(false));
        let rendered = format!("{:?}", weighed.content);
        assert!(rendered.contains("trust default"), "{rendered}");
        assert!(rendered.contains("holds: ship"), "{rendered}");
    }

    #[tokio::test]
    async fn tools_answer_against_a_temporary_layout() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        std::fs::create_dir_all(layout.projects_dir()).unwrap();
        ops::create(&layout, "sample", "first", CreateOpts::default()).unwrap();

        let server = VissueServer::with_layout(layout);
        let listed = server
            .vissue_list(Parameters(ListArgs {
                project: None,
                state: None,
            }))
            .await
            .unwrap();
        assert_eq!(listed.is_error, Some(false));

        let counted = server
            .vissue_count(Parameters(CountArgs {
                project: None,
                state: None,
                ready: Some(true),
            }))
            .await
            .unwrap();
        assert_eq!(counted.is_error, Some(false));
    }

    /// The tools that write, exercised in the order an agent uses them.
    ///
    /// The read-only surface is covered above; the create/update/claim/note
    /// path was not, and it is the half that changes the corpus.
    #[tokio::test]
    async fn the_write_tools_carry_their_arguments_through_to_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        std::fs::create_dir_all(layout.projects_dir()).unwrap();
        let server = VissueServer::with_layout(layout.clone());

        let made = server
            .vissue_create(Parameters(CreateArgs {
                project: "atlas".into(),
                title: "Rotate the signing key".into(),
                priority: Some("A".into()),
                issue_type: Some("chore".into()),
                tags: Some("ops,security".into()),
                parent: None,
                body: Some("The old one expires this quarter.".into()),
                deadline: Some("[2026-06-30]".into()),
                scheduled: None,
            }))
            .await
            .unwrap();
        assert_eq!(made.is_error, Some(false));

        let file = std::fs::read_to_string(layout.project_issues_path("atlas")).unwrap();
        assert!(
            file.contains("DEADLINE") && file.contains("2026-06-30"),
            "the tool accepted a deadline and did not write it: {file}"
        );
        assert!(file.contains("Rotate the signing key"), "{file}");
        assert!(file.contains("[#A]"), "priority not carried: {file}");
        assert!(file.contains(":TYPE:       chore"), "{file}");
        assert!(
            file.contains("expires this quarter"),
            "body missing: {file}"
        );
        assert!(
            file.contains(":ops:") && file.contains(":security:"),
            "{file}"
        );

        let id = file
            .lines()
            .find_map(|l| l.trim().strip_prefix(":ID:"))
            .map(|s| s.trim().to_string())
            .expect("an id");

        // Claim, then note: neither may disturb the other's stamp.
        assert_eq!(
            server
                .vissue_claim(Parameters(ClaimArgs {
                    issue_id: id.clone(),
                    force: None,
                }))
                .await
                .unwrap()
                .is_error,
            Some(false)
        );
        assert_eq!(
            server
                .vissue_note(Parameters(NoteArgs {
                    issue_id: id.clone(),
                    text: "waiting on the vault rotation window".into(),
                }))
                .await
                .unwrap()
                .is_error,
            Some(false)
        );
        // A written report goes into the body, where markdown is safe.
        assert_eq!(
            server
                .vissue_append(Parameters(AppendArgs {
                    issue_id: id.clone(),
                    text: "## What changed\n\n* rotated the key\n".into(),
                }))
                .await
                .unwrap()
                .is_error,
            Some(false)
        );

        let after = std::fs::read_to_string(layout.project_issues_path("atlas")).unwrap();
        assert!(after.contains("## What changed"), "{after}");
        assert!(after.contains(":CLAIMED_BY:"), "{after}");
        assert!(after.contains("vault rotation window"), "{after}");

        // Closing reports the change; the tool surfaces hints alongside it.
        let closed = server
            .vissue_update(Parameters(UpdateArgs {
                issue_id: id.clone(),
                state: Some("DONE".into()),
                priority: Some("C".into()),
                block: None,
                unblock: None,
                if_state: None,
                if_gen: None,
            }))
            .await
            .unwrap();
        assert_eq!(closed.is_error, Some(false));
        let done = std::fs::read_to_string(layout.project_issues_path("atlas")).unwrap();
        assert!(done.contains("DONE"), "{done}");
        assert!(done.contains("[#C]"), "{done}");
    }

    #[tokio::test]
    async fn a_write_tool_reports_an_unknown_id_as_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        std::fs::create_dir_all(layout.projects_dir()).unwrap();
        let server = VissueServer::with_layout(layout);

        let err = server
            .vissue_note(Parameters(NoteArgs {
                issue_id: "atlas-zzzz".into(),
                text: "into the void".into(),
            }))
            .await
            .unwrap_err();
        assert!(format!("{err:?}").contains("atlas-zzzz"), "{err:?}");
    }

    #[tokio::test]
    async fn an_unknown_mirror_format_is_an_invalid_parameter() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        let server = VissueServer::with_layout(layout);
        let err = server
            .vissue_mirror(Parameters(MirrorArgs {
                projects: None,
                format: Some("pdf".into()),
                state: None,
            }))
            .await
            .unwrap_err();
        assert!(format!("{err:?}").contains("pdf"), "{err:?}");
    }

    #[tokio::test]
    async fn read_only_tools_cover_the_fixture_tracker_surface() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixture_vault");
        let server = VissueServer::with_layout(Layout::new(&root, DEFAULT_PREFIX));

        assert!(
            !server
                .vissue_projects()
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_list(Parameters(ListArgs {
                    project: Some("atlas".into()),
                    state: Some("TODO".into()),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_ready(Parameters(ProjectArgs {
                    project: Some("atlas".into()),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_show(Parameters(IdArgs {
                    issue_id: "atlas-2c3d".into(),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_claims(Parameters(ClaimsArgs {
                    holder: None,
                    project: Some("atlas".into()),
                    json: Some(true),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_agenda(Parameters(AgendaArgs {
                    days: Some(7),
                    project: Some("atlas".into()),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_search(Parameters(SearchArgs {
                    query: "fixture".into(),
                    limit: Some(5),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_related(Parameters(RelatedArgs {
                    issue_id: "atlas-1a2b".into(),
                    depth: Some(2),
                    limit: Some(5),
                    format: Some("org".into()),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_children(Parameters(IdArgs {
                    issue_id: "atlas-1a2b".into(),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_backlinks(Parameters(IdArgs {
                    issue_id: "atlas-1a2b".into(),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_waiting_on(Parameters(IdArgs {
                    issue_id: "atlas-1a2b".into(),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_org(Parameters(IdArgs {
                    issue_id: "atlas-2c3d".into(),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_body_excerpt(Parameters(IdArgs {
                    issue_id: "atlas-2c3d".into(),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_tree(Parameters(TreeArgs {
                    issue_id: "atlas-1a2b".into(),
                    format: Some("ascii".into()),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_graph(Parameters(ProjectArgs {
                    project: Some("atlas".into()),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_roadmap(Parameters(ProjectArgs {
                    project: Some("atlas".into()),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_export(Parameters(ProjectArgs {
                    project: Some("atlas".into()),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_check()
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_hygiene(Parameters(HygieneArgs {
                    stale_days: Some(30)
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_digest(Parameters(DigestArgs {
                    projects: Some(vec!["atlas".into()]),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_mirror(Parameters(MirrorArgs {
                    projects: Some(vec!["atlas".into()]),
                    format: Some("markdown".into()),
                    state: Some("TODO".into()),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_events(Parameters(EventsArgs {
                    since: Some(0),
                    limit: Some(10),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(!server.vissue_gen().await.unwrap().is_error.unwrap_or(false));
        assert!(
            !server
                .vissue_identity()
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_ancestors(Parameters(DepthArgs {
                    issue_id: "atlas-3e4f".into(),
                    depth: Some(2),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_impact(Parameters(DepthArgs {
                    issue_id: "atlas-1a2b".into(),
                    depth: Some(2),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_cycles()
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_whoami()
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_wait(Parameters(WaitArgs {
                    last: Some(0),
                    id: None,
                    until_terminal: None,
                    poll_ms: Some(10),
                    timeout_ms: Some(30),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        assert!(
            !server
                .vissue_wait(Parameters(WaitArgs {
                    last: None,
                    id: Some("atlas-4g5h".into()),
                    until_terminal: Some(true),
                    poll_ms: Some(10),
                    timeout_ms: Some(200),
                }))
                .await
                .unwrap()
                .is_error
                .unwrap_or(false)
        );
        let info = server.get_info();
        assert!(info.capabilities.tools.is_some());
    }
}
