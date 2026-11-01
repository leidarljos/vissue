//! JSON-RPC control wire: socket paths, framing, peer credentials, and types.
//!
//! This crate does not bind a socket. The owner lives in `vissue-serve`.

pub mod frame;
pub mod path;
pub mod rpc;

#[cfg(unix)]
pub mod client;
#[cfg(unix)]
pub mod peercred;

pub use frame::{
    FrameError, Framing, MAX_HEADER_LINES, MAX_MESSAGE_BYTES, classify_first_line, is_header_line,
    read_message, write_message,
};
pub use path::{
    HUD_LOG_ENV, HUD_SOCKET_ENV, SERVE_LOG_ENV, SOCKET_ENV, beside_socket, control_log_path,
    default_socket_path, hud_log_path, hud_socket_path, runtime_dir, socket_lock_path,
    socket_pid_path,
};
pub use rpc::{
    Error, Event, InitializeParams, InitializeResult, JsonRpcError, JsonRpcId, JsonRpcRequest,
    JsonRpcResponse, Method, NOTIFY_ISSUE_SELECTED, NOTIFY_SHUTTING_DOWN, NOTIFY_VAULT_CHANGED,
    Notification, PROTOCOL_VERSION, Request, Response, V1_CAPABILITIES, error_from_core,
    invalid_params, invalid_request, method_not_found, parse_error, parse_initialize_params,
};
pub use vissue_core::views::*;
