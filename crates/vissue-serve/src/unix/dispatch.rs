//! JSON-RPC method table. Reads use the cached catalog; mutations call `ops`.

use std::time::Instant;

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use vissue_control::rpc::{
    AgendaParams, AppendParams, ClaimParams, ClaimsParams, ConsensusParams, CountParams,
    CreateParams, DeedParams, DigestParams, EventsGenResult, EventsSinceParams, EventsSinceResult,
    FoldParams, HygieneParams, IdParams, IdentityResult, InitializeResult, IssueGetResult,
    IssueListParams, IssueListResult, IssueSelected, JsonRpcError, JsonRpcRequest, JsonRpcResponse,
    MirrorCheckParams, MutResult, NormalizeParams, NoteParams, Notification, PROTOCOL_VERSION,
    PingParams, ProjectFilterParams, ProjectListResult, RecallParams, RefileParams, RejectParams,
    RelatedParams, ResolveParams, SearchParams, StaleParams, TreeParams, TreeResult, UpdateParams,
    VoteParams, WaitParams, WalkParams, error_from_core, internal_error, invalid_params,
    method_not_found, parse_initialize_params,
};
use vissue_core::catalog::{CatalogService, load_recs, tree_text_from};
use vissue_core::config::Layout;
use vissue_core::error::Error as CoreError;
use vissue_core::events;
use vissue_core::ops::{self, CreateOpts};
use vissue_core::report;
use vissue_core::store::find_by_id;
use vissue_core::views::{IssueDetail, ListQuery};

use super::catalog::{INITIAL_REVISION, load_project_recs};
use super::owner::{OwnerState, Session};
use crate::LIVE_CAPABILITIES;

#[derive(Debug)]
pub struct DispatchOut {
    pub response: Option<JsonRpcResponse>,
    pub after: Vec<Notification>,
}

pub fn dispatch_ex(state: &OwnerState, session: &mut Session, req: &JsonRpcRequest) -> DispatchOut {
    if req.is_notification() {
        return DispatchOut {
            response: None,
            after: Vec::new(),
        };
    }
    let id = req.id.clone();
    let started = Instant::now();
    let mut after = Vec::new();
    let mut result = match req.method.as_str() {
        "initialize" => dispatch_initialize(state, session, req.params.as_ref()),
        "identity/get" => dispatch_identity(state, session),
        "issue/list" => dispatch_list(state, req.params.as_ref(), false),
        "issue/ready" => dispatch_list(state, req.params.as_ref(), true),
        "issue/get" | "issue/show" => dispatch_get(state, req.params.as_ref()),
        "issue/excerpt" => dispatch_excerpt(state, req.params.as_ref()),
        "issue/search" => dispatch_search(state, req.params.as_ref()),
        "issue/claims" => dispatch_claims(state, req.params.as_ref()),
        "issue/agenda" => dispatch_agenda(state, req.params.as_ref()),
        "issue/tree" => dispatch_tree(state, req.params.as_ref()),
        "issue/related" => dispatch_related(state, req.params.as_ref()),
        "issue/children" => dispatch_children(state, req.params.as_ref()),
        "issue/ancestors" => dispatch_walk(state, req.params.as_ref(), WalkKind::Ancestors),
        "issue/impact" => dispatch_walk(state, req.params.as_ref(), WalkKind::Impact),
        "issue/backlinks" => dispatch_backlinks(state, req.params.as_ref()),
        "issue/open" => dispatch_open(state, req.params.as_ref(), &mut after),
        "issue/create" => dispatch_create(state, session, req.params.as_ref()),
        "issue/update" => dispatch_update(state, session, req.params.as_ref()),
        "issue/claim" => dispatch_claim(state, session, req.params.as_ref()),
        "issue/vote" => dispatch_vote(state, session, req.params.as_ref()),
        "issue/deed" => dispatch_deed(state, req.params.as_ref()),
        "issue/recall" => dispatch_recall(state, req.params.as_ref()),
        "issue/consensus" => dispatch_consensus(state, req.params.as_ref()),
        "issue/append" => dispatch_append(state, session, req.params.as_ref()),
        "issue/resolve" => dispatch_resolve(state, session, req.params.as_ref()),
        "issue/reject" => dispatch_reject(state, session, req.params.as_ref()),
        "issue/fold" => dispatch_fold(state, session, req.params.as_ref()),
        "issue/normalize" => dispatch_normalize(state, session, req.params.as_ref()),
        "issue/check" => dispatch_check(state, req.params.as_ref()),
        "issue/count" => dispatch_count(state, req.params.as_ref()),
        "issue/cycles" => dispatch_cycles(state),
        "issue/digest" => dispatch_digest(state, req.params.as_ref()),
        "issue/export" => dispatch_export(state, req.params.as_ref()),
        "issue/graph" => dispatch_graph(state, req.params.as_ref()),
        "issue/roadmap" => dispatch_roadmap(state, req.params.as_ref()),
        "issue/stale" => dispatch_stale(state, req.params.as_ref()),
        "issue/hygiene" => dispatch_hygiene(state, req.params.as_ref()),
        "issue/waiting_on" => dispatch_waiting_on(state, req.params.as_ref()),
        "issue/mirror_check" => dispatch_mirror_check(state, req.params.as_ref()),
        "events/ping" => dispatch_ping(state, req.params.as_ref()),
        "events/wait" => dispatch_wait(state, req.params.as_ref()),
        "issue/note" => dispatch_note(state, session, req.params.as_ref()),
        "issue/refile" => dispatch_refile(state, session, req.params.as_ref()),
        "project/list" => dispatch_projects(state),
        "events/since" => dispatch_events_since(state, req.params.as_ref()),
        "events/gen" => dispatch_events_gen(state),
        other => Err(method_not_found(other)),
    };
    if is_mutating(req.method.as_str()) {
        if let Ok(value) = result.as_mut() {
            let revision = refresh_after_write(state);
            if let Some(slot) = value.get_mut("revision") {
                *slot = json!(revision);
            }
        }
        let ms = started.elapsed().as_millis();
        let id_label = req
            .params
            .as_ref()
            .and_then(|p| p.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("-");
        let status = if result.is_ok() { "ok" } else { "err" };
        eprintln!(
            "vissue serve: {} id={id_label} ms={ms} {status}",
            req.method
        );
    }
    DispatchOut {
        response: Some(match result {
            Ok(value) => JsonRpcResponse::ok(id, value),
            Err(err) => JsonRpcResponse::err(id, err),
        }),
        after,
    }
}

/// Rebuild the catalog from disk and hand back the revision it now carries.
///
/// Mutations go through `ops`, which write the org files directly. The
/// catalog learns about a write from the watcher, which polls on its own
/// cadence, so a client that writes and then reads over the same connection
/// reads a catalog that predates its own write: `issue/get` on the id
/// `issue/create` just handed back answers "not found". The write path
/// therefore refreshes before it answers, and the watcher keeps its job of
/// picking up edits made outside this process.
///
/// A full reload, rather than the watcher's partial path, because `refile`
/// touches two projects and the handlers do not report which ones they wrote.
/// If the catalog cannot be read the previous revision stands: the write
/// already succeeded, and reporting it as failed would be worse than
/// answering from a cache the watcher is about to replace.
fn refresh_after_write(state: &OwnerState) -> u64 {
    let Ok(recs) = load_recs(&state.layout) else {
        return catalog_revision(state);
    };
    let mut cat = state.catalog.write().unwrap_or_else(|p| p.into_inner());
    cat.apply_full(&state.layout, recs, Vec::new(), None);
    cat.revision
}

fn is_mutating(method: &str) -> bool {
    matches!(
        method,
        "issue/create"
            | "issue/update"
            | "issue/claim"
            | "issue/note"
            | "issue/refile"
            | "issue/vote"
            | "issue/deed"
            | "issue/append"
            | "issue/resolve"
            | "issue/reject"
            | "issue/fold"
            | "issue/normalize"
    )
}

fn decode<T: DeserializeOwned>(params: Option<&Value>) -> Result<T, JsonRpcError> {
    let value = match params {
        None | Some(Value::Null) => Value::Object(Default::default()),
        Some(v) => v.clone(),
    };
    serde_json::from_value(value).map_err(|e| invalid_params(e.to_string()))
}

fn map_core(err: CoreError) -> JsonRpcError {
    error_from_core(&err)
}

fn map_json(err: serde_json::Error) -> JsonRpcError {
    internal_error(err.to_string())
}

fn catalog_revision(state: &OwnerState) -> u64 {
    state
        .catalog
        .read()
        .map(|c| c.revision)
        .unwrap_or(INITIAL_REVISION)
}

fn catalog_generation(state: &OwnerState) -> u64 {
    state
        .catalog
        .read()
        .map(|c| c.generation)
        .unwrap_or_else(|_| events::generation(&state.layout))
}

fn with_service<T>(
    state: &OwnerState,
    f: impl FnOnce(&CatalogService<'_>, u64, u64) -> Result<T, JsonRpcError>,
) -> Result<T, JsonRpcError> {
    let cat = state.catalog.read().unwrap_or_else(|p| p.into_inner());
    let svc = CatalogService::from_recs(&cat.issues);
    f(&svc, cat.revision, cat.generation)
}

fn dispatch_initialize(
    state: &OwnerState,
    session: &mut Session,
    params: Option<&Value>,
) -> Result<Value, JsonRpcError> {
    let params = parse_initialize_params(params.unwrap_or(&json!({})))?;
    session.agent = Some(params.agent.clone());
    let result = InitializeResult {
        protocol_version: PROTOCOL_VERSION,
        capabilities: LIVE_CAPABILITIES.iter().map(|s| (*s).to_string()).collect(),
        root: state.layout.root().display().to_string(),
        prefix: state.layout.prefix().to_string(),
        generation: catalog_generation(state),
        revision: catalog_revision(state),
        identity: params.agent,
    };
    serde_json::to_value(result).map_err(map_json)
}

fn dispatch_identity(state: &OwnerState, session: &Session) -> Result<Value, JsonRpcError> {
    let identity = session
        .agent
        .clone()
        .ok_or_else(|| invalid_params("initialize required"))?;
    let result = IdentityResult {
        identity,
        root: state.layout.root().display().to_string(),
        prefix: state.layout.prefix().to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    serde_json::to_value(result).map_err(map_json)
}

fn dispatch_list(
    state: &OwnerState,
    params: Option<&Value>,
    ready_forced: bool,
) -> Result<Value, JsonRpcError> {
    let params: IssueListParams = decode(params)?;
    with_service(state, |svc, revision, generation| {
        if let Some(since) = params.since_revision
            && since == revision
        {
            return serde_json::to_value(IssueListResult {
                issues: Vec::new(),
                total: 0,
                matched: 0,
                revision,
                generation,
                unchanged: true,
            })
            .map_err(map_json);
        }
        let ready = ready_forced || params.ready.unwrap_or(false);
        let total = svc
            .issues_rows(ListQuery {
                project: params.project.clone(),
                ..ListQuery::default()
            })
            .map_err(map_core)?
            .len() as u64;
        let matched_rows = svc
            .issues_rows(ListQuery {
                project: params.project.clone(),
                state: params.state.clone(),
                ready,
                query: params.query.clone(),
                limit: None,
                offset: None,
            })
            .map_err(map_core)?;
        let matched = matched_rows.len() as u64;
        let offset = params.offset.unwrap_or(0);
        let mut issues = if offset >= matched_rows.len() {
            Vec::new()
        } else {
            matched_rows[offset..].to_vec()
        };
        if let Some(limit) = params.limit {
            issues.truncate(limit);
        }
        serde_json::to_value(IssueListResult {
            issues,
            total,
            matched,
            revision,
            generation,
            unchanged: false,
        })
        .map_err(map_json)
    })
}

fn dispatch_get(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: IdParams = decode(params)?;
    with_service(state, |svc, revision, _| {
        let issue = svc.detail(&params.id).map_err(map_core)?;
        serde_json::to_value(IssueGetResult { issue, revision }).map_err(map_json)
    })
}

fn dispatch_excerpt(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: IdParams = decode(params)?;
    with_service(state, |svc, _, _| {
        serde_json::to_value(svc.excerpt(&params.id).map_err(map_core)?).map_err(map_json)
    })
}

fn dispatch_search(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: SearchParams = decode(params)?;
    let limit = params.limit.unwrap_or(20);
    with_service(state, |svc, _, _| {
        serde_json::to_value(svc.search(&params.query, limit).map_err(map_core)?).map_err(map_json)
    })
}

fn dispatch_claims(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: ClaimsParams = decode(params)?;
    with_service(state, |svc, _, _| {
        serde_json::to_value(
            svc.claims(params.holder.as_deref(), params.project.as_deref())
                .map_err(map_core)?,
        )
        .map_err(map_json)
    })
}

fn dispatch_agenda(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: AgendaParams = decode(params)?;
    let days = params.days.unwrap_or(14);
    with_service(state, |svc, _, _| {
        serde_json::to_value(
            svc.agenda(days, params.project.as_deref())
                .map_err(map_core)?,
        )
        .map_err(map_json)
    })
}

fn dispatch_tree(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: TreeParams = decode(params)?;
    let format = params.format.as_deref().unwrap_or("nodes");
    let cat = state.catalog.read().unwrap_or_else(|p| p.into_inner());
    match format {
        "nodes" => {
            let svc = CatalogService::from_recs(&cat.issues);
            serde_json::to_value(TreeResult::Nodes(svc.tree(&params.id).map_err(map_core)?))
                .map_err(map_json)
        }
        "ascii" | "text" | "dot" => {
            let text = tree_text_from(&cat.issues, &params.id, format).map_err(map_core)?;
            serde_json::to_value(TreeResult::Text { text }).map_err(map_json)
        }
        other => Err(invalid_params(format!(
            "unknown tree format {other:?}; allowed: nodes, ascii, dot"
        ))),
    }
}

fn dispatch_related(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: RelatedParams = decode(params)?;
    let depth = params.depth.unwrap_or(2);
    let limit = params.limit.unwrap_or(20);
    with_service(state, |svc, _, _| {
        serde_json::to_value(svc.related(&params.id, depth, limit).map_err(map_core)?)
            .map_err(map_json)
    })
}

/// `issue/deed`: cite, drop, or read the deeds an issue produced.
///
/// A mutation even when both lists are empty, as far as the dispatch table is
/// concerned, because the request that carries them is the same one. The core
/// verb reads without rewriting in that case, so an empty call still costs
/// nothing but a parse.
fn dispatch_deed(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: DeedParams = decode(params)?;
    let report =
        ops::deed(&state.layout, &params.id, &params.add, &params.remove).map_err(map_core)?;
    let issue = detail_one(&state.layout, &params.id);
    mut_result(state, report, issue)
}

/// `issue/recall`: the working set for an issue, as structure.
fn dispatch_recall(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: RecallParams = decode(params)?;
    let depth = params.depth.unwrap_or(1);
    with_service(state, |svc, _, _| {
        serde_json::to_value(
            svc.recall(&params.id, depth, params.excerpts)
                .map_err(map_core)?,
        )
        .map_err(map_json)
    })
}

/// `issue/consensus`: DeGroot over the issue's ballots, as structure.
///
/// Off the files rather than the cached catalog: the trust rows come from the
/// configuration, which the catalog does not hold, and a consensus computed
/// against a stale copy of either would be reported with the same confidence as
/// a fresh one.
fn dispatch_consensus(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: ConsensusParams = decode(params)?;
    if params.children {
        let roll = vissue_core::consensus::of_plan(&state.layout, &params.id).map_err(map_core)?;
        return serde_json::to_value(roll).map_err(map_json);
    }
    let outcome = vissue_core::consensus::of_issue(&state.layout, &params.id).map_err(map_core)?;
    serde_json::to_value(outcome).map_err(map_json)
}

fn dispatch_children(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: WalkParams = decode(params)?;
    with_service(state, |svc, _, _| {
        serde_json::to_value(svc.children(&params.id).map_err(map_core)?).map_err(map_json)
    })
}

enum WalkKind {
    Ancestors,
    Impact,
}

fn dispatch_walk(
    state: &OwnerState,
    params: Option<&Value>,
    kind: WalkKind,
) -> Result<Value, JsonRpcError> {
    let params: WalkParams = decode(params)?;
    let depth = params.depth.unwrap_or(3);
    with_service(state, |svc, _, _| {
        let rows = match kind {
            WalkKind::Ancestors => svc.ancestors(&params.id, depth),
            WalkKind::Impact => svc.impact(&params.id, depth),
        }
        .map_err(map_core)?;
        serde_json::to_value(rows).map_err(map_json)
    })
}

fn dispatch_backlinks(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: WalkParams = decode(params)?;
    with_service(state, |svc, _, _| {
        serde_json::to_value(svc.backlinks(&params.id).map_err(map_core)?).map_err(map_json)
    })
}

fn dispatch_open(
    state: &OwnerState,
    params: Option<&Value>,
    after: &mut Vec<Notification>,
) -> Result<Value, JsonRpcError> {
    let params: IdParams = decode(params)?;
    let (issue, revision) = with_service(state, |svc, revision, _| {
        Ok((svc.detail(&params.id).map_err(map_core)?, revision))
    })?;
    {
        let mut sel = state.selection.lock().unwrap_or_else(|p| p.into_inner());
        *sel = Some((issue.id.clone(), issue.project.clone()));
    }
    after.push(Notification::IssueSelected(IssueSelected {
        id: issue.id.clone(),
        project: issue.project.clone(),
    }));
    serde_json::to_value(IssueGetResult { issue, revision }).map_err(map_json)
}

fn dispatch_projects(state: &OwnerState) -> Result<Value, JsonRpcError> {
    let cat = state.catalog.read().unwrap_or_else(|p| p.into_inner());
    serde_json::to_value(ProjectListResult {
        projects: cat.projects.clone(),
        revision: cat.revision,
    })
    .map_err(map_json)
}

fn dispatch_events_since(
    state: &OwnerState,
    params: Option<&Value>,
) -> Result<Value, JsonRpcError> {
    let params: EventsSinceParams = decode(params)?;
    let limit = params.limit.unwrap_or(50);
    let events = events::since(&state.layout, params.since, limit).map_err(map_core)?;
    serde_json::to_value(EventsSinceResult {
        events,
        generation: events::generation(&state.layout),
    })
    .map_err(map_json)
}

fn dispatch_events_gen(state: &OwnerState) -> Result<Value, JsonRpcError> {
    serde_json::to_value(EventsGenResult {
        generation: events::generation(&state.layout),
        revision: catalog_revision(state),
    })
    .map_err(map_json)
}

fn resolve_agent(session: &Session, override_agent: Option<&str>) -> Result<String, JsonRpcError> {
    if let Some(agent) = override_agent {
        let agent = agent.trim();
        if !agent.is_empty() {
            return Ok(agent.to_string());
        }
    }
    session
        .agent
        .clone()
        .ok_or_else(|| invalid_params("initialize required"))
}

fn mut_result(
    state: &OwnerState,
    report: String,
    issue: Option<IssueDetail>,
) -> Result<Value, JsonRpcError> {
    serde_json::to_value(MutResult {
        ok: true,
        report,
        issue,
        revision: catalog_revision(state),
        generation: events::generation(&state.layout),
    })
    .map_err(map_json)
}

fn detail_one(layout: &Layout, id: &str) -> Option<IssueDetail> {
    let (_, _, project) = find_by_id(layout, id).ok().flatten()?;
    let recs = load_project_recs(layout, &project).ok()?;
    CatalogService::from_recs(&recs).detail(id).ok()
}

fn dispatch_create(
    state: &OwnerState,
    session: &Session,
    params: Option<&Value>,
) -> Result<Value, JsonRpcError> {
    let params: CreateParams = decode(params)?;
    let _ = resolve_agent(session, params.agent.as_deref())?;
    let report = ops::create(
        &state.layout,
        &params.project,
        &params.title,
        CreateOpts {
            priority: params.priority,
            issue_type: params.issue_type.as_deref(),
            deadline: params.deadline.as_deref(),
            scheduled: params.scheduled.as_deref(),
            tags: params.tags.as_deref(),
            parent: params.parent.as_deref(),
            quiet: false,
            body: params.body.as_deref(),
            // One layout per server, so there is no twin file to reserve against.
            extra_id_paths: &[],
            extra_ids: &[],
        },
    )
    .map_err(map_core)?;
    let id = report.split_whitespace().next().unwrap_or("");
    let issue = if id.is_empty() {
        None
    } else {
        detail_one(&state.layout, id)
    };
    mut_result(state, report, issue)
}

fn dispatch_update(
    state: &OwnerState,
    session: &Session,
    params: Option<&Value>,
) -> Result<Value, JsonRpcError> {
    let params: UpdateParams = decode(params)?;
    let agent = resolve_agent(session, params.agent.as_deref())?;
    let priority = parse_priority(params.priority.as_deref())?;
    let outcome = ops::update_as_pred(
        &state.layout,
        &params.id,
        params.state.as_deref(),
        priority,
        params.block.as_deref(),
        params.unblock.as_deref(),
        &agent,
        vissue_core::UpdatePred {
            if_state: params.if_state.as_deref(),
            if_gen: params.if_gen,
        },
    )
    .map_err(map_core)?;
    let issue = detail_one(&state.layout, &params.id);
    mut_result(state, outcome.report, issue)
}

fn parse_priority(raw: Option<&str>) -> Result<Option<char>, JsonRpcError> {
    let Some(s) = raw else {
        return Ok(None);
    };
    let s = s
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_start_matches('#');
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if "ABC".contains(c) => Ok(Some(c)),
        _ => Err(invalid_params(format!("invalid priority {raw:?}"))),
    }
}

fn dispatch_claim(
    state: &OwnerState,
    session: &Session,
    params: Option<&Value>,
) -> Result<Value, JsonRpcError> {
    let params: ClaimParams = decode(params)?;
    let agent = resolve_agent(session, params.agent.as_deref())?;
    let report =
        ops::claim_as(&state.layout, &params.id, params.force, &agent).map_err(map_core)?;
    let issue = detail_one(&state.layout, &params.id);
    mut_result(state, report, issue)
}

/// A ballot is cast as the session's agent, not as the process identity, because
/// one server serves several agents and the ballot has to name the one that voted.
fn dispatch_vote(
    state: &OwnerState,
    session: &Session,
    params: Option<&Value>,
) -> Result<Value, JsonRpcError> {
    let params: VoteParams = decode(params)?;
    let agent = resolve_agent(session, params.agent.as_deref())?;
    let report =
        ops::vote(&state.layout, &params.id, params.choice.as_deref(), &agent).map_err(map_core)?;
    let issue = detail_one(&state.layout, &params.id);
    mut_result(state, report, issue)
}

/// The five verbs that used to exist only on the command line.
///
/// A client that had to reach for the command line for `append` was writing the
/// same files behind the server's back. The locks make that safe, so this is not a
/// correctness fix; it is what lets the socket be a complete surface rather than
/// most of one, and a caller can now do everything over one connection and see the
/// change stream for all of it.
fn dispatch_append(
    state: &OwnerState,
    session: &Session,
    params: Option<&Value>,
) -> Result<Value, JsonRpcError> {
    let params: AppendParams = decode(params)?;
    let agent = resolve_agent(session, params.agent.as_deref())?;
    let report =
        ops::append_body_as(&state.layout, &params.id, &params.text, &agent).map_err(map_core)?;
    let issue = detail_one(&state.layout, &params.id);
    mut_result(state, report, issue)
}

fn dispatch_resolve(
    state: &OwnerState,
    session: &Session,
    params: Option<&Value>,
) -> Result<Value, JsonRpcError> {
    let params: ResolveParams = decode(params)?;
    let _ = resolve_agent(session, None)?;
    let report =
        ops::resolve_terminal(&state.layout, &params.id, &params.state).map_err(map_core)?;
    let issue = detail_one(&state.layout, &params.id);
    mut_result(state, report, issue)
}

fn dispatch_reject(
    state: &OwnerState,
    session: &Session,
    params: Option<&Value>,
) -> Result<Value, JsonRpcError> {
    let params: RejectParams = decode(params)?;
    let _ = resolve_agent(session, None)?;
    // One layout per server, so the successor lands here and there is no twin file
    // to reserve against.
    let report = ops::reject(
        &state.layout,
        &params.id,
        ops::RejectOpts {
            to: params.to.as_deref(),
            project: params.project.as_deref(),
            title: params.title.as_deref(),
            reason: params.reason.as_deref(),
            dst_layout: None,
            dst_extra_id_paths: &[],
        },
    )
    .map_err(map_core)?;
    let issue = detail_one(&state.layout, &params.id);
    mut_result(state, report, issue)
}

fn dispatch_fold(
    state: &OwnerState,
    session: &Session,
    params: Option<&Value>,
) -> Result<Value, JsonRpcError> {
    let params: FoldParams = decode(params)?;
    let _ = resolve_agent(session, None)?;
    let project =
        ops::resolve_project(&state.layout, params.project.as_deref()).map_err(map_core)?;
    let report =
        ops::fold(&state.layout, std::path::Path::new(&params.file), &project).map_err(map_core)?;
    mut_result(state, report, None)
}

fn dispatch_normalize(
    state: &OwnerState,
    session: &Session,
    params: Option<&Value>,
) -> Result<Value, JsonRpcError> {
    let params: NormalizeParams = decode(params)?;
    let _ = resolve_agent(session, None)?;
    let report = ops::normalize(&state.layout, params.project.as_deref(), params.dry_run)
        .map_err(map_core)?;
    mut_result(state, report, None)
}

/// The reads that had no method, so a socket client had to shell out for them.
///
/// One layout per server, so each calls the single-layout core function directly
/// rather than the routed helper the command line needs. Every one returns text,
/// because that is what the report functions produce and inventing a structure here
/// would be a second contract to keep in step with the first.
fn text_result(report: Result<String, vissue_core::error::Error>) -> Result<Value, JsonRpcError> {
    Ok(json!({"report": report.map_err(map_core)?}))
}

fn dispatch_check(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let _params: ProjectFilterParams = decode(params)?;
    let report = report::check(&state.layout).map_err(map_core)?;
    // The counts travel beside the text, because the command line exits non-zero on
    // an error count and a client needs the same signal without parsing prose.
    Ok(json!({
        "report": report.text,
        "errors": report.errors,
        "warnings": report.warnings,
    }))
}

fn dispatch_count(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: CountParams = decode(params)?;
    text_result(report::count(
        &state.layout,
        params.project.as_deref(),
        params.state.as_deref(),
        params.ready_only,
    ))
}

fn dispatch_cycles(state: &OwnerState) -> Result<Value, JsonRpcError> {
    text_result(report::cycles(&state.layout))
}

fn dispatch_digest(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: DigestParams = decode(params)?;
    let digest =
        vissue_core::digest::corpus_digest(&state.layout, &params.projects).map_err(map_core)?;
    // generation as well, which `digest --json` has always carried and this method
    // omitted: a client comparing a digest across time needs to know which event-log
    // generation it was taken at, and had no way to see that it was missing.
    Ok(json!({
        "combined": digest.combined,
        "issues": digest.issues,
        "generation": digest.generation,
        "projects": digest
            .projects
            .iter()
            .map(|p| json!({"project": p.project, "digest": p.digest, "issues": p.issues}))
            .collect::<Vec<_>>(),
    }))
}

fn dispatch_export(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: ProjectFilterParams = decode(params)?;
    text_result(report::export(&state.layout, params.project.as_deref()))
}

/// Grouped per project, the way the command line groups it.
///
/// `report::graph` over a whole layout emits every node and then every edge, while
/// the command line emits a project's nodes and edges together, because it builds one
/// document out of per-project bodies. Both are valid dot and they are not the same
/// bytes, and two surfaces answering one question differently is the kind of
/// divergence nobody notices until they diff it.
fn dispatch_graph(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: ProjectFilterParams = decode(params)?;
    if params.project.is_some() {
        return text_result(report::graph(&state.layout, params.project.as_deref()));
    }
    let body = per_project(state, report::graph_body)?;
    Ok(json!({"report": format!("{}{body}{}", report::GRAPH_HEADER, report::GRAPH_FOOTER)}))
}

/// Concatenate a per-project body over every project in this layout.
fn per_project(
    state: &OwnerState,
    mut f: impl FnMut(&Layout, Option<&str>) -> vissue_core::Result<String>,
) -> Result<String, JsonRpcError> {
    let projects = vissue_core::store::list_projects(&state.layout).map_err(map_core)?;
    let mut out = String::new();
    for project in projects {
        out.push_str(&f(&state.layout, Some(&project)).map_err(map_core)?);
    }
    Ok(out)
}

/// The title belongs to the document, so it goes above the projects rather than above
/// each one, which is also how the command line builds it.
fn dispatch_roadmap(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: ProjectFilterParams = decode(params)?;
    if params.project.is_some() {
        return text_result(report::roadmap(&state.layout, params.project.as_deref()));
    }
    let body = per_project(state, report::roadmap_body)?;
    Ok(json!({"report": format!("{}{body}", report::ROADMAP_HEADER)}))
}

fn dispatch_stale(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: StaleParams = decode(params)?;
    text_result(report::stale(
        &state.layout,
        params.days,
        params.project.as_deref(),
    ))
}

fn dispatch_hygiene(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: HygieneParams = decode(params)?;
    text_result(vissue_core::agent::hygiene(
        &state.layout,
        params.stale_days,
    ))
}

fn dispatch_waiting_on(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: IdParams = decode(params)?;
    text_result(vissue_core::agent::waiting_on(&state.layout, &params.id))
}

/// Freshness of a mirror file, which is what `mirror --check` answers.
///
/// The first version of this returned the corpus digest, because a digest was to
/// hand and needed no path. That answered a question nobody asked under a name that
/// promises another: a caller asking whether its mirror is current would have been
/// told a hash and had no way to tell the difference.
///
/// Rendering a mirror is deliberately not here. It writes a file, and a file written
/// by the server lands on the server's disk rather than the caller's, so a remote
/// client would get a success and no file.
fn dispatch_mirror_check(
    state: &OwnerState,
    params: Option<&Value>,
) -> Result<Value, JsonRpcError> {
    let params: MirrorCheckParams = decode(params)?;
    let verdict = vissue_core::mirror::check(
        &state.layout,
        std::path::Path::new(&params.path),
        &params.projects,
    )
    .map_err(map_core)?;
    Ok(json!({"fresh": verdict.fresh, "report": verdict.report}))
}

fn dispatch_ping(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: PingParams = decode(params)?;
    text_result(vissue_core::events::ping_report(
        &state.layout,
        params.detail.as_deref(),
    ))
}

fn dispatch_wait(state: &OwnerState, params: Option<&Value>) -> Result<Value, JsonRpcError> {
    let params: WaitParams = decode(params)?;
    let poll = params.poll_ms.unwrap_or(200);
    let timeout = params.timeout_ms.unwrap_or(30_000);
    if let Some(id) = params.id.as_deref() {
        let reached = vissue_core::events::wait_until_terminal(&state.layout, id, poll, timeout)
            .map_err(map_core)?;
        // Matched exhaustively on purpose: a new terminal outcome should fail to
        // compile here rather than be reported as a timeout.
        let (state_name, generation, timed_out) = match reached {
            vissue_core::events::TerminalWait::Done { generation } => {
                ("DONE".to_string(), generation, false)
            }
            vissue_core::events::TerminalWait::Cancelled { generation } => {
                ("CANCELLED".to_string(), generation, false)
            }
            vissue_core::events::TerminalWait::Timeout { generation, state } => {
                (state, generation, true)
            }
        };
        return Ok(json!({
            "state": state_name,
            "generation": generation,
            "timed_out": timed_out,
        }));
    }
    // `gen` is a reserved word in this edition, so the binding is spelled out.
    let generation =
        vissue_core::events::wait_generation(&state.layout, params.last, poll, timeout)
            .map_err(map_core)?;
    Ok(json!({"generation": generation}))
}

fn dispatch_note(
    state: &OwnerState,
    session: &Session,
    params: Option<&Value>,
) -> Result<Value, JsonRpcError> {
    let params: NoteParams = decode(params)?;
    let _ = resolve_agent(session, None)?;
    let report = ops::note(&state.layout, &params.id, &params.text).map_err(map_core)?;
    let issue = detail_one(&state.layout, &params.id);
    mut_result(state, report, issue)
}

fn dispatch_refile(
    state: &OwnerState,
    session: &Session,
    params: Option<&Value>,
) -> Result<Value, JsonRpcError> {
    let params: RefileParams = decode(params)?;
    let _ = resolve_agent(session, None)?;
    let report = ops::refile(&state.layout, &params.id, &params.to).map_err(map_core)?;
    let issue = detail_one(&state.layout, &params.id);
    mut_result(state, report, issue)
}
