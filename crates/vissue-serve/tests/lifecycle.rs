//! Crash, restart and handover of the socket owner, across real processes.
//!
//! Each owner is this test binary run again as [`owner_process`], which calls
//! the same `invoke(Action::Foreground)` that `vissue serve --foreground` does.
//! It has to be another process: only a SIGKILL leaves the socket file behind,
//! and only another process can hold the lock against this one.

#![cfg(unix)]
#![allow(missing_docs)]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use serde_json::json;
use vissue_control::Error;
use vissue_control::client::Client;
use vissue_core::config::Layout;
use vissue_serve::{Action, EnsureResult, ServeConfig, accept_timeout, ensure_serve, invoke};

const ROOT_ENV: &str = "VISSUE_LIFECYCLE_ROOT";
const PREFIX_ENV: &str = "VISSUE_LIFECYCLE_PREFIX";
const SOCKET_ENV: &str = "VISSUE_LIFECYCLE_SOCKET";

/// Where each owner process appends its pid as it starts.
fn owners_log(socket: &Path) -> PathBuf {
    vissue_control::beside_socket(socket, "owners")
}

/// The owner body. It does nothing unless another test started it.
#[test]
#[ignore = "an owner process the other tests start"]
fn owner_process() {
    let var = |name| std::env::var_os(name).map(PathBuf::from);
    let (Some(root), Some(prefix), Some(socket)) = (
        var(ROOT_ENV),
        std::env::var(PREFIX_ENV).ok(),
        var(SOCKET_ENV),
    ) else {
        return;
    };
    // One write, so two owners starting at once cannot interleave a line.
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(owners_log(&socket))
        .and_then(|mut log| log.write_all(format!("{}\n", std::process::id()).as_bytes()))
        .unwrap();
    let cfg = ServeConfig {
        layout: Layout::new(root, prefix),
        socket,
        exe: None,
    };
    let code = invoke(Action::Foreground, &cfg).unwrap_or_else(|err| {
        eprintln!("vissue serve: {err}");
        1
    });
    std::process::exit(code);
}

fn copy_dir(src: &Path, dest: &Path) {
    fs::create_dir_all(dest).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dest.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            fs::copy(entry.path(), to).unwrap();
        }
    }
}

/// What the detach spawns: `serve --foreground --root R --prefix P --socket S`
/// turned into a run of [`owner_process`].
///
/// The script waits on the owner rather than exec'ing it. The owner's parent
/// is then the script, which reaps it the moment it dies; were it this test
/// process, a killed owner would stay a zombie that still answers `kill -0`.
///
/// One script serves every test, written before any of them spawns. A file
/// still open for writing in one thread while another forks cannot be
/// executed (`ETXTBSY`), so a script per test races the other tests' spawns.
/// It is renamed into place so a concurrent run of this binary only ever
/// executes a complete copy.
fn serve_exe() -> &'static Path {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| {
        let test_bin = std::env::current_exe().unwrap();
        let name = test_bin.file_name().unwrap().to_string_lossy();
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
        let path = dir.join(format!("{name}-serve"));
        let draft = dir.join(format!("{name}-serve.{}", std::process::id()));
        fs::write(
            &draft,
            format!(
                "#!/bin/sh\n\
                 while [ $# -gt 0 ]; do\n\
                 \tcase \"$1\" in\n\
                 \t--root) root=\"$2\"; shift ;;\n\
                 \t--prefix) prefix=\"$2\"; shift ;;\n\
                 \t--socket) socket=\"$2\"; shift ;;\n\
                 \tesac\n\
                 \tshift\n\
                 done\n\
                 {ROOT_ENV}=\"$root\" {PREFIX_ENV}=\"$prefix\" {SOCKET_ENV}=\"$socket\" \
                 '{bin}' owner_process --exact --ignored --quiet\n\
                 exit $?\n",
                bin = test_bin.display(),
            ),
        )
        .unwrap();
        let mut perm = fs::metadata(&draft).unwrap().permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&draft, perm).unwrap();
        fs::rename(&draft, &path).unwrap();
        path
    })
}

struct Harness {
    _tmp: tempfile::TempDir,
    cfg: ServeConfig,
}

impl Harness {
    fn new() -> Self {
        let exe = serve_exe().to_path_buf();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("vault");
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixture_vault"),
            &root,
        );
        let cfg = ServeConfig {
            layout: Layout::new(&root, "Software"),
            socket: tmp.path().join("run/control.sock"),
            exe: Some(exe),
        };
        Self { _tmp: tmp, cfg }
    }

    fn start(&self) -> EnsureResult {
        let result = ensure_serve(&self.cfg).unwrap();
        assert!(result.ok, "start failed: {:?}", result.error);
        result
    }

    fn connect(&self) -> Client {
        let mut client = Client::connect(&self.cfg.socket).unwrap();
        client
            .request(
                "initialize",
                json!({"protocolVersion": 1, "client": "lifecycle", "agent": "lifecycle"}),
            )
            .unwrap();
        client
    }

    /// Every owner process that has started, in order, by its own account.
    fn started(&self) -> Vec<u32> {
        fs::read_to_string(owners_log(&self.cfg.socket))
            .unwrap_or_default()
            .lines()
            .map(|l| l.parse().unwrap())
            .collect()
    }

    fn newest_owner(&self) -> u32 {
        *self.started().last().expect("no owner has started")
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = invoke(Action::Stop, &self.cfg);
    }
}

fn alive(pid: u32) -> bool {
    !matches!(kill(Pid::from_raw(pid as i32), None), Err(Errno::ESRCH))
}

/// How often [`wait_for`] looks: a fifth of the 50 ms the detach polls at, so
/// a test is never waiting on its own poll. The deadline, not this, fails it.
const POLL: Duration = Duration::from_millis(10);

/// Poll `done` for as long as a fresh owner is given to start, which is also
/// ample for one to die or a loser to give up.
fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + accept_timeout();
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(POLL);
    }
}

/// SIGKILL `pid`, so nothing of the owner runs: no drop, no unlink.
fn crash(pid: u32) {
    kill(Pid::from_raw(pid as i32), Signal::SIGKILL).unwrap();
    wait_for("the killed owner to be gone", || !alive(pid));
}

#[test]
fn a_start_after_a_sigkill_reclaims_the_stale_socket() {
    let h = Harness::new();
    h.start();
    let killed = h.newest_owner();
    crash(killed);

    // What the crash left: the path is still there and nothing answers on it.
    assert!(h.cfg.socket.exists(), "SIGKILL ran no cleanup");
    assert!(!vissue_serve::socket_accepts(&h.cfg.socket));
    assert_eq!(invoke(Action::Status { json: false }, &h.cfg).unwrap(), 1);

    // No `serve stop` and no unlink first.
    let next = h.start();
    assert!(next.spawned && !next.already_running, "{next:?}");
    let pid = h.newest_owner();
    assert_ne!(pid, killed);
    assert!(alive(pid));
    let listed = h.connect().request("issue/list", json!({})).unwrap();
    assert!(!listed["issues"].as_array().unwrap().is_empty(), "{listed}");

    assert_eq!(invoke(Action::Stop, &h.cfg).unwrap(), 0);
    assert!(!h.cfg.socket.exists());
}

#[test]
fn a_client_from_before_the_crash_errors_then_reconnects() {
    let h = Harness::new();
    h.start();
    let killed = h.newest_owner();
    let mut held = h.connect();
    let made = held
        .request(
            "issue/create",
            json!({"project": "atlas", "title": "written before the crash"}),
        )
        .unwrap();
    let id = made["issue"]["id"].as_str().unwrap().to_string();
    crash(killed);

    // On a thread, so a request that hangs fails the test instead of stalling it.
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(held.request("issue/list", json!({})));
    });
    let err = rx
        .recv_timeout(accept_timeout())
        .expect("the held connection hung after the owner died")
        .expect_err("a dead owner answered");
    // A transport failure, which a client can tell from any reply an owner sends.
    assert!(
        matches!(err, Error::Io(_) | Error::Frame(_)),
        "not a transport error: {err:?}"
    );

    h.start();
    let mut again = h.connect();
    let got = again.request("issue/get", json!({"id": id})).unwrap();
    assert_eq!(got["id"], id.as_str(), "a crash lost a write: {got}");
}

#[test]
fn restart_after_a_sigkill_brings_up_a_new_owner() {
    let h = Harness::new();
    h.start();
    let killed = h.newest_owner();
    crash(killed);

    assert_eq!(invoke(Action::Restart, &h.cfg).unwrap(), 0);
    let now = h.start();
    assert!(now.already_running, "restart left nothing running: {now:?}");
    let pid = h.newest_owner();
    assert_ne!(pid, killed);
    assert!(alive(pid));
    h.connect();
}

#[test]
fn two_starts_at_once_leave_one_owner() {
    let h = Harness::new();
    // From what a crash leaves, so both also race to reclaim the stale socket.
    h.start();
    crash(h.newest_owner());

    let gate = Arc::new(Barrier::new(2));
    let results: Vec<EnsureResult> = thread::scope(|s| {
        let racers: Vec<_> = (0..2)
            .map(|_| {
                let gate = Arc::clone(&gate);
                let cfg = &h.cfg;
                s.spawn(move || {
                    gate.wait();
                    ensure_serve(cfg).unwrap()
                })
            })
            .collect();
        racers.into_iter().map(|r| r.join().unwrap()).collect()
    });
    for result in &results {
        assert!(result.ok, "a racer got no owner: {result:?}");
    }

    // One owner process per spawn, after the one that was killed.
    let spawned = results.iter().filter(|r| r.spawned).count();
    wait_for("every spawned owner to start", || {
        h.started().len() == 1 + spawned
    });
    let racers: Vec<u32> = h.started().into_iter().skip(1).collect();
    wait_for("the losing owner to give up", || {
        racers.iter().filter(|p| alive(**p)).count() <= 1
    });
    let survivors: Vec<u32> = racers.into_iter().filter(|p| alive(*p)).collect();
    assert_eq!(survivors.len(), 1, "owners: {survivors:?}");
    h.connect();

    // Stop signals the pid the pid file names, so this also shows that file
    // names the one owner left.
    assert_eq!(invoke(Action::Stop, &h.cfg).unwrap(), 0);
    wait_for("the owner to stop", || !alive(survivors[0]));
}
