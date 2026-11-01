//! Bind, flock, accept, and the per-connection read loop.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{Context, anyhow};

use crate::error::Result;
use fs2::FileExt;
use serde_json::Value;
use tokio::net::UnixListener;
use tokio::sync::Semaphore;
use vissue_control::frame::{FrameError, Framing, read_message, write_message};
use vissue_control::peercred::accept_socket;
use vissue_control::rpc::{JsonRpcResponse, Notification, invalid_request, parse_error};
use vissue_control::{beside_socket, socket_lock_path, socket_pid_path};

use super::bus::Bus;
use super::catalog::Catalog;
use super::dispatch::dispatch_ex;
use crate::ServeConfig;

pub(super) struct OwnerState {
    pub layout: vissue_core::config::Layout,
    pub clients: AtomicUsize,
    pub catalog: RwLock<Catalog>,
    pub selection: Mutex<Option<(String, String)>>,
    pub bus: Bus,
    pub reload_sem: Semaphore,
}

impl OwnerState {
    pub(super) fn new(layout: vissue_core::config::Layout) -> Result<Self> {
        let catalog = Catalog::load(&layout)?;
        Ok(Self {
            layout,
            clients: AtomicUsize::new(0),
            catalog: RwLock::new(catalog),
            selection: Mutex::new(None),
            bus: Bus::new(),
            reload_sem: Semaphore::new(2),
        })
    }
}

pub(super) struct Session {
    pub agent: Option<String>,
}

/// In-process owner for tests and later attach helpers.
///
/// [`Self::spawn`] binds the socket on a background thread. Drop sends
/// shutdown and joins that thread.
pub struct OwnerHandle {
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Socket this owner bound.
    pub socket: PathBuf,
}

impl std::fmt::Debug for OwnerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerHandle")
            .field("socket", &self.socket)
            .finish_non_exhaustive()
    }
}

impl OwnerHandle {
    /// Bind an in-process owner on a background thread and wait until it accepts.
    ///
    /// # Errors
    ///
    /// Fails when the owner does not accept on the socket within
    /// [`crate::accept_timeout`].
    pub fn spawn(cfg: ServeConfig) -> Result<Self> {
        use super::lifecycle::wait_until_accepts;

        let socket = cfg.socket.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let thread = std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    eprintln!("vissue serve: runtime: {err}");
                    return;
                }
            };
            rt.block_on(async move {
                if let Err(err) = run_owner(cfg, rx).await {
                    eprintln!("vissue serve: {err:#}");
                }
            });
        });
        if !wait_until_accepts(&socket, crate::accept_timeout()) {
            return Err(anyhow!("owner did not accept on {}", socket.display()).into());
        }
        Ok(Self {
            shutdown: Some(tx),
            thread: Some(thread),
            socket,
        })
    }
}

impl Drop for OwnerHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

struct Owner {
    listener: UnixListener,
    lock: File,
    socket: PathBuf,
    pid_path: PathBuf,
    state: Arc<OwnerState>,
}

impl Drop for Owner {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket);
        let _ = fs::remove_file(&self.pid_path);
        let _ = self.lock.set_len(0);
    }
}

pub(super) fn run_foreground(cfg: &ServeConfig) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("start tokio runtime")?;
    rt.block_on(serve(cfg))
}

async fn serve(cfg: &ServeConfig) -> Result<()> {
    let owner = Owner::bind(cfg)?;
    eprintln!("vissue serve: control socket {}", owner.socket.display());
    eprintln!("  root={}", cfg.layout.root().display());
    eprintln!("  prefix={}", cfg.layout.prefix());
    eprintln!("  pid={}", std::process::id());
    let state = Arc::clone(&owner.state);
    tokio::select! {
        result = owner.accept_loop() => result,
        result = super::watcher::run(state.clone()) => result,
        _ = shutdown_signal() => {
            state.bus.broadcast(&Notification::ServeShuttingDown);
            Ok(())
        }
    }
}

async fn run_owner(cfg: ServeConfig, shutdown: tokio::sync::oneshot::Receiver<()>) -> Result<()> {
    let owner = Owner::bind(&cfg)?;
    let state = Arc::clone(&owner.state);
    tokio::select! {
        result = owner.accept_loop() => result,
        result = super::watcher::run(state.clone()) => result,
        _ = shutdown => {
            state.bus.broadcast(&Notification::ServeShuttingDown);
            Ok(())
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(s) => s,
        Err(_) => {
            let _ = ctrl_c.await;
            return;
        }
    };
    tokio::select! {
        _ = ctrl_c => {}
        _ = term.recv() => {}
    }
}

impl Owner {
    fn bind(cfg: &ServeConfig) -> Result<Self> {
        let socket = cfg.socket.clone();
        prepare_socket_dir(&socket)?;
        let lock = acquire_lock(&socket)?;
        takeover_or_fail(&socket)?;
        let listener =
            UnixListener::bind(&socket).with_context(|| format!("bind {}", socket.display()))?;
        chmod_path(&socket, 0o600)?;
        write_pid_file(&socket, std::process::id())?;
        Ok(Self {
            listener,
            lock,
            pid_path: socket_pid_path(&socket),
            socket,
            state: Arc::new(OwnerState::new(cfg.layout.clone())?),
        })
    }

    async fn accept_loop(&self) -> Result<()> {
        loop {
            let (stream, _) = self.listener.accept().await.context("accept")?;
            if !accept_socket(&stream) {
                if let Ok(uid) = vissue_control::peercred::peer_uid(&stream) {
                    eprintln!("vissue serve: rejected peer uid={uid}");
                } else {
                    eprintln!("vissue serve: rejected peer");
                }
                continue;
            }
            let std_stream = match stream.into_std() {
                Ok(s) => s,
                Err(err) => {
                    eprintln!("vissue serve: into_std: {err}");
                    continue;
                }
            };
            if let Err(err) = std_stream.set_nonblocking(false) {
                eprintln!("vissue serve: set_nonblocking: {err}");
                continue;
            }
            let state = Arc::clone(&self.state);
            std::thread::spawn(move || {
                state.clients.fetch_add(1, Ordering::Relaxed);
                handle_client(std_stream, &state);
                state.clients.fetch_sub(1, Ordering::Relaxed);
            });
        }
    }
}

pub(super) fn prepare_socket_dir(socket: &Path) -> Result<()> {
    let parent = socket.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(dir) = parent {
        let existed = dir.exists();
        fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        if may_chmod_socket_parent(dir, existed) {
            chmod_path(dir, 0o700)?;
        }
    }
    Ok(())
}

/// Tighten only a dedicated leaf (`vissue` / `run`) that we just created.
/// Shared parents (`/tmp`, `/var/tmp`, `/run`, `XDG_RUNTIME_DIR`) and
/// directories that already existed are left alone.
pub(super) fn may_chmod_socket_parent(dir: &Path, existed: bool) -> bool {
    if existed || is_shared_parent(dir) || !is_dedicated_leaf(dir) {
        return false;
    }
    dir_owned_by_current_uid(dir)
}

fn is_shared_parent(dir: &Path) -> bool {
    const SHARED: &[&str] = &["/tmp", "/var/tmp", "/run", "/var/run", "/dev/shm"];
    if SHARED.iter().any(|p| paths_equal(dir, Path::new(p))) {
        return true;
    }
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(xdg) if !xdg.is_empty() => paths_equal(dir, Path::new(&xdg)),
        _ => false,
    }
}

fn is_dedicated_leaf(dir: &Path) -> bool {
    matches!(
        dir.file_name().and_then(|s| s.to_str()),
        Some("vissue") | Some("run")
    )
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn dir_owned_by_current_uid(dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match fs::metadata(dir) {
        Ok(meta) => meta.uid() == vissue_control::peercred::current_uid(),
        Err(_) => false,
    }
}

fn chmod_path(path: &Path, mode: u32) -> Result<()> {
    let mut perms = fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?
        .permissions();
    perms.set_mode(mode);
    fs::set_permissions(path, perms)
        .with_context(|| format!("chmod {:o} {}", mode, path.display()))
        .map_err(crate::error::Error::from)
}

fn acquire_lock(socket: &Path) -> Result<File> {
    let lock_path = socket_lock_path(socket);
    if let Some(parent) = lock_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .open(&lock_path)
        .with_context(|| format!("open {}", lock_path.display()))?;
    file.try_lock_exclusive()
        .with_context(|| format!("control socket already in use: {}", socket.display()))?;
    file.set_len(0)?;
    writeln!(file, "{}", std::process::id())?;
    file.sync_all()?;
    Ok(file)
}

/// True when nothing holds the exclusive lock. The lock file is never unlinked.
pub(super) fn lock_is_free(socket: &Path) -> bool {
    let lock_path = socket_lock_path(socket);
    let file = match OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(_) => return false,
    };
    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = fs2::FileExt::unlock(&file);
            true
        }
        Err(_) => false,
    }
}

fn takeover_or_fail(socket: &Path) -> Result<()> {
    if !socket.exists() {
        return Ok(());
    }
    match StdUnixStream::connect(socket) {
        Ok(_) => Err(anyhow!("control socket already in use: {}", socket.display()).into()),
        Err(err)
            if err.kind() == io::ErrorKind::ConnectionRefused
                || err.kind() == io::ErrorKind::NotFound =>
        {
            fs::remove_file(socket)
                .with_context(|| format!("unlink stale socket {}", socket.display()))?;
            Ok(())
        }
        Err(err) => Err(anyhow!(
            "control socket already in use: {} ({err})",
            socket.display()
        )
        .into()),
    }
}

pub(super) fn write_pid_file(socket: &Path, pid: u32) -> Result<PathBuf> {
    let path = socket_pid_path(socket);
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(&path, format!("{pid}\n")).with_context(|| format!("write {}", path.display()))?;
    chmod_path(&path, 0o600)?;
    Ok(path)
}

pub(super) fn read_pid_file(socket: &Path) -> Option<u32> {
    let raw = fs::read_to_string(socket_pid_path(socket)).ok()?;
    raw.split_whitespace().next()?.parse().ok()
}

pub(super) fn remove_pid_file(socket: &Path) {
    let _ = fs::remove_file(socket_pid_path(socket));
}

pub(super) fn log_path(socket: &Path) -> PathBuf {
    match std::env::var(vissue_control::SERVE_LOG_ENV) {
        Ok(value) if !value.trim().is_empty() => PathBuf::from(value),
        _ => beside_socket(socket, "control.log"),
    }
}

fn handle_client(stream: StdUnixStream, state: &OwnerState) {
    let cloned = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut reader = BufReader::new(cloned);
    let writer = Arc::new(Mutex::new(stream));
    let mut session = Session { agent: None };
    let mut sink_id: Option<u64> = None;
    loop {
        let (payload, framing) = match read_message(&mut reader) {
            Ok(v) => v,
            Err(FrameError::Incomplete) => break,
            Err(FrameError::Io(err)) if err.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(_) => {
                let _ = write_locked(
                    &writer,
                    &JsonRpcResponse::err(Some(vissue_control::JsonRpcId::Null), parse_error()),
                    Framing::Jsonl,
                );
                break;
            }
        };
        if sink_id.is_none() {
            sink_id = Some(state.bus.register(Arc::clone(&writer), framing));
        }
        if payload.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let value: Value = match serde_json::from_slice(&payload) {
            Ok(v) => v,
            Err(_) => {
                let _ = write_locked(
                    &writer,
                    &JsonRpcResponse::err(Some(vissue_control::JsonRpcId::Null), parse_error()),
                    framing,
                );
                continue;
            }
        };
        let req: vissue_control::JsonRpcRequest = match serde_json::from_value(value) {
            Ok(r) => r,
            Err(_) => {
                let _ = write_locked(
                    &writer,
                    &JsonRpcResponse::err(Some(vissue_control::JsonRpcId::Null), invalid_request()),
                    framing,
                );
                continue;
            }
        };
        let out = dispatch_ex(state, &mut session, &req);
        if let Some(resp) = out.response {
            let _ = write_locked(&writer, &resp, framing);
        }
        for note in out.after {
            state.bus.broadcast(&note);
        }
    }
    if let Some(id) = sink_id {
        state.bus.unregister(id);
    }
}

fn write_locked(
    writer: &Mutex<StdUnixStream>,
    resp: &JsonRpcResponse,
    framing: Framing,
) -> io::Result<()> {
    let bytes = serde_json::to_vec(resp)?;
    let mut guard = writer.lock().unwrap_or_else(|p| p.into_inner());
    write_message(&mut *guard, &bytes, framing)?;
    guard.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::net::UnixListener as StdUnixListener;
    use vissue_control::JsonRpcId;
    use vissue_control::JsonRpcRequest;
    use vissue_core::config::Layout;

    fn state(dir: &Path) -> OwnerState {
        OwnerState::new(Layout::new(dir, "Software")).unwrap()
    }

    #[test]
    fn initialize_requires_agent() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let mut session = Session { agent: None };
        let req = JsonRpcRequest::call(
            JsonRpcId::Number(1),
            "initialize",
            json!({"protocolVersion": 1, "client": "tui"}),
        );
        let resp = dispatch_ex(&state, &mut session, &req).response.unwrap();
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32602);
        assert_eq!(err.message, "agent is required");
        assert!(session.agent.is_none());
    }

    #[test]
    fn initialize_empty_agent_is_invalid_params() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let mut session = Session { agent: None };
        let req = JsonRpcRequest::call(
            JsonRpcId::Number(1),
            "initialize",
            json!({"protocolVersion": 1, "agent": "  "}),
        );
        let err = dispatch_ex(&state, &mut session, &req)
            .response
            .unwrap()
            .error
            .unwrap();
        assert_eq!(err.code, -32602);
    }

    #[test]
    fn initialize_version_2_names_supported() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let mut session = Session { agent: None };
        let req = JsonRpcRequest::call(
            JsonRpcId::Number(1),
            "initialize",
            json!({"protocolVersion": 2, "agent": "tui"}),
        );
        let err = dispatch_ex(&state, &mut session, &req)
            .response
            .unwrap()
            .error
            .unwrap();
        assert_eq!(err.code, -32602);
        assert_eq!(err.data, Some(json!({"supported": 1})));
    }

    #[test]
    fn initialize_returns_live_capabilities_and_agent_identity() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let mut session = Session { agent: None };
        let req = JsonRpcRequest::call(
            JsonRpcId::Number(1),
            "initialize",
            json!({"protocolVersion": 1, "client": "tui", "agent": "tui-agent"}),
        );
        let resp = dispatch_ex(&state, &mut session, &req).response.unwrap();
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], 1);
        assert_eq!(result["identity"], "tui-agent");
        assert_eq!(result["prefix"], "Software");
        assert_eq!(result["revision"], 1);
        let caps = result["capabilities"].as_array().unwrap();
        assert!(caps.iter().any(|c| c == "identity/get"));
        assert!(caps.iter().any(|c| c == "issue/list"));
        assert!(caps.iter().any(|c| c == "issue/ready"));
        assert_eq!(session.agent.as_deref(), Some("tui-agent"));
    }

    #[test]
    fn identity_get_requires_initialize() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let mut session = Session { agent: None };
        let req = JsonRpcRequest::call(JsonRpcId::Number(2), "identity/get", json!({}));
        let err = dispatch_ex(&state, &mut session, &req)
            .response
            .unwrap()
            .error
            .unwrap();
        assert_eq!(err.code, -32602);
        assert_eq!(err.message, "initialize required");
    }

    #[test]
    fn identity_get_returns_connection_agent() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let mut session = Session {
            agent: Some("tui-agent".into()),
        };
        let req = JsonRpcRequest::call(JsonRpcId::Number(2), "identity/get", json!({}));
        let result = dispatch_ex(&state, &mut session, &req)
            .response
            .unwrap()
            .result
            .unwrap();
        assert_eq!(result["identity"], "tui-agent");
        assert_eq!(result["prefix"], "Software");
        assert_eq!(result["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(result["root"], dir.path().display().to_string());
    }

    #[test]
    fn unknown_method_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let mut session = Session { agent: None };
        // Deliberately not a verb that might grow a method later. This test used
        // "issue/fold" until fold got one, and then it was asserting that a real
        // method was missing.
        let req = JsonRpcRequest::call(JsonRpcId::Number(3), "issue/no-such-method", json!({}));
        let err = dispatch_ex(&state, &mut session, &req)
            .response
            .unwrap()
            .error
            .unwrap();
        assert_eq!(err.code, -32601);
        assert_eq!(err.data, Some(json!({"method": "issue/no-such-method"})));
    }

    #[test]
    fn notifications_have_no_reply() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let mut session = Session { agent: None };
        let req = JsonRpcRequest::notification("serve/shutting_down", json!({}));
        assert!(dispatch_ex(&state, &mut session, &req).response.is_none());
    }

    #[test]
    fn pid_file_roundtrip_is_mode_600() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("control.sock");
        write_pid_file(&socket, 4242).unwrap();
        assert_eq!(read_pid_file(&socket), Some(4242));
        let mode = fs::metadata(socket_pid_path(&socket))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        remove_pid_file(&socket);
        assert!(read_pid_file(&socket).is_none());
    }

    #[test]
    fn prepare_socket_dir_is_0700() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("vissue/control.sock");
        prepare_socket_dir(&socket).unwrap();
        let mode = fs::metadata(socket.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn prepare_socket_dir_does_not_chmod_an_existing_parent() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("vissue");
        fs::create_dir(&parent).unwrap();
        let mut perms = fs::metadata(&parent).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&parent, perms).unwrap();
        let socket = parent.join("control.sock");
        prepare_socket_dir(&socket).unwrap();
        let mode = fs::metadata(&parent).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
    }

    #[test]
    fn prepare_socket_dir_does_not_chmod_tmp() {
        assert!(!may_chmod_socket_parent(Path::new("/tmp"), false));
        assert!(!may_chmod_socket_parent(Path::new("/tmp"), true));
        assert!(!may_chmod_socket_parent(Path::new("/var/tmp"), false));
        assert!(!may_chmod_socket_parent(Path::new("/run"), false));
        assert!(!may_chmod_socket_parent(Path::new("/var/run"), false));
        assert!(!is_shared_parent(Path::new("/home/me/.vissue/run")));
        assert!(is_dedicated_leaf(Path::new("/run/user/1000/vissue")));
        assert!(is_dedicated_leaf(Path::new("/home/me/.vissue/run")));
        assert!(!is_dedicated_leaf(Path::new("/tmp")));
    }

    #[test]
    fn lock_is_free_when_unlocked_and_held_when_locked() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("control.sock");
        prepare_socket_dir(&socket).unwrap();
        assert!(lock_is_free(&socket));
        let held = acquire_lock(&socket).unwrap();
        assert!(!lock_is_free(&socket));
        drop(held);
        assert!(lock_is_free(&socket));
        assert!(socket_lock_path(&socket).exists());
    }

    #[test]
    fn takeover_unlinks_a_refused_stale_socket() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("control.sock");
        let listener = StdUnixListener::bind(&socket).unwrap();
        drop(listener);
        assert!(socket.exists());
        takeover_or_fail(&socket).unwrap();
        assert!(!socket.exists());
    }

    #[test]
    fn takeover_refuses_a_live_socket() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("control.sock");
        let _listener = StdUnixListener::bind(&socket).unwrap();
        let err = takeover_or_fail(&socket).unwrap_err();
        assert!(err.to_string().contains("already in use"), "{err}");
    }

    #[test]
    fn log_path_sits_beside_the_socket() {
        let socket = Path::new("/run/user/1000/vissue/control.sock");
        assert_eq!(
            log_path(socket),
            PathBuf::from("/run/user/1000/vissue/control.log")
        );
    }

    fn start_test_owner(dir: &Path) -> OwnerHandle {
        let socket = dir.join("run/control.sock");
        let layout = Layout::new(dir.join("vault"), "Software");
        let _ = fs::create_dir_all(layout.projects_dir());
        OwnerHandle::spawn(ServeConfig {
            layout,
            socket,
            exe: None,
        })
        .expect("spawn test owner")
    }

    #[test]
    fn in_process_owner_answers_initialize_and_identity() {
        use vissue_control::client::Client;

        let dir = tempfile::tempdir().unwrap();
        let owner = start_test_owner(dir.path());
        let meta = fs::metadata(owner.socket.parent().unwrap()).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o700);
        let meta = fs::metadata(&owner.socket).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);

        let mut client = Client::connect(&owner.socket).unwrap();
        let init = client
            .request(
                "initialize",
                json!({"protocolVersion": 1, "client": "test", "agent": "test-agent"}),
            )
            .unwrap();
        assert_eq!(init["identity"], "test-agent");
        assert_eq!(init["prefix"], "Software");
        let ident = client.request("identity/get", json!({})).unwrap();
        assert_eq!(ident["identity"], "test-agent");

        let list = client.request("issue/list", json!({})).unwrap();
        assert_eq!(list["unchanged"], false);
        assert!(list["issues"].as_array().unwrap().is_empty());
        assert_eq!(list["revision"], 1);
    }

    #[test]
    fn in_process_owner_rejects_initialize_without_agent() {
        use vissue_control::client::Client;

        let dir = tempfile::tempdir().unwrap();
        let owner = start_test_owner(dir.path());
        let mut client = Client::connect(&owner.socket).unwrap();
        let err = client
            .request(
                "initialize",
                json!({"protocolVersion": 1, "client": "test"}),
            )
            .unwrap_err();
        match err {
            vissue_control::Error::Rpc(e) => {
                assert_eq!(e.code, -32602);
                assert_eq!(e.message, "agent is required");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn header_framing_roundtrips_on_the_owner() {
        use vissue_control::Framing;
        use vissue_control::client::Client;

        let dir = tempfile::tempdir().unwrap();
        let owner = start_test_owner(dir.path());
        let mut client = Client::connect_with_framing(&owner.socket, Framing::Headers).unwrap();
        let init = client
            .request("initialize", json!({"protocolVersion": 1, "agent": "hdr"}))
            .unwrap();
        assert_eq!(init["identity"], "hdr");
    }

    #[test]
    fn second_bind_fails_while_lock_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let owner = start_test_owner(dir.path());
        let cfg = ServeConfig {
            layout: Layout::new(dir.path().join("vault"), "Software"),
            socket: owner.socket.clone(),
            exe: None,
        };
        let err = match Owner::bind(&cfg) {
            Ok(_) => panic!("second bind succeeded"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("already in use"), "{err:#}");
    }

    /// Everything the server advertises, it answers.
    ///
    /// `initialize` hands the client `LIVE_CAPABILITIES` as the list of
    /// methods it may call. Adding a name there without wiring the dispatch
    /// arm, or renaming an arm without the list, turns that promise into a
    /// "method not found" the client only discovers at run time. Empty params
    /// are enough: a routed method answers or rejects the arguments, and
    /// either one proves it is reachable.
    #[test]
    fn every_advertised_capability_is_routed() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let mut session = Session {
            agent: Some("probe".to_string()),
        };

        for (i, method) in crate::LIVE_CAPABILITIES.iter().enumerate() {
            let req = JsonRpcRequest::call(
                JsonRpcId::Number(i as i64 + 1),
                *method,
                serde_json::json!({}),
            );
            let resp = dispatch_ex(&state, &mut session, &req)
                .response
                .unwrap_or_else(|| panic!("{method} answered nothing"));
            if let Some(err) = resp.error {
                assert_ne!(
                    err.code,
                    vissue_control::rpc::METHOD_NOT_FOUND,
                    "{method} is advertised but not dispatched"
                );
            }
        }
    }

    /// The reverse: nothing is advertised that the protocol cannot name.
    #[test]
    fn the_advertised_list_is_the_protocol_surface() {
        for name in crate::LIVE_CAPABILITIES {
            vissue_control::rpc::Method::parse(name)
                .unwrap_or_else(|_| panic!("{name} is advertised but is not a Method"));
        }
        // `initialize` is the handshake, not a capability a client selects.
        assert!(
            !crate::LIVE_CAPABILITIES.contains(&"initialize"),
            "initialize is the handshake, not a listed capability"
        );
    }

    /// A writer reads its own write without waiting for the watcher.
    ///
    /// No watcher runs in this test, which is the point: every read here is
    /// served by the catalog the write path left behind. A create that does
    /// not refresh leaves `issue/list` short by one and `issue/get` on the
    /// freshly minted id answering "not found".
    #[test]
    fn a_write_is_visible_to_the_next_read_on_the_same_session() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let mut session = Session {
            agent: Some("probe".to_string()),
        };

        let before = dispatch_ex(
            &state,
            &mut session,
            &JsonRpcRequest::call(JsonRpcId::Number(1), "issue/list", json!({})),
        )
        .response
        .unwrap()
        .result
        .unwrap()["issues"]
            .as_array()
            .unwrap()
            .len();

        let created = dispatch_ex(
            &state,
            &mut session,
            &JsonRpcRequest::call(
                JsonRpcId::Number(2),
                "issue/create",
                json!({"project": "atlas", "title": "Read your own write"}),
            ),
        )
        .response
        .unwrap();
        assert!(created.error.is_none(), "{:?}", created.error);
        let made = created.result.unwrap();
        let id = made["issue"]["id"].as_str().unwrap().to_string();

        let listed = dispatch_ex(
            &state,
            &mut session,
            &JsonRpcRequest::call(JsonRpcId::Number(3), "issue/list", json!({})),
        )
        .response
        .unwrap()
        .result
        .unwrap();
        let ids: Vec<&str> = listed["issues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids.len(), before + 1, "{ids:?}");
        assert!(ids.contains(&id.as_str()), "{id} missing from {ids:?}");

        let got = dispatch_ex(
            &state,
            &mut session,
            &JsonRpcRequest::call(JsonRpcId::Number(4), "issue/get", json!({ "id": id })),
        )
        .response
        .unwrap();
        assert!(got.error.is_none(), "issue/get {id}: {:?}", got.error);
        // `IssueGetResult` flattens the detail, so the id sits at the top.
        assert_eq!(got.result.unwrap()["id"], id.as_str());

        // The revision the writer was handed is the one the catalog now has,
        // so a client can use it to tell its own write from someone else's.
        let revision = made["revision"].as_u64().unwrap();
        assert_eq!(revision, state.catalog.read().unwrap().revision);
    }
}
