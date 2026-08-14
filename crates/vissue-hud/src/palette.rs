//! Overlay state: filter, excerpt, claim, note. No iced types.

use std::path::Path;

use vissue_core::config::Layout;
use vissue_core::ops::{self, CreateOpts};
use vissue_core::views::{Excerpt, IssueDetail, IssueRow, ListQuery, SearchHit};
use vissue_tui::attach::{AttachHooks, AttachOutcome, ServeStatus};
use vissue_tui::backend::{BoardBackend, UpdateReq};
use vissue_tui::CoreBackend;

use crate::attach;
use crate::fuzzy::rank_indices;
use crate::summon::{SummonAction, SummonRequest};

const HELP: &str = "\
vissue hud  (same board as vissue tui)

j/k, arrows   move
Tab, 1-5      pane (Ready List Claims Agenda Search)
Enter         cycle detail (show / excerpt / tree / related / notes)
p             cycle project filter
/             search
a             add a task
c             claim
n             note
s             cycle TODO / STARTED / BLOCKED
space / D     DONE (D confirms)
X             CANCELLED (confirm)
o             open heading
y             copy id
R             reload
?             this help
esc           back / hide

Body edits stay in the file.
";

/// Where a row came from before the filter merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemSource {
    Ready,
    Search,
    List,
    Claims,
    Agenda,
}

/// Same panes as the terminal board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardFilter {
    Ready,
    List,
    Claims,
    Agenda,
    Search,
}

impl BoardFilter {
    /// Chip order with labels.
    pub const ALL: [(Self, &'static str); 5] = [
        (Self::Ready, "Ready"),
        (Self::List, "List"),
        (Self::Claims, "Claims"),
        (Self::Agenda, "Agenda"),
        (Self::Search, "Search"),
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::List => "List",
            Self::Claims => "Claims",
            Self::Agenda => "Agenda",
            Self::Search => "Search",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|(p, _)| *p == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()].0
    }
}

/// Detail card. Same tabs as the terminal board, plus the logbook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Show,
    Excerpt,
    Tree,
    Related,
    Notes,
}

impl DetailTab {
    pub const ALL: [Self; 5] = [
        Self::Show,
        Self::Excerpt,
        Self::Tree,
        Self::Related,
        Self::Notes,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Show => "show",
            Self::Excerpt => "excerpt",
            Self::Tree => "tree",
            Self::Related => "related",
            Self::Notes => "notes",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// Which field owns typing. List is the default so j/k move rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    List,
    Search,
    Add,
    Note,
    Project,
    Help,
}

/// One selectable palette row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudItem {
    pub id: String,
    pub title: String,
    pub project: String,
    pub state: String,
    pub priority: String,
    pub source: ItemSource,
    pub claimed_by: Option<String>,
    pub due: Option<String>,
    pub blocked_by: Vec<String>,
    pub extra: String,
}

impl HudItem {
    fn from_row(row: IssueRow) -> Self {
        Self {
            id: row.id,
            title: row.title,
            project: row.project,
            state: row.state,
            priority: row.priority,
            source: ItemSource::Ready,
            claimed_by: row.claimed_by.clone(),
            due: None,
            blocked_by: row.blocked_by,
            extra: row.claimed_by.unwrap_or_default(),
        }
    }

    fn from_search(hit: SearchHit) -> Self {
        Self {
            id: hit.id,
            title: hit.title,
            project: hit.project,
            state: hit.state,
            priority: hit.priority,
            source: ItemSource::Search,
            claimed_by: None,
            due: None,
            blocked_by: Vec::new(),
            extra: hit.snippet,
        }
    }

    fn from_claim(row: vissue_core::views::ClaimRow) -> Self {
        let extra = format!(
            "{}  {}d",
            row.holder.clone().unwrap_or_default(),
            row.age_days
        );
        Self {
            id: row.id,
            title: row.title,
            project: row.project,
            state: row.state,
            priority: row.priority,
            source: ItemSource::Claims,
            claimed_by: row.holder,
            due: None,
            blocked_by: Vec::new(),
            extra,
        }
    }

    fn from_agenda(row: vissue_core::views::AgendaRow) -> Self {
        let extra = format!("{}  {}", row.kind, row.date);
        Self {
            id: row.id,
            title: row.title,
            project: row.project,
            state: row.state,
            priority: row.priority,
            source: ItemSource::Agenda,
            claimed_by: None,
            due: Some(row.date),
            blocked_by: Vec::new(),
            extra,
        }
    }

    fn from_list(row: IssueRow) -> Self {
        let mut item = Self::from_row(row);
        item.source = ItemSource::List;
        item
    }
}

/// Key the overlay understands. iced maps native keys onto this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteKey {
    Char(char),
    Enter,
    Esc,
    Up,
    Down,
    Backspace,
    Space,
    Tab,
}

/// Summonable palette. Talks only to [`BoardBackend`].
pub struct Palette {
    backend: Box<dyn BoardBackend>,
    agent: String,
    status: ServeStatus,
    message: String,
    query: String,
    items: Vec<HudItem>,
    filtered: Vec<usize>,
    selected: usize,
    excerpt: Option<Excerpt>,
    detail: Option<IssueDetail>,
    note_draft: Option<String>,
    add_draft: String,
    filter: BoardFilter,
    focus: Focus,
    detail_tab: DetailTab,
    detail_body: String,
    project: Option<String>,
    projects: Vec<String>,
    confirm: Option<ConfirmKind>,
    clipboard: String,
    ready_count: usize,
    list_count: usize,
    claims_count: usize,
    agenda_count: usize,
    search_count: usize,
    visible: bool,
}

/// Close or cancel confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmKind {
    Done,
    Cancelled,
}

impl ConfirmKind {
    pub fn state(self) -> &'static str {
        match self {
            Self::Done => "DONE",
            Self::Cancelled => "CANCELLED",
        }
    }
}

impl Palette {
    pub fn open_core(layout: Layout, agent: String) -> anyhow::Result<Self> {
        let backend = CoreBackend::open(layout, agent.clone())?;
        Self::with_backend(Box::new(backend), agent, ServeStatus::Offline)
    }

    pub fn with_backend(
        backend: Box<dyn BoardBackend>,
        agent: String,
        status: ServeStatus,
    ) -> anyhow::Result<Self> {
        let projects = backend.projects().unwrap_or_default();
        let mut palette = Self {
            backend,
            agent,
            status,
            message: String::new(),
            query: String::new(),
            items: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            excerpt: None,
            detail: None,
            note_draft: None,
            add_draft: String::new(),
            filter: BoardFilter::Ready,
            focus: Focus::List,
            detail_tab: DetailTab::Show,
            detail_body: String::new(),
            project: None,
            projects,
            confirm: None,
            clipboard: String::new(),
            ready_count: 0,
            list_count: 0,
            claims_count: 0,
            agenda_count: 0,
            search_count: 0,
            visible: true,
        };
        palette.reload()?;
        Ok(palette)
    }

    pub fn serve_status(&self) -> ServeStatus {
        self.status
    }

    pub fn agent(&self) -> &str {
        &self.agent
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn excerpt(&self) -> Option<&Excerpt> {
        self.excerpt.as_ref()
    }

    pub fn detail(&self) -> Option<&IssueDetail> {
        self.detail.as_ref()
    }

    pub fn note_draft(&self) -> Option<&str> {
        self.note_draft.as_deref()
    }

    pub fn add_draft(&self) -> &str {
        &self.add_draft
    }

    pub fn filter(&self) -> BoardFilter {
        self.filter
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    pub fn count(&self, filter: BoardFilter) -> usize {
        match filter {
            BoardFilter::Ready => self.ready_count,
            BoardFilter::List => self.list_count,
            BoardFilter::Claims => self.claims_count,
            BoardFilter::Agenda => self.agenda_count,
            BoardFilter::Search => self.search_count,
        }
    }

    pub fn detail_tab(&self) -> DetailTab {
        self.detail_tab
    }

    pub fn detail_body(&self) -> &str {
        &self.detail_body
    }

    pub fn project(&self) -> Option<&str> {
        self.project.as_deref()
    }

    pub fn confirm(&self) -> Option<ConfirmKind> {
        self.confirm
    }

    pub fn clipboard(&self) -> &str {
        &self.clipboard
    }

    pub fn help_text(&self) -> &'static str {
        HELP
    }

    pub fn backend(&self) -> &dyn BoardBackend {
        self.backend.as_ref()
    }

    pub fn generation(&self) -> u64 {
        self.backend.generation()
    }

    pub fn revision(&self) -> u64 {
        self.backend.revision()
    }

    pub fn filtered_items(&self) -> Vec<&HudItem> {
        self.filtered
            .iter()
            .filter_map(|&i| self.items.get(i))
            .collect()
    }

    pub fn selected_item(&self) -> Option<&HudItem> {
        self.filtered
            .get(self.selected)
            .and_then(|&i| self.items.get(i))
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.selected_item().map(|i| i.id.as_str())
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

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
        if !self.message.is_empty() {
            line.push_str("  ");
            line.push_str(&self.message);
        }
        line
    }

    /// Post-paint attach. `--offline` never probes the socket.
    pub fn attach(
        &mut self,
        socket: &Path,
        offline: bool,
        hooks: &AttachHooks,
    ) -> anyhow::Result<()> {
        let layout = self.backend.layout().clone();
        let agent = self.agent.clone();
        match attach::attach(&layout, socket, &agent, offline, hooks) {
            AttachOutcome::Switch { backend, status } => {
                self.backend = backend;
                self.status = status;
                self.agent = self.backend.identity().to_string();
                self.message.clear();
            }
            AttachOutcome::Stay { status, message } => {
                self.status = status;
                self.message = message;
            }
        }
        self.reload()
    }

    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.excerpt = None;
        let _ = self.reload();
    }

    pub fn show(&mut self) {
        self.visible = true;
    }

    pub fn hide(&mut self) {
        self.note_draft = None;
        self.add_draft.clear();
        self.confirm = None;
        self.focus = Focus::List;
        self.visible = false;
    }

    pub fn toggle(&mut self) {
        if self.visible {
            self.hide();
        } else {
            self.show();
        }
    }

    pub fn apply_summon(&mut self, req: &SummonRequest) {
        match req.action {
            SummonAction::Show => self.show(),
            SummonAction::Hide => self.hide(),
            SummonAction::Toggle => self.toggle(),
        }
    }

    pub fn handle_key(&mut self, key: PaletteKey) {
        if !self.visible {
            return;
        }
        if self.focus == Focus::Help {
            if matches!(
                key,
                PaletteKey::Esc | PaletteKey::Char('?') | PaletteKey::Char('q')
            ) {
                self.focus = Focus::List;
            }
            return;
        }
        if self.confirm.is_some() {
            self.handle_confirm_key(key);
            return;
        }
        if self.focus == Focus::Note || self.note_draft.is_some() {
            self.handle_note_key(key);
            return;
        }
        if self.focus == Focus::Add {
            self.handle_add_key(key);
            return;
        }
        if self.focus == Focus::Search {
            self.handle_search_key(key);
            return;
        }
        match key {
            PaletteKey::Esc => {
                if self.detail_tab != DetailTab::Show || self.excerpt.is_some() {
                    self.detail_tab = DetailTab::Show;
                    self.excerpt = None;
                    self.refresh_detail();
                } else {
                    self.hide();
                }
            }
            PaletteKey::Enter => self.cycle_detail_tab(),
            PaletteKey::Tab => self.set_filter(self.filter.next()),
            PaletteKey::Up => self.move_sel(-1),
            PaletteKey::Down => self.move_sel(1),
            PaletteKey::Backspace => {}
            PaletteKey::Space => {
                if let Some(id) = self.selected_id().map(str::to_string) {
                    self.toggle_done(&id);
                }
            }
            PaletteKey::Char('j') => self.move_sel(1),
            PaletteKey::Char('k') => self.move_sel(-1),
            PaletteKey::Char('c') => self.claim_selected(),
            PaletteKey::Char('n') => {
                if self.selected_id().is_some() {
                    self.note_draft = Some(String::new());
                    self.focus = Focus::Note;
                }
            }
            PaletteKey::Char('a') => self.focus_add(),
            PaletteKey::Char('/') => {
                self.set_filter(BoardFilter::Search);
                self.focus_search();
            }
            PaletteKey::Char('p') => self.cycle_project(),
            PaletteKey::Char('s') => self.cycle_state(),
            PaletteKey::Char('D') => self.confirm = Some(ConfirmKind::Done),
            PaletteKey::Char('X') => self.confirm = Some(ConfirmKind::Cancelled),
            PaletteKey::Char('o') => self.open_selected(),
            PaletteKey::Char('y') => self.copy_selected(),
            PaletteKey::Char('R') => {
                self.backend.invalidate_since();
                let _ = self.reload();
            }
            PaletteKey::Char('?') => self.focus = Focus::Help,
            PaletteKey::Char('1') => self.set_filter(BoardFilter::Ready),
            PaletteKey::Char('2') => self.set_filter(BoardFilter::List),
            PaletteKey::Char('3') => self.set_filter(BoardFilter::Claims),
            PaletteKey::Char('4') => self.set_filter(BoardFilter::Agenda),
            PaletteKey::Char('5') => self.set_filter(BoardFilter::Search),
            PaletteKey::Char(_) => {}
        }
    }

    fn handle_note_key(&mut self, key: PaletteKey) {
        let Some(mut text) = self.note_draft.take() else {
            return;
        };
        match key {
            PaletteKey::Esc => {
                self.message.clear();
                self.focus = Focus::List;
            }
            PaletteKey::Enter => {
                if let Some(id) = self.selected_id().map(str::to_string) {
                    match self.backend.note(&id, &text) {
                        Ok(result) => {
                            self.message = result.report.trim().to_string();
                            self.focus = Focus::List;
                            self.detail_tab = DetailTab::Notes;
                            let _ = self.reload();
                        }
                        Err(err) => {
                            self.message = err.to_string();
                            self.note_draft = Some(text);
                            self.focus = Focus::Note;
                        }
                    }
                }
            }
            PaletteKey::Backspace => {
                text.pop();
                self.note_draft = Some(text);
            }
            PaletteKey::Char(c) => {
                text.push(c);
                self.note_draft = Some(text);
            }
            PaletteKey::Up | PaletteKey::Down | PaletteKey::Space | PaletteKey::Tab => {
                self.note_draft = Some(text);
            }
        }
    }

    fn handle_add_key(&mut self, key: PaletteKey) {
        match key {
            PaletteKey::Esc => {
                self.add_draft.clear();
                self.focus = Focus::List;
            }
            PaletteKey::Enter => self.submit_add(),
            PaletteKey::Backspace => {
                self.add_draft.pop();
            }
            PaletteKey::Char(c) => self.add_draft.push(c),
            PaletteKey::Up | PaletteKey::Down | PaletteKey::Space | PaletteKey::Tab => {}
        }
    }

    fn handle_search_key(&mut self, key: PaletteKey) {
        match key {
            PaletteKey::Esc => {
                if self.query.is_empty() {
                    self.focus = Focus::List;
                } else {
                    self.query.clear();
                    let _ = self.reload();
                }
            }
            PaletteKey::Enter => self.focus = Focus::List,
            PaletteKey::Up => self.move_sel(-1),
            PaletteKey::Down => self.move_sel(1),
            PaletteKey::Backspace => {
                self.query.pop();
                let _ = self.reload();
            }
            PaletteKey::Space => self.query.push(' '),
            PaletteKey::Tab => self.set_filter(self.filter.next()),
            PaletteKey::Char(c) => {
                self.query.push(c);
                self.excerpt = None;
                let _ = self.reload();
            }
        }
    }

    pub fn set_filter(&mut self, filter: BoardFilter) {
        if self.filter == filter {
            self.focus = if filter == BoardFilter::Search {
                Focus::Search
            } else {
                Focus::List
            };
            return;
        }
        self.filter = filter;
        self.excerpt = None;
        self.focus = if filter == BoardFilter::Search {
            Focus::Search
        } else {
            Focus::List
        };
        self.backend.invalidate_since();
        self.items.clear();
        let _ = self.reload();
    }

    fn handle_confirm_key(&mut self, key: PaletteKey) {
        match key {
            PaletteKey::Char('y') | PaletteKey::Char('Y') | PaletteKey::Enter => {
                if let Some(kind) = self.confirm.take() {
                    self.apply_state(kind.state());
                }
            }
            PaletteKey::Esc | PaletteKey::Char('n') | PaletteKey::Char('N') => {
                self.confirm = None;
            }
            _ => {}
        }
    }

    pub fn cycle_detail_tab(&mut self) {
        self.detail_tab = self.detail_tab.next();
        self.refresh_detail();
    }

    pub fn set_detail_tab(&mut self, tab: DetailTab) {
        self.detail_tab = tab;
        self.refresh_detail();
    }

    fn cycle_project(&mut self) {
        if self.projects.is_empty() {
            if let Ok(list) = self.backend.projects() {
                self.projects = list;
            }
        }
        self.project = match self.project.as_deref() {
            None => self.projects.first().cloned(),
            Some(cur) => {
                let i = self.projects.iter().position(|p| p == cur);
                match i {
                    Some(i) if i + 1 < self.projects.len() => Some(self.projects[i + 1].clone()),
                    _ => None,
                }
            }
        };
        self.backend.invalidate_since();
        let _ = self.reload();
    }

    fn cycle_state(&mut self) {
        let Some(state) = self.selected_item().map(|i| i.state.clone()) else {
            return;
        };
        let next = match state.as_str() {
            "TODO" => "STARTED",
            "STARTED" => "BLOCKED",
            "BLOCKED" => "TODO",
            other => {
                self.message = format!("{other}; s cycles TODO/STARTED/BLOCKED");
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
            id,
            state: Some(state.to_string()),
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
            Ok(_) => {
                self.message = format!("opened {id}");
                self.refresh_detail();
            }
            Err(err) => self.message = err.to_string(),
        }
    }

    fn copy_selected(&mut self) {
        let Some(id) = self.selected_id().map(str::to_string) else {
            return;
        };
        self.clipboard = id.clone();
        self.message = format!("copied {id}");
    }

    fn refresh_detail(&mut self) {
        let Some(id) = self.selected_id().map(str::to_string) else {
            self.detail = None;
            self.excerpt = None;
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
                    self.detail_body = excerpt.text.clone();
                    self.excerpt = Some(excerpt);
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
            DetailTab::Notes => match self.backend.get(&id) {
                Ok(detail) => {
                    self.detail_body = format_logbook(&detail.logbook);
                    self.detail = Some(detail);
                }
                Err(err) => self.detail_body = err.to_string(),
            },
        }
    }

    pub fn select_id(&mut self, id: &str) {
        if let Some(pos) = self
            .filtered
            .iter()
            .position(|&i| self.items.get(i).is_some_and(|item| item.id == id))
        {
            self.selected = pos;
            self.refresh_detail();
        }
    }

    pub fn toggle_done(&mut self, id: &str) {
        let current = match self.backend.get(id) {
            Ok(detail) => detail.state,
            Err(err) => {
                self.message = err.to_string();
                return;
            }
        };
        let next = if current == "DONE" { "TODO" } else { "DONE" };
        match self.backend.update(UpdateReq {
            id: id.to_string(),
            state: Some(next.to_string()),
            ..UpdateReq::default()
        }) {
            Ok(result) => {
                self.message = result.report.trim().to_string();
                let _ = self.reload();
            }
            Err(err) => self.message = err.to_string(),
        }
    }

    pub fn set_add_draft(&mut self, text: impl Into<String>) {
        self.add_draft = text.into();
        self.focus = Focus::Add;
    }

    pub fn submit_add(&mut self) {
        let title = self.add_draft.trim().to_string();
        if title.is_empty() {
            return;
        }
        let project = self.selected_item().map(|i| i.project.clone()).or_else(|| {
            self.backend
                .projects()
                .ok()
                .and_then(|p| p.into_iter().next())
        });
        let Some(project) = project else {
            self.message = "no project to add into".into();
            return;
        };
        match ops::create(
            self.backend.layout(),
            &project,
            &title,
            CreateOpts {
                quiet: true,
                ..CreateOpts::default()
            },
        ) {
            Ok(report) => {
                self.message = report.trim().to_string();
                self.add_draft.clear();
                self.focus = Focus::List;
                let _ = self.backend.refresh();
                self.backend.invalidate_since();
                let _ = self.reload();
            }
            Err(err) => self.message = err.to_string(),
        }
    }

    pub fn set_note_draft(&mut self, text: impl Into<String>) {
        self.note_draft = Some(text.into());
        self.focus = Focus::Note;
    }

    pub fn submit_note(&mut self) {
        let Some(text) = self.note_draft.clone() else {
            return;
        };
        self.handle_note_key(PaletteKey::Enter);
        if self.note_draft.is_none() && text.is_empty() {
            self.focus = Focus::List;
        }
    }

    pub fn focus_add(&mut self) {
        self.focus = Focus::Add;
    }

    pub fn focus_search(&mut self) {
        if self.filter != BoardFilter::Search {
            self.set_filter(BoardFilter::Search);
        }
        self.focus = Focus::Search;
    }

    pub fn focus_list(&mut self) {
        self.focus = Focus::List;
    }

    pub fn show_excerpt(&mut self) {
        let Some(id) = self.selected_id().map(str::to_string) else {
            return;
        };
        match self.backend.excerpt(&id) {
            Ok(excerpt) => {
                self.excerpt = Some(excerpt);
                self.detail = self.backend.get(&id).ok();
            }
            Err(err) => self.message = err.to_string(),
        }
    }

    pub fn claim_selected(&mut self) {
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

    pub fn poll_updates(&mut self) {
        let last = match self.backend.live() {
            vissue_tui::BackendKind::Control => self.backend.revision(),
            vissue_tui::BackendKind::Core => self.backend.generation(),
        };
        if let Ok(next) = self.backend.wait(last, 1) {
            if next > last {
                let _ = self.reload();
            }
        }
    }

    pub fn reload(&mut self) -> anyhow::Result<()> {
        let project = self.project.clone();
        let project = project.as_deref();
        match self.filter {
            BoardFilter::Ready => self.reload_ready(project),
            BoardFilter::List => self.reload_list(project),
            BoardFilter::Claims => self.reload_claims(project),
            BoardFilter::Agenda => self.reload_agenda(project),
            BoardFilter::Search => self.reload_search(),
        }
        if !self.query.is_empty() && self.filter != BoardFilter::Search {
            self.filtered = rank_indices(&self.query, &self.items);
        } else {
            self.filtered = (0..self.items.len()).collect();
        }
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
        self.refresh_detail();
        Ok(())
    }

    fn reload_ready(&mut self, project: Option<&str>) {
        // Last full ready page stays when serve answers `{unchanged: true,
        // issues: []}`.
        self.items.retain(|item| item.source == ItemSource::Ready);
        if let Ok(page) = self.backend.ready(project) {
            if !page.unchanged {
                self.items = page.issues.into_iter().map(HudItem::from_row).collect();
                self.ready_count = self.items.len();
            }
        }
    }

    fn reload_list(&mut self, project: Option<&str>) {
        let q = ListQuery {
            project: project.map(str::to_string),
            limit: Some(200),
            ..ListQuery::default()
        };
        if let Ok(page) = self.backend.list(q) {
            if !page.unchanged {
                self.items = page.issues.into_iter().map(HudItem::from_list).collect();
                self.list_count = self.items.len();
            }
        }
    }

    fn reload_claims(&mut self, project: Option<&str>) {
        if let Ok(rows) = self.backend.claims(None, project) {
            self.items = rows.into_iter().map(HudItem::from_claim).collect();
            self.claims_count = self.items.len();
        }
    }

    fn reload_agenda(&mut self, project: Option<&str>) {
        if let Ok(rows) = self.backend.agenda(14, project) {
            self.items = rows.into_iter().map(HudItem::from_agenda).collect();
            self.agenda_count = self.items.len();
        }
    }

    fn reload_search(&mut self) {
        if self.query.is_empty() {
            self.items.clear();
            self.search_count = 0;
            return;
        }
        if let Ok(hits) = self.backend.search(&self.query, 50) {
            self.items = hits.into_iter().map(HudItem::from_search).collect();
            self.search_count = self.items.len();
        }
    }

    fn move_sel(&mut self, delta: i32) {
        if self.filtered.is_empty() {
            return;
        }
        let len = self.filtered.len() as i32;
        let next = (self.selected as i32 + delta).clamp(0, len - 1) as usize;
        if next != self.selected {
            self.selected = next;
            self.refresh_detail();
        }
    }
}

fn format_show(d: &IssueDetail) -> String {
    let mut out = format!(
        "{}  [{}]  [#{}]\n{}\nfile: {}\n",
        d.id, d.state, d.priority, d.title, d.file
    );
    if let Some(parent) = &d.parent {
        out.push_str(&format!("parent: {parent}\n"));
    }
    if !d.blocked_by.is_empty() {
        out.push_str(&format!("blocked by: {}\n", d.blocked_by.join(", ")));
    }
    if let Some(who) = &d.claimed_by {
        out.push_str(&format!("claimed by: {who}\n"));
    }
    if !d.tags.is_empty() {
        out.push_str(&format!("tags: {}\n", d.tags.join(", ")));
    }
    if !d.org_tags.is_empty() {
        out.push_str(&format!("org tags: {}\n", d.org_tags.join(", ")));
    }
    for (k, v) in &d.properties {
        if matches!(k.as_str(), "ID" | "CREATED") {
            continue;
        }
        out.push_str(&format!("{k}: {v}\n"));
    }
    out
}

fn format_tree(node: &vissue_core::views::TreeNode, depth: usize) -> String {
    let pad = "  ".repeat(depth);
    let mut out = format!("{pad}{} [{}] {}\n", node.id, node.state, node.title);
    if !node.blocked_by.is_empty() {
        out.push_str(&format!(
            "{pad}  blocked by: {}\n",
            node.blocked_by.join(", ")
        ));
    }
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
            "{} [{}] {}  {:.2}  {}\n",
            hit.id,
            hit.state,
            hit.title,
            hit.score,
            hit.evidence.join(", ")
        ));
    }
    out
}

fn format_logbook(entries: &[vissue_core::LogEntry]) -> String {
    if entries.is_empty() {
        return "no logbook entries\n".into();
    }
    let mut out = String::new();
    for entry in entries {
        out.push_str(&entry.render());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use vissue_core::config::DEFAULT_PREFIX;
    use vissue_tui::attach::AttachFail;
    use vissue_tui::backend::BackendKind;

    static TOUCHED: AtomicBool = AtomicBool::new(false);

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixture_vault")
    }

    fn copy_tree(src: &Path, dest: &Path) {
        std::fs::create_dir_all(dest).unwrap();
        for entry in std::fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let target = dest.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), &target).unwrap();
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

    fn panic_probe(_: &Path) -> bool {
        TOUCHED.store(true, Ordering::SeqCst);
        panic!("offline must not probe");
    }
    fn panic_ensure(_: &vissue_serve::ServeConfig) -> Result<vissue_serve::EnsureResult, String> {
        TOUCHED.store(true, Ordering::SeqCst);
        panic!("offline must not ensure");
    }
    fn panic_connect(_: &Path, _: &Layout, _: &str) -> Result<Box<dyn BoardBackend>, AttachFail> {
        TOUCHED.store(true, Ordering::SeqCst);
        panic!("offline must not connect");
    }
    fn yes_probe(_: &Path) -> bool {
        true
    }
    fn connect_mismatch(
        _: &Path,
        _: &Layout,
        _: &str,
    ) -> Result<Box<dyn BoardBackend>, AttachFail> {
        Err(AttachFail::Mismatch("other root".into()))
    }

    /// Second `ready` returns `{unchanged: true, issues: []}`, like serve
    /// when `since_revision` matches the catalog head.
    struct UnchangedAfterFirst {
        inner: CoreBackend,
        ready_calls: std::sync::atomic::AtomicUsize,
    }

    impl UnchangedAfterFirst {
        fn new(inner: CoreBackend) -> Self {
            Self {
                inner,
                ready_calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    impl BoardBackend for UnchangedAfterFirst {
        fn layout(&self) -> &Layout {
            self.inner.layout()
        }
        fn generation(&self) -> u64 {
            self.inner.generation()
        }
        fn revision(&self) -> u64 {
            5
        }
        fn live(&self) -> BackendKind {
            BackendKind::Control
        }
        fn identity(&self) -> &str {
            self.inner.identity()
        }
        fn list(
            &self,
            q: vissue_core::views::ListQuery,
        ) -> Result<vissue_tui::ListPage, vissue_core::error::Error> {
            self.inner.list(q)
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
        fn get(
            &self,
            id: &str,
        ) -> Result<vissue_core::views::IssueDetail, vissue_core::error::Error> {
            self.inner.get(id)
        }
        fn excerpt(
            &self,
            id: &str,
        ) -> Result<vissue_core::views::Excerpt, vissue_core::error::Error> {
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
        fn tree(
            &self,
            id: &str,
        ) -> Result<vissue_core::views::TreeNode, vissue_core::error::Error> {
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
        fn projects(&self) -> Result<Vec<String>, vissue_core::error::Error> {
            self.inner.projects()
        }
        fn claim(
            &self,
            id: &str,
            f: bool,
        ) -> Result<vissue_tui::MutResult, vissue_core::error::Error> {
            self.inner.claim(id, f)
        }
        fn note(
            &self,
            id: &str,
            t: &str,
        ) -> Result<vissue_tui::MutResult, vissue_core::error::Error> {
            self.inner.note(id, t)
        }
        fn update(
            &self,
            r: vissue_tui::UpdateReq,
        ) -> Result<vissue_tui::MutResult, vissue_core::error::Error> {
            self.inner.update(r)
        }
        fn open(
            &self,
            id: &str,
        ) -> Result<vissue_core::views::IssueDetail, vissue_core::error::Error> {
            self.inner.open(id)
        }
        fn wait(&self, last: u64, ms: u64) -> Result<u64, vissue_core::error::Error> {
            self.inner.wait(last, ms)
        }
    }

    #[test]
    fn first_paint_lists_ready_ids() {
        let palette =
            Palette::open_core(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap".into()).unwrap();
        let ids: Vec<_> = palette
            .filtered_items()
            .into_iter()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(ids, ["atlas-1a2b", "atlas-2c3d", "beacon-5j6k"]);
        assert!(palette.status_line().contains("serve:offline"));
        assert_eq!(palette.revision(), 0);
    }

    #[test]
    fn enter_shows_excerpt() {
        let (_dir, layout) = writable();
        let mut palette = Palette::open_core(layout, "hud-test".into()).unwrap();
        palette.handle_key(PaletteKey::Down);
        assert_eq!(palette.selected_id(), Some("atlas-2c3d"));
        palette.handle_key(PaletteKey::Enter);
        let text = palette.excerpt().map(|e| e.text.as_str()).unwrap_or("");
        assert!(text.contains("summary"), "{text}");
    }

    #[test]
    fn esc_hides_and_process_state_stays() {
        let palette_layout = Layout::new(fixture_root(), DEFAULT_PREFIX);
        let mut palette = Palette::open_core(palette_layout, "snap".into()).unwrap();
        assert!(palette.visible());
        palette.handle_key(PaletteKey::Esc);
        assert!(!palette.visible());
        assert_eq!(palette.selected_id(), Some("atlas-1a2b"));
    }

    #[test]
    fn offline_attach_never_touches_socket() {
        TOUCHED.store(false, Ordering::SeqCst);
        let mut palette =
            Palette::open_core(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap".into()).unwrap();
        let hooks = AttachHooks {
            probe: panic_probe,
            ensure: panic_ensure,
            connect: panic_connect,
        };
        palette
            .attach(Path::new("/tmp/vissue-hud-offline.sock"), true, &hooks)
            .unwrap();
        assert_eq!(palette.serve_status(), ServeStatus::Offline);
        assert_eq!(palette.backend().live(), BackendKind::Core);
        assert!(!TOUCHED.load(Ordering::SeqCst));
    }

    #[test]
    fn mismatch_stays_core_and_claims_via_ops() {
        let (_dir, layout) = writable();
        let mut palette = Palette::open_core(layout, "hud-test".into()).unwrap();
        let hooks = AttachHooks {
            probe: yes_probe,
            ensure: panic_ensure,
            connect: connect_mismatch,
        };
        palette
            .attach(Path::new("/tmp/vissue-hud-mis.sock"), false, &hooks)
            .unwrap();
        assert_eq!(palette.serve_status(), ServeStatus::Mismatch);
        assert_eq!(palette.backend().live(), BackendKind::Core);
        palette.set_query("atlas-2c3d");
        assert_eq!(palette.selected_id(), Some("atlas-2c3d"));
        palette.claim_selected();
        assert_eq!(
            palette
                .backend()
                .get("atlas-2c3d")
                .unwrap()
                .claimed_by
                .as_deref(),
            Some("hud-test")
        );
        assert_eq!(
            palette.backend().get("atlas-2c3d").unwrap().state,
            "STARTED"
        );
    }

    #[test]
    fn c_claims_and_n_notes_selected() {
        let (_dir, layout) = writable();
        let mut palette = Palette::open_core(layout, "hud-test".into()).unwrap();
        palette.set_query("atlas-2c3d");
        palette.handle_key(PaletteKey::Char('c'));
        assert_eq!(
            palette
                .backend()
                .get("atlas-2c3d")
                .unwrap()
                .claimed_by
                .as_deref(),
            Some("hud-test")
        );
        palette.handle_key(PaletteKey::Char('n'));
        assert!(palette.note_draft().is_some());
        palette.handle_key(PaletteKey::Char('h'));
        palette.handle_key(PaletteKey::Char('i'));
        palette.handle_key(PaletteKey::Enter);
        assert!(palette.note_draft().is_none());
    }

    #[test]
    fn summon_toggle_hides() {
        let mut palette =
            Palette::open_core(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap".into()).unwrap();
        palette.apply_summon(&SummonRequest::new(SummonAction::Toggle));
        assert!(!palette.visible());
        palette.apply_summon(&SummonRequest::new(SummonAction::Show));
        assert!(palette.visible());
        palette.apply_summon(&SummonRequest::new(SummonAction::Hide));
        assert!(!palette.visible());
        palette.handle_key(PaletteKey::Char('c'));
        assert!(!palette.visible());
    }

    #[test]
    fn keys_move_filter_and_backspace() {
        let mut palette =
            Palette::open_core(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap".into()).unwrap();
        assert_eq!(palette.agent(), "snap");
        assert_eq!(palette.generation(), 0);
        assert!(palette.message().is_empty());
        assert!(palette.note_draft().is_none());
        palette.handle_key(PaletteKey::Down);
        assert_eq!(palette.selected_id(), Some("atlas-2c3d"));
        palette.handle_key(PaletteKey::Up);
        assert_eq!(palette.selected_id(), Some("atlas-1a2b"));
        palette.handle_key(PaletteKey::Char('j'));
        assert_eq!(palette.selected_id(), Some("atlas-2c3d"));
        palette.handle_key(PaletteKey::Char('k'));
        assert_eq!(palette.selected_id(), Some("atlas-1a2b"));
        palette.handle_key(PaletteKey::Char('/'));
        palette.handle_key(PaletteKey::Char('z'));
        assert_eq!(palette.query(), "z");
        assert!(palette.filtered_items().is_empty());
        palette.handle_key(PaletteKey::Backspace);
        assert_eq!(palette.query(), "");
        palette.handle_key(PaletteKey::Esc);
        assert_eq!(palette.focus(), Focus::List);
        palette.handle_key(PaletteKey::Char('1'));
        palette.handle_key(PaletteKey::Enter);
        assert!(palette.excerpt().is_some() || !palette.detail_body().is_empty());
        palette.handle_key(PaletteKey::Esc);
        assert!(palette.excerpt().is_none());
        assert!(palette.visible());
        palette.poll_updates();
        let _ = palette.status_line();
    }

    #[test]
    fn note_prompt_esc_and_empty_claim_are_noops() {
        let (_dir, layout) = writable();
        let mut palette = Palette::open_core(layout, "hud-test".into()).unwrap();
        palette.handle_key(PaletteKey::Char('n'));
        assert!(palette.note_draft().is_some());
        palette.handle_key(PaletteKey::Up);
        palette.handle_key(PaletteKey::Down);
        palette.handle_key(PaletteKey::Esc);
        assert!(palette.note_draft().is_none());
        palette.set_query("no-such-id");
        assert!(palette.selected_id().is_none());
        palette.claim_selected();
        palette.show_excerpt();
        palette.handle_key(PaletteKey::Char('n'));
        assert!(palette.note_draft().is_none());
    }

    #[test]
    fn unchanged_ready_does_not_wipe_rows() {
        let backend = UnchangedAfterFirst::new(
            CoreBackend::open(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap").unwrap(),
        );
        let mut palette =
            Palette::with_backend(Box::new(backend), "snap".into(), ServeStatus::Live).unwrap();
        let first: Vec<_> = palette
            .filtered_items()
            .into_iter()
            .map(|i| i.id.clone())
            .collect();
        assert_eq!(first, ["atlas-1a2b", "atlas-2c3d", "beacon-5j6k"]);
        palette.reload().unwrap();
        let second: Vec<_> = palette
            .filtered_items()
            .into_iter()
            .map(|i| i.id.clone())
            .collect();
        assert_eq!(first, second);
        palette.set_query("z");
        palette.set_query("");
        assert_eq!(palette.query(), "");
        let after_backspace: Vec<_> = palette
            .filtered_items()
            .into_iter()
            .map(|i| i.id.clone())
            .collect();
        assert_eq!(first, after_backspace);
    }

    #[test]
    fn digits_switch_sidebar_filters() {
        let mut palette =
            Palette::open_core(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap".into()).unwrap();
        assert_eq!(palette.filter(), BoardFilter::Ready);
        assert!(palette.count(BoardFilter::Ready) >= 3);
        palette.handle_key(PaletteKey::Char('2'));
        assert_eq!(palette.filter(), BoardFilter::List);
        assert!(!palette.filtered_items().is_empty());
        palette.handle_key(PaletteKey::Char('3'));
        assert_eq!(palette.filter(), BoardFilter::Claims);
        palette.handle_key(PaletteKey::Char('1'));
        assert_eq!(palette.filter(), BoardFilter::Ready);
        let ids: Vec<_> = palette
            .filtered_items()
            .into_iter()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(ids, ["atlas-1a2b", "atlas-2c3d", "beacon-5j6k"]);
    }

    #[test]
    fn space_marks_selected_done() {
        let (_dir, layout) = writable();
        let mut palette = Palette::open_core(layout, "hud-test".into()).unwrap();
        palette.set_filter(BoardFilter::List);
        palette.set_query("atlas-2c3d");
        palette.focus_list();
        assert_eq!(palette.selected_id(), Some("atlas-2c3d"));
        palette.handle_key(PaletteKey::Space);
        assert_eq!(palette.backend().get("atlas-2c3d").unwrap().state, "DONE");
        palette.select_id("atlas-2c3d");
        palette.handle_key(PaletteKey::Space);
        assert_eq!(palette.backend().get("atlas-2c3d").unwrap().state, "TODO");
    }

    #[test]
    fn a_then_enter_creates_a_ready_row() {
        let (_dir, layout) = writable();
        let mut palette = Palette::open_core(layout, "hud-test".into()).unwrap();
        palette.handle_key(PaletteKey::Char('a'));
        assert_eq!(palette.focus(), Focus::Add);
        for c in "desk lamp".chars() {
            palette.handle_key(PaletteKey::Char(c));
        }
        palette.handle_key(PaletteKey::Enter);
        assert_eq!(palette.focus(), Focus::List);
        assert!(palette.add_draft().is_empty());
        palette.set_query("desk lamp");
        assert!(
            !palette.detail_body().is_empty(),
            "selected row has a show card"
        );
        assert!(
            palette
                .filtered_items()
                .iter()
                .any(|i| i.title == "desk lamp"),
            "{:?}",
            palette
                .filtered_items()
                .iter()
                .map(|i| i.title.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn s_cycles_todo_started_blocked() {
        let (_dir, layout) = writable();
        let mut palette = Palette::open_core(layout, "hud-test".into()).unwrap();
        palette.set_query("atlas-2c3d");
        palette.focus_list();
        assert_eq!(palette.backend().get("atlas-2c3d").unwrap().state, "TODO");
        palette.handle_key(PaletteKey::Char('s'));
        assert_eq!(
            palette.backend().get("atlas-2c3d").unwrap().state,
            "STARTED"
        );
        palette.handle_key(PaletteKey::Char('s'));
        assert_eq!(
            palette.backend().get("atlas-2c3d").unwrap().state,
            "BLOCKED"
        );
    }

    #[test]
    fn notes_tab_renders_logbook_and_a_submitted_note() {
        let (_dir, layout) = writable();
        let mut palette = Palette::open_core(layout, "hud-test".into()).unwrap();
        assert_eq!(palette.selected_id(), Some("atlas-1a2b"));
        palette.set_detail_tab(DetailTab::Notes);
        assert_eq!(palette.detail_tab(), DetailTab::Notes);
        let body = palette.detail_body();
        assert!(body.contains("[2026-01-14 Wed 09:12]"), "{body}");
        assert!(body.contains(r#"State "STARTED" from "TODO""#), "{body}");
        assert!(body.contains("CLOCK:"), "{body}");

        palette.handle_key(PaletteKey::Char('n'));
        assert!(palette.note_draft().is_some());
        for c in "from the notes tab".chars() {
            palette.handle_key(PaletteKey::Char(c));
        }
        palette.handle_key(PaletteKey::Enter);
        assert!(palette.note_draft().is_none());
        assert_eq!(palette.detail_tab(), DetailTab::Notes);
        let body = palette.detail_body();
        assert!(body.contains("from the notes tab"), "{body}");
        assert!(body.contains(r#"Note: "from the notes tab""#), "{body}");
    }

    #[test]
    fn notes_tab_says_when_the_logbook_is_empty() {
        let (_dir, layout) = writable();
        let mut palette = Palette::open_core(layout, "hud-test".into()).unwrap();
        palette.set_query("atlas-2c3d");
        palette.focus_list();
        palette.set_detail_tab(DetailTab::Notes);
        assert_eq!(palette.detail_body(), "no logbook entries\n");
    }
}
