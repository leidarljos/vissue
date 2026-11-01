//! Detach, stop, status, and ensure. The detach parent never starts tokio.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};

use crate::error::Result;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use serde_json::json;
use vissue_control::client::Client;
use vissue_core::events;

use super::owner::{lock_is_free, log_path, prepare_socket_dir, read_pid_file, remove_pid_file};
use crate::{EnsureResult, SERVE_REVISION, ServeConfig, Status};

const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// `vissue-hud` is a separate binary. Spawning `current_exe() serve` then
/// becomes `vissue-hud serve`, which clap rejects. Prefer an explicit path,
/// then a sibling `vissue`, then `$PATH`.
fn resolve_serve_exe(cfg: &ServeConfig) -> Result<PathBuf> {
    if let Some(path) = &cfg.exe {
        return Ok(path.clone());
    }
    let current = std::env::current_exe().context("resolve current executable")?;
    serve_exe_from(&current, std::env::var_os("PATH").as_deref())
}

fn serve_exe_from(current: &Path, path_var: Option<&std::ffi::OsStr>) -> Result<PathBuf> {
    let name = current
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if name == "vissue" || name == "vissue.exe" {
        return Ok(current.to_path_buf());
    }
    if let Some(dir) = current.parent() {
        for candidate in ["vissue", "vissue.exe"] {
            let sibling = dir.join(candidate);
            if sibling.is_file() {
                return Ok(sibling);
            }
        }
    }
    if let Some(path_var) = path_var {
        for dir in std::env::split_paths(path_var) {
            let candidate = dir.join("vissue");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(anyhow!(
        "cannot find a vissue CLI to spawn serve (running as {})",
        current.display()
    )
    .into())
}
const TERM_WAIT: Duration = Duration::from_secs(5);
const KILL_WAIT: Duration = Duration::from_secs(2);

/// True when a connect to `path` succeeds within a short retry window.
///
/// Connection refused or a missing path is false. Transient `WouldBlock`,
/// `Interrupted`, and `TimedOut` retry for about 150 ms.
pub fn socket_accepts(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    let deadline = Instant::now() + Duration::from_millis(150);
    let mut delay = Duration::from_millis(20);
    loop {
        match UnixStream::connect(path) {
            Ok(_) => return true,
            Err(err)
                if err.kind() == io::ErrorKind::ConnectionRefused
                    || err.kind() == io::ErrorKind::NotFound =>
            {
                return false;
            }
            Err(err)
                if err.kind() == io::ErrorKind::WouldBlock
                    || err.kind() == io::ErrorKind::Interrupted
                    || err.kind() == io::ErrorKind::TimedOut =>
            {
                if Instant::now() >= deadline {
                    return false;
                }
                thread::sleep(delay);
                delay = delay.saturating_mul(2).min(Duration::from_millis(50));
            }
            Err(_) => return false,
        }
    }
}

pub fn wait_until_accepts(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if socket_accepts(path) {
            return true;
        }
        thread::sleep(POLL_INTERVAL);
    }
    socket_accepts(path)
}

pub fn pid_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    match kill(Pid::from_raw(pid as i32), None) {
        Ok(()) => true,
        Err(nix::errno::Errno::ESRCH) => false,
        Err(_) => true,
    }
}

fn signal_pid(pid: u32, sig: Signal) -> io::Result<()> {
    let raw = pid as i32;
    let group = kill(Pid::from_raw(-raw), sig);
    if group.is_ok() {
        return Ok(());
    }
    kill(Pid::from_raw(raw), sig).map_err(|e| io::Error::from_raw_os_error(e as i32))
}

fn wait_pid_gone(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !pid_is_alive(pid) {
            return true;
        }
        thread::sleep(POLL_INTERVAL);
    }
    !pid_is_alive(pid)
}

pub fn status(cfg: &ServeConfig) -> Status {
    let live = socket_accepts(&cfg.socket);
    let pid = read_pid_file(&cfg.socket);
    let mut snap = Status {
        live,
        pid,
        socket: cfg.socket.clone(),
        root: cfg.layout.root().to_path_buf(),
        prefix: cfg.layout.prefix().to_string(),
        generation: events::generation(&cfg.layout),
        revision: SERVE_REVISION,
        clients: 0,
    };
    if live {
        enrich_from_owner(&mut snap);
    }
    snap
}

fn enrich_from_owner(snap: &mut Status) {
    let mut client = match Client::connect(&snap.socket) {
        Ok(c) => c,
        Err(_) => return,
    };
    let result = client.request(
        "initialize",
        json!({
            "protocolVersion": 1,
            "client": "vissue-status",
            "agent": "vissue-status",
        }),
    );
    let Ok(value) = result else {
        return;
    };
    if let Some(root) = value.get("root").and_then(serde_json::Value::as_str) {
        snap.root = Path::new(root).to_path_buf();
    }
    if let Some(prefix) = value.get("prefix").and_then(serde_json::Value::as_str) {
        snap.prefix = prefix.to_string();
    }
    if let Some(generation) = value.get("generation").and_then(serde_json::Value::as_u64) {
        snap.generation = generation;
    }
    if let Some(revision) = value.get("revision").and_then(serde_json::Value::as_u64) {
        snap.revision = revision;
    }
}

pub fn print_status(status: &Status, json: bool) -> Result<()> {
    if json {
        let rendered = serde_json::to_string_pretty(status).context("encode status")?;
        println!("{rendered}");
        return Ok(());
    }
    println!("live: {}", status.live);
    match status.pid {
        Some(pid) => println!("pid: {pid}"),
        None => println!("pid:"),
    }
    println!("socket: {}", status.socket.display());
    println!("root: {}", status.root.display());
    println!("prefix: {}", status.prefix);
    println!("generation: {}", status.generation);
    println!("revision: {}", status.revision);
    println!("clients: {}", status.clients);
    Ok(())
}

pub fn stop(cfg: &ServeConfig) -> Result<i32> {
    let socket = &cfg.socket;
    let pid = read_pid_file(socket);
    let accepts = socket_accepts(socket);

    if lock_is_free(socket) {
        if accepts {
            eprintln!("error: control socket is live but the ownership lock is free; not stopping");
            return Ok(1);
        }
        unlink_stale_socket_if_lock_free(socket);
        remove_pid_file(socket);
        eprintln!("already stopped  socket={}", socket.display());
        return Ok(0);
    }

    if accepts {
        match pid {
            None => {
                eprintln!(
                    "error: control socket is live but no daemon pid file \
                     (owner is not a vissue serve process; not stopping)"
                );
                return Ok(1);
            }
            Some(pid) if !pid_is_alive(pid) => {
                eprintln!(
                    "error: pid {pid} is not running but socket still accepts connections; \
                     not unlinking live socket"
                );
                remove_pid_file(socket);
                return Ok(1);
            }
            Some(_) => {}
        }
    }

    if let Some(pid) = pid.filter(|p| pid_is_alive(*p)) {
        return stop_pid(pid, socket);
    }

    unlink_stale_socket_if_lock_free(socket);
    remove_pid_file(socket);
    eprintln!("already stopped  socket={}", socket.display());
    Ok(0)
}

fn stop_pid(pid: u32, socket: &Path) -> Result<i32> {
    if let Err(err) = signal_pid(pid, Signal::SIGTERM) {
        if err.kind() == io::ErrorKind::NotFound {
            unlink_stale_socket_if_lock_free(socket);
            remove_pid_file(socket);
            eprintln!("already stopped  socket={}", socket.display());
            return Ok(0);
        }
        eprintln!("error: could not signal pid {pid}: {err}");
        return Ok(1);
    }
    if wait_pid_gone(pid, TERM_WAIT) {
        unlink_stale_socket_if_lock_free(socket);
        remove_pid_file(socket);
        eprintln!("stopped pid={pid}");
        return Ok(0);
    }
    let _ = signal_pid(pid, Signal::SIGKILL);
    if wait_pid_gone(pid, KILL_WAIT) {
        unlink_stale_socket_if_lock_free(socket);
        remove_pid_file(socket);
        eprintln!("stopped pid={pid} (SIGKILL)");
        return Ok(0);
    }
    eprintln!(
        "error: pid {pid} did not exit within {}s",
        TERM_WAIT.as_secs()
    );
    Ok(1)
}

fn unlink_stale_socket_if_lock_free(socket: &Path) {
    if socket_accepts(socket) {
        return;
    }
    if !lock_is_free(socket) {
        return;
    }
    if socket.exists() {
        let _ = fs::remove_file(socket);
    }
}

pub fn start_detached(cfg: &ServeConfig) -> Result<EnsureResult> {
    if socket_accepts(&cfg.socket) {
        return Ok(EnsureResult {
            ok: true,
            already_running: true,
            spawned: false,
            pid: read_pid_file(&cfg.socket),
            socket: cfg.socket.clone(),
            error: None,
        });
    }

    prepare_socket_dir(&cfg.socket)?;
    let log = log_path(&cfg.socket);
    if let Some(parent) = log.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .with_context(|| format!("open {}", log.display()))?;
    let _ = writeln!(
        log_file,
        "\n--- vissue serve --foreground spawn parent={} ---",
        std::process::id()
    );

    let exe = resolve_serve_exe(cfg)?;
    let stdout = log_file.try_clone().context("clone serve log for stdout")?;
    let mut cmd = Command::new(&exe);
    cmd.arg("serve")
        .arg("--foreground")
        .arg("--root")
        .arg(cfg.layout.root())
        .arg("--prefix")
        .arg(cfg.layout.prefix())
        .arg("--socket")
        .arg(&cfg.socket)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(log_file))
        .process_group(0);

    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            return Ok(EnsureResult {
                ok: false,
                already_running: false,
                spawned: false,
                pid: None,
                socket: cfg.socket.clone(),
                error: Some(format!("spawn failed: {err}")),
            });
        }
    };

    let timeout = crate::accept_timeout();
    if wait_until_accepts(&cfg.socket, timeout) {
        return Ok(EnsureResult {
            ok: true,
            already_running: false,
            spawned: true,
            pid: read_pid_file(&cfg.socket).or(Some(child.id())),
            socket: cfg.socket.clone(),
            error: None,
        });
    }

    let _ = signal_pid(child.id(), Signal::SIGTERM);
    let mut err = format!(
        "control socket did not accept within {}s",
        timeout.as_secs()
    );
    if let Ok(tail) = fs::read_to_string(&log) {
        let snippet = if tail.len() > 800 {
            &tail[tail.len() - 800..]
        } else {
            &tail
        };
        if !snippet.trim().is_empty() {
            err = format!("{err}\nlog tail:\n{snippet}");
        }
    }
    Ok(EnsureResult {
        ok: false,
        already_running: false,
        spawned: true,
        pid: Some(child.id()),
        socket: cfg.socket.clone(),
        error: Some(err),
    })
}

/// Return a live owner: reuse one that already accepts, otherwise detach.
///
/// Spawn failure and an accept timeout come back as [`EnsureResult`] with
/// [`EnsureResult::ok`] false, not as an error.
///
/// # Errors
///
/// Fails when the socket directory or serve log cannot be created, the log
/// handle cannot be cloned, or no `vissue` executable can be resolved.
pub fn ensure_serve(cfg: &ServeConfig) -> Result<EnsureResult> {
    if socket_accepts(&cfg.socket) {
        return Ok(EnsureResult {
            ok: true,
            already_running: true,
            spawned: false,
            pid: read_pid_file(&cfg.socket),
            socket: cfg.socket.clone(),
            error: None,
        });
    }
    start_detached(cfg)
}

#[cfg(test)]
mod tests {
    use super::super::owner::write_pid_file;
    use super::*;
    use crate::Action;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use vissue_core::config::Layout;

    fn cfg(dir: &Path) -> ServeConfig {
        ServeConfig {
            layout: Layout::new(dir.join("vault"), "Software"),
            socket: dir.join("run/control.sock"),
            exe: None,
        }
    }

    #[test]
    fn missing_socket_does_not_accept() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!socket_accepts(&dir.path().join("nope.sock")));
    }

    #[test]
    fn bound_socket_accepts() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("control.sock");
        let _listener = UnixListener::bind(&socket).unwrap();
        assert!(socket_accepts(&socket));
    }

    #[test]
    fn status_when_down_exits_1() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cfg(dir.path());
        let snap = status(&cfg);
        assert!(!snap.live);
        assert_eq!(snap.revision, SERVE_REVISION);
        assert_eq!(snap.prefix, "Software");
        assert_eq!(
            crate::invoke(Action::Status { json: false }, &cfg).unwrap(),
            1
        );
    }

    #[test]
    fn stop_when_already_stopped_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cfg(dir.path());
        prepare_socket_dir(&cfg.socket).unwrap();
        assert_eq!(stop(&cfg).unwrap(), 0);
        assert!(
            vissue_control::socket_lock_path(&cfg.socket).exists() || lock_is_free(&cfg.socket)
        );
    }

    #[test]
    fn stop_unlinks_stale_socket_only_when_lock_is_free() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cfg(dir.path());
        prepare_socket_dir(&cfg.socket).unwrap();
        let listener = UnixListener::bind(&cfg.socket).unwrap();
        drop(listener);
        assert!(cfg.socket.exists());
        assert_eq!(stop(&cfg).unwrap(), 0);
        assert!(!cfg.socket.exists());
        assert!(
            vissue_control::socket_lock_path(&cfg.socket).exists(),
            "lock file must not be unlinked"
        );
    }

    #[test]
    fn stop_does_not_unlink_a_live_socket_without_pid() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cfg(dir.path());
        prepare_socket_dir(&cfg.socket).unwrap();
        let _listener = UnixListener::bind(&cfg.socket).unwrap();
        assert_eq!(stop(&cfg).unwrap(), 1);
        assert!(cfg.socket.exists());
    }

    #[test]
    fn stop_does_not_signal_a_reused_pid_when_lock_is_free() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cfg(dir.path());
        prepare_socket_dir(&cfg.socket).unwrap();
        let us = std::process::id();
        write_pid_file(&cfg.socket, us).unwrap();
        assert!(lock_is_free(&cfg.socket));
        assert_eq!(stop(&cfg).unwrap(), 0);
        assert!(
            pid_is_alive(us),
            "stop must not signal a pid-file number when the lock is free"
        );
        assert!(read_pid_file(&cfg.socket).is_none());
    }

    #[test]
    fn stop_refuses_a_live_socket_when_lock_is_free() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cfg(dir.path());
        prepare_socket_dir(&cfg.socket).unwrap();
        let _listener = UnixListener::bind(&cfg.socket).unwrap();
        let us = std::process::id();
        write_pid_file(&cfg.socket, us).unwrap();
        assert!(lock_is_free(&cfg.socket));
        assert_eq!(stop(&cfg).unwrap(), 1);
        assert!(pid_is_alive(us));
        assert!(cfg.socket.exists());
        assert_eq!(read_pid_file(&cfg.socket), Some(us));
    }

    #[test]
    fn detach_reports_already_running_when_socket_accepts() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cfg(dir.path());
        prepare_socket_dir(&cfg.socket).unwrap();
        let _listener = UnixListener::bind(&cfg.socket).unwrap();
        write_pid_file(&cfg.socket, 99).unwrap();
        let result = start_detached(&cfg).unwrap();
        assert!(result.ok);
        assert!(result.already_running);
        assert!(!result.spawned);
        assert_eq!(result.pid, Some(99));
        let ensured = ensure_serve(&cfg).unwrap();
        assert!(ensured.already_running);
        assert!(ensured.live());
    }

    #[test]
    fn serve_exe_from_uses_current_when_named_vissue() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("vissue");
        fs::write(&current, b"").unwrap();
        assert_eq!(serve_exe_from(&current, None).unwrap(), current);
    }

    #[test]
    fn serve_exe_from_prefers_sibling_when_running_as_hud() {
        let dir = tempfile::tempdir().unwrap();
        let hud = dir.path().join("vissue-hud");
        let cli = dir.path().join("vissue");
        fs::write(&hud, b"").unwrap();
        fs::write(&cli, b"").unwrap();
        assert_eq!(serve_exe_from(&hud, None).unwrap(), cli);
    }

    #[test]
    fn serve_exe_from_searches_path_when_no_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let hud = dir.path().join("vissue-hud");
        fs::write(&hud, b"").unwrap();
        let bindir = dir.path().join("bin");
        fs::create_dir(&bindir).unwrap();
        let cli = bindir.join("vissue");
        fs::write(&cli, b"").unwrap();
        let found = serve_exe_from(&hud, Some(bindir.as_os_str())).unwrap();
        assert_eq!(found, cli);
    }

    #[test]
    fn serve_exe_from_errors_when_hud_has_no_cli() {
        let dir = tempfile::tempdir().unwrap();
        let hud = dir.path().join("vissue-hud");
        fs::write(&hud, b"").unwrap();
        let err = serve_exe_from(&hud, None).unwrap_err().to_string();
        assert!(err.contains("cannot find a vissue CLI"), "{err}");
    }

    #[test]
    fn detach_missing_exe_fails_without_binding() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = cfg(dir.path());
        cfg.exe = Some(dir.path().join("no-such-vissue"));
        let result = start_detached(&cfg).unwrap();
        assert!(!result.ok);
        assert!(result.error.as_deref().unwrap().contains("spawn failed"));
        assert!(!cfg.socket.exists());
    }

    /// `status` against a live owner reports what the owner says, not what
    /// the caller guessed.
    ///
    /// The unenriched snapshot carries the caller's own layout and a
    /// placeholder revision; the fields that matter come back over the
    /// socket, so an owner serving a different root has to show through.
    #[test]
    fn status_against_a_live_owner_reports_the_owners_view() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path().join("vault"), "Software");
        std::fs::create_dir_all(layout.projects_dir()).unwrap();
        let cfg = ServeConfig {
            layout: layout.clone(),
            socket: dir.path().join("control.sock"),
            exe: None,
        };
        let owner = crate::OwnerHandle::spawn(ServeConfig {
            layout,
            socket: cfg.socket.clone(),
            exe: None,
        })
        .unwrap();

        let snap = status(&cfg);
        assert!(snap.live, "{snap:?}");
        assert_eq!(snap.prefix, "Software");
        // A running owner starts at 1; SERVE_REVISION is the no-owner value.
        assert!(
            snap.revision > crate::SERVE_REVISION,
            "revision not enriched: {snap:?}"
        );
        assert_eq!(snap.socket, cfg.socket);
        drop(owner);
    }

    /// A socket that accepts but answers nothing leaves the snapshot alone.
    ///
    /// `enrich_from_owner` talks to whatever is listening. Something that is
    /// not a vissue owner must not take the status call down with it.
    #[test]
    fn status_survives_a_socket_that_is_not_an_owner() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("mute.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        // Accept and immediately drop, so initialize never gets an answer.
        let mute = std::thread::spawn(move || {
            for stream in listener.incoming().take(1) {
                drop(stream);
            }
        });

        let cfg = ServeConfig {
            layout: Layout::new(dir.path().join("vault"), "Software"),
            socket: socket.clone(),
            exe: None,
        };
        let snap = status(&cfg);
        assert!(snap.live, "the socket does accept");
        assert_eq!(
            snap.revision,
            crate::SERVE_REVISION,
            "nothing answered, so nothing should have been enriched"
        );
        let _ = mute.join();
    }

    /// A stand-in for the serve binary, invoked exactly as the detach does.
    ///
    /// The detach execs `<exe> serve --foreground --root R --prefix P
    /// --socket S`, so the script finds the socket in its own arguments and
    /// then behaves as the case under test requires.
    fn fake_serve(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("fake-serve");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\n\
                 sock=\"\"\n\
                 while [ $# -gt 0 ]; do\n\
                 \tif [ \"$1\" = \"--socket\" ]; then sock=\"$2\"; fi\n\
                 \tshift\n\
                 done\n\
                 {body}\n"
            ),
        )
        .unwrap();
        let mut perm = std::fs::metadata(&path).unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&path, perm).unwrap();
        path
    }

    #[test]
    fn detach_reports_the_child_once_it_accepts() {
        let dir = tempfile::tempdir().unwrap();
        // A separate file rather than an inline -c program, so the shell and
        // Rust are not both escaping the same source.
        let binder = dir.path().join("bind.py");
        // A raw string with real newlines: a continuation escape leaves the
        // indentation in, and Python will not have it.
        std::fs::write(
            &binder,
            r#"import socket, sys, time
s = socket.socket(socket.AF_UNIX)
s.bind(sys.argv[1])
s.listen(8)
time.sleep(60)
"#,
        )
        .unwrap();
        let exe = fake_serve(
            dir.path(),
            &format!("exec python3 {} \"$sock\"", binder.display()),
        );
        let mut cfg = cfg(dir.path());
        cfg.exe = Some(exe);

        let result = start_detached(&cfg).unwrap();
        assert!(result.ok, "{:?}", result.error);
        assert!(result.spawned, "{result:?}");
        assert!(!result.already_running, "{result:?}");
        let pid = result.pid.expect("a pid");
        assert!(socket_accepts(&cfg.socket), "the socket does not accept");

        // The detach only returns once the socket answers, so a caller may
        // connect straight away rather than polling for it.
        assert!(pid_is_alive(pid), "the child was gone before it was used");

        // Cleanup only. Nothing reaps the child in this process, so it stays
        // a visible zombie after the signal and there is nothing to wait for.
        let _ = signal_pid(pid, Signal::SIGTERM);
    }

    #[test]
    fn a_live_socket_is_reported_rather_than_started_again() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cfg(dir.path());
        std::fs::create_dir_all(cfg.socket.parent().unwrap()).unwrap();
        let _listener = UnixListener::bind(&cfg.socket).unwrap();

        for result in [start_detached(&cfg).unwrap(), ensure_serve(&cfg).unwrap()] {
            assert!(result.ok, "{result:?}");
            assert!(result.already_running, "{result:?}");
            assert!(!result.spawned, "a live socket was served twice");
        }
    }

    /// A child that starts and never accepts is reported with its own output.
    ///
    /// The log tail is the only account of why: the child writes to a file
    /// rather than to the caller's terminal, so without it the failure is
    /// "did not accept" and nothing else.
    #[test]
    fn a_child_that_never_accepts_is_reported_with_the_log_tail() {
        let dir = tempfile::tempdir().unwrap();
        let exe = fake_serve(
            dir.path(),
            "echo 'refusing: the vault is on fire' >&2\nexit 3",
        );
        let mut cfg = cfg(dir.path());
        cfg.exe = Some(exe);

        let result = start_detached(&cfg).unwrap();
        assert!(!result.ok, "{result:?}");
        assert!(result.spawned, "the child was started: {result:?}");
        let err = result.error.expect("a reason");
        assert!(err.contains("did not accept"), "{err}");
        assert!(
            err.contains("the vault is on fire"),
            "the child's own account was dropped: {err}"
        );
    }

    #[test]
    fn pid_zero_is_not_alive() {
        assert!(!pid_is_alive(0));
    }

    #[test]
    fn this_process_is_alive() {
        assert!(pid_is_alive(std::process::id()));
    }

    #[test]
    fn print_status_json_mentions_live() {
        let status = Status {
            live: false,
            pid: None,
            socket: "/tmp/control.sock".into(),
            root: "/tmp/vault".into(),
            prefix: "Software".into(),
            generation: 0,
            revision: 0,
            clients: 0,
        };
        print_status(&status, true).unwrap();
    }

    #[test]
    fn socket_dir_mode_is_0700_after_prepare() {
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
}
