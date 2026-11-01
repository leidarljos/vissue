//! Attach story: `--offline` never connects; first list after init drops since.

#![allow(missing_docs)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use vissue_core::config::Layout;
use vissue_core::views::ListQuery;
use vissue_serve::ServeConfig;
use vissue_tui::attach::{AttachFail, AttachHooks, AttachOutcome, ServeStatus, try_attach};
use vissue_tui::backend::{BoardBackend, ListPage, MutResult, UpdateReq};
use vissue_tui::{BackendKind, SinceGate};

struct CountingHooks {
    probes: AtomicUsize,
    ensures: AtomicUsize,
    connects: AtomicUsize,
}

impl CountingHooks {
    fn new() -> Self {
        Self {
            probes: AtomicUsize::new(0),
            ensures: AtomicUsize::new(0),
            connects: AtomicUsize::new(0),
        }
    }
}

static COUNTS: Mutex<Option<CountingHooks>> = Mutex::new(None);

fn bump_probe(_: &Path) -> bool {
    COUNTS
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .probes
        .fetch_add(1, Ordering::SeqCst);
    panic!("offline must not probe");
}

fn bump_ensure(_: &ServeConfig) -> Result<vissue_serve::EnsureResult, String> {
    COUNTS
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .ensures
        .fetch_add(1, Ordering::SeqCst);
    panic!("offline must not ensure");
}

fn bump_connect(_: &Path, _: &Layout, _: &str) -> Result<Box<dyn BoardBackend>, AttachFail> {
    COUNTS
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .connects
        .fetch_add(1, Ordering::SeqCst);
    panic!("offline must not connect");
}

#[test]
fn offline_never_touches_the_socket() {
    *COUNTS.lock().unwrap() = Some(CountingHooks::new());
    let layout = Layout::new("/tmp/vissue-tui-offline", "Software");
    let hooks = AttachHooks {
        probe: bump_probe,
        ensure: bump_ensure,
        connect: bump_connect,
    };
    let outcome = try_attach(
        &layout,
        &PathBuf::from("/tmp/does-not-exist.sock"),
        "agent",
        true,
        &hooks,
    );
    match outcome {
        AttachOutcome::Stay {
            status: ServeStatus::Offline,
            ..
        } => {}
        _ => panic!("expected stay offline"),
    }
}

/// Records `since_revision` the way `ControlBackend` does after initialize.
#[derive(Debug)]
struct RecordingBackend {
    gate: SinceGate,
    last: Mutex<Option<Option<u64>>>,
    revision: u64,
    layout: Layout,
}

impl RecordingBackend {
    fn after_initialize(revision: u64) -> Self {
        Self {
            gate: SinceGate::after_attach(),
            last: Mutex::new(None),
            revision,
            layout: Layout::new("/tmp/vissue-tui-record", "Software"),
        }
    }
}

impl BoardBackend for RecordingBackend {
    fn layout(&self) -> &Layout {
        &self.layout
    }
    fn generation(&self) -> u64 {
        3
    }
    fn revision(&self) -> u64 {
        self.revision
    }
    fn live(&self) -> BackendKind {
        BackendKind::Control
    }
    fn identity(&self) -> &str {
        "tui"
    }
    fn list(&self, _q: ListQuery) -> Result<ListPage, vissue_core::error::Error> {
        let since = self.gate.next(self.revision);
        *self.last.lock().unwrap() = Some(since);
        Ok(ListPage {
            revision: self.revision,
            generation: 3,
            ..ListPage::default()
        })
    }
    fn ready(&self, _project: Option<&str>) -> Result<ListPage, vissue_core::error::Error> {
        self.list(ListQuery {
            ready: true,
            ..ListQuery::default()
        })
    }
    fn get(&self, _id: &str) -> Result<vissue_core::views::IssueDetail, vissue_core::error::Error> {
        panic!("RecordingBackend::get is unused in this test")
    }
    fn excerpt(&self, _id: &str) -> Result<vissue_core::views::Excerpt, vissue_core::error::Error> {
        panic!("RecordingBackend::excerpt is unused in this test")
    }
    fn search(
        &self,
        _q: &str,
        _n: usize,
    ) -> Result<Vec<vissue_core::views::SearchHit>, vissue_core::error::Error> {
        panic!("RecordingBackend::search is unused in this test")
    }
    fn claims(
        &self,
        _h: Option<&str>,
        _p: Option<&str>,
    ) -> Result<Vec<vissue_core::views::ClaimRow>, vissue_core::error::Error> {
        panic!("RecordingBackend::claims is unused in this test")
    }
    fn agenda(
        &self,
        _d: i64,
        _p: Option<&str>,
    ) -> Result<Vec<vissue_core::views::AgendaRow>, vissue_core::error::Error> {
        panic!("RecordingBackend::agenda is unused in this test")
    }
    fn tree(&self, _id: &str) -> Result<vissue_core::views::TreeNode, vissue_core::error::Error> {
        panic!("RecordingBackend::tree is unused in this test")
    }
    fn related(
        &self,
        _id: &str,
        _d: usize,
        _n: usize,
    ) -> Result<Vec<vissue_core::views::RelatedHit>, vissue_core::error::Error> {
        panic!("RecordingBackend::related is unused in this test")
    }
    fn recall(
        &self,
        _id: &str,
        _depth: usize,
    ) -> Result<vissue_core::views::Recall, vissue_core::error::Error> {
        panic!("RecordingBackend::recall is unused in this test")
    }
    fn deed(&self, _id: &str, _add: &[String]) -> Result<MutResult, vissue_core::error::Error> {
        panic!("RecordingBackend::deed is unused in this test")
    }
    fn projects(&self) -> Result<Vec<String>, vissue_core::error::Error> {
        Ok(vec![])
    }
    fn claim(&self, _id: &str, _f: bool) -> Result<MutResult, vissue_core::error::Error> {
        panic!("RecordingBackend::claim is unused in this test")
    }
    fn note(&self, _id: &str, _t: &str) -> Result<MutResult, vissue_core::error::Error> {
        panic!("RecordingBackend::note is unused in this test")
    }
    fn update(&self, _r: UpdateReq) -> Result<MutResult, vissue_core::error::Error> {
        panic!("RecordingBackend::update is unused in this test")
    }
    fn create(&self, _project: &str, _title: &str) -> Result<MutResult, vissue_core::error::Error> {
        panic!("RecordingBackend::create is unused in this test")
    }
    fn open(
        &self,
        _id: &str,
    ) -> Result<vissue_core::views::IssueDetail, vissue_core::error::Error> {
        panic!("RecordingBackend::open is unused in this test")
    }
    fn wait(&self, last: u64, _ms: u64) -> Result<u64, vissue_core::error::Error> {
        Ok(last)
    }
    fn last_since_revision(&self) -> Option<Option<u64>> {
        *self.last.lock().unwrap()
    }
}

fn connect_ok(_: &Path, _: &Layout, _: &str) -> Result<Box<dyn BoardBackend>, AttachFail> {
    Ok(Box::new(RecordingBackend::after_initialize(9)))
}

fn probe_yes(_: &Path) -> bool {
    true
}

fn ensure_unused(_: &ServeConfig) -> Result<vissue_serve::EnsureResult, String> {
    panic!("probe already accepted")
}

#[test]
fn accept_then_connect_switches_to_control() {
    let layout = Layout::new("/tmp/vissue-tui-switch", "Software");
    let hooks = AttachHooks {
        probe: probe_yes,
        ensure: ensure_unused,
        connect: connect_ok,
    };
    let outcome = try_attach(
        &layout,
        &PathBuf::from("/tmp/vissue-tui-switch.sock"),
        "agent",
        false,
        &hooks,
    );
    match outcome {
        AttachOutcome::Switch { backend, status } => {
            assert_eq!(status, ServeStatus::Live);
            assert_eq!(backend.live(), BackendKind::Control);
            assert_eq!(backend.revision(), 9);
        }
        _ => panic!("expected switch"),
    }
}

#[test]
fn after_mocked_initialize_next_list_has_no_since_revision() {
    let backend = RecordingBackend::after_initialize(41);
    let page = backend.ready(None).unwrap();
    assert!(!page.unchanged);
    assert_eq!(backend.last_since_revision(), Some(None));
    let _ = backend.list(ListQuery::default()).unwrap();
    assert_eq!(backend.last_since_revision(), Some(Some(41)));
}

// The attach decision, one case per set of hooks.
//
// `try_attach` chooses between three outcomes: talk to a live serve, spawn
// one and talk to it, or stay on the files and say why. Only the offline
// branch was covered, so the branch that decides whether a board writes
// through a socket or to the files directly was taken on trust.
//
// The hooks are plain function pointers, so each case needs its own pair
// rather than a closure over test state.

mod decision {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use vissue_tui::CoreBackend;

    fn scratch(name: &str) -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().expect("tempdir");
        let layout = Layout::new(dir.path(), "Software");
        std::fs::create_dir_all(layout.projects_dir()).expect("projects dir");
        let _ = name;
        (dir, layout)
    }

    fn a_backend(layout: &Layout, agent: &str) -> Box<dyn BoardBackend> {
        Box::new(CoreBackend::open(layout.clone(), agent).expect("core backend"))
    }

    fn socket() -> PathBuf {
        PathBuf::from("/tmp/vissue-attach-decision.sock")
    }

    fn never_ensure(_: &ServeConfig) -> Result<vissue_serve::EnsureResult, String> {
        panic!("a live socket must not be respawned")
    }

    // 1. The socket already answers.
    fn live_probe(_: &Path) -> bool {
        true
    }
    fn ok_connect(
        _: &Path,
        layout: &Layout,
        agent: &str,
    ) -> Result<Box<dyn BoardBackend>, AttachFail> {
        Ok(a_backend(layout, agent))
    }

    #[test]
    fn a_socket_that_answers_is_attached_to() {
        let (_dir, layout) = scratch("live");
        let hooks = AttachHooks {
            probe: live_probe,
            ensure: never_ensure,
            connect: ok_connect,
        };
        let outcome = try_attach(&layout, &socket(), "agent", false, &hooks);
        match outcome {
            AttachOutcome::Switch {
                status: ServeStatus::Live,
                ..
            } => {}
            AttachOutcome::Stay { status, message } => {
                panic!("stayed on the files: {status:?} {message}")
            }
            _ => panic!("attached with the wrong status"),
        }
    }

    // 2. It answers, but it is serving another tracker.
    fn mismatch_connect(
        _: &Path,
        _: &Layout,
        _: &str,
    ) -> Result<Box<dyn BoardBackend>, AttachFail> {
        Err(AttachFail::Mismatch(
            "want /a/Software got /b/Software".into(),
        ))
    }

    #[test]
    fn an_owner_on_another_tracker_leaves_the_board_on_the_files() {
        let (_dir, layout) = scratch("mismatch");
        let hooks = AttachHooks {
            probe: live_probe,
            ensure: never_ensure,
            connect: mismatch_connect,
        };
        let outcome = try_attach(&layout, &socket(), "agent", false, &hooks);
        match outcome {
            AttachOutcome::Stay { status, message } => {
                // Distinct from Offline: the board says which vault it is on,
                // because writing into the wrong one is the harm being avoided.
                assert_eq!(status, ServeStatus::Mismatch, "{message}");
                assert!(message.contains("want"), "{message}");
                assert!(message.contains("got"), "{message}");
            }
            _ => panic!("attached to an owner serving another tracker"),
        }
    }

    // 3. It answers, then refuses for some other reason.
    fn failing_connect(_: &Path, _: &Layout, _: &str) -> Result<Box<dyn BoardBackend>, AttachFail> {
        Err(AttachFail::Other("connection reset".into()))
    }

    #[test]
    fn a_refused_connection_stays_on_the_files_and_says_why() {
        let (_dir, layout) = scratch("refused");
        let hooks = AttachHooks {
            probe: live_probe,
            ensure: never_ensure,
            connect: failing_connect,
        };
        let outcome = try_attach(&layout, &socket(), "agent", false, &hooks);
        match outcome {
            AttachOutcome::Stay { status, message } => {
                assert_eq!(status, ServeStatus::Offline);
                assert!(message.contains("connection reset"), "{message}");
            }
            _ => panic!("attached to a socket that refused"),
        }
    }

    // 4. Nothing is listening, so serve is started and then attached to.
    static SPAWNED: AtomicBool = AtomicBool::new(false);
    static ENSURES: AtomicUsize = AtomicUsize::new(0);

    fn probe_after_spawn(_: &Path) -> bool {
        SPAWNED.load(Ordering::SeqCst)
    }
    fn spawning_ensure(_: &ServeConfig) -> Result<vissue_serve::EnsureResult, String> {
        ENSURES.fetch_add(1, Ordering::SeqCst);
        SPAWNED.store(true, Ordering::SeqCst);
        Ok(vissue_serve::EnsureResult {
            ok: true,
            already_running: false,
            spawned: true,
            pid: Some(4242),
            socket: socket(),
            error: None,
        })
    }

    #[test]
    fn a_free_socket_is_served_first_and_then_attached_to() {
        let (_dir, layout) = scratch("spawn");
        SPAWNED.store(false, Ordering::SeqCst);
        ENSURES.store(0, Ordering::SeqCst);
        let hooks = AttachHooks {
            probe: probe_after_spawn,
            ensure: spawning_ensure,
            connect: ok_connect,
        };
        let outcome = try_attach(&layout, &socket(), "agent", false, &hooks);
        match outcome {
            AttachOutcome::Switch {
                status: ServeStatus::Live,
                ..
            } => {}
            AttachOutcome::Stay { status, message } => {
                panic!("did not attach after spawning: {status:?} {message}")
            }
            _ => panic!("wrong status after a spawn"),
        }
        assert_eq!(ENSURES.load(Ordering::SeqCst), 1, "serve was started twice");
    }

    // 5. Serve reports success but the socket still does not answer.
    fn never_answers(_: &Path) -> bool {
        false
    }
    fn silent_ensure(_: &ServeConfig) -> Result<vissue_serve::EnsureResult, String> {
        Ok(vissue_serve::EnsureResult {
            ok: true,
            already_running: false,
            spawned: true,
            pid: Some(4243),
            socket: socket(),
            error: Some("started but never accepted".into()),
        })
    }

    #[test]
    fn a_serve_that_never_accepts_leaves_the_board_on_the_files() {
        let (_dir, layout) = scratch("silent");
        let hooks = AttachHooks {
            probe: never_answers,
            ensure: silent_ensure,
            connect: ok_connect,
        };
        let outcome = try_attach(&layout, &socket(), "agent", false, &hooks);
        match outcome {
            AttachOutcome::Stay { status, message } => {
                assert_eq!(status, ServeStatus::Offline);
                assert!(message.contains("never accepted"), "{message}");
            }
            _ => panic!("attached to a socket that never answered"),
        }
    }

    // 6. Serve could not be started at all.
    fn failing_ensure(_: &ServeConfig) -> Result<vissue_serve::EnsureResult, String> {
        Err("no such binary".into())
    }

    #[test]
    fn a_serve_that_will_not_start_is_reported_not_hidden() {
        let (_dir, layout) = scratch("nostart");
        let hooks = AttachHooks {
            probe: never_answers,
            ensure: failing_ensure,
            connect: ok_connect,
        };
        let outcome = try_attach(&layout, &socket(), "agent", false, &hooks);
        match outcome {
            AttachOutcome::Stay { status, message } => {
                assert_eq!(status, ServeStatus::Offline);
                assert!(message.contains("no such binary"), "{message}");
            }
            _ => panic!("attached without a serve"),
        }
    }
}
