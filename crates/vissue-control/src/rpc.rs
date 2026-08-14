//! JSON-RPC 2.0 types. Handshake is camelCase; issue payloads are snake_case.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};

use crate::frame::FrameError;
use vissue_core::error::Error as CoreError;
use vissue_core::views::{
    AgendaRow, ClaimRow, Excerpt, IssueDetail, IssueRow, RelatedHit, SearchHit, TreeNode, WalkHit,
};

pub use vissue_core::events::Event;

/// Protocol version accepted by `initialize`.
pub const PROTOCOL_VERSION: u32 = 1;

pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;
pub const NOT_FOUND: i32 = -32004;
pub const CONFLICT: i32 = -32009;
pub const INVALID_STATE: i32 = -32010;
pub const CYCLE: i32 = -32022;

pub const NOTIFY_VAULT_CHANGED: &str = "vault/changed";
pub const NOTIFY_ISSUE_SELECTED: &str = "issue/selected";
pub const NOTIFY_SHUTTING_DOWN: &str = "serve/shutting_down";

/// Wire-level failure for a control client or dispatcher.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Json(serde_json::Error),
    Frame(FrameError),
    Rpc(JsonRpcError),
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
    Number(i64),
    String(String),
    Null,
}

/// JSON-RPC 2.0 request or notification envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<JsonRpcId>,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn call(id: JsonRpcId, method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id: Some(id),
            method: method.into(),
            params: Some(params),
        }
    }

    pub fn notification(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id: None,
            method: method.into(),
            params: Some(params),
        }
    }

    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }
}

/// JSON-RPC 2.0 response envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<JsonRpcId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn ok(id: Option<JsonRpcId>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

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
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

pub fn parse_error() -> JsonRpcError {
    JsonRpcError {
        code: PARSE_ERROR,
        message: "parse error".into(),
        data: None,
    }
}

pub fn invalid_request() -> JsonRpcError {
    JsonRpcError {
        code: INVALID_REQUEST,
        message: "invalid request".into(),
        data: None,
    }
}

pub fn method_not_found(method: &str) -> JsonRpcError {
    JsonRpcError {
        code: METHOD_NOT_FOUND,
        message: "method not found".into(),
        data: Some(json!({ "method": method })),
    }
}

pub fn invalid_params(message: impl Into<String>) -> JsonRpcError {
    JsonRpcError {
        code: INVALID_PARAMS,
        message: message.into(),
        data: None,
    }
}

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
        CoreError::Other(_) => internal_error(err.to_string()),
    }
}

/// v1 methods the owner advertises on `initialize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Initialize,
    IdentityGet,
    IssueList,
    IssueGet,
    IssueReady,
    IssueSearch,
    IssueClaims,
    IssueAgenda,
    IssueShow,
    IssueExcerpt,
    IssueTree,
    IssueRelated,
    IssueChildren,
    IssueAncestors,
    IssueImpact,
    IssueBacklinks,
    IssueOpen,
    IssueCreate,
    IssueUpdate,
    IssueClaim,
    IssueNote,
    IssueRefile,
    ProjectList,
    EventsSince,
    EventsGen,
}

impl Method {
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
        }
    }

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
            other => Err(method_not_found(other)),
        }
    }
}

/// Capability strings returned by `initialize` (v1). `initialize` itself is omitted.
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
    "project/list",
    "events/since",
    "events/gen",
    "identity/get",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub protocol_version: u32,
    #[serde(default)]
    pub client: String,
    pub agent: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: u32,
    pub capabilities: Vec<String>,
    pub root: String,
    pub prefix: String,
    pub generation: u64,
    pub revision: u64,
    pub identity: String,
}

/// Parse `initialize` params. Missing/empty `agent` and version != 1 are -32602.
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IssueListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IssueListResult {
    #[serde(default)]
    pub issues: Vec<IssueRow>,
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub matched: u64,
    pub revision: u64,
    #[serde(default)]
    pub generation: u64,
    #[serde(default)]
    pub unchanged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdParams {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueGetResult {
    #[serde(flatten)]
    pub issue: IssueDetail,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchParams {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ClaimsParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgendaParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeParams {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TreeResult {
    Nodes(TreeNode),
    Text { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedParams {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalkParams {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProjectListResult {
    pub projects: Vec<String>,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EventsSinceParams {
    pub since: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventsSinceResult {
    pub events: Vec<Event>,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventsGenResult {
    pub generation: u64,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityResult {
    pub identity: String,
    pub root: String,
    pub prefix: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateParams {
    pub project: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<char>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateParams {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unblock: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimParams {
    pub id: String,
    #[serde(default)]
    pub force: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteParams {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefileParams {
    pub id: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutResult {
    pub ok: bool,
    pub report: String,
    #[serde(default)]
    pub issue: Option<IssueDetail>,
    pub revision: u64,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultChanged {
    pub generation: u64,
    pub revision: u64,
    #[serde(default)]
    pub projects: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueSelected {
    pub id: String,
    pub project: String,
}

/// Push notifications. No `id` on the wire.
#[derive(Debug, Clone, PartialEq)]
pub enum Notification {
    VaultChanged(VaultChanged),
    IssueSelected(IssueSelected),
    ServeShuttingDown,
    Unknown { method: String, params: Value },
}

impl Notification {
    pub fn method(&self) -> &str {
        match self {
            Self::VaultChanged(_) => NOTIFY_VAULT_CHANGED,
            Self::IssueSelected(_) => NOTIFY_ISSUE_SELECTED,
            Self::ServeShuttingDown => NOTIFY_SHUTTING_DOWN,
            Self::Unknown { method, .. } => method,
        }
    }

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
    Initialize(InitializeParams),
    IdentityGet,
    IssueList(IssueListParams),
    IssueGet(IdParams),
    IssueReady(IssueListParams),
    IssueSearch(SearchParams),
    IssueClaims(ClaimsParams),
    IssueAgenda(AgendaParams),
    IssueShow(IdParams),
    IssueExcerpt(IdParams),
    IssueTree(TreeParams),
    IssueRelated(RelatedParams),
    IssueChildren(WalkParams),
    IssueAncestors(WalkParams),
    IssueImpact(WalkParams),
    IssueBacklinks(WalkParams),
    IssueOpen(IdParams),
    IssueCreate(CreateParams),
    IssueUpdate(UpdateParams),
    IssueClaim(ClaimParams),
    IssueNote(NoteParams),
    IssueRefile(RefileParams),
    ProjectList,
    EventsSince(EventsSinceParams),
    EventsGen,
}

impl Request {
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
            Self::ProjectList => Method::ProjectList,
            Self::EventsSince(_) => Method::EventsSince,
            Self::EventsGen => Method::EventsGen,
        }
    }

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
        }
    }

    pub fn to_params(&self) -> Value {
        match self {
            Self::Initialize(p) => serde_json::to_value(p).unwrap_or(Value::Null),
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
    Initialize(InitializeResult),
    IdentityGet(IdentityResult),
    IssueList(IssueListResult),
    IssueGet(IssueGetResult),
    IssueReady(IssueListResult),
    IssueSearch(Vec<SearchHit>),
    IssueClaims(Vec<ClaimRow>),
    IssueAgenda(Vec<AgendaRow>),
    IssueShow(IssueGetResult),
    IssueExcerpt(Excerpt),
    IssueTree(TreeResult),
    IssueRelated(Vec<RelatedHit>),
    IssueChildren(Vec<WalkHit>),
    IssueAncestors(Vec<WalkHit>),
    IssueImpact(Vec<WalkHit>),
    IssueBacklinks(Vec<WalkHit>),
    IssueOpen(IssueGetResult),
    IssueCreate(MutResult),
    IssueUpdate(MutResult),
    IssueClaim(MutResult),
    IssueNote(MutResult),
    IssueRefile(MutResult),
    ProjectList(ProjectListResult),
    EventsSince(EventsSinceResult),
    EventsGen(EventsGenResult),
}

impl Response {
    pub fn to_value(&self) -> Result<Value, serde_json::Error> {
        match self {
            Self::Initialize(v) => serde_json::to_value(v),
            Self::IdentityGet(v) => serde_json::to_value(v),
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
        let err = Method::parse("issue/fold").unwrap_err();
        assert_eq!(err.code, METHOD_NOT_FOUND);
        assert_eq!(err.data, Some(json!({"method": "issue/fold"})));
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
        assert!(value["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("issue/list")));
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
            parent: None,
            claimed_by: None,
            claimed_at: None,
            file: "issues.org:1-2".into(),
            line_start: 1,
            line_end: 2,
            logbook: vec![],
        };
        let get = IssueGetResult {
            issue: detail.clone(),
            revision: 1,
        };
        assert!(Response::IssueGet(get.clone()).to_value().unwrap()["id"] == "atlas-1a2b");
        assert!(Response::IssueShow(get.clone()).to_value().is_ok());
        assert!(Response::IssueOpen(get).to_value().is_ok());
        assert!(Response::IssueExcerpt(Excerpt {
            id: "atlas-1a2b".into(),
            file: "issues.org".into(),
            line_start: 1,
            line_end: 2,
            text: "body".into(),
            suppressed: false,
        })
        .to_value()
        .is_ok());
        assert!(Response::IssueSearch(vec![]).to_value().unwrap().is_array());
        assert!(Response::IssueClaims(vec![]).to_value().unwrap().is_array());
        assert!(Response::IssueAgenda(vec![]).to_value().unwrap().is_array());
        assert!(Response::IssueRelated(vec![])
            .to_value()
            .unwrap()
            .is_array());
        assert!(Response::IssueChildren(vec![])
            .to_value()
            .unwrap()
            .is_array());
        assert!(Response::IssueAncestors(vec![])
            .to_value()
            .unwrap()
            .is_array());
        assert!(Response::IssueImpact(vec![]).to_value().unwrap().is_array());
        assert!(Response::IssueBacklinks(vec![])
            .to_value()
            .unwrap()
            .is_array());
        assert!(Response::ProjectList(ProjectListResult {
            projects: vec!["atlas".into()],
            revision: 1,
        })
        .to_value()
        .is_ok());
        assert!(Response::EventsGen(EventsGenResult {
            generation: 1,
            revision: 1,
        })
        .to_value()
        .is_ok());
        assert!(Response::EventsSince(EventsSinceResult {
            events: vec![],
            generation: 1,
        })
        .to_value()
        .is_ok());
        assert!(Response::IdentityGet(IdentityResult {
            identity: "a".into(),
            root: "/".into(),
            prefix: "Software".into(),
            version: "0.2.0".into(),
        })
        .to_value()
        .is_ok());
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
        assert!(Response::IssueTree(TreeResult::Text { text: "* a".into() })
            .to_value()
            .is_ok());
        assert!(Response::IssueList(IssueListResult {
            revision: 1,
            ..IssueListResult::default()
        })
        .to_value()
        .is_ok());
        assert!(Response::IssueReady(IssueListResult {
            revision: 1,
            ..IssueListResult::default()
        })
        .to_value()
        .is_ok());
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
}
