//! Scripted keys and a first-paint snapshot on a tempfile vault.

#![allow(missing_docs)]

use std::fs;
use std::path::{Path, PathBuf};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use vissue_core::config::{DEFAULT_PREFIX, Layout};
use vissue_tui::view::render_plain;
use vissue_tui::{Action, App, BoardBackend, CoreBackend, ServeStatus};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixture_vault")
}

fn copy_tree(src: &Path, dest: &Path) {
    fs::create_dir_all(dest).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let target = dest.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn writable() -> (tempfile::TempDir, Layout) {
    let dir = tempfile::tempdir().unwrap();
    copy_tree(
        &fixture_root().join(DEFAULT_PREFIX),
        &dir.path().join(DEFAULT_PREFIX),
    );
    let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
    (dir, layout)
}

fn fixture_layout() -> Layout {
    Layout::new(fixture_root(), DEFAULT_PREFIX)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ch(c: char) -> KeyEvent {
    key(KeyCode::Char(c))
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        assert_eq!(app.handle_key(ch(c)), Action::Continue);
    }
}

#[test]
fn first_paint_ready_pane_lists_fixture_ids() {
    let backend = CoreBackend::open(fixture_layout(), "snap").unwrap();
    let app = App::with_backend(Box::new(backend), "snap".into(), ServeStatus::Offline).unwrap();
    let text = render_plain(&app, 100, 24).unwrap();
    assert!(text.contains("Ready"), "{text}");
    assert!(text.contains("atlas-1a2b"), "{text}");
    assert!(text.contains("atlas-2c3d"), "{text}");
    assert!(text.contains("beacon-5j6k"), "{text}");
    assert!(text.contains("serve:offline"), "{text}");
    assert!(text.contains("gen=0"), "{text}");
    assert!(text.contains("rev=0"), "{text}");
    assert!(text.contains("agent=snap"), "{text}");
    assert!(!text.contains("atlas-3e4f"), "{text}");
}

#[test]
fn j_then_enter_shows_the_selected_id() {
    let (_dir, layout) = writable();
    let backend = CoreBackend::open(layout, "tui-test").unwrap();
    let mut app =
        App::with_backend(Box::new(backend), "tui-test".into(), ServeStatus::Offline).unwrap();
    assert_eq!(app.selected_id(), Some("atlas-1a2b"));
    app.handle_key(ch('j'));
    assert_eq!(app.selected_id(), Some("atlas-2c3d"));
    app.handle_key(key(KeyCode::Enter));
    let text = render_plain(&app, 100, 24).unwrap();
    assert!(text.contains("atlas-2c3d"), "{text}");
    assert_eq!(
        app.detail.as_ref().map(|d| d.id.as_str()),
        Some("atlas-2c3d")
    );
}

#[test]
fn c_claims_the_selected_issue() {
    let (_dir, layout) = writable();
    let backend = CoreBackend::open(layout, "tui-test").unwrap();
    let mut app =
        App::with_backend(Box::new(backend), "tui-test".into(), ServeStatus::Offline).unwrap();
    app.handle_key(ch('j'));
    assert_eq!(app.selected_id(), Some("atlas-2c3d"));
    app.handle_key(ch('c'));
    let detail = BoardBackend::get(app.backend(), "atlas-2c3d").unwrap();
    assert_eq!(detail.claimed_by.as_deref(), Some("tui-test"));
    assert_eq!(detail.state, "STARTED");
}

#[test]
fn slash_foo_filters_search() {
    let (_dir, layout) = writable();
    let backend = CoreBackend::open(layout, "tui-test").unwrap();
    let mut app =
        App::with_backend(Box::new(backend), "tui-test".into(), ServeStatus::Offline).unwrap();
    app.handle_key(ch('/'));
    type_text(&mut app, "retry");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.pane, vissue_tui::Pane::Search);
    assert_eq!(
        app.rows.len(),
        1,
        "{:?}",
        app.rows.iter().map(|r| &r.id).collect::<Vec<_>>()
    );
    assert_eq!(app.selected_id(), Some("beacon-5j6k"));
}

#[test]
fn s_cycles_todo_started_blocked_only() {
    let (_dir, layout) = writable();
    let backend = CoreBackend::open(layout, "tui-test").unwrap();
    let mut app =
        App::with_backend(Box::new(backend), "tui-test".into(), ServeStatus::Offline).unwrap();
    app.handle_key(ch('2'));
    // List is priority, state, id: A STARTED, A BLOCKED, then B TODO atlas-2c3d.
    app.handle_key(ch('j'));
    app.handle_key(ch('j'));
    assert_eq!(app.selected_id(), Some("atlas-2c3d"));
    assert_eq!(app.selected_state(), Some("TODO"));
    app.handle_key(ch('s'));
    assert_eq!(
        BoardBackend::get(app.backend(), "atlas-2c3d")
            .unwrap()
            .state,
        "STARTED"
    );
    app.handle_key(ch('s'));
    assert_eq!(
        BoardBackend::get(app.backend(), "atlas-2c3d")
            .unwrap()
            .state,
        "BLOCKED"
    );
    app.handle_key(ch('s'));
    assert_eq!(
        BoardBackend::get(app.backend(), "atlas-2c3d")
            .unwrap()
            .state,
        "TODO"
    );
    app.handle_key(ch('s'));
    assert_eq!(
        BoardBackend::get(app.backend(), "atlas-2c3d")
            .unwrap()
            .state,
        "STARTED"
    );
}

#[test]
fn d_and_x_need_confirm() {
    let (_dir, layout) = writable();
    let backend = CoreBackend::open(layout, "tui-test").unwrap();
    let mut app =
        App::with_backend(Box::new(backend), "tui-test".into(), ServeStatus::Offline).unwrap();
    app.handle_key(ch('2'));
    app.handle_key(ch('j'));
    app.handle_key(ch('j'));
    let id = app.selected_id().unwrap().to_string();
    assert_eq!(id, "atlas-2c3d");
    assert_eq!(BoardBackend::get(app.backend(), &id).unwrap().state, "TODO");

    app.handle_key(ch('D'));
    assert_eq!(BoardBackend::get(app.backend(), &id).unwrap().state, "TODO");
    app.handle_key(ch('n'));
    assert_eq!(BoardBackend::get(app.backend(), &id).unwrap().state, "TODO");

    app.handle_key(ch('X'));
    assert_eq!(BoardBackend::get(app.backend(), &id).unwrap().state, "TODO");
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(BoardBackend::get(app.backend(), &id).unwrap().state, "TODO");

    app.handle_key(ch('D'));
    app.handle_key(ch('y'));
    assert_eq!(BoardBackend::get(app.backend(), &id).unwrap().state, "DONE");
}

#[test]
fn y_copies_id_and_note_writes() {
    let (_dir, layout) = writable();
    let backend = CoreBackend::open(layout, "tui-test").unwrap();
    let mut app =
        App::with_backend(Box::new(backend), "tui-test".into(), ServeStatus::Offline).unwrap();
    app.handle_key(ch('y'));
    assert_eq!(app.clipboard, "atlas-1a2b");
    app.handle_key(ch('n'));
    type_text(&mut app, "progress");
    app.handle_key(key(KeyCode::Enter));
    let excerpt = BoardBackend::excerpt(app.backend(), "atlas-1a2b").unwrap();
    assert!(excerpt.text.contains("progress"), "{}", excerpt.text);
}

#[test]
fn project_filter_and_panes() {
    let (_dir, layout) = writable();
    let backend = CoreBackend::open(layout, "tui-test").unwrap();
    let mut app =
        App::with_backend(Box::new(backend), "tui-test".into(), ServeStatus::Offline).unwrap();
    app.handle_key(ch('p'));
    type_text(&mut app, "beacon");
    app.handle_key(key(KeyCode::Enter));
    assert!(app.rows.iter().all(|r| r.project == "beacon"));
    assert!(app.status_line().contains("project=beacon"));
    app.handle_key(ch('3'));
    assert_eq!(app.pane, vissue_tui::Pane::Claims);
    app.handle_key(ch('4'));
    assert_eq!(app.pane, vissue_tui::Pane::Agenda);
    app.handle_key(ch('?'));
    let text = render_plain(&app, 100, 24).unwrap();
    assert!(text.contains("cycle TODO"), "{text}");
    app.handle_key(key(KeyCode::Esc));
    assert!(!app.help);
    assert_eq!(app.handle_key(ch('q')), Action::Quit);
}

#[test]
fn detail_tabs_open_reload_and_prompt_escape() {
    let (_dir, layout) = writable();
    let backend = CoreBackend::open(layout, "tui-test").unwrap();
    let mut app =
        App::with_backend(Box::new(backend), "tui-test".into(), ServeStatus::Offline).unwrap();
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.focus, vissue_tui::Focus::Detail);
    assert!(
        app.detail_body.contains("atlas-1a2b"),
        "{}",
        app.detail_body
    );
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.detail_tab, vissue_tui::DetailTab::Excerpt);
    assert!(
        app.detail_body.contains("body lives in file"),
        "{}",
        app.detail_body
    );
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.detail_tab, vissue_tui::DetailTab::Tree);
    assert!(
        app.detail_body.contains("atlas-1a2b"),
        "{}",
        app.detail_body
    );
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.detail_tab, vissue_tui::DetailTab::Related);
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.detail_tab, vissue_tui::DetailTab::Recall);
    assert!(
        !app.detail_body.is_empty(),
        "the working set pane says what the node stands on, or that it stands on nothing"
    );
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.detail_tab, vissue_tui::DetailTab::Show);
    app.handle_key(ch('o'));
    assert!(app.status_line().contains("opened atlas-1a2b"));
    app.handle_key(ch('R'));
    assert!(app.status_line().contains("reloaded"));
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.focus, vissue_tui::Focus::Rows);
    app.handle_key(ch('n'));
    app.handle_key(key(KeyCode::Backspace));
    app.handle_key(ch('x'));
    app.handle_key(key(KeyCode::Esc));
    assert!(app.prompt.is_none());
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.pane, vissue_tui::Pane::List);
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Up));
    app.poll_updates();
    app.attach(
        Path::new("/tmp/vissue-tui-never.sock"),
        true,
        &vissue_tui::AttachHooks {
            probe: |_| panic!("offline"),
            ensure: |_| panic!("offline"),
            connect: |_, _, _| panic!("offline"),
        },
    )
    .unwrap();
    assert_eq!(app.serve_status(), ServeStatus::Offline);
}

#[test]
fn s_ignores_closed_states() {
    let (_dir, layout) = writable();
    let backend = CoreBackend::open(layout, "tui-test").unwrap();
    let mut app =
        App::with_backend(Box::new(backend), "tui-test".into(), ServeStatus::Offline).unwrap();
    app.handle_key(ch('2'));
    // last rows are C DONE / C CANCELLED
    for _ in 0..10 {
        app.handle_key(ch('j'));
    }
    let id = app.selected_id().unwrap().to_string();
    let before = BoardBackend::get(app.backend(), &id).unwrap().state;
    assert!(before == "DONE" || before == "CANCELLED", "{before}");
    app.handle_key(ch('s'));
    assert_eq!(BoardBackend::get(app.backend(), &id).unwrap().state, before);
}

#[test]
fn empty_search_and_project_clear() {
    let (_dir, layout) = writable();
    let backend = CoreBackend::open(layout, "tui-test").unwrap();
    let mut app =
        App::with_backend(Box::new(backend), "tui-test".into(), ServeStatus::Offline).unwrap();
    app.handle_key(ch('5'));
    assert!(app.rows.is_empty());
    app.handle_key(ch('p'));
    type_text(&mut app, "atlas");
    app.handle_key(key(KeyCode::Enter));
    app.handle_key(ch('1'));
    assert!(app.rows.iter().all(|r| r.project == "atlas"));
    app.handle_key(ch('p'));
    for _ in 0..8 {
        app.handle_key(key(KeyCode::Backspace));
    }
    app.handle_key(key(KeyCode::Enter));
    assert!(app.project.is_none());
}

/// Second Ready/List fetch returns `{unchanged: true, issues: []}`, like
/// serve when `since_revision` matches the catalog head.
#[derive(Debug)]
struct UnchangedAfterFirst {
    inner: CoreBackend,
    ready_calls: std::sync::atomic::AtomicUsize,
    list_calls: std::sync::atomic::AtomicUsize,
}

impl UnchangedAfterFirst {
    fn new(inner: CoreBackend) -> Self {
        Self {
            inner,
            ready_calls: std::sync::atomic::AtomicUsize::new(0),
            list_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl BoardBackend for UnchangedAfterFirst {
    fn layout(&self) -> &vissue_core::config::Layout {
        self.inner.layout()
    }
    fn generation(&self) -> u64 {
        self.inner.generation()
    }
    fn revision(&self) -> u64 {
        5
    }
    fn live(&self) -> vissue_tui::BackendKind {
        vissue_tui::BackendKind::Control
    }
    fn identity(&self) -> &str {
        self.inner.identity()
    }
    fn list(
        &self,
        q: vissue_core::views::ListQuery,
    ) -> Result<vissue_tui::ListPage, vissue_core::error::Error> {
        let n = self
            .list_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == 0 {
            self.inner.list(q)
        } else {
            Ok(vissue_tui::ListPage {
                unchanged: true,
                revision: 5,
                ..vissue_tui::ListPage::default()
            })
        }
    }
    fn ready(
        &self,
        project: Option<&str>,
    ) -> Result<vissue_tui::ListPage, vissue_core::error::Error> {
        let n = self
            .ready_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == 0 {
            self.inner.ready(project)
        } else {
            Ok(vissue_tui::ListPage {
                unchanged: true,
                revision: 5,
                ..vissue_tui::ListPage::default()
            })
        }
    }
    fn get(&self, id: &str) -> Result<vissue_core::views::IssueDetail, vissue_core::error::Error> {
        self.inner.get(id)
    }
    fn excerpt(&self, id: &str) -> Result<vissue_core::views::Excerpt, vissue_core::error::Error> {
        self.inner.excerpt(id)
    }
    fn search(
        &self,
        q: &str,
        n: usize,
    ) -> Result<Vec<vissue_core::views::SearchHit>, vissue_core::error::Error> {
        self.inner.search(q, n)
    }
    fn claims(
        &self,
        h: Option<&str>,
        p: Option<&str>,
    ) -> Result<Vec<vissue_core::views::ClaimRow>, vissue_core::error::Error> {
        self.inner.claims(h, p)
    }
    fn agenda(
        &self,
        d: i64,
        p: Option<&str>,
    ) -> Result<Vec<vissue_core::views::AgendaRow>, vissue_core::error::Error> {
        self.inner.agenda(d, p)
    }
    fn tree(&self, id: &str) -> Result<vissue_core::views::TreeNode, vissue_core::error::Error> {
        self.inner.tree(id)
    }
    fn related(
        &self,
        id: &str,
        d: usize,
        n: usize,
    ) -> Result<Vec<vissue_core::views::RelatedHit>, vissue_core::error::Error> {
        self.inner.related(id, d, n)
    }
    fn recall(
        &self,
        id: &str,
        depth: usize,
    ) -> Result<vissue_core::views::Recall, vissue_core::error::Error> {
        self.inner.recall(id, depth)
    }
    fn deed(
        &self,
        id: &str,
        add: &[String],
    ) -> Result<vissue_tui::MutResult, vissue_core::error::Error> {
        self.inner.deed(id, add)
    }
    fn projects(&self) -> Result<Vec<String>, vissue_core::error::Error> {
        self.inner.projects()
    }
    fn claim(&self, id: &str, f: bool) -> Result<vissue_tui::MutResult, vissue_core::error::Error> {
        self.inner.claim(id, f)
    }
    fn note(&self, id: &str, t: &str) -> Result<vissue_tui::MutResult, vissue_core::error::Error> {
        self.inner.note(id, t)
    }
    fn update(
        &self,
        r: vissue_tui::UpdateReq,
    ) -> Result<vissue_tui::MutResult, vissue_core::error::Error> {
        self.inner.update(r)
    }
    fn create(
        &self,
        project: &str,
        title: &str,
    ) -> Result<vissue_tui::MutResult, vissue_core::error::Error> {
        self.inner.create(project, title)
    }
    fn open(&self, id: &str) -> Result<vissue_core::views::IssueDetail, vissue_core::error::Error> {
        self.inner.open(id)
    }
    fn wait(&self, last: u64, ms: u64) -> Result<u64, vissue_core::error::Error> {
        self.inner.wait(last, ms)
    }
}

#[test]
fn unchanged_list_does_not_wipe_rows() {
    let backend = UnchangedAfterFirst::new(CoreBackend::open(fixture_layout(), "snap").unwrap());
    let mut app = App::with_backend(Box::new(backend), "snap".into(), ServeStatus::Live).unwrap();
    let first: Vec<_> = app.rows.iter().map(|r| r.id.clone()).collect();
    assert!(first.contains(&"atlas-1a2b".into()), "{first:?}");
    assert!(first.contains(&"atlas-2c3d".into()), "{first:?}");
    app.reload().unwrap();
    let second: Vec<_> = app.rows.iter().map(|r| r.id.clone()).collect();
    assert_eq!(first, second);
    let text = render_plain(&app, 100, 24).unwrap();
    assert!(text.contains("atlas-1a2b"), "{text}");
    assert!(!text.contains("(empty)"), "{text}");
}

/// After attach, `since_revision` must be omitted when the pane changes.
/// Serve `unchanged` is catalog-wide; a Ready page is not a List page.
#[derive(Debug)]
struct SinceOnRepeat {
    inner: CoreBackend,
    skip_since: std::sync::atomic::AtomicBool,
    last_since: std::sync::Mutex<Option<Option<u64>>>,
}

impl SinceOnRepeat {
    fn new(inner: CoreBackend) -> Self {
        Self {
            inner,
            skip_since: std::sync::atomic::AtomicBool::new(true),
            last_since: std::sync::Mutex::new(None),
        }
    }

    fn take_since(&self) -> Option<u64> {
        if self
            .skip_since
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            None
        } else {
            Some(5)
        }
    }

    fn page(
        &self,
        since: Option<u64>,
        full: vissue_tui::ListPage,
    ) -> Result<vissue_tui::ListPage, vissue_core::error::Error> {
        *self.last_since.lock().unwrap() = Some(since);
        if since.is_some() {
            Ok(vissue_tui::ListPage {
                unchanged: true,
                revision: 5,
                ..vissue_tui::ListPage::default()
            })
        } else {
            Ok(full)
        }
    }
}

impl BoardBackend for SinceOnRepeat {
    fn layout(&self) -> &vissue_core::config::Layout {
        self.inner.layout()
    }
    fn generation(&self) -> u64 {
        self.inner.generation()
    }
    fn revision(&self) -> u64 {
        5
    }
    fn live(&self) -> vissue_tui::BackendKind {
        vissue_tui::BackendKind::Control
    }
    fn identity(&self) -> &str {
        self.inner.identity()
    }
    fn list(
        &self,
        q: vissue_core::views::ListQuery,
    ) -> Result<vissue_tui::ListPage, vissue_core::error::Error> {
        self.page(self.take_since(), self.inner.list(q)?)
    }
    fn ready(
        &self,
        project: Option<&str>,
    ) -> Result<vissue_tui::ListPage, vissue_core::error::Error> {
        self.page(self.take_since(), self.inner.ready(project)?)
    }
    fn get(&self, id: &str) -> Result<vissue_core::views::IssueDetail, vissue_core::error::Error> {
        self.inner.get(id)
    }
    fn excerpt(&self, id: &str) -> Result<vissue_core::views::Excerpt, vissue_core::error::Error> {
        self.inner.excerpt(id)
    }
    fn search(
        &self,
        q: &str,
        n: usize,
    ) -> Result<Vec<vissue_core::views::SearchHit>, vissue_core::error::Error> {
        self.inner.search(q, n)
    }
    fn claims(
        &self,
        h: Option<&str>,
        p: Option<&str>,
    ) -> Result<Vec<vissue_core::views::ClaimRow>, vissue_core::error::Error> {
        self.inner.claims(h, p)
    }
    fn agenda(
        &self,
        d: i64,
        p: Option<&str>,
    ) -> Result<Vec<vissue_core::views::AgendaRow>, vissue_core::error::Error> {
        self.inner.agenda(d, p)
    }
    fn tree(&self, id: &str) -> Result<vissue_core::views::TreeNode, vissue_core::error::Error> {
        self.inner.tree(id)
    }
    fn related(
        &self,
        id: &str,
        d: usize,
        n: usize,
    ) -> Result<Vec<vissue_core::views::RelatedHit>, vissue_core::error::Error> {
        self.inner.related(id, d, n)
    }
    fn recall(
        &self,
        id: &str,
        depth: usize,
    ) -> Result<vissue_core::views::Recall, vissue_core::error::Error> {
        self.inner.recall(id, depth)
    }
    fn deed(
        &self,
        id: &str,
        add: &[String],
    ) -> Result<vissue_tui::MutResult, vissue_core::error::Error> {
        self.inner.deed(id, add)
    }
    fn projects(&self) -> Result<Vec<String>, vissue_core::error::Error> {
        self.inner.projects()
    }
    fn claim(&self, id: &str, f: bool) -> Result<vissue_tui::MutResult, vissue_core::error::Error> {
        self.inner.claim(id, f)
    }
    fn note(&self, id: &str, t: &str) -> Result<vissue_tui::MutResult, vissue_core::error::Error> {
        self.inner.note(id, t)
    }
    fn update(
        &self,
        r: vissue_tui::UpdateReq,
    ) -> Result<vissue_tui::MutResult, vissue_core::error::Error> {
        self.inner.update(r)
    }
    fn create(
        &self,
        project: &str,
        title: &str,
    ) -> Result<vissue_tui::MutResult, vissue_core::error::Error> {
        self.inner.create(project, title)
    }
    fn open(&self, id: &str) -> Result<vissue_core::views::IssueDetail, vissue_core::error::Error> {
        self.inner.open(id)
    }
    fn wait(&self, last: u64, ms: u64) -> Result<u64, vissue_core::error::Error> {
        self.inner.wait(last, ms)
    }
    fn last_since_revision(&self) -> Option<Option<u64>> {
        *self.last_since.lock().unwrap()
    }
    fn invalidate_since(&self) {
        self.skip_since
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn switching_ready_to_list_fetches_a_full_list() {
    let backend = SinceOnRepeat::new(CoreBackend::open(fixture_layout(), "snap").unwrap());
    let mut app = App::with_backend(Box::new(backend), "snap".into(), ServeStatus::Live).unwrap();
    let ready_ids: Vec<String> = app.rows.iter().map(|r| r.id.clone()).collect();
    assert!(
        ready_ids.iter().any(|id| id == "atlas-1a2b"),
        "{ready_ids:?}"
    );
    assert!(
        !ready_ids.iter().any(|id| id == "atlas-4g5h"),
        "ready must not include DONE: {ready_ids:?}"
    );
    assert_eq!(app.backend().last_since_revision(), Some(None));

    app.handle_key(ch('2'));
    assert_eq!(app.pane, vissue_tui::Pane::List);
    assert_eq!(app.backend().last_since_revision(), Some(None));
    let list_ids: Vec<String> = app.rows.iter().map(|r| r.id.clone()).collect();
    assert!(
        list_ids.iter().any(|id| id == "atlas-4g5h"),
        "list must include DONE, not the ready subset: {list_ids:?}"
    );
    assert!(list_ids.iter().any(|id| id == "atlas-3e4f"), "{list_ids:?}");
    assert!(list_ids.len() > ready_ids.len(), "{list_ids:?}");

    app.handle_key(ch('5'));
    assert!(app.rows.is_empty());
    app.handle_key(ch('1'));
    assert_eq!(app.pane, vissue_tui::Pane::Ready);
    assert_eq!(app.backend().last_since_revision(), Some(None));
    let back: Vec<_> = app.rows.iter().map(|r| r.id.as_str()).collect();
    assert!(
        back.contains(&"atlas-1a2b"),
        "ready must not stay empty: {back:?}"
    );
    assert!(!back.is_empty());
}

#[test]
fn claims_and_search_draw_extra() {
    let backend = CoreBackend::open(fixture_layout(), "snap").unwrap();
    let mut app =
        App::with_backend(Box::new(backend), "snap".into(), ServeStatus::Offline).unwrap();
    app.handle_key(ch('3'));
    assert_eq!(app.pane, vissue_tui::Pane::Claims);
    let holder = app
        .rows
        .iter()
        .find(|r| r.id == "atlas-1a2b")
        .map(|r| r.extra.clone())
        .unwrap();
    assert!(holder.contains("fixture-agent"), "{holder}");
    let text = render_plain(&app, 120, 24).unwrap();
    assert!(text.contains("fixture-agent"), "{text}");

    app.handle_key(ch('/'));
    type_text(&mut app, "retry");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.selected_id(), Some("beacon-5j6k"));
    let snippet = &app.rows[0].extra;
    assert!(!snippet.is_empty(), "search extra empty");
    let text = render_plain(&app, 120, 24).unwrap();
    assert!(text.contains(snippet.trim()), "{text}");
}

/// The board can cite a deed, and a value that is not an accession is refused
/// where the person typing it can see it.
///
/// The refusal is the half worth driving. A citation that resolves to nothing
/// fails in whatever opens it later, in another process on another day, so a
/// board that swallowed the message would be handing the failure forward.
#[test]
fn the_board_cites_a_deed_and_shows_the_refusal() {
    let (_dir, layout) = writable();
    let backend = CoreBackend::open(layout.clone(), "deeder").unwrap();
    let mut app =
        App::with_backend(Box::new(backend), "deeder".into(), ServeStatus::Offline).unwrap();
    let id = app.selected_id().expect("a ready row").to_string();

    app.handle_key(ch('d'));
    assert!(app.prompt.is_some(), "d opens the deed field");
    type_text(&mut app, "deed-file-thing");
    app.handle_key(key(KeyCode::Enter));
    assert!(
        app.status_line().contains("deeds += deed-file-thing"),
        "{}",
        app.status_line()
    );
    let cited = |layout: &Layout, id: &str| {
        vissue_core::store::find_by_id(layout, id)
            .unwrap()
            .expect("issue")
            .0
            .deeds()
    };
    assert_eq!(cited(&layout, &id), vec!["deed-file-thing".to_string()]);

    app.handle_key(ch('d'));
    type_text(&mut app, "/tmp/note.md");
    app.handle_key(key(KeyCode::Enter));
    assert!(
        app.status_line().contains("not a deed accession"),
        "the refusal has to reach the board: {}",
        app.status_line()
    );
    assert_eq!(
        cited(&layout, &id).len(),
        1,
        "a refused citation must not land"
    );
}
