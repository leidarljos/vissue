//! Board state and key handling. Drawing lives in [`crate::view`].

use std::path::PathBuf;

use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyEvent;
use vissue_core::config::Layout;
use vissue_core::views::{IssueDetail, ListQuery};

use crate::attach::{AttachHooks, AttachOutcome, ServeStatus, try_attach};
use crate::backend::{BoardBackend, ListPage, UpdateReq};
use crate::core_backend::CoreBackend;
use crate::keys::{
    Action, ConfirmKind, DetailTab, Focus, HELP, Pane, PromptKind, char_of, is_press,
};

/// One displayed row. Every pane maps onto this shape so keys share a path.
#[derive(Debug, Clone)]
pub struct BoardRow {
    /// Issue id shown in the first column.
    pub id: String,
    /// Org TODO state (`TODO`, `STARTED`, ...).
    pub state: String,
    /// Priority letter (`A`, `B`, or `C`).
    pub priority: String,
    /// Heading title.
    pub title: String,
    /// Project name.
    pub project: String,
    /// Pane-specific suffix: holder, agenda date, or search snippet.
    pub extra: String,
}

/// Interactive board. Talks only to [`BoardBackend`].
#[derive(Debug)]
pub struct App {
    backend: Box<dyn BoardBackend>,
    agent: String,
    status: ServeStatus,
    message: String,
    /// Active list pane.
    pub pane: Pane,
    /// Detail pane tab (show / excerpt / tree / related).
    pub detail_tab: DetailTab,
    /// Whether keys target the row list or the detail pane.
    pub focus: Focus,
    /// Rows for the current pane.
    pub rows: Vec<BoardRow>,
    /// Index into [`Self::rows`].
    pub selected: usize,
    /// Project filter, if any.
    pub project: Option<String>,
    /// Known project names for the `p` prompt.
    pub projects: Vec<String>,
    /// Last loaded issue detail, if any.
    pub detail: Option<IssueDetail>,
    /// Text drawn in the detail pane.
    pub detail_body: String,
    /// Open line prompt and its buffer.
    pub prompt: Option<(PromptKind, String)>,
    /// Pending DONE/CANCELLED confirmation.
    pub confirm: Option<ConfirmKind>,
    /// Help overlay is visible.
    pub help: bool,
    /// Last id copied with `y`.
    pub clipboard: String,
    search_query: String,
}

impl App {
    /// Open a file-backed board and load the Ready pane.
    ///
    /// # Errors
    ///
    /// Returns an error if the vault cannot be parsed or the first pane cannot
    /// be loaded.
    pub fn open_core(layout: Layout, agent: String) -> Result<Self, vissue_core::error::Error> {
        let backend = CoreBackend::open(layout, agent.clone())?;
        Self::with_backend(Box::new(backend), agent, ServeStatus::Offline)
    }

    /// Build a board around an existing backend and load the Ready pane.
    ///
    /// # Errors
    ///
    /// Returns an error if the first pane cannot be loaded.
    pub fn with_backend(
        backend: Box<dyn BoardBackend>,
        agent: String,
        status: ServeStatus,
    ) -> Result<Self, vissue_core::error::Error> {
        let projects = backend.projects().unwrap_or_default();
        let mut app = Self {
            backend,
            agent,
            status,
            message: String::new(),
            pane: Pane::Ready,
            detail_tab: DetailTab::Show,
            focus: Focus::Rows,
            rows: Vec::new(),
            selected: 0,
            project: None,
            projects,
            detail: None,
            detail_body: String::new(),
            prompt: None,
            confirm: None,
            help: false,
            clipboard: String::new(),
            search_query: String::new(),
        };
        app.reload()?;
        Ok(app)
    }

    /// How the status line labels the current store.
    pub fn serve_status(&self) -> ServeStatus {
        self.status
    }

    /// Identity used for claims and updates.
    pub fn agent(&self) -> &str {
        &self.agent
    }

    /// Catalog generation from the current backend.
    pub fn generation(&self) -> u64 {
        self.backend.generation()
    }

    /// Serve revision from the current backend. Core is always 0.
    pub fn revision(&self) -> u64 {
        self.backend.revision()
    }

    /// Id of the selected row, if the pane is not empty.
    pub fn selected_id(&self) -> Option<&str> {
        self.rows.get(self.selected).map(|r| r.id.as_str())
    }

    /// Org state of the selected row, if the pane is not empty.
    pub fn selected_state(&self) -> Option<&str> {
        self.rows.get(self.selected).map(|r| r.state.as_str())
    }

    /// The store this board is talking to.
    pub fn backend(&self) -> &dyn BoardBackend {
        self.backend.as_ref()
    }

    /// Swap the store and adopt its identity. Does not reload rows.
    pub fn replace_backend(&mut self, backend: Box<dyn BoardBackend>, status: ServeStatus) {
        self.backend = backend;
        self.status = status;
        self.agent = self.backend.identity().to_string();
    }

    /// Post-paint attach. `--offline` never probes the socket.
    ///
    /// # Errors
    ///
    /// Returns an error if the pane cannot be reloaded after the attach attempt.
    pub fn attach(
        &mut self,
        socket: &std::path::Path,
        offline: bool,
        hooks: &AttachHooks,
    ) -> Result<(), vissue_core::error::Error> {
        let layout = self.backend.layout().clone();
        let agent = self.agent.clone();
        match try_attach(&layout, socket, &agent, offline, hooks) {
            AttachOutcome::Switch { backend, status } => {
                self.replace_backend(backend, status);
                self.message.clear();
            }
            AttachOutcome::Stay { status, message } => {
                self.status = status;
                self.message = message;
            }
        }
        self.reload()
    }

    /// One-line `serve:` / gen / rev / agent / project / message summary.
    pub fn status_line(&self) -> String {
        let kind = match self.status {
            ServeStatus::Live => "live",
            ServeStatus::Offline => "offline",
            ServeStatus::Mismatch => "mismatch",
        };
        let mut line = format!(
            "serve:{kind} gen={} rev={} agent={}",
            self.backend.generation(),
            self.backend.revision(),
            self.agent
        );
        if let Some(project) = &self.project {
            line.push_str(" project=");
            line.push_str(project);
        }
        if !self.message.is_empty() {
            line.push_str("  ");
            line.push_str(&self.message);
        }
        line
    }

    /// Fetch the current pane from the backend and refresh detail.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend cannot load the pane.
    pub fn reload(&mut self) -> Result<(), vissue_core::error::Error> {
        let project = self.project.as_deref();
        match self.pane {
            Pane::Ready => self.apply_issue_page(self.backend.ready(project)?),
            Pane::List => self.apply_issue_page(self.backend.list(ListQuery {
                project: project.map(str::to_string),
                ..ListQuery::default()
            })?),
            Pane::Claims => {
                self.rows = self
                    .backend
                    .claims(None, project)?
                    .into_iter()
                    .map(row_from_claim)
                    .collect();
            }
            Pane::Agenda => {
                self.rows = self
                    .backend
                    .agenda(14, project)?
                    .into_iter()
                    .map(row_from_agenda)
                    .collect();
            }
            Pane::Search => {
                self.rows = if self.search_query.is_empty() {
                    Vec::new()
                } else {
                    self.backend
                        .search(&self.search_query, 50)?
                        .into_iter()
                        .map(row_from_search)
                        .collect()
                };
            }
        }
        if self.selected >= self.rows.len() {
            self.selected = self.rows.len().saturating_sub(1);
        }
        self.refresh_detail();
        Ok(())
    }

    /// Serve answers `{unchanged: true, issues: []}` when `since_revision`
    /// matches the catalog. Keep the rows from the last full page.
    fn apply_issue_page(&mut self, page: ListPage) {
        if page.unchanged {
            return;
        }
        self.rows = page.issues.into_iter().map(row_from_issue).collect();
    }

    /// Wait briefly for a catalog change and reload when the watermark moves.
    pub fn poll_updates(&mut self) {
        let last = match self.backend.live() {
            crate::backend::BackendKind::Control => self.backend.revision(),
            crate::backend::BackendKind::Core => self.backend.generation(),
        };
        if let Ok(next) = self.backend.wait(last, 1)
            && next > last
        {
            let _ = self.reload();
        }
    }

    /// Dispatch one key. Repeat and press count; release is ignored.
    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if !is_press(key) {
            return Action::Continue;
        }
        if self.help {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?')
            ) {
                self.help = false;
            }
            return Action::Continue;
        }
        if self.confirm.is_some() {
            return self.handle_confirm(key);
        }
        if self.prompt.is_some() {
            return self.handle_prompt(key);
        }
        match key.code {
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Esc => {
                if self.focus == Focus::Detail {
                    self.focus = Focus::Rows;
                    Action::Continue
                } else {
                    Action::Quit
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_sel(1);
                Action::Continue
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_sel(-1);
                Action::Continue
            }
            KeyCode::Tab => self.goto_pane(self.pane.next()),
            KeyCode::Char('1') => self.goto_pane(Pane::Ready),
            KeyCode::Char('2') => self.goto_pane(Pane::List),
            KeyCode::Char('3') => self.goto_pane(Pane::Claims),
            KeyCode::Char('4') => self.goto_pane(Pane::Agenda),
            KeyCode::Char('5') => self.goto_pane(Pane::Search),
            KeyCode::Enter => {
                if self.focus == Focus::Detail {
                    self.detail_tab = self.detail_tab.next();
                    self.refresh_detail();
                } else {
                    self.focus = Focus::Detail;
                    self.refresh_detail();
                }
                Action::Continue
            }
            KeyCode::Char('p') => {
                self.prompt = Some((
                    PromptKind::Project,
                    self.project.clone().unwrap_or_default(),
                ));
                Action::Continue
            }
            KeyCode::Char('/') => {
                if self.pane != Pane::Search {
                    self.backend.invalidate_since();
                    self.pane = Pane::Search;
                }
                self.prompt = Some((PromptKind::Search, self.search_query.clone()));
                Action::Continue
            }
            KeyCode::Char('c') => {
                self.claim_selected();
                Action::Continue
            }
            KeyCode::Char('n') => {
                if self.selected_id().is_some() {
                    self.prompt = Some((PromptKind::Note, String::new()));
                }
                Action::Continue
            }
            KeyCode::Char('d') => {
                if self.selected_id().is_some() {
                    self.prompt = Some((PromptKind::Deed, String::new()));
                }
                Action::Continue
            }
            KeyCode::Char('s') => {
                self.cycle_state();
                Action::Continue
            }
            KeyCode::Char('D') => {
                if self.selected_id().is_some() {
                    self.confirm = Some(ConfirmKind::Done);
                    self.message = "confirm DONE? y/n".into();
                }
                Action::Continue
            }
            KeyCode::Char('X') => {
                if self.selected_id().is_some() {
                    self.confirm = Some(ConfirmKind::Cancelled);
                    self.message = "confirm CANCELLED? y/n".into();
                }
                Action::Continue
            }
            KeyCode::Char('o') => {
                self.open_selected();
                Action::Continue
            }
            KeyCode::Char('y') => {
                if let Some(id) = self.selected_id().map(str::to_string) {
                    self.clipboard = id.clone();
                    self.message = format!("copied {id}");
                }
                Action::Continue
            }
            KeyCode::Char('R') => {
                let _ = self.reload();
                self.message = "reloaded".into();
                Action::Continue
            }
            KeyCode::Char('?') => {
                self.help = true;
                Action::Continue
            }
            _ => Action::Continue,
        }
    }

    fn handle_confirm(&mut self, key: KeyEvent) -> Action {
        let kind = self.confirm.unwrap();
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                self.confirm = None;
                self.apply_state(kind.state());
            }
            _ => {
                self.confirm = None;
                self.message.clear();
            }
        }
        Action::Continue
    }

    fn handle_prompt(&mut self, key: KeyEvent) -> Action {
        let Some((kind, mut text)) = self.prompt.take() else {
            return Action::Continue;
        };
        match key.code {
            KeyCode::Esc => {
                self.message.clear();
            }
            KeyCode::Enter => match kind {
                PromptKind::Search => {
                    self.search_query = text;
                    if self.pane != Pane::Search {
                        self.backend.invalidate_since();
                    }
                    self.pane = Pane::Search;
                    let _ = self.reload();
                }
                PromptKind::Note => {
                    if let Some(id) = self.selected_id().map(str::to_string) {
                        match self.backend.note(&id, &text) {
                            Ok(result) => {
                                self.message = result.report.trim().to_string();
                                let _ = self.reload();
                            }
                            Err(err) => self.message = err.to_string(),
                        }
                    }
                }
                PromptKind::Deed => {
                    // The refusal has to reach the board. A citation that
                    // resolves to nothing fails in whatever opens it later, in
                    // another process on another day, so the message belongs on
                    // the screen of whoever typed it.
                    if let Some(id) = self.selected_id().map(str::to_string) {
                        let accession = text.trim().to_string();
                        if accession.is_empty() {
                            self.message = "no deed cited".to_string();
                        } else {
                            match self.backend.deed(&id, &[accession]) {
                                Ok(result) => {
                                    self.message = result.report.trim().to_string();
                                    let _ = self.reload();
                                }
                                Err(err) => self.message = err.to_string(),
                            }
                        }
                    }
                }
                PromptKind::Project => {
                    let trimmed = text.trim();
                    let next = if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    };
                    if next != self.project {
                        self.backend.invalidate_since();
                    }
                    self.project = next;
                    let _ = self.reload();
                }
            },
            KeyCode::Backspace => {
                text.pop();
                self.prompt = Some((kind, text));
            }
            _ => {
                if let Some(c) = char_of(key) {
                    text.push(c);
                }
                self.prompt = Some((kind, text));
            }
        }
        Action::Continue
    }

    fn goto_pane(&mut self, pane: Pane) -> Action {
        if self.pane != pane {
            self.backend.invalidate_since();
            self.pane = pane;
        }
        let _ = self.reload();
        Action::Continue
    }

    fn move_sel(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let len = self.rows.len() as i32;
        let next = (self.selected as i32 + delta).clamp(0, len - 1) as usize;
        if next != self.selected {
            self.selected = next;
            self.refresh_detail();
        }
    }

    fn claim_selected(&mut self) {
        let Some(id) = self.selected_id().map(str::to_string) else {
            return;
        };
        match self.backend.claim(&id, false) {
            Ok(result) => {
                self.message = result.report.trim().to_string();
                let _ = self.reload();
            }
            Err(err) => self.message = err.to_string(),
        }
    }

    fn cycle_state(&mut self) {
        let Some(id) = self.selected_id().map(str::to_string) else {
            return;
        };
        let Some(state) = self.selected_state() else {
            return;
        };
        let next = match state {
            "TODO" => "STARTED",
            "STARTED" => "BLOCKED",
            "BLOCKED" => "TODO",
            _ => {
                self.message = format!("{id} is {state}; s cycles TODO/STARTED/BLOCKED");
                return;
            }
        };
        self.apply_state(next);
    }

    fn apply_state(&mut self, state: &str) {
        let Some(id) = self.selected_id().map(str::to_string) else {
            return;
        };
        match self.backend.update(UpdateReq {
            id: id.clone(),
            state: Some(state.to_string()),
            if_state: self.selected_state().map(str::to_string),
            ..UpdateReq::default()
        }) {
            Ok(result) => {
                self.message = result.report.trim().to_string();
                let _ = self.reload();
            }
            Err(err) => self.message = err.to_string(),
        }
    }

    fn open_selected(&mut self) {
        let Some(id) = self.selected_id().map(str::to_string) else {
            return;
        };
        match self.backend.open(&id) {
            Ok(detail) => {
                self.detail = Some(detail);
                self.message = format!("opened {id}");
                self.refresh_detail();
            }
            Err(err) => self.message = err.to_string(),
        }
    }

    fn refresh_detail(&mut self) {
        let Some(id) = self.selected_id().map(str::to_string) else {
            self.detail = None;
            self.detail_body.clear();
            return;
        };
        match self.detail_tab {
            DetailTab::Show => match self.backend.get(&id) {
                Ok(detail) => {
                    self.detail_body = format_show(&detail);
                    self.detail = Some(detail);
                }
                Err(err) => self.detail_body = err.to_string(),
            },
            DetailTab::Excerpt => match self.backend.excerpt(&id) {
                Ok(excerpt) => {
                    let mut text = excerpt.text;
                    if !text.ends_with('\n') {
                        text.push('\n');
                    }
                    text.push_str("body lives in file; open the range above");
                    self.detail_body = text;
                }
                Err(err) => self.detail_body = err.to_string(),
            },
            DetailTab::Tree => match self.backend.tree(&id) {
                Ok(node) => self.detail_body = format_tree(&node, 0),
                Err(err) => self.detail_body = err.to_string(),
            },
            DetailTab::Related => match self.backend.related(&id, 2, 20) {
                Ok(hits) => self.detail_body = format_related(&hits),
                Err(err) => self.detail_body = err.to_string(),
            },
            DetailTab::Recall => match self.backend.recall(&id, 1) {
                Ok(set) => self.detail_body = format_recall(&set),
                Err(err) => self.detail_body = err.to_string(),
            },
        }
    }

    /// Label and buffer for the open prompt, if any.
    pub fn prompt_line(&self) -> Option<String> {
        self.prompt.as_ref().map(|(kind, text)| {
            let label = match kind {
                PromptKind::Search => "search",
                PromptKind::Note => "note",
                PromptKind::Deed => "deed",
                PromptKind::Project => "project",
            };
            format!("{label}: {text}")
        })
    }

    /// Confirmation line for DONE/CANCELLED, if any.
    pub fn confirm_line(&self) -> Option<String> {
        self.confirm
            .map(|kind| format!("confirm {}? y/n", kind.state()))
    }

    /// Text drawn on `?`.
    pub fn help_text(&self) -> &'static str {
        HELP
    }
}

fn row_from_issue(row: vissue_core::views::IssueRow) -> BoardRow {
    let extra = row.claimed_by.unwrap_or_default();
    BoardRow {
        id: row.id,
        state: row.state,
        priority: row.priority,
        title: row.title,
        project: row.project,
        extra,
    }
}

fn row_from_claim(row: vissue_core::views::ClaimRow) -> BoardRow {
    BoardRow {
        id: row.id,
        state: row.state,
        priority: row.priority,
        title: row.title,
        project: row.project,
        extra: format!("{} {}d", row.holder.unwrap_or_default(), row.age_days),
    }
}

fn row_from_agenda(row: vissue_core::views::AgendaRow) -> BoardRow {
    BoardRow {
        id: row.id,
        state: row.state,
        priority: row.priority,
        title: row.title,
        project: row.project,
        extra: format!("{} {}", row.kind, row.date),
    }
}

fn row_from_search(row: vissue_core::views::SearchHit) -> BoardRow {
    BoardRow {
        id: row.id,
        state: row.state,
        priority: row.priority,
        title: row.title,
        project: row.project,
        extra: row.snippet,
    }
}

fn format_show(d: &IssueDetail) -> String {
    let mut out = format!(
        "id: {}\nproject: {}\nstate: {}\npriority: {}\ntitle: {}\nfile: {}\n",
        d.id, d.project, d.state, d.priority, d.title, d.file
    );
    if let Some(parent) = &d.parent {
        out.push_str(&format!("parent: {parent}\n"));
    }
    if !d.blocked_by.is_empty() {
        out.push_str(&format!("blocked_by: {}\n", d.blocked_by.join(", ")));
    }
    if let Some(who) = &d.claimed_by {
        out.push_str(&format!("claimed_by: {who}\n"));
    }
    if !d.tags.is_empty() {
        out.push_str(&format!("tags: {}\n", d.tags.join(", ")));
    }
    out
}

fn format_tree(node: &vissue_core::views::TreeNode, depth: usize) -> String {
    let pad = "  ".repeat(depth);
    let mut out = format!("{pad}{} [{}] {}\n", node.id, node.state, node.title);
    for child in &node.children {
        out.push_str(&format_tree(child, depth + 1));
    }
    out
}

fn format_related(hits: &[vissue_core::views::RelatedHit]) -> String {
    if hits.is_empty() {
        return "no related issues\n".into();
    }
    let mut out = String::new();
    for hit in hits {
        out.push_str(&format!(
            "{} [{}] {}  score={:.2}  {}\n",
            hit.id,
            hit.state,
            hit.title,
            hit.score,
            hit.evidence.join(", ")
        ));
    }
    out
}

/// The working set, laid out for the detail pane.
///
/// Narrower than the command line's rendering: the pane is a column beside a
/// list, so the plan and the inputs are one line each and the deed accessions
/// hang under the input that produced them.
fn format_recall(set: &vissue_core::views::Recall) -> String {
    let mut out = String::new();
    for step in &set.plan {
        out.push_str(&format!(
            "plan {} [{}] {}\n",
            step.id, step.state, step.title
        ));
    }
    if set.inputs.is_empty() {
        out.push_str("no declared inputs\n");
    }
    for input in &set.inputs {
        out.push_str(&format!(
            "{} [{}] {}  ({})\n",
            input.id, input.state, input.title, input.relation
        ));
        for deed in &input.deeds {
            out.push_str(&format!("  {deed}\n"));
        }
        if let Some(note) = &input.last_note {
            out.push_str(&format!(
                "  note: {}\n",
                note.lines().next().unwrap_or_default().trim()
            ));
        }
    }
    if !set.produced.is_empty() {
        out.push_str("produced here\n");
        for deed in &set.produced {
            out.push_str(&format!("  {deed}\n"));
        }
    }
    out
}

/// Options for the interactive `vissue tui` entry point.
#[derive(Debug)]
pub struct RunOpts {
    /// Vault root and project prefix.
    pub layout: Layout,
    /// Control socket to attach after first paint.
    pub socket: PathBuf,
    /// Skip the socket and stay on [`CoreBackend`].
    pub offline: bool,
    /// Identity stamped on claims and updates.
    pub agent: String,
}

/// First paint via core, then attach unless `--offline`, then the crossterm loop.
///
/// # Errors
///
/// Returns an error if the vault cannot be opened, the terminal cannot be
/// installed or drawn, attach reload fails, or a terminal event cannot be read.
pub fn run(opts: RunOpts) -> Result<(), vissue_core::error::Error> {
    let mut app = App::open_core(opts.layout.clone(), opts.agent.clone())?;
    let mut terminal = crate::view::install()?;
    let result = (|| {
        terminal.draw(|f| crate::view::draw(f, &app))?;
        app.attach(&opts.socket, opts.offline, &AttachHooks::default())?;
        loop {
            terminal.draw(|f| crate::view::draw(f, &app))?;
            if ratatui::crossterm::event::poll(std::time::Duration::from_millis(200))? {
                if let ratatui::crossterm::event::Event::Key(key) =
                    ratatui::crossterm::event::read()?
                    && app.handle_key(key) == Action::Quit
                {
                    break;
                }
            } else {
                app.poll_updates();
            }
        }
        Ok(())
    })();
    crate::view::restore()?;
    result
}
