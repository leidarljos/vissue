//! Unix summon socket for compositor binds (`vissue hud --toggle`).
//!
//! Commands are one line: `show`, `hide`, `toggle`, plus an optional
//! xdg-activation token. No CR/LF inside the token; max 512 bytes.

#[cfg(unix)]
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvError, SyncSender, TryRecvError};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

/// Env override for the summon socket path.
pub const SOCKET_ENV: &str = vissue_control::HUD_SOCKET_ENV;

/// Operator action for the iced loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummonAction {
    /// Map the HUD window.
    Show,
    /// Unmap the HUD window.
    Hide,
    /// Invert mapped state.
    Toggle,
}

/// One summon request: verb plus optional xdg-activation token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummonRequest {
    /// Verb the iced loop applies.
    pub action: SummonAction,
    /// Optional xdg-activation token; never set on [`SummonAction::Hide`].
    pub token: Option<String>,
}

impl SummonRequest {
    /// Request `action` with no activation token.
    pub fn new(action: SummonAction) -> Self {
        Self {
            action,
            token: None,
        }
    }
}

/// Why a summon send or bind failed.
#[derive(Debug)]
pub enum SummonError {
    /// Summon sockets are Unix-only.
    Unsupported,
    /// Socket path resolved empty.
    NoPath,
    /// No HUD is accepting on this path.
    NotRunning(String),
    /// Another HUD already owns this path.
    AlreadyRunning(String),
    /// Filesystem or stream I/O failed.
    Io(std::io::Error),
    /// Thread spawn or other bind-side failure.
    Other(String),
}

impl std::fmt::Display for SummonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => write!(f, "summon socket not available on this platform"),
            Self::NoPath => write!(f, "summon socket path could not be resolved"),
            Self::NotRunning(path) => write!(f, "HUD summon socket not accepting ({path})"),
            Self::AlreadyRunning(path) => write!(f, "HUD summon socket already in use ({path})"),
            Self::Io(err) => write!(f, "{err}"),
            Self::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for SummonError {}

impl From<std::io::Error> for SummonError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Holds the listener thread and bound path for process lifetime.
#[derive(Debug)]
pub struct SummonServer {
    path: PathBuf,
    #[allow(dead_code)]
    join: Option<thread::JoinHandle<()>>,
}

impl Drop for SummonServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Parse a single command line (trimmed, case-insensitive).
pub fn parse_command(raw: &str) -> Option<SummonAction> {
    parse_request(raw).map(|r| r.action)
}

/// Parse `show` / `hide` / `toggle` and an optional same-line token.
pub fn parse_request(raw: &str) -> Option<SummonRequest> {
    let line = raw.trim();
    if line.is_empty() {
        return None;
    }
    let mut parts = line.splitn(2, char::is_whitespace);
    let verb = parts.next()?.to_ascii_lowercase();
    let action = match verb.as_str() {
        "show" => SummonAction::Show,
        "hide" => SummonAction::Hide,
        "toggle" => SummonAction::Toggle,
        _ => return None,
    };
    let token = parts
        .next()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .and_then(sanitize_token);
    let token = match action {
        SummonAction::Hide => None,
        _ => token,
    };
    Some(SummonRequest { action, token })
}

/// Token after trim: 1..=512 bytes, no CR/LF. Empty or oversize is absent.
pub fn sanitize_token(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() || t.len() > 512 || t.contains('\n') || t.contains('\r') {
        return None;
    }
    Some(t.to_string())
}

/// Read `XDG_ACTIVATION_TOKEN` and unset it and `DESKTOP_STARTUP_ID`.
pub fn take_env_token() -> Option<String> {
    let raw = std::env::var("XDG_ACTIVATION_TOKEN").ok();
    unset_activation_vars();
    raw.as_deref().and_then(sanitize_token)
}

#[allow(unsafe_code)]
fn unset_activation_vars() {
    // SAFETY: compositor tokens are single-use. Edition 2024 marks
    // `remove_var` unsafe; the HUD process must drop both names so a
    // later spawn cannot reuse them. Hide still unsets.
    unsafe {
        std::env::remove_var("XDG_ACTIVATION_TOKEN");
        std::env::remove_var("DESKTOP_STARTUP_ID");
    }
}

/// True when the socket is absent so a compositor bind cannot talk to a HUD.
pub fn is_summon_miss(err: &SummonError) -> bool {
    matches!(
        err,
        SummonError::NotRunning(_) | SummonError::NoPath | SummonError::Unsupported
    )
}

/// What `--show` / `--hide` / `--toggle` should do after talking to the socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummonCli {
    /// Command delivered, or hide with nothing running.
    Done,
    /// Start a new HUD and show the overlay.
    StartShown,
}

/// Plan the binary's next step after [`send_command`].
///
/// Hide with no listener exits successfully. Show and toggle start a HUD.
///
/// # Errors
///
/// Returns the send error when it is not a miss (socket absent / unsupported).
pub fn plan_summon_cli(
    action: SummonAction,
    result: Result<(), SummonError>,
) -> Result<SummonCli, SummonError> {
    match result {
        Ok(()) => Ok(SummonCli::Done),
        Err(err) if is_summon_miss(&err) && matches!(action, SummonAction::Hide) => {
            Ok(SummonCli::Done)
        }
        Err(err) if is_summon_miss(&err) => Ok(SummonCli::StartShown),
        Err(err) => Err(err),
    }
}

/// Wire form for `action` (one word, no newline).
pub fn command_word(action: SummonAction) -> &'static str {
    match action {
        SummonAction::Show => "show",
        SummonAction::Hide => "hide",
        SummonAction::Toggle => "toggle",
    }
}

/// Default path: `$XDG_RUNTIME_DIR/vissue/hud.sock`, or the env override.
pub fn default_socket_path() -> PathBuf {
    vissue_control::hud_socket_path()
}

/// True when a HUD already owns the default summon socket.
pub fn already_running() -> bool {
    socket_accepts(&default_socket_path())
}

/// True when a listener is bound (connect succeeds).
pub fn socket_accepts(path: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::net::UnixStream::connect(path).is_ok()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

/// Send one summon command to a running HUD.
///
/// # Errors
///
/// Returns an error if the platform has no summon socket, no HUD is accepting
/// on the default path, or the write fails.
pub fn send_command(action: SummonAction) -> Result<(), SummonError> {
    let token = match action {
        SummonAction::Hide => {
            let _ = take_env_token();
            None
        }
        _ => take_env_token(),
    };
    send_request(SummonRequest { action, token })
}

/// Send a parsed request to the default socket.
///
/// # Errors
///
/// Returns an error if the platform has no summon socket, no HUD is accepting
/// on the default path, or the write fails.
pub fn send_request(req: SummonRequest) -> Result<(), SummonError> {
    #[cfg(unix)]
    {
        send_request_to(&default_socket_path(), &req)
    }
    #[cfg(not(unix))]
    {
        let _ = req;
        Err(SummonError::Unsupported)
    }
}

/// Send `action` to `path` (no env token).
///
/// # Errors
///
/// Returns an error if the platform has no summon socket, no HUD is accepting
/// on `path`, or the write fails.
pub fn send_command_to(path: &Path, action: SummonAction) -> Result<(), SummonError> {
    send_request_to(path, &SummonRequest::new(action))
}

/// Write one wire line to `path`.
///
/// # Errors
///
/// Returns an error if the platform has no summon socket, no HUD is accepting
/// on `path`, or the write fails.
pub fn send_request_to(path: &Path, req: &SummonRequest) -> Result<(), SummonError> {
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;
        let mut stream = UnixStream::connect(path)
            .map_err(|err| SummonError::NotRunning(format!("{}: {err}", path.display())))?;
        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
        let line = encode_request(req);
        stream.write_all(line.as_bytes())?;
        stream.flush()?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (path, req);
        Err(SummonError::Unsupported)
    }
}

/// Canonical wire: `verb` or `verb token`, always one LF.
pub fn encode_request(req: &SummonRequest) -> String {
    match req.action {
        SummonAction::Hide => format!("{}\n", command_word(req.action)),
        _ => match req.token.as_deref().and_then(sanitize_token) {
            Some(tok) => format!("{} {tok}\n", command_word(req.action)),
            None => format!("{}\n", command_word(req.action)),
        },
    }
}

/// Bind the summon socket and start the accept thread.
///
/// # Errors
///
/// Returns an error if the platform has no summon socket, the path cannot be
/// resolved, another HUD already owns the socket, the listener cannot bind,
/// or the accept thread cannot spawn.
pub fn install() -> Result<SummonServer, SummonError> {
    #[cfg(unix)]
    {
        install_unix()
    }
    #[cfg(not(unix))]
    {
        Err(SummonError::Unsupported)
    }
}

/// Non-blocking read of the next queued summon request.
pub fn try_recv() -> Option<SummonRequest> {
    let guard = action_pair().1.lock().ok()?;
    guard.try_recv().ok()
}

/// Block until the next summon request.
///
/// # Errors
///
/// Returns an error if the accept thread has dropped the sender.
///
/// # Panics
///
/// Panics if the summon action mutex is poisoned.
pub fn recv_action() -> Result<SummonRequest, RecvError> {
    loop {
        let outcome = {
            let guard = action_pair().1.lock().expect("summon action mutex");
            guard.try_recv()
        };
        match outcome {
            Ok(action) => return Ok(action),
            Err(TryRecvError::Disconnected) => return Err(RecvError),
            Err(TryRecvError::Empty) => {
                thread::sleep(Duration::from_millis(25));
            }
        }
    }
}

fn action_pair() -> &'static (SyncSender<SummonRequest>, Mutex<Receiver<SummonRequest>>) {
    static PAIR: OnceLock<(SyncSender<SummonRequest>, Mutex<Receiver<SummonRequest>>)> =
        OnceLock::new();
    PAIR.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel(16);
        (tx, Mutex::new(rx))
    })
}

#[cfg(unix)]
fn action_sender() -> SyncSender<SummonRequest> {
    action_pair().0.clone()
}

/// Probe before unlink: a live HUD keeps the inode; only a stale path is removed.
#[cfg(unix)]
fn prepare_bind_path(path: &Path) -> Result<(), SummonError> {
    if socket_accepts(path) {
        return Err(SummonError::AlreadyRunning(path.display().to_string()));
    }
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

#[cfg(unix)]
fn install_unix() -> Result<SummonServer, SummonError> {
    use std::os::unix::net::UnixListener;

    let path = default_socket_path();
    if path.as_os_str().is_empty() {
        return Err(SummonError::NoPath);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }
    prepare_bind_path(&path)?;
    let listener = UnixListener::bind(&path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::AddrInUse {
            SummonError::AlreadyRunning(path.display().to_string())
        } else {
            SummonError::Io(err)
        }
    })?;
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    let _ = action_sender();
    let path_log = path.clone();
    let join = thread::Builder::new()
        .name("vissue-hud-summon".into())
        .spawn(move || accept_loop(listener))
        .map_err(|err| SummonError::Other(format!("spawn summon thread: {err}")))?;
    crate::log::info(&format!("summon socket {}", path_log.display()));
    Ok(SummonServer {
        path,
        join: Some(join),
    })
}

#[cfg(unix)]
fn accept_loop(listener: std::os::unix::net::UnixListener) {
    let tx = action_sender();
    loop {
        let Ok((stream, _)) = listener.accept() else {
            thread::sleep(Duration::from_millis(50));
            continue;
        };
        if let Some(req) = read_action(stream)
            && tx.send(req).is_err()
        {
            break;
        }
    }
}

#[cfg(unix)]
fn read_action(stream: std::os::unix::net::UnixStream) -> Option<SummonRequest> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    parse_request(&line)
}

#[cfg(test)]
#[allow(deprecated_safe_2024)]
mod tests {
    use super::*;

    #[test]
    fn parse_command_words() {
        assert_eq!(parse_command("show"), Some(SummonAction::Show));
        assert_eq!(parse_command(" HIDE\n"), Some(SummonAction::Hide));
        assert_eq!(parse_command("Toggle"), Some(SummonAction::Toggle));
        assert_eq!(parse_command("quit"), None);
        assert_eq!(parse_command(""), None);
    }

    #[test]
    fn parse_request_keeps_token_on_show_and_toggle() {
        assert_eq!(
            parse_request("toggle abc.def"),
            Some(SummonRequest {
                action: SummonAction::Toggle,
                token: Some("abc.def".into()),
            })
        );
        assert_eq!(
            parse_request("show  tok-1 "),
            Some(SummonRequest {
                action: SummonAction::Show,
                token: Some("tok-1".into()),
            })
        );
    }

    #[test]
    fn parse_request_strips_token_on_hide() {
        assert_eq!(
            parse_request("hide leftover"),
            Some(SummonRequest::new(SummonAction::Hide))
        );
    }

    #[test]
    fn hide_miss_is_done() {
        let err = SummonError::NotRunning("/tmp/missing.sock".into());
        assert_eq!(
            plan_summon_cli(SummonAction::Hide, Err(err)).unwrap(),
            SummonCli::Done
        );
    }

    #[test]
    fn plan_summon_cli_starts_when_show_misses() {
        let miss = SummonError::NotRunning("gone".into());
        assert_eq!(
            plan_summon_cli(SummonAction::Show, Err(miss)).unwrap(),
            SummonCli::StartShown
        );
        assert_eq!(
            plan_summon_cli(SummonAction::Toggle, Err(SummonError::NoPath)).unwrap(),
            SummonCli::StartShown
        );
        assert_eq!(
            plan_summon_cli(SummonAction::Hide, Err(SummonError::Unsupported)).unwrap(),
            SummonCli::Done
        );
        assert_eq!(
            plan_summon_cli(SummonAction::Show, Ok(())).unwrap(),
            SummonCli::Done
        );
        assert!(plan_summon_cli(SummonAction::Show, Err(SummonError::Other("x".into()))).is_err());
    }

    #[test]
    #[allow(unsafe_code)]
    fn take_env_token_unsets_activation_vars() {
        let _guard = crate::env_lock();
        // SAFETY: test-only process env, serialized by env_lock.
        unsafe {
            std::env::set_var("XDG_ACTIVATION_TOKEN", "tok-xyz");
            std::env::set_var("DESKTOP_STARTUP_ID", "startup");
        }
        let tok = take_env_token();
        assert_eq!(tok.as_deref(), Some("tok-xyz"));
        assert!(std::env::var("XDG_ACTIVATION_TOKEN").is_err());
        assert!(std::env::var("DESKTOP_STARTUP_ID").is_err());
    }

    #[test]
    fn sanitize_token_rejects_empty_newline_and_oversize() {
        assert_eq!(sanitize_token("  "), None);
        assert_eq!(sanitize_token("a\nb"), None);
        assert_eq!(sanitize_token("a\rb"), None);
        assert_eq!(sanitize_token(&"x".repeat(513)), None);
        assert_eq!(sanitize_token("ok"), Some("ok".into()));
    }

    #[test]
    fn encode_request_one_line() {
        assert_eq!(
            encode_request(&SummonRequest::new(SummonAction::Toggle)),
            "toggle\n"
        );
        assert_eq!(
            encode_request(&SummonRequest {
                action: SummonAction::Show,
                token: Some("t1".into()),
            }),
            "show t1\n"
        );
        assert_eq!(
            encode_request(&SummonRequest {
                action: SummonAction::Toggle,
                token: Some("a\nb".into()),
            }),
            "toggle\n"
        );
        assert_eq!(
            encode_request(&SummonRequest {
                action: SummonAction::Hide,
                token: Some("ignored".into()),
            }),
            "hide\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn send_round_trip_on_temp_socket() {
        use std::os::unix::net::UnixListener;

        let dir = std::env::temp_dir().join(format!("vissue-hud-summon-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("hud.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind");
        let (tx, rx) = mpsc::sync_channel(1);
        let path_server = path.clone();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let action = read_action(stream).expect("action");
            tx.send(action).unwrap();
            let _ = std::fs::remove_file(&path_server);
        });
        send_command_to(&path, SummonAction::Toggle).expect("send");
        let got = rx.recv_timeout(Duration::from_secs(2)).expect("recv");
        assert_eq!(got, SummonRequest::new(SummonAction::Toggle));
        handle.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn command_word_round_trip() {
        for action in [SummonAction::Show, SummonAction::Hide, SummonAction::Toggle] {
            assert_eq!(parse_command(command_word(action)), Some(action));
        }
    }

    #[test]
    fn summon_error_display_names_the_case() {
        assert!(SummonError::Unsupported.to_string().contains("platform"));
        assert!(SummonError::NoPath.to_string().contains("path"));
        assert!(
            SummonError::NotRunning("x".into())
                .to_string()
                .contains("x")
        );
        assert!(
            SummonError::AlreadyRunning("y".into())
                .to_string()
                .contains("y")
        );
        assert!(SummonError::Other("z".into()).to_string().contains("z"));
        let io = SummonError::from(std::io::Error::other("io"));
        assert!(io.to_string().contains("io"));
    }

    #[test]
    fn default_path_ends_with_hud_sock() {
        let _guard = crate::env_lock();
        vissue_core::process_env::override_var(SOCKET_ENV, None);
        let path = default_socket_path();
        vissue_core::process_env::clear_override(SOCKET_ENV);
        assert_eq!(path.file_name().unwrap(), "hud.sock");
    }

    #[cfg(unix)]
    #[test]
    fn prepare_bind_path_keeps_live_and_removes_stale() {
        use std::os::unix::net::UnixListener;

        let dir =
            std::env::temp_dir().join(format!("vissue-hud-summon-live-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("hud.sock");
        let _ = std::fs::remove_file(&path);
        let _listener = UnixListener::bind(&path).expect("bind");
        let err = prepare_bind_path(&path).expect_err("live");
        assert!(path.exists());
        assert!(matches!(err, SummonError::AlreadyRunning(_)));
        assert!(socket_accepts(&path));
        drop(_listener);
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"").expect("stale");
        prepare_bind_path(&path).expect("stale");
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn send_request_round_trip_with_token() {
        use std::os::unix::net::UnixListener;

        let dir =
            std::env::temp_dir().join(format!("vissue-hud-summon-tok-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("hud.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind");
        let (tx, rx) = mpsc::sync_channel(1);
        let path_server = path.clone();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let req = read_action(stream).expect("request");
            tx.send(req).unwrap();
            let _ = std::fs::remove_file(&path_server);
        });
        send_request_to(
            &path,
            &SummonRequest {
                action: SummonAction::Toggle,
                token: Some("act.token".into()),
            },
        )
        .expect("send");
        let got = rx.recv_timeout(Duration::from_secs(2)).expect("recv");
        assert_eq!(
            got,
            SummonRequest {
                action: SummonAction::Toggle,
                token: Some("act.token".into()),
            }
        );
        handle.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn install_then_send_reaches_try_recv() {
        let _guard = crate::env_lock();
        let dir =
            std::env::temp_dir().join(format!("vissue-hud-summon-install-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("hud.sock");
        let _ = std::fs::remove_file(&path);
        vissue_core::process_env::override_var(SOCKET_ENV, path.to_str());
        let server = install().expect("install");
        assert!(already_running());
        send_command(SummonAction::Show).expect("send");
        let mut got = None;
        for _ in 0..40 {
            if let Some(req) = try_recv() {
                got = Some(req);
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(got, Some(SummonRequest::new(SummonAction::Show)));
        drop(server);
        vissue_core::process_env::clear_override(SOCKET_ENV);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn send_to_missing_socket_is_not_running() {
        let path = std::env::temp_dir().join("vissue-hud-missing.sock");
        let _ = std::fs::remove_file(&path);
        let err = send_command_to(&path, SummonAction::Hide).unwrap_err();
        assert!(matches!(
            err,
            SummonError::NotRunning(_) | SummonError::Unsupported
        ));
    }
}
