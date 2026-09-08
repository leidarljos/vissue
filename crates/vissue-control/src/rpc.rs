//! JSON-RPC 2.0 types. Handshake is camelCase; issue payloads are snake_case.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::frame::FrameError;
use vissue_core::error::Error as CoreError;
use vissue_core::views::{
    AgendaRow, ClaimRow, Excerpt, IssueDetail, IssueRow, Recall, RelatedHit, SearchHit, TreeNode,
    WalkHit,
};

/// One entry in the on-disk change log.
pub use vissue_core::events::Event;

/// Protocol version accepted by `initialize`.
pub const PROTOCOL_VERSION: u32 = 1;

/// JSON-RPC parse error (`-32700`).
pub const PARSE_ERROR: i32 = -32700;
/// JSON-RPC invalid request (`-32600`).
pub const INVALID_REQUEST: i32 = -32600;
/// JSON-RPC method not found (`-32601`).
pub const METHOD_NOT_FOUND: i32 = -32601;
/// JSON-RPC invalid params (`-32602`).
pub const INVALID_PARAMS: i32 = -32602;
/// JSON-RPC internal error (`-32603`).
pub const INTERNAL_ERROR: i32 = -32603;
/// Issue not found (`-32004`).
pub const NOT_FOUND: i32 = -32004;
/// Claim conflict (`-32009`).
pub const CONFLICT: i32 = -32009;
/// Closed issue or invalid state (`-32010`).
pub const INVALID_STATE: i32 = -32010;
/// Blocker cycle (`-32022`).
pub const CYCLE: i32 = -32022;

/// Catalog rebuilt. Params: [`VaultChanged`].
pub const NOTIFY_VAULT_CHANGED: &str = "vault/changed";
/// Shared selection. Params: [`IssueSelected`].
pub const NOTIFY_ISSUE_SELECTED: &str = "issue/selected";
/// Owner is exiting. Params: `{}`.
pub const NOTIFY_SHUTTING_DOWN: &str = "serve/shutting_down";

/// Wire-level failure for a control client or dispatcher.
#[derive(Debug)]
pub enum Error {
    /// Socket or file I/O.
    Io(std::io::Error),
    /// JSON encode or decode.
    Json(serde_json::Error),
    /// Frame read or write.
    Frame(FrameError),
    /// Server JSON-RPC error object.
    Rpc(JsonRpcError),
    /// Method or platform the client cannot handle.
    Unsupported(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(err) => write!(f, "{err}"),
            Error::Json(err) => write!(f, "{err}"),
            Error::Frame(err) => write!(f, "{err}"),
            Error::Rpc(err) => write!(f, "{}", err.message),
            Error::Unsupported(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(err) => Some(err),
            Error::Json(err) => Some(err),
            Error::Frame(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err)
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Json(err)
    }
}

impl From<FrameError> for Error {
    fn from(err: FrameError) -> Self {
        Error::Frame(err)
    }
}

impl From<JsonRpcError> for Error {
    fn from(err: JsonRpcError) -> Self {
        Error::Rpc(err)
    }
}

/// JSON-RPC request or notification id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    /// Numeric id.
    Number(i64),
    /// String id.
    String(String),
    /// JSON `null`. A response, never a notification.
    Null,
}

/// JSON-RPC 2.0 request or notification envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Present on a call; absent on a notification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<JsonRpcId>,
    /// Method name.
    pub method: String,
    /// Params object, or omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    /// Request with `id`.
    pub fn call(id: JsonRpcId, method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id: Some(id),
            method: method.into(),
            params: Some(params),
        }
    }

    /// Notification (no `id`).
    pub fn notification(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id: None,
            method: method.into(),
            params: Some(params),
        }
    }

    /// True when `id` is absent.
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }
}

/// JSON-RPC 2.0 response envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Request id echoed back. `None` or [`JsonRpcId::Null`] on some errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<JsonRpcId>,
    /// Success body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Failure body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    /// Success response.
    pub fn ok(id: Option<JsonRpcId>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Error response.
    pub fn err(id: Option<JsonRpcId>, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// JSON-RPC error object. Application codes carry `data.code`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// JSON-RPC or application numeric code.
    pub code: i32,
    /// Human-readable message.
    pub message: String,
    /// Optional payload. Application codes put `code` here as a string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// `-32700` parse error.
pub fn parse_error() -> JsonRpcError {
    JsonRpcError {
        code: PARSE_ERROR,
        message: "parse error".into(),
        data: None,
    }
}

/// `-32600` invalid request.
pub fn invalid_request() -> JsonRpcError {
    JsonRpcError {
        code: INVALID_REQUEST,
        message: "invalid request".into(),
        data: None,
    }
}

/// `-32601` method not found. `data.method` is `method`.
pub fn method_not_found(method: &str) -> JsonRpcError {
    JsonRpcError {
        code: METHOD_NOT_FOUND,
        message: "method not found".into(),
        data: Some(json!({ "method": method })),
    }
}

/// `-32602` invalid params with `message`.
pub fn invalid_params(message: impl Into<String>) -> JsonRpcError {
    JsonRpcError {
        code: INVALID_PARAMS,
        message: message.into(),
        data: None,
    }
}

/// `-32603` internal error with `message`.
pub fn internal_error(message: impl Into<String>) -> JsonRpcError {
    JsonRpcError {
        code: INTERNAL_ERROR,
        message: message.into(),
        data: None,
    }
}

/// Map a typed core error onto the control-plane codes.
pub fn error_from_core(err: &CoreError) -> JsonRpcError {
    match err {
        CoreError::IssueNotFound { id } => JsonRpcError {
            code: NOT_FOUND,
            message: err.to_string(),
            data: Some(json!({ "code": "not_found", "id": id })),
        },
        CoreError::DuplicateId { id, paths } => JsonRpcError {
            code: CONFLICT,
            message: err.to_string(),
            data: Some(json!({
                "code": "duplicate_id",
                "id": id,
                "paths": paths,
            })),
        },
        CoreError::ClaimConflict { id, holder, .. } => JsonRpcError {
            code: CONFLICT,
            message: err.to_string(),
            data: Some(json!({ "code": "conflict", "id": id, "holder": holder })),
        },
        CoreError::BlockerCycle { blocker, issue } => JsonRpcError {
            code: CYCLE,
            message: err.to_string(),
            data: Some(json!({ "code": "cycle", "id": issue, "block": blocker })),
        },
        CoreError::InvalidState { id, state } => JsonRpcError {
            code: INVALID_STATE,
            message: err.to_string(),
            data: Some(json!({ "code": "invalid_state", "id": id, "state": state })),
        },
        CoreError::StaleWrite {
            id,
            expected_state,
            actual_state,
            expected_gen,
            actual_gen,
        } => JsonRpcError {
            code: INVALID_STATE,
            message: err.to_string(),
            data: Some(json!({
                "code": "stale",
                "id": id,
                "expected_state": expected_state,
                "actual_state": actual_state,
                "expected_gen": expected_gen,
                "actual_gen": actual_gen,
            })),
        },
        CoreError::TerminalConflict {
            id,
            held,
            attempted,
        } => JsonRpcError {
            code: CONFLICT,
            message: err.to_string(),
            data: Some(json!({
                "code": "terminal_conflict",
                "id": id,
                "held": held,
                "attempted": attempted,
            })),
        },
        CoreError::Other(_) => internal_error(err.to_string()),
    }
}

/// v1 methods the owner advertises on `initialize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    /// Handshake. Not listed in [`V1_CAPABILITIES`].
    Initialize,
    /// Process identity, root, prefix, and crate version.
    IdentityGet,
    /// Filtered issue rows.
    IssueList,
    /// One issue plus revision.
    IssueGet,
    /// Frontier: `issue/list` with `ready: true`.
    IssueReady,
    /// Substring search over id, title, properties, tags, and body.
    IssueSearch,
    /// Live claims.
    IssueClaims,
    /// Deadlines and scheduled starts.
    IssueAgenda,
    /// Alias of [`Self::IssueGet`].
    IssueShow,
    /// Secret-screened body range.
    IssueExcerpt,
    /// Children and blockers.
    IssueTree,
    /// Bounded neighborhood with evidence.
    IssueRelated,
    /// Direct children.
    IssueChildren,
    /// Walk up the blocker graph.
    IssueAncestors,
    /// Walk down the blocker graph.
    IssueImpact,
    /// Everything pointing at the id.
    IssueBacklinks,
    /// Shared selection; notifies `issue/selected`.
    IssueOpen,
    /// Create an issue.
    IssueCreate,
    /// State, priority, block, unblock.
    IssueUpdate,
    /// Take the issue.
    IssueClaim,
    /// Dated logbook entry.
    IssueNote,
    /// Move to another project.
    IssueRefile,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueAppend,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueReject,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueResolve,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueVote,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueDeed,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueRecall,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueConsensus,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueFold,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueNormalize,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueCheck,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueCount,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueCycles,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueDigest,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueExport,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueGraph,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueRoadmap,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueStale,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueHygiene,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueWaitingOn,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    IssueMirror,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    EventsPing,
    /// Operation added after v1's first draft; see schema/vissue.capnp.
    EventsWait,
    /// Project names plus revision.
    ProjectList,
    /// Pull of the on-disk event log.
    EventsSince,
    /// Current generation and revision.
    EventsGen,
}

impl Method {
    /// Wire method name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Initialize => "initialize",
            Self::IdentityGet => "identity/get",
            Self::IssueList => "issue/list",
            Self::IssueGet => "issue/get",
            Self::IssueReady => "issue/ready",
            Self::IssueSearch => "issue/search",
            Self::IssueClaims => "issue/claims",
            Self::IssueAgenda => "issue/agenda",
            Self::IssueShow => "issue/show",
            Self::IssueExcerpt => "issue/excerpt",
            Self::IssueTree => "issue/tree",
            Self::IssueRelated => "issue/related",
            Self::IssueChildren => "issue/children",
            Self::IssueAncestors => "issue/ancestors",
            Self::IssueImpact => "issue/impact",
            Self::IssueBacklinks => "issue/backlinks",
            Self::IssueOpen => "issue/open",
            Self::IssueCreate => "issue/create",
            Self::IssueUpdate => "issue/update",
            Self::IssueClaim => "issue/claim",
            Self::IssueNote => "issue/note",
            Self::IssueRefile => "issue/refile",
            Self::ProjectList => "project/list",
            Self::EventsSince => "events/since",
            Self::EventsGen => "events/gen",
            Self::IssueAppend => "issue/append",
            Self::IssueReject => "issue/reject",
            Self::IssueResolve => "issue/resolve",
            Self::IssueVote => "issue/vote",
            Self::IssueDeed => "issue/deed",
            Self::IssueRecall => "issue/recall",
            Self::IssueConsensus => "issue/consensus",
            Self::IssueFold => "issue/fold",
            Self::IssueNormalize => "issue/normalize",
            Self::IssueCheck => "issue/check",
            Self::IssueCount => "issue/count",
            Self::IssueCycles => "issue/cycles",
            Self::IssueDigest => "issue/digest",
            Self::IssueExport => "issue/export",
            Self::IssueGraph => "issue/graph",
            Self::IssueRoadmap => "issue/roadmap",
            Self::IssueStale => "issue/stale",
            Self::IssueHygiene => "issue/hygiene",
            Self::IssueWaitingOn => "issue/waiting_on",
            Self::IssueMirror => "issue/mirror_check",
            Self::EventsPing => "events/ping",
            Self::EventsWait => "events/wait",
        }
    }

    /// Parse a v1 wire name.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is not a v1 method.
    pub fn parse(name: &str) -> Result<Self, JsonRpcError> {
        match name {
            "initialize" => Ok(Self::Initialize),
            "identity/get" => Ok(Self::IdentityGet),
            "issue/list" => Ok(Self::IssueList),
            "issue/get" => Ok(Self::IssueGet),
            "issue/ready" => Ok(Self::IssueReady),
            "issue/search" => Ok(Self::IssueSearch),
            "issue/claims" => Ok(Self::IssueClaims),
            "issue/agenda" => Ok(Self::IssueAgenda),
            "issue/show" => Ok(Self::IssueShow),
            "issue/excerpt" => Ok(Self::IssueExcerpt),
            "issue/tree" => Ok(Self::IssueTree),
            "issue/related" => Ok(Self::IssueRelated),
            "issue/children" => Ok(Self::IssueChildren),
            "issue/ancestors" => Ok(Self::IssueAncestors),
            "issue/impact" => Ok(Self::IssueImpact),
            "issue/backlinks" => Ok(Self::IssueBacklinks),
            "issue/open" => Ok(Self::IssueOpen),
            "issue/create" => Ok(Self::IssueCreate),
            "issue/update" => Ok(Self::IssueUpdate),
            "issue/claim" => Ok(Self::IssueClaim),
            "issue/note" => Ok(Self::IssueNote),
            "issue/refile" => Ok(Self::IssueRefile),
            "project/list" => Ok(Self::ProjectList),
            "events/since" => Ok(Self::EventsSince),
            "events/gen" => Ok(Self::EventsGen),
            "issue/append" => Ok(Self::IssueAppend),
            "issue/reject" => Ok(Self::IssueReject),
            "issue/resolve" => Ok(Self::IssueResolve),
            "issue/vote" => Ok(Self::IssueVote),
            "issue/deed" => Ok(Self::IssueDeed),
            "issue/recall" => Ok(Self::IssueRecall),
            "issue/consensus" => Ok(Self::IssueConsensus),
            "issue/fold" => Ok(Self::IssueFold),
            "issue/normalize" => Ok(Self::IssueNormalize),
            "issue/check" => Ok(Self::IssueCheck),
            "issue/count" => Ok(Self::IssueCount),
            "issue/cycles" => Ok(Self::IssueCycles),
            "issue/digest" => Ok(Self::IssueDigest),
            "issue/export" => Ok(Self::IssueExport),
            "issue/graph" => Ok(Self::IssueGraph),
            "issue/roadmap" => Ok(Self::IssueRoadmap),
            "issue/stale" => Ok(Self::IssueStale),
            "issue/hygiene" => Ok(Self::IssueHygiene),
            "issue/waiting_on" => Ok(Self::IssueWaitingOn),
            "issue/mirror_check" => Ok(Self::IssueMirror),
            "events/ping" => Ok(Self::EventsPing),
            "events/wait" => Ok(Self::EventsWait),
            other => Err(method_not_found(other)),
        }
    }
}

/// Capability strings returned by `initialize` (v1). `initialize` itself is omitted.
///
/// This is the fourth place the method set is written down, after the dispatch table,
/// the schema and the reference, and it is the one a client reads to decide what it
/// may call. It fell nineteen methods behind while the other three agreed with each
/// other, so a client inspecting capabilities would have concluded that append,
/// vote, fold and every read added beside them did not exist.
///
/// `capabilities_match_the_schema` in vissue-serve holds this to the schema now.
pub const V1_CAPABILITIES: &[&str] = &[
    "issue/list",
    "issue/get",
    "issue/ready",
    "issue/search",
    "issue/claims",
    "issue/agenda",
    "issue/show",
    "issue/excerpt",
    "issue/tree",
    "issue/related",
    "issue/children",
    "issue/ancestors",
    "issue/impact",
    "issue/backlinks",
    "issue/open",
    "issue/create",
    "issue/update",
    "issue/claim",
    "issue/note",
    "issue/refile",
    "issue/append",
    "issue/reject",
    "issue/resolve",
    "issue/vote",
    "issue/deed",
    "issue/recall",
    "issue/consensus",
    "issue/fold",
    "issue/normalize",
    "issue/check",
    "issue/count",
    "issue/cycles",
    "issue/digest",
    "issue/export",
    "issue/graph",
    "issue/roadmap",
    "issue/stale",
    "issue/hygiene",
    "issue/waiting_on",
    "issue/mirror_check",
    "project/list",
    "events/since",
    "events/gen",
    "events/ping",
    "events/wait",
    "identity/get",
];

/// `initialize` params. camelCase on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    /// Must be [`PROTOCOL_VERSION`].
    pub protocol_version: u32,
    /// Client name, e.g. `vissue-tui`. Empty when omitted.
    #[serde(default)]
    pub client: String,
    /// Connection identity. Required and non-empty.
    pub agent: String,
}

/// `initialize` result. camelCase on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    /// Echo of [`PROTOCOL_VERSION`].
    pub protocol_version: u32,
    /// Advertised methods. See [`V1_CAPABILITIES`].
    pub capabilities: Vec<String>,
    /// Tracker root the owner bound.
    pub root: String,
    /// Layout prefix the owner bound.
    pub prefix: String,
    /// On-disk generation counter.
    pub generation: u64,
    /// Serve-local catalog revision. Starts at 1.
    pub revision: u64,
    /// Owner identity.
    pub identity: String,
}

/// Parse `initialize` params. Missing/empty `agent` and version != 1 are -32602.
///
/// # Errors
///
/// Returns an error when `value` is not an object, `protocolVersion` is
/// missing, not a number, or not 1, or `agent` is missing or empty.
pub fn parse_initialize_params(value: &Value) -> Result<InitializeParams, JsonRpcError> {
    let obj = value
        .as_object()
        .ok_or_else(|| invalid_params("params must be an object"))?;
    let version = match obj.get("protocolVersion") {
        Some(Value::Number(n)) => n
            .as_u64()
            .ok_or_else(|| invalid_params("protocolVersion must be a number"))?,
        Some(_) => return Err(invalid_params("protocolVersion must be a number")),
        None => return Err(invalid_params("protocolVersion is required")),
    };
    if version != u64::from(PROTOCOL_VERSION) {
        return Err(JsonRpcError {
            code: INVALID_PARAMS,
            message: "unsupported protocol version".into(),
            data: Some(json!({ "supported": PROTOCOL_VERSION })),
        });
    }
    let agent = match obj.get("agent") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
        _ => return Err(invalid_params("agent is required")),
    };
    let client = obj
        .get("client")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Ok(InitializeParams {
        protocol_version: PROTOCOL_VERSION,
        client,
        agent,
    })
}

/// Filters for `issue/list` and `issue/ready`. snake_case on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IssueListParams {
    /// Restrict to this project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Restrict to this TODO keyword.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// When `true`, only the frontier (no open blockers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<bool>,
    /// Case-insensitive substring over id, title, tags, and properties.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Max rows after offset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Skip this many matching rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    /// When this equals the current revision, the result is unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_revision: Option<u64>,
}

/// Page of issue rows, or an unchanged marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IssueListResult {
    /// Matching rows. Empty when [`Self::unchanged`].
    #[serde(default)]
    pub issues: Vec<IssueRow>,
    /// Issues in the selected project (or whole vault) before other filters.
    #[serde(default)]
    pub total: u64,
    /// Rows matching state, ready, and query, before limit and offset.
    #[serde(default)]
    pub matched: u64,
    /// Current serve revision.
    pub revision: u64,
    /// Current on-disk generation.
    #[serde(default)]
    pub generation: u64,
    /// `since_revision` matched; `issues` is empty.
    #[serde(default)]
    pub unchanged: bool,
}

/// Single issue id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdParams {
    /// Issue id.
    pub id: String,
}

/// One issue plus the serve revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueGetResult {
    /// Flattened detail fields on the wire.
    #[serde(flatten)]
    pub issue: IssueDetail,
    /// Current serve revision.
    pub revision: u64,
}

/// `issue/search` params. Default `limit` is 20.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchParams {
    /// Substring over id, title, properties, tags, and body.
    pub query: String,
    /// Max hits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// `issue/claims` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ClaimsParams {
    /// Restrict to this holder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<String>,
    /// Restrict to this project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// `issue/agenda` params. Default `days` is 14.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgendaParams {
    /// Horizon in days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<i64>,
    /// Restrict to this project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// `issue/tree` params. `format` is `nodes`, `ascii`, or `dot`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeParams {
    /// Root issue id.
    pub id: String,
    /// `nodes` (default), `ascii`, or `dot`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

/// `issue/tree` result: a node graph or rendered text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TreeResult {
    /// Structured tree (`format` omitted or `nodes`).
    Nodes(TreeNode),
    /// Rendered `ascii` or `dot`.
    Text {
        /// Graph text.
        text: String,
    },
}

/// `issue/related` params. Default `depth` is 2 and `limit` is 20.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedParams {
    /// Center issue id.
    pub id: String,
    /// Graph walk depth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<usize>,
    /// Max hits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// Params for children, ancestors, impact, and backlinks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalkParams {
    /// Start issue id.
    pub id: String,
    /// Walk depth. Omitted means the method default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<usize>,
}

/// `project/list` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProjectListResult {
    /// Project names under the prefix.
    pub projects: Vec<String>,
    /// Current serve revision.
    pub revision: u64,
}

/// `events/since` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EventsSinceParams {
    /// Return events with sequence greater than this.
    pub since: u64,
    /// Max events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// Pull of the on-disk event log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventsSinceResult {
    /// Events after `since`.
    pub events: Vec<Event>,
    /// Current generation after the pull.
    pub generation: u64,
}

/// `events/gen` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventsGenResult {
    /// On-disk generation counter.
    pub generation: u64,
    /// Serve-local catalog revision.
    pub revision: u64,
}

/// `identity/get` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityResult {
    /// Connection or process identity.
    pub identity: String,
    /// Tracker root.
    pub root: String,
    /// Layout prefix.
    pub prefix: String,
    /// Crate version string.
    pub version: String,
}

/// `issue/create` params. Fields match the CLI create verb.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateParams {
    /// Target project.
    pub project: String,
    /// Heading title.
    pub title: String,
    /// Override the connection agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Priority letter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<char>,
    /// `:TYPE:` property.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_type: Option<String>,
    /// Org deadline stamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    /// Org scheduled stamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled: Option<String>,
    /// Space-separated tags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<String>,
    /// Parent issue id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Body prose written under the properties drawer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// `issue/update` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateParams {
    /// Issue id.
    pub id: String,
    /// New TODO keyword.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// New priority letter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// Add this blocker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<String>,
    /// Remove this blocker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unblock: Option<String>,
    /// Refuse unless the heading is still this state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_state: Option<String>,
    /// Refuse unless the corpus generation is still this value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_gen: Option<u64>,
    /// Override the connection agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// `issue/claim` params. `force` defaults to false.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimParams {
    /// Issue id.
    pub id: String,
    /// Take over an existing claim.
    #[serde(default)]
    pub force: bool,
    /// Override the connection agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// `issue/vote` params. `choice` absent reads the tally without casting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoteParams {
    /// Issue id.
    pub id: String,
    /// What to vote for, one line. Absent reads the tally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice: Option<String>,
    /// Override the connection agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// `issue/deed` params. Both lists absent reads the citations without writing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeedParams {
    /// Issue id.
    pub id: String,
    /// Deed accessions this issue produced.
    #[serde(default)]
    pub add: Vec<String>,
    /// Citations to drop.
    #[serde(default)]
    pub remove: Vec<String>,
}

/// `issue/recall` params. `depth` bounds the blocker walk and defaults to one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecallParams {
    /// Issue id.
    pub id: String,
    /// Hops of the blocker walk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<usize>,
    /// Include a capped excerpt of each input's heading.
    #[serde(default)]
    pub excerpts: bool,
}

/// `issue/consensus` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsensusParams {
    /// Issue id.
    pub id: String,
    /// Roll up over the issue's children instead of its own ballots.
    #[serde(default)]
    pub children: bool,
}

/// Params for the reads that take an optional project filter: `issue/export`,
/// `issue/graph`, `issue/roadmap`, `issue/cycles`, `issue/check`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFilterParams {
    /// Only this project; every project when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// `issue/count` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountParams {
    /// Only this project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Only this state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Only issues with no live blocker.
    #[serde(default)]
    pub ready_only: bool,
}

/// `issue/stale` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaleParams {
    /// How many days without a change counts as stale.
    pub days: i64,
    /// Only this project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// `issue/hygiene` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HygieneParams {
    /// Days before a claim counts as stalled; the default when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_days: Option<i64>,
}

/// `events/ping` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PingParams {
    /// Which detail to report; the summary when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// `events/wait` params. Waits for the corpus generation to pass `last`, or for
/// `id` to reach a terminal state when one is given.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitParams {
    /// Generation to wait past.
    #[serde(default)]
    pub last: u64,
    /// Wait for this issue to reach a terminal state instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Poll interval in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_ms: Option<u64>,
    /// Give up after this many milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

/// `issue/mirror` params. Checks a mirror file's stamp against the tracker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirrorCheckParams {
    /// Mirror file whose SYNC stamp is compared against the corpus.
    pub path: String,
    /// Only these projects; the stamp's own list when empty, since the file records
    /// what it covered.
    #[serde(default)]
    pub projects: Vec<String>,
}

/// `issue/mirror` reply.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirrorCheckResult {
    /// Whether the stamp still matches the tracker.
    pub fresh: bool,
    /// The verdict, naming which projects moved when stale.
    pub report: String,
}

/// `issue/digest` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestParams {
    /// Only these projects; every project when empty.
    #[serde(default)]
    pub projects: Vec<String>,
}

/// A report-shaped reply: the same text the subcommand prints.
///
/// Shared by the reads that produce prose rather than structure. Giving each its own
/// type would be a contract per report to keep in step with the text, and the text is
/// the part anyone reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportResult {
    /// The report, as the subcommand would print it.
    pub report: String,
}

/// `issue/check` reply. The counts travel beside the text because the subcommand
/// exits non-zero on an error count and a client needs the same signal.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    /// Findings, ending in a summary line.
    pub report: String,
    /// Count of `[err]` findings.
    pub errors: usize,
    /// Count of `[warn]` findings.
    pub warnings: usize,
}

/// One project's digest inside [`DigestResult`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectDigestResult {
    /// Project directory name.
    pub project: String,
    /// Hash over that project's export.
    pub digest: String,
    /// Issue count in that project.
    pub issues: usize,
}

/// `issue/digest` reply.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestResult {
    /// Hash over the per-project digests.
    pub combined: String,
    /// Sum of the per-project issue counts.
    pub issues: usize,
    /// Event-log generation the digest was taken at, so two digests can be placed in
    /// time relative to each other.
    pub generation: u64,
    /// Per project, sorted by name.
    pub projects: Vec<ProjectDigestResult>,
}

/// `events/wait` reply. Waiting on a generation fills `generation` only; waiting on
/// an issue fills `state` and says whether it gave up.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitResult {
    /// Generation at the moment the wait returned.
    pub generation: u64,
    /// Heading state, when the wait was for an issue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// True when the timeout expired before a terminal state.
    #[serde(default)]
    pub timed_out: bool,
}

/// `issue/append` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendParams {
    /// Issue id.
    pub id: String,
    /// Report text, written under the heading with a dated stamp.
    pub text: String,
    /// Override the connection agent, which the stamp records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// `issue/resolve` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveParams {
    /// Issue id whose sibling terminal is being picked.
    pub id: String,
    /// Terminal state to settle on.
    pub state: String,
}

/// `issue/reject` params. Either `to` or `project` has to say where the
/// successor goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectParams {
    /// Issue being cancelled.
    pub id: String,
    /// Existing issue to point at instead of creating a successor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Project to create the successor in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Successor title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Why the original was rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `issue/fold` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldParams {
    /// Inbox file whose unstamped `* TODO` headings become issues.
    pub file: String,
    /// Project the new issues land in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// `issue/normalize` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizeParams {
    /// Only this project; every project when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Report what would change without writing it.
    #[serde(default)]
    pub dry_run: bool,
}

/// `issue/note` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteParams {
    /// Issue id.
    pub id: String,
    /// Logbook text.
    pub text: String,
}

/// `issue/refile` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefileParams {
    /// Issue id.
    pub id: String,
    /// Destination project.
    pub to: String,
}

/// Mutation result shared by create, update, claim, note, and refile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutResult {
    /// True when the write succeeded.
    pub ok: bool,
    /// Same text the CLI would print.
    pub report: String,
    /// Post-write detail. Null on refile of a vanished source.
    #[serde(default)]
    pub issue: Option<IssueDetail>,
    /// Serve revision after the write.
    pub revision: u64,
    /// On-disk generation after the write.
    pub generation: u64,
}

/// `vault/changed` params. Broadcast after a catalog rebuild.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultChanged {
    /// On-disk generation.
    pub generation: u64,
    /// Serve-local revision.
    pub revision: u64,
    /// Dirty project names.
    #[serde(default)]
    pub projects: Vec<String>,
    /// Touched issue ids, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
}

/// `issue/selected` params. Broadcast after `issue/open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueSelected {
    /// Selected issue id.
    pub id: String,
    /// Project of that issue.
    pub project: String,
}

/// Push notifications. No `id` on the wire.
#[derive(Debug, Clone, PartialEq)]
pub enum Notification {
    /// [`NOTIFY_VAULT_CHANGED`].
    VaultChanged(VaultChanged),
    /// [`NOTIFY_ISSUE_SELECTED`].
    IssueSelected(IssueSelected),
    /// [`NOTIFY_SHUTTING_DOWN`].
    ServeShuttingDown,
    /// Method the client does not know, or params that failed to decode.
    Unknown {
        /// Wire method name.
        method: String,
        /// Raw params.
        params: Value,
    },
}

impl Notification {
    /// Wire method name.
    pub fn method(&self) -> &str {
        match self {
            Self::VaultChanged(_) => NOTIFY_VAULT_CHANGED,
            Self::IssueSelected(_) => NOTIFY_ISSUE_SELECTED,
            Self::ServeShuttingDown => NOTIFY_SHUTTING_DOWN,
            Self::Unknown { method, .. } => method,
        }
    }

    /// Parse a method/params pair. Unknown names stay [`Self::Unknown`].
    pub fn parse(method: &str, params: Value) -> Self {
        match method {
            NOTIFY_VAULT_CHANGED => match serde_json::from_value(params.clone()) {
                Ok(body) => Self::VaultChanged(body),
                Err(_) => Self::Unknown {
                    method: method.into(),
                    params,
                },
            },
            NOTIFY_ISSUE_SELECTED => match serde_json::from_value(params.clone()) {
                Ok(body) => Self::IssueSelected(body),
                Err(_) => Self::Unknown {
                    method: method.into(),
                    params,
                },
            },
            NOTIFY_SHUTTING_DOWN => Self::ServeShuttingDown,
            other => Self::Unknown {
                method: other.into(),
                params,
            },
        }
    }

    /// Params object for the wire.
    pub fn to_params(&self) -> Value {
        match self {
            Self::VaultChanged(body) => serde_json::to_value(body).unwrap_or(Value::Null),
            Self::IssueSelected(body) => serde_json::to_value(body).unwrap_or(Value::Null),
            Self::ServeShuttingDown => json!({}),
            Self::Unknown { params, .. } => params.clone(),
        }
    }
}

/// Typed v1 request.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Handshake.
    Initialize(InitializeParams),
    /// Process identity, root, prefix, and crate version.
    IdentityGet,
    /// Filtered issue rows.
    IssueList(IssueListParams),
    /// One issue plus revision.
    IssueGet(IdParams),
    /// Frontier: `issue/list` with `ready: true`.
    IssueReady(IssueListParams),
    /// Substring search over id, title, properties, tags, and body.
    IssueSearch(SearchParams),
    /// Live claims.
    IssueClaims(ClaimsParams),
    /// Deadlines and scheduled starts.
    IssueAgenda(AgendaParams),
    /// Alias of [`Self::IssueGet`].
    IssueShow(IdParams),
    /// Secret-screened body range.
    IssueExcerpt(IdParams),
    /// Children and blockers.
    IssueTree(TreeParams),
    /// Bounded neighborhood with evidence.
    IssueRelated(RelatedParams),
    /// Direct children.
    IssueChildren(WalkParams),
    /// Walk up the blocker graph.
    IssueAncestors(WalkParams),
    /// Walk down the blocker graph.
    IssueImpact(WalkParams),
    /// Everything pointing at the id.
    IssueBacklinks(WalkParams),
    /// Shared selection; notifies `issue/selected`.
    IssueOpen(IdParams),
    /// Create an issue.
    IssueCreate(CreateParams),
    /// State, priority, block, unblock.
    IssueUpdate(UpdateParams),
    /// Take the issue.
    IssueClaim(ClaimParams),
    /// Dated logbook entry.
    IssueNote(NoteParams),
    /// Move to another project.
    IssueRefile(RefileParams),
    /// Dated report under the heading.
    IssueAppend(AppendParams),
    /// Cancel and point at a successor.
    IssueReject(RejectParams),
    /// Settle on a sibling terminal state.
    IssueResolve(ResolveParams),
    /// Cast a ballot, or read the tally.
    IssueVote(VoteParams),
    /// Cite, drop, or read the deeds an issue produced.
    IssueDeed(DeedParams),
    /// The working set for an issue.
    IssueRecall(RecallParams),
    /// DeGroot consensus over an issue's ballots.
    IssueConsensus(ConsensusParams),
    /// Inbox headings become issues.
    IssueFold(FoldParams),
    /// Rewrite onto the property split.
    IssueNormalize(NormalizeParams),
    /// Validate the corpus.
    IssueCheck(ProjectFilterParams),
    /// Counts by project, state, readiness.
    IssueCount(CountParams),
    /// Blocker cycles, if any.
    IssueCycles(ProjectFilterParams),
    /// Corpus hash, combined and per project.
    IssueDigest(DigestParams),
    /// The corpus as text.
    IssueExport(ProjectFilterParams),
    /// One dot document.
    IssueGraph(ProjectFilterParams),
    /// One roadmap document.
    IssueRoadmap(ProjectFilterParams),
    /// Issues untouched for a number of days.
    IssueStale(StaleParams),
    /// Stalled claims plus validation.
    IssueHygiene(HygieneParams),
    /// What blocks one issue.
    IssueWaitingOn(IdParams),
    /// The mirror's stamp.
    IssueMirror(MirrorCheckParams),
    /// Liveness and detail.
    EventsPing(PingParams),
    /// Block until the generation moves, or an issue is terminal.
    EventsWait(WaitParams),
    /// Project names plus revision.
    ProjectList,
    /// Pull of the on-disk event log.
    EventsSince(EventsSinceParams),
    /// Current generation and revision.
    EventsGen,
}

impl Request {
    /// Wire [`Method`] for this request.
    pub fn method(&self) -> Method {
        match self {
            Self::Initialize(_) => Method::Initialize,
            Self::IdentityGet => Method::IdentityGet,
            Self::IssueList(_) => Method::IssueList,
            Self::IssueGet(_) => Method::IssueGet,
            Self::IssueReady(_) => Method::IssueReady,
            Self::IssueSearch(_) => Method::IssueSearch,
            Self::IssueClaims(_) => Method::IssueClaims,
            Self::IssueAgenda(_) => Method::IssueAgenda,
            Self::IssueShow(_) => Method::IssueShow,
            Self::IssueExcerpt(_) => Method::IssueExcerpt,
            Self::IssueTree(_) => Method::IssueTree,
            Self::IssueRelated(_) => Method::IssueRelated,
            Self::IssueChildren(_) => Method::IssueChildren,
            Self::IssueAncestors(_) => Method::IssueAncestors,
            Self::IssueImpact(_) => Method::IssueImpact,
            Self::IssueBacklinks(_) => Method::IssueBacklinks,
            Self::IssueOpen(_) => Method::IssueOpen,
            Self::IssueCreate(_) => Method::IssueCreate,
            Self::IssueUpdate(_) => Method::IssueUpdate,
            Self::IssueClaim(_) => Method::IssueClaim,
            Self::IssueNote(_) => Method::IssueNote,
            Self::IssueRefile(_) => Method::IssueRefile,
            Self::IssueAppend(_) => Method::IssueAppend,
            Self::IssueReject(_) => Method::IssueReject,
            Self::IssueResolve(_) => Method::IssueResolve,
            Self::IssueVote(_) => Method::IssueVote,
            Self::IssueDeed(_) => Method::IssueDeed,
            Self::IssueRecall(_) => Method::IssueRecall,
            Self::IssueConsensus(_) => Method::IssueConsensus,
            Self::IssueFold(_) => Method::IssueFold,
            Self::IssueNormalize(_) => Method::IssueNormalize,
            Self::IssueCheck(_) => Method::IssueCheck,
            Self::IssueCount(_) => Method::IssueCount,
            Self::IssueCycles(_) => Method::IssueCycles,
            Self::IssueDigest(_) => Method::IssueDigest,
            Self::IssueExport(_) => Method::IssueExport,
            Self::IssueGraph(_) => Method::IssueGraph,
            Self::IssueRoadmap(_) => Method::IssueRoadmap,
            Self::IssueStale(_) => Method::IssueStale,
            Self::IssueHygiene(_) => Method::IssueHygiene,
            Self::IssueWaitingOn(_) => Method::IssueWaitingOn,
            Self::IssueMirror(_) => Method::IssueMirror,
            Self::EventsPing(_) => Method::EventsPing,
            Self::EventsWait(_) => Method::EventsWait,
            Self::ProjectList => Method::ProjectList,
            Self::EventsSince(_) => Method::EventsSince,
            Self::EventsGen => Method::EventsGen,
        }
    }

    /// Parse a method/params pair.
    ///
    /// # Errors
    ///
    /// Returns an error when `method` is unknown or `params` fail to decode.
    pub fn parse(method: &str, params: Option<Value>) -> Result<Self, JsonRpcError> {
        let method = Method::parse(method)?;
        let params = match params {
            None | Some(Value::Null) => Value::Object(Default::default()),
            Some(v) => v,
        };
        match method {
            Method::Initialize => Ok(Self::Initialize(parse_initialize_params(&params)?)),
            Method::IdentityGet => Ok(Self::IdentityGet),
            Method::IssueList => Ok(Self::IssueList(decode_params(params)?)),
            Method::IssueGet => Ok(Self::IssueGet(decode_params(params)?)),
            Method::IssueReady => Ok(Self::IssueReady(decode_params(params)?)),
            Method::IssueSearch => Ok(Self::IssueSearch(decode_params(params)?)),
            Method::IssueClaims => Ok(Self::IssueClaims(decode_params(params)?)),
            Method::IssueAgenda => Ok(Self::IssueAgenda(decode_params(params)?)),
            Method::IssueShow => Ok(Self::IssueShow(decode_params(params)?)),
            Method::IssueExcerpt => Ok(Self::IssueExcerpt(decode_params(params)?)),
            Method::IssueTree => Ok(Self::IssueTree(decode_params(params)?)),
            Method::IssueRelated => Ok(Self::IssueRelated(decode_params(params)?)),
            Method::IssueChildren => Ok(Self::IssueChildren(decode_params(params)?)),
            Method::IssueAncestors => Ok(Self::IssueAncestors(decode_params(params)?)),
            Method::IssueImpact => Ok(Self::IssueImpact(decode_params(params)?)),
            Method::IssueBacklinks => Ok(Self::IssueBacklinks(decode_params(params)?)),
            Method::IssueOpen => Ok(Self::IssueOpen(decode_params(params)?)),
            Method::IssueCreate => Ok(Self::IssueCreate(decode_params(params)?)),
            Method::IssueUpdate => Ok(Self::IssueUpdate(decode_params(params)?)),
            Method::IssueClaim => Ok(Self::IssueClaim(decode_params(params)?)),
            Method::IssueNote => Ok(Self::IssueNote(decode_params(params)?)),
            Method::IssueRefile => Ok(Self::IssueRefile(decode_params(params)?)),
            Method::ProjectList => Ok(Self::ProjectList),
            Method::EventsSince => Ok(Self::EventsSince(decode_params(params)?)),
            Method::EventsGen => Ok(Self::EventsGen),
            Method::IssueAppend => Ok(Self::IssueAppend(decode_params(params)?)),
            Method::IssueReject => Ok(Self::IssueReject(decode_params(params)?)),
            Method::IssueResolve => Ok(Self::IssueResolve(decode_params(params)?)),
            Method::IssueVote => Ok(Self::IssueVote(decode_params(params)?)),
            Method::IssueDeed => Ok(Self::IssueDeed(decode_params(params)?)),
            Method::IssueRecall => Ok(Self::IssueRecall(decode_params(params)?)),
            Method::IssueConsensus => Ok(Self::IssueConsensus(decode_params(params)?)),
            Method::IssueFold => Ok(Self::IssueFold(decode_params(params)?)),
            Method::IssueNormalize => Ok(Self::IssueNormalize(decode_params(params)?)),
            Method::IssueCheck => Ok(Self::IssueCheck(decode_params(params)?)),
            Method::IssueCount => Ok(Self::IssueCount(decode_params(params)?)),
            Method::IssueCycles => Ok(Self::IssueCycles(decode_params(params)?)),
            Method::IssueDigest => Ok(Self::IssueDigest(decode_params(params)?)),
            Method::IssueExport => Ok(Self::IssueExport(decode_params(params)?)),
            Method::IssueGraph => Ok(Self::IssueGraph(decode_params(params)?)),
            Method::IssueRoadmap => Ok(Self::IssueRoadmap(decode_params(params)?)),
            Method::IssueStale => Ok(Self::IssueStale(decode_params(params)?)),
            Method::IssueHygiene => Ok(Self::IssueHygiene(decode_params(params)?)),
            Method::IssueWaitingOn => Ok(Self::IssueWaitingOn(decode_params(params)?)),
            Method::IssueMirror => Ok(Self::IssueMirror(decode_params(params)?)),
            Method::EventsPing => Ok(Self::EventsPing(decode_params(params)?)),
            Method::EventsWait => Ok(Self::EventsWait(decode_params(params)?)),
        }
    }

    /// Params object for the wire.
    pub fn to_params(&self) -> Value {
        match self {
            Self::Initialize(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueAppend(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueReject(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueResolve(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueVote(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueDeed(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueRecall(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueConsensus(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueFold(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueNormalize(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueCheck(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueCount(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueCycles(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueDigest(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueExport(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueGraph(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueRoadmap(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueStale(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueHygiene(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueWaitingOn(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueMirror(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::EventsPing(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::EventsWait(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IdentityGet | Self::ProjectList | Self::EventsGen => json!({}),
            Self::IssueList(p) | Self::IssueReady(p) => {
                serde_json::to_value(p).unwrap_or(Value::Null)
            }
            Self::IssueGet(p) | Self::IssueShow(p) | Self::IssueExcerpt(p) | Self::IssueOpen(p) => {
                serde_json::to_value(p).unwrap_or(Value::Null)
            }
            Self::IssueSearch(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueClaims(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueAgenda(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueTree(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueRelated(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueChildren(p)
            | Self::IssueAncestors(p)
            | Self::IssueImpact(p)
            | Self::IssueBacklinks(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueCreate(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueUpdate(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueClaim(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueNote(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::IssueRefile(p) => serde_json::to_value(p).unwrap_or(Value::Null),
            Self::EventsSince(p) => serde_json::to_value(p).unwrap_or(Value::Null),
        }
    }
}

/// Typed v1 result body.
#[derive(Debug, Clone)]
pub enum Response {
    /// Handshake.
    Initialize(InitializeResult),
    /// Process identity, root, prefix, and crate version.
    IdentityGet(IdentityResult),
    /// Filtered issue rows.
    IssueList(IssueListResult),
    /// One issue plus revision.
    IssueGet(IssueGetResult),
    /// Frontier page.
    IssueReady(IssueListResult),
    /// Search hits.
    IssueSearch(Vec<SearchHit>),
    /// Live claims.
    IssueClaims(Vec<ClaimRow>),
    /// Deadlines and scheduled starts.
    IssueAgenda(Vec<AgendaRow>),
    /// Alias of [`Self::IssueGet`].
    IssueShow(IssueGetResult),
    /// Secret-screened body range.
    IssueExcerpt(Excerpt),
    /// Children and blockers.
    IssueTree(TreeResult),
    /// Bounded neighborhood with evidence.
    IssueRelated(Vec<RelatedHit>),
    /// Direct children.
    IssueChildren(Vec<WalkHit>),
    /// Walk up the blocker graph.
    IssueAncestors(Vec<WalkHit>),
    /// Walk down the blocker graph.
    IssueImpact(Vec<WalkHit>),
    /// Everything pointing at the id.
    IssueBacklinks(Vec<WalkHit>),
    /// Shared selection result.
    IssueOpen(IssueGetResult),
    /// Create result.
    IssueCreate(MutResult),
    /// Update result.
    IssueUpdate(MutResult),
    /// Claim result.
    IssueClaim(MutResult),
    /// Note result.
    IssueNote(MutResult),
    /// Refile result.
    IssueRefile(MutResult),
    /// Dated report under the heading.
    IssueAppend(MutResult),
    /// Cancel and point at a successor.
    IssueReject(MutResult),
    /// Settle on a sibling terminal state.
    IssueResolve(MutResult),
    /// Cast a ballot, or read the tally.
    IssueVote(MutResult),
    /// Cite, drop, or read the deeds an issue produced.
    IssueDeed(MutResult),
    /// The working set for an issue.
    IssueRecall(Recall),
    /// Consensus over an issue's ballots, or over its children.
    IssueConsensus(Value),
    /// Inbox headings become issues.
    IssueFold(MutResult),
    /// Rewrite onto the property split.
    IssueNormalize(MutResult),
    /// Validation findings plus counts.
    IssueCheck(CheckResult),
    /// Counts by project, state, readiness.
    IssueCount(ReportResult),
    /// Blocker cycles, if any.
    IssueCycles(ReportResult),
    /// Corpus hash, combined and per project.
    IssueDigest(DigestResult),
    /// The corpus as text.
    IssueExport(ReportResult),
    /// One dot document.
    IssueGraph(ReportResult),
    /// One roadmap document.
    IssueRoadmap(ReportResult),
    /// Issues untouched for a number of days.
    IssueStale(ReportResult),
    /// Stalled claims plus validation.
    IssueHygiene(ReportResult),
    /// What blocks one issue.
    IssueWaitingOn(ReportResult),
    /// The mirror's stamp.
    IssueMirror(MirrorCheckResult),
    /// Liveness and detail.
    EventsPing(ReportResult),
    /// Generation reached, or the state waited for.
    EventsWait(WaitResult),
    /// Project names plus revision.
    ProjectList(ProjectListResult),
    /// Pull of the on-disk event log.
    EventsSince(EventsSinceResult),
    /// Current generation and revision.
    EventsGen(EventsGenResult),
}

impl Response {
    /// Serialize the result body (not the envelope).
    ///
    /// # Errors
    ///
    /// Returns an error when the body cannot be encoded as JSON.
    pub fn to_value(&self) -> Result<Value, serde_json::Error> {
        match self {
            Self::Initialize(v) => serde_json::to_value(v),
            Self::IdentityGet(v) => serde_json::to_value(v),
            Self::IssueAppend(v) => serde_json::to_value(v),
            Self::IssueReject(v) => serde_json::to_value(v),
            Self::IssueResolve(v) => serde_json::to_value(v),
            Self::IssueVote(v) => serde_json::to_value(v),
            Self::IssueDeed(v) => serde_json::to_value(v),
            Self::IssueRecall(v) => serde_json::to_value(v),
            Self::IssueConsensus(v) => serde_json::to_value(v),
            Self::IssueFold(v) => serde_json::to_value(v),
            Self::IssueNormalize(v) => serde_json::to_value(v),
            Self::IssueCheck(v) => serde_json::to_value(v),
            Self::IssueCount(v) => serde_json::to_value(v),
            Self::IssueCycles(v) => serde_json::to_value(v),
            Self::IssueDigest(v) => serde_json::to_value(v),
            Self::IssueExport(v) => serde_json::to_value(v),
            Self::IssueGraph(v) => serde_json::to_value(v),
            Self::IssueRoadmap(v) => serde_json::to_value(v),
            Self::IssueStale(v) => serde_json::to_value(v),
            Self::IssueHygiene(v) => serde_json::to_value(v),
            Self::IssueWaitingOn(v) => serde_json::to_value(v),
            Self::IssueMirror(v) => serde_json::to_value(v),
            Self::EventsPing(v) => serde_json::to_value(v),
            Self::EventsWait(v) => serde_json::to_value(v),
            Self::IssueList(v) | Self::IssueReady(v) => serde_json::to_value(v),
            Self::IssueGet(v) | Self::IssueShow(v) | Self::IssueOpen(v) => serde_json::to_value(v),
            Self::IssueSearch(v) => serde_json::to_value(v),
            Self::IssueClaims(v) => serde_json::to_value(v),
            Self::IssueAgenda(v) => serde_json::to_value(v),
            Self::IssueExcerpt(v) => serde_json::to_value(v),
            Self::IssueTree(v) => serde_json::to_value(v),
            Self::IssueRelated(v) => serde_json::to_value(v),
            Self::IssueChildren(v)
            | Self::IssueAncestors(v)
            | Self::IssueImpact(v)
            | Self::IssueBacklinks(v) => serde_json::to_value(v),
            Self::IssueCreate(v)
            | Self::IssueUpdate(v)
            | Self::IssueClaim(v)
            | Self::IssueNote(v)
            | Self::IssueRefile(v) => serde_json::to_value(v),
            Self::ProjectList(v) => serde_json::to_value(v),
            Self::EventsSince(v) => serde_json::to_value(v),
            Self::EventsGen(v) => serde_json::to_value(v),
        }
    }
}

fn decode_params<T: DeserializeOwned>(value: Value) -> Result<T, JsonRpcError> {
    serde_json::from_value(value).map_err(|e| invalid_params(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn initialize_missing_agent_is_invalid_params() {
        let err = parse_initialize_params(&json!({
            "protocolVersion": 1,
            "client": "vissue-tui"
        }))
        .unwrap_err();
        assert_eq!(err.code, INVALID_PARAMS);
        assert_eq!(err.message, "agent is required");

        let err = parse_initialize_params(&json!({
            "protocolVersion": 1,
            "agent": ""
        }))
        .unwrap_err();
        assert_eq!(err.code, INVALID_PARAMS);
        assert_eq!(err.message, "agent is required");

        let err = Request::parse(
            "initialize",
            Some(json!({"protocolVersion": 1, "agent": "   "})),
        )
        .unwrap_err();
        assert_eq!(err.code, INVALID_PARAMS);
    }

    #[test]
    fn protocol_version_2_is_rejected() {
        let err = parse_initialize_params(&json!({
            "protocolVersion": 2,
            "agent": "rg@host"
        }))
        .unwrap_err();
        assert_eq!(err.code, INVALID_PARAMS);
        assert_eq!(err.message, "unsupported protocol version");
        assert_eq!(err.data, Some(json!({"supported": 1})));
    }

    #[test]
    fn initialize_version_1_is_accepted() {
        let params = parse_initialize_params(&json!({
            "protocolVersion": 1,
            "client": "vissue-tui",
            "agent": "rg@host"
        }))
        .unwrap();
        assert_eq!(params.protocol_version, 1);
        assert_eq!(params.agent, "rg@host");
        assert_eq!(params.client, "vissue-tui");
    }

    #[test]
    fn handshake_fields_are_camel_case() {
        let params = InitializeParams {
            protocol_version: 1,
            client: "vissue-tui".into(),
            agent: "rg@host".into(),
        };
        let value = serde_json::to_value(&params).unwrap();
        assert_eq!(value["protocolVersion"], 1);
        assert!(value.get("protocol_version").is_none());

        let result = InitializeResult {
            protocol_version: 1,
            capabilities: vec!["issue/list".into()],
            root: "/tmp/tracker".into(),
            prefix: "Software".into(),
            generation: 3,
            revision: 1,
            identity: "rg@host".into(),
        };
        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(value["protocolVersion"], 1);
        assert_eq!(value["generation"], 3);
    }

    #[test]
    fn issue_payloads_are_snake_case() {
        let params = IssueListParams {
            since_revision: Some(41),
            ..IssueListParams::default()
        };
        let value = serde_json::to_value(&params).unwrap();
        assert_eq!(value["since_revision"], 41);
        assert!(value.get("sinceRevision").is_none());
    }

    #[test]
    fn unknown_method_is_not_found() {
        // Deliberately a name no verb will ever take. This test used "issue/fold"
        // until fold became a method, and the same trap caught the owner's copy of
        // this test on the same day: an example chosen because it sounds plausible is
        // an example that will one day be real, and then the test asserts that a
        // working method is missing.
        const NEVER: &str = "issue/no-such-method";
        let err = Method::parse(NEVER).unwrap_err();
        assert_eq!(err.code, METHOD_NOT_FOUND);
        assert_eq!(err.data, Some(json!({"method": NEVER})));
    }

    #[test]
    fn every_v1_capability_parses() {
        for name in V1_CAPABILITIES {
            assert!(Method::parse(name).is_ok(), "{name}");
        }
        assert_eq!(Method::Initialize.as_str(), "initialize");
    }

    #[test]
    fn request_parse_roundtrips_issue_get() {
        let req = Request::parse("issue/get", Some(json!({"id": "atlas-1a2b"}))).unwrap();
        assert_eq!(req.method(), Method::IssueGet);
        assert_eq!(req.to_params()["id"], "atlas-1a2b");
    }

    #[test]
    fn missing_id_on_issue_get_is_invalid_params() {
        let err = Request::parse("issue/get", Some(json!({}))).unwrap_err();
        assert_eq!(err.code, INVALID_PARAMS);
    }

    #[test]
    fn core_errors_carry_data_code() {
        let err = error_from_core(&CoreError::IssueNotFound {
            id: "atlas-1a2b".into(),
        });
        assert_eq!(err.code, NOT_FOUND);
        assert_eq!(err.data.unwrap()["code"], "not_found");

        let err = error_from_core(&CoreError::ClaimConflict {
            id: "atlas-1a2b".into(),
            holder: "other".into(),
            claimed_at: None,
        });
        assert_eq!(err.code, CONFLICT);
        let data = err.data.unwrap();
        assert_eq!(data["code"], "conflict");
        assert_eq!(data["holder"], "other");

        let err = error_from_core(&CoreError::BlockerCycle {
            blocker: "a".into(),
            issue: "b".into(),
        });
        assert_eq!(err.code, CYCLE);
        let data = err.data.unwrap();
        assert_eq!(data["code"], "cycle");
        assert_eq!(data["id"], "b");
        assert_eq!(data["block"], "a");

        let err = error_from_core(&CoreError::InvalidState {
            id: "atlas-4g5h".into(),
            state: "DONE".into(),
        });
        assert_eq!(err.code, INVALID_STATE);
        assert_eq!(err.data.unwrap()["code"], "invalid_state");

        let err = error_from_core(&CoreError::DuplicateId {
            id: "atlas-1a2b".into(),
            paths: vec![
                std::path::PathBuf::from("/a/issues.org"),
                std::path::PathBuf::from("/b/issues.org"),
            ],
        });
        assert_eq!(err.code, CONFLICT);
        assert_eq!(err.data.unwrap()["code"], "duplicate_id");
    }

    #[test]
    fn notification_parse_known_methods() {
        let n = Notification::parse(
            NOTIFY_VAULT_CHANGED,
            json!({"generation": 1, "revision": 2, "projects": ["atlas"]}),
        );
        assert!(matches!(n, Notification::VaultChanged(_)));
        assert_eq!(n.method(), NOTIFY_VAULT_CHANGED);

        let n = Notification::parse(
            NOTIFY_ISSUE_SELECTED,
            json!({"id": "atlas-1a2b", "project": "atlas"}),
        );
        assert!(matches!(n, Notification::IssueSelected(_)));

        let n = Notification::parse(NOTIFY_SHUTTING_DOWN, json!({}));
        assert!(matches!(n, Notification::ServeShuttingDown));
        assert_eq!(n.to_params(), json!({}));
    }

    #[test]
    fn list_unchanged_deserializes_without_rows() {
        let page: IssueListResult =
            serde_json::from_value(json!({"unchanged": true, "revision": 41})).unwrap();
        assert!(page.unchanged);
        assert!(page.issues.is_empty());
        assert_eq!(page.revision, 41);
    }

    #[test]
    fn response_to_value_serializes_initialize() {
        let resp = Response::Initialize(InitializeResult {
            protocol_version: 1,
            capabilities: V1_CAPABILITIES.iter().map(|s| (*s).to_string()).collect(),
            root: "/tmp".into(),
            prefix: "Software".into(),
            generation: 1,
            revision: 1,
            identity: "agent".into(),
        });
        let value = resp.to_value().unwrap();
        assert_eq!(value["protocolVersion"], 1);
        assert!(
            value["capabilities"]
                .as_array()
                .unwrap()
                .contains(&json!("issue/list"))
        );
    }

    #[test]
    fn envelope_helpers_roundtrip() {
        let req = JsonRpcRequest::call(JsonRpcId::Number(1), "identity/get", json!({}));
        let bytes = serde_json::to_vec(&req).unwrap();
        let back: JsonRpcRequest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.method, "identity/get");
        assert!(!back.is_notification());

        let note = JsonRpcRequest::notification(NOTIFY_SHUTTING_DOWN, json!({}));
        assert!(note.is_notification());

        let ok = JsonRpcResponse::ok(Some(JsonRpcId::Number(1)), json!({"ok": true}));
        assert_eq!(ok.result.unwrap()["ok"], true);
        let err = JsonRpcResponse::err(Some(JsonRpcId::Null), parse_error());
        assert_eq!(err.error.unwrap().code, PARSE_ERROR);
    }

    #[test]
    fn mut_and_walk_params_decode() {
        let claim = Request::parse(
            "issue/claim",
            Some(json!({"id": "atlas-1a2b", "force": true})),
        )
        .unwrap();
        match claim {
            Request::IssueClaim(p) => {
                assert!(p.force);
                assert_eq!(p.id, "atlas-1a2b");
            }
            other => panic!("{other:?}"),
        }
        let create = Request::parse(
            "issue/create",
            Some(json!({"project": "atlas", "title": "x"})),
        )
        .unwrap();
        assert_eq!(create.method(), Method::IssueCreate);
        assert!(Request::parse("issue/create", Some(json!({"title": "x"}))).is_err());
        assert_eq!(
            Request::parse("events/gen", None).unwrap().method(),
            Method::EventsGen
        );
        let _ = Request::IssueNote(NoteParams {
            id: "a".into(),
            text: "n".into(),
        })
        .to_params();
        let _ = Request::IssueRefile(RefileParams {
            id: "a".into(),
            to: "b".into(),
        })
        .to_params();
        let _ = Request::IssueUpdate(UpdateParams {
            id: "a".into(),
            state: Some("STARTED".into()),
            priority: None,
            block: None,
            unblock: None,
            if_state: None,
            if_gen: None,
            agent: None,
        })
        .to_params();
        let _ = Request::EventsSince(EventsSinceParams {
            since: 0,
            limit: Some(10),
        })
        .to_params();
        let _ = Request::IssueTree(TreeParams {
            id: "a".into(),
            format: Some("ascii".into()),
        })
        .to_params();
        let _ = Request::IssueRelated(RelatedParams {
            id: "a".into(),
            depth: Some(2),
            limit: Some(20),
        })
        .to_params();
        let _ = Request::IssueChildren(WalkParams {
            id: "a".into(),
            depth: None,
        })
        .to_params();
        let _ = Request::IssueSearch(SearchParams {
            query: "q".into(),
            limit: None,
        })
        .to_params();
        let _ = Request::IssueClaims(ClaimsParams::default()).to_params();
        let _ = Request::IssueAgenda(AgendaParams::default()).to_params();
        let _ = Request::IdentityGet.to_params();
    }

    #[test]
    fn response_variants_serialize() {
        let detail = IssueDetail {
            id: "atlas-1a2b".into(),
            project: "atlas".into(),
            title: "t".into(),
            state: "TODO".into(),
            priority: "B".into(),
            properties: BTreeMap::new(),
            org_tags: vec![],
            tags: vec![],
            blocked_by: vec![],
            deeds: vec![],
            parent: None,
            claimed_by: None,
            claimed_at: None,
            file: "issues.org:1-2".into(),
            line_start: 1,
            line_end: 2,
            body: "what the issue asks for".into(),
            logbook: vec![],
        };
        let get = IssueGetResult {
            issue: detail.clone(),
            revision: 1,
        };
        assert!(Response::IssueGet(get.clone()).to_value().unwrap()["id"] == "atlas-1a2b");
        assert!(Response::IssueShow(get.clone()).to_value().is_ok());
        assert!(Response::IssueOpen(get).to_value().is_ok());
        assert!(
            Response::IssueExcerpt(Excerpt {
                id: "atlas-1a2b".into(),
                file: "issues.org".into(),
                line_start: 1,
                line_end: 2,
                text: "body".into(),
                suppressed: false,
            })
            .to_value()
            .is_ok()
        );
        assert!(Response::IssueSearch(vec![]).to_value().unwrap().is_array());
        assert!(Response::IssueClaims(vec![]).to_value().unwrap().is_array());
        assert!(Response::IssueAgenda(vec![]).to_value().unwrap().is_array());
        assert!(
            Response::IssueRelated(vec![])
                .to_value()
                .unwrap()
                .is_array()
        );
        assert!(
            Response::IssueChildren(vec![])
                .to_value()
                .unwrap()
                .is_array()
        );
        assert!(
            Response::IssueAncestors(vec![])
                .to_value()
                .unwrap()
                .is_array()
        );
        assert!(Response::IssueImpact(vec![]).to_value().unwrap().is_array());
        assert!(
            Response::IssueBacklinks(vec![])
                .to_value()
                .unwrap()
                .is_array()
        );
        assert!(
            Response::ProjectList(ProjectListResult {
                projects: vec!["atlas".into()],
                revision: 1,
            })
            .to_value()
            .is_ok()
        );
        assert!(
            Response::EventsGen(EventsGenResult {
                generation: 1,
                revision: 1,
            })
            .to_value()
            .is_ok()
        );
        assert!(
            Response::EventsSince(EventsSinceResult {
                events: vec![],
                generation: 1,
            })
            .to_value()
            .is_ok()
        );
        assert!(
            Response::IdentityGet(IdentityResult {
                identity: "a".into(),
                root: "/".into(),
                prefix: "Software".into(),
                version: "0.2.0".into(),
            })
            .to_value()
            .is_ok()
        );
        let mut_ok = MutResult {
            ok: true,
            report: "ok".into(),
            issue: Some(detail),
            revision: 2,
            generation: 3,
        };
        assert!(Response::IssueClaim(mut_ok.clone()).to_value().is_ok());
        assert!(Response::IssueCreate(mut_ok.clone()).to_value().is_ok());
        assert!(Response::IssueUpdate(mut_ok.clone()).to_value().is_ok());
        assert!(Response::IssueNote(mut_ok.clone()).to_value().is_ok());
        assert!(Response::IssueRefile(mut_ok).to_value().is_ok());
        assert!(
            Response::IssueTree(TreeResult::Text { text: "* a".into() })
                .to_value()
                .is_ok()
        );
        assert!(
            Response::IssueList(IssueListResult {
                revision: 1,
                ..IssueListResult::default()
            })
            .to_value()
            .is_ok()
        );
        assert!(
            Response::IssueReady(IssueListResult {
                revision: 1,
                ..IssueListResult::default()
            })
            .to_value()
            .is_ok()
        );
    }

    #[test]
    fn parse_every_method_with_minimal_params() {
        let id = json!({"id": "atlas-1a2b"});
        for (method, params) in [
            ("identity/get", json!({})),
            ("issue/list", json!({})),
            ("issue/get", id.clone()),
            ("issue/ready", json!({})),
            ("issue/search", json!({"query": "q"})),
            ("issue/claims", json!({})),
            ("issue/agenda", json!({})),
            ("issue/show", id.clone()),
            ("issue/excerpt", id.clone()),
            ("issue/tree", id.clone()),
            ("issue/related", id.clone()),
            ("issue/children", id.clone()),
            ("issue/ancestors", id.clone()),
            ("issue/impact", id.clone()),
            ("issue/backlinks", id.clone()),
            ("issue/open", id.clone()),
            ("issue/create", json!({"project": "atlas", "title": "t"})),
            ("issue/update", id.clone()),
            ("issue/claim", id.clone()),
            ("issue/note", json!({"id": "atlas-1a2b", "text": "n"})),
            ("issue/refile", json!({"id": "atlas-1a2b", "to": "beacon"})),
            ("project/list", json!({})),
            ("events/since", json!({"since": 0})),
            ("events/gen", json!({})),
        ] {
            let req = Request::parse(method, Some(params)).expect(method);
            assert_eq!(req.method().as_str(), method);
            let _ = req.to_params();
        }
    }

    #[test]
    fn helper_errors_have_stable_codes() {
        assert_eq!(invalid_request().code, INVALID_REQUEST);
        assert_eq!(internal_error("x").code, INTERNAL_ERROR);
        assert_eq!(parse_error().code, PARSE_ERROR);
        let err = Error::Rpc(invalid_params("agent is required"));
        assert_eq!(err.to_string(), "agent is required");
        let _ = Error::Unsupported("unix only");
        let _ = Notification::parse("vault/changed", json!(null));
        let _ = Notification::parse("issue/selected", json!(null));
        let _ = Notification::parse("other/x", json!({"a": 1}));
        let n = Notification::Unknown {
            method: "x".into(),
            params: json!({"a": 1}),
        };
        assert_eq!(n.to_params()["a"], 1);
        assert_eq!(n.method(), "x");
    }
    /// Every method has a typed request form, and it round-trips.
    ///
    /// Nineteen methods reached the wire with no typed form for a while, and the
    /// typed helpers answered "send it untyped" per method. That was honest and it
    /// was a hole: a client wanting typed access to `issue/check` could not have it,
    /// and the two enums drifted from the method list by exactly the amount nobody
    /// was checking.
    ///
    /// Driven from `V1_CAPABILITIES`, so a method added to the wire without a typed
    /// form fails here rather than being discovered by whoever wanted it.
    #[test]
    fn every_advertised_method_has_a_typed_request() {
        for name in V1_CAPABILITIES {
            let method = Method::parse(name).unwrap_or_else(|_| panic!("{name} does not parse"));
            assert_eq!(
                method.as_str(),
                *name,
                "{name} does not round-trip as a method"
            );

            // Empty params: what matters here is that a typed form exists and that
            // its required fields are the reason a decode fails, not the absence of
            // any form at all.
            let parsed = Request::parse(name, Some(json!({})));
            if let Ok(req) = parsed {
                assert_eq!(
                    req.method().as_str(),
                    *name,
                    "{name} parsed into a request that reports a different method"
                );
                // And the params it holds serialize back to an object.
                assert!(
                    req.to_params().is_object(),
                    "{name} does not serialize its params to an object"
                );
            }
        }
    }

    /// And every typed response encodes.
    #[test]
    fn the_new_typed_responses_encode() {
        let cases = vec![
            Response::IssueCheck(CheckResult {
                report: "ok".into(),
                errors: 0,
                warnings: 2,
            }),
            Response::IssueCount(ReportResult {
                report: "3 issues".into(),
            }),
            Response::IssueDigest(DigestResult {
                combined: "abcd".into(),
                issues: 3,
                generation: 4,
                projects: vec![ProjectDigestResult {
                    project: "atlas".into(),
                    digest: "beef".into(),
                    issues: 3,
                }],
            }),
            Response::EventsWait(WaitResult {
                generation: 7,
                state: Some("DONE".into()),
                timed_out: false,
            }),
        ];
        for case in cases {
            let value = case.to_value().expect("encode");
            assert!(value.is_object(), "{value} is not an object");
        }

        // The check counts survive the trip, since a client acts on them.
        let encoded = Response::IssueCheck(CheckResult {
            report: "two warnings".into(),
            errors: 0,
            warnings: 2,
        })
        .to_value()
        .unwrap();
        assert_eq!(encoded["warnings"], 2);
        assert_eq!(encoded["errors"], 0);
    }
}
