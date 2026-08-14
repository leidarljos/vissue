//! The MCP tool surface, calling vissue-core in process.

use rmcp::{
    handler::server::wrapper::Parameters, handler::server::ServerHandler, model::*, tool,
    tool_handler, tool_router, ErrorData as McpError,
};

use vissue_core::config::Layout;
use vissue_core::mirror::{self, Format};
use vissue_core::ops::{self, CreateOpts};
use vissue_core::store;
use vissue_core::{agent, events, report};

use crate::tools::*;

/// The tool router is built by `#[tool_handler]` through `Self::tool_router()`,
/// so the server carries only the layout it acts on.
#[derive(Clone)]
pub struct VissueServer {
    layout: Layout,
}

fn text(result: anyhow::Result<String>) -> Result<CallToolResult, McpError> {
    match result {
        Ok(s) => Ok(CallToolResult::success(vec![Content::text(s)])),
        Err(e) => Err(McpError::internal_error(format!("{e:#}"), None)),
    }
}

fn json(result: anyhow::Result<serde_json::Value>) -> Result<CallToolResult, McpError> {
    match result {
        Ok(v) => {
            let rendered = serde_json::to_string_pretty(&v).unwrap_or_else(|_| "null".to_string());
            Ok(CallToolResult::success(vec![Content::text(rendered)]))
        }
        Err(e) => Err(McpError::internal_error(format!("{e:#}"), None)),
    }
}

#[tool_router]
impl VissueServer {
    /// Resolve the layout from `VISSUE_ROOT` and `VISSUE_PREFIX`, or the
    /// current directory.
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self::with_layout(Layout::resolve(None, None)?))
    }

    pub fn with_layout(layout: Layout) -> Self {
        Self { layout }
    }

    #[tool(description = "List the projects that hold an issues.org under the tracker root.")]
    async fn vissue_projects(&self) -> Result<CallToolResult, McpError> {
        text(store::list_projects(&self.layout).map(|ps| format!("{}\n", ps.join("\n"))))
    }

    #[tool(description = "List issues, optionally filtered by project and state.")]
    async fn vissue_list(
        &self,
        Parameters(args): Parameters<ListArgs>,
    ) -> Result<CallToolResult, McpError> {
        json(agent::issues_json(
            &self.layout,
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
        json(agent::issues_json(
            &self.layout,
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
        json(agent::show_json(&self.layout, &args.issue_id))
    }

    #[tool(description = "Create an issue in a project's issues.org.")]
    async fn vissue_create(
        &self,
        Parameters(args): Parameters<CreateArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(ops::create(
            &self.layout,
            &args.project,
            &args.title,
            CreateOpts {
                priority: priority_char(args.priority.as_ref()),
                issue_type: args.issue_type.as_deref(),
                tags: args.tags.as_deref(),
                parent: args.parent.as_deref(),
                body: args.body.as_deref(),
                ..Default::default()
            },
        ))
    }

    #[tool(description = "Update an issue's state, priority, or blocker edges.")]
    async fn vissue_update(
        &self,
        Parameters(args): Parameters<UpdateArgs>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = ops::update(
            &self.layout,
            &args.issue_id,
            args.state.as_deref(),
            priority_char(args.priority.as_ref()),
            args.block.as_deref(),
            args.unblock.as_deref(),
        );
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
        text(agent::claim(
            &self.layout,
            &args.issue_id,
            args.force.unwrap_or(false),
        ))
    }

    #[tool(description = "Add a dated note to an issue's logbook without touching state or claim.")]
    async fn vissue_note(
        &self,
        Parameters(args): Parameters<NoteArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(ops::note(&self.layout, &args.issue_id, &args.text))
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
        text(ops::fold(
            &self.layout,
            std::path::Path::new(&args.file),
            &args.project,
        ))
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
        text(report::related(
            &self.layout,
            &args.issue_id,
            args.depth.unwrap_or(2),
            args.limit.unwrap_or(20),
            args.format.as_deref().unwrap_or("text"),
        ))
    }

    #[tool(description = "List issues whose PARENT property matches this id.")]
    async fn vissue_children(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(report::children(&self.layout, &args.issue_id))
    }

    #[tool(description = "List issues that refer to this id through any relation.")]
    async fn vissue_backlinks(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(report::backlinks(&self.layout, &args.issue_id))
    }

    #[tool(description = "Issues waiting on this id. Dependency hygiene alias for backlinks.")]
    async fn vissue_waiting_on(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(agent::waiting_on(&self.layout, &args.issue_id))
    }

    #[tool(description = "The first lines of an issue's file range, screened for secrets.")]
    async fn vissue_body_excerpt(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(agent::body_excerpt(&self.layout, &args.issue_id))
    }

    #[tool(description = "Children and blockers below an id, as ascii indent or Graphviz DOT.")]
    async fn vissue_tree(
        &self,
        Parameters(args): Parameters<TreeArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(report::tree(
            &self.layout,
            &args.issue_id,
            args.format.as_deref().unwrap_or("ascii"),
        ))
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
        text(Ok(format!("{}\n", events::generation(&self.layout))))
    }

    #[tool(description = "Report the server version and the resolved root and prefix.")]
    async fn vissue_identity(&self) -> Result<CallToolResult, McpError> {
        text(Ok(format!(
            "vissue-mcp {}\nroot:   {}\nprefix: {}\nroot={}\nprefix={}\n",
            env!("CARGO_PKG_VERSION"),
            self.layout.root().display(),
            self.layout.prefix(),
            self.layout.root().display(),
            self.layout.prefix()
        )))
    }

    #[tool(description = "Transitive blocker ancestors, bounded by hop depth.")]
    async fn vissue_ancestors(
        &self,
        Parameters(args): Parameters<DepthArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(report::ancestors(
            &self.layout,
            &args.issue_id,
            args.depth.unwrap_or(3),
        ))
    }

    #[tool(description = "Issues transitively waiting on this id, bounded by hop depth.")]
    async fn vissue_impact(
        &self,
        Parameters(args): Parameters<DepthArgs>,
    ) -> Result<CallToolResult, McpError> {
        text(report::impact(
            &self.layout,
            &args.issue_id,
            args.depth.unwrap_or(3),
        ))
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
        text(ops::refile(&self.layout, &args.issue_id, &args.to))
    }

    #[tool(description = "Block until the generation counter passes last. Returns the generation.")]
    async fn vissue_wait(
        &self,
        Parameters(args): Parameters<WaitArgs>,
    ) -> Result<CallToolResult, McpError> {
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
        text(Ok(format!(
            "{}\n",
            vissue_core::config::identity(&self.layout)
        )))
    }
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

        assert!(!server
            .vissue_projects()
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_list(Parameters(ListArgs {
                project: Some("atlas".into()),
                state: Some("TODO".into()),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_ready(Parameters(ProjectArgs {
                project: Some("atlas".into()),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_show(Parameters(IdArgs {
                issue_id: "atlas-2c3d".into(),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_claims(Parameters(ClaimsArgs {
                holder: None,
                project: Some("atlas".into()),
                json: Some(true),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_agenda(Parameters(AgendaArgs {
                days: Some(7),
                project: Some("atlas".into()),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_search(Parameters(SearchArgs {
                query: "fixture".into(),
                limit: Some(5),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_related(Parameters(RelatedArgs {
                issue_id: "atlas-1a2b".into(),
                depth: Some(2),
                limit: Some(5),
                format: Some("org".into()),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_children(Parameters(IdArgs {
                issue_id: "atlas-1a2b".into(),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_backlinks(Parameters(IdArgs {
                issue_id: "atlas-1a2b".into(),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_waiting_on(Parameters(IdArgs {
                issue_id: "atlas-1a2b".into(),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_body_excerpt(Parameters(IdArgs {
                issue_id: "atlas-2c3d".into(),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_tree(Parameters(TreeArgs {
                issue_id: "atlas-1a2b".into(),
                format: Some("ascii".into()),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_graph(Parameters(ProjectArgs {
                project: Some("atlas".into()),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_roadmap(Parameters(ProjectArgs {
                project: Some("atlas".into()),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_export(Parameters(ProjectArgs {
                project: Some("atlas".into()),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_check()
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_hygiene(Parameters(HygieneArgs {
                stale_days: Some(30)
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_digest(Parameters(DigestArgs {
                projects: Some(vec!["atlas".into()]),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_mirror(Parameters(MirrorArgs {
                projects: Some(vec!["atlas".into()]),
                format: Some("markdown".into()),
                state: Some("TODO".into()),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_events(Parameters(EventsArgs {
                since: Some(0),
                limit: Some(10),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server.vissue_gen().await.unwrap().is_error.unwrap_or(false));
        assert!(!server
            .vissue_identity()
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_ancestors(Parameters(DepthArgs {
                issue_id: "atlas-3e4f".into(),
                depth: Some(2),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_impact(Parameters(DepthArgs {
                issue_id: "atlas-1a2b".into(),
                depth: Some(2),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_cycles()
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_whoami()
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert!(!server
            .vissue_wait(Parameters(WaitArgs {
                last: Some(0),
                poll_ms: Some(10),
                timeout_ms: Some(30),
            }))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        let info = server.get_info();
        assert!(info.capabilities.tools.is_some());
    }
}
