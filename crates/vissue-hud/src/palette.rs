//! Overlay state: filter, excerpt, claim, note. No iced types.

use std::path::Path;

use vissue_core::config::Layout;
use vissue_core::views::{Excerpt, IssueDetail, IssueRow, ListQuery, SearchHit};
use vissue_tui::CoreBackend;
use vissue_tui::attach::{AttachHooks, AttachOutcome, ServeStatus};
use vissue_tui::backend::{BoardBackend, UpdateReq};

use crate::attach;
use crate::dates::format_org_stamps;
use crate::fuzzy::rank_indices;
use crate::keys::{ActionId, KeyMap};
use crate::summon::{SummonAction, SummonRequest};

const HELP: &str = "\
vissue hud

Home is the project list. Enter opens one.
Esc from a project returns to that list.

j/k, arrows   move
Tab, 1-4      pane (Ready List Claims Agenda)
Enter         open project / cycle detail
p             next project
/             search this project
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
    /// Actionable ready queue.
    Ready,
    /// Title and body search.
    Search,
    /// Full filtered list.
    List,
    /// Open claims.
    Claims,
    /// Deadlines and scheduled dates.
    Agenda,
}

/// Same panes as the terminal board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardFilter {
    /// Actionable ready queue.
    Ready,
    /// Full filtered list.
    List,
    /// Open claims.
    Claims,
    /// Deadlines and scheduled dates.
    Agenda,
    /// Title and body search.
    Search,
}

impl BoardFilter {
    /// Chip order with labels. Search is the search field, not a chip.
    pub const ALL: [(Self, &'static str); 5] = [
        (Self::Ready, "Ready"),
        (Self::List, "List"),
        (Self::Claims, "Claims"),
        (Self::Agenda, "Agenda"),
        (Self::Search, "Search"),
    ];

    /// Visible filter chips. Search lives in the search field.
    pub const CHIPS: [(Self, &'static str); 4] = [
        (Self::Ready, "Ready"),
        (Self::List, "List"),
        (Self::Claims, "Claims"),
        (Self::Agenda, "Agenda"),
    ];

    /// Chip label drawn on the board.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::List => "List",
            Self::Claims => "Claims",
            Self::Agenda => "Agenda",
            Self::Search => "Search",
        }
    }

    /// Next filter in chip order. Search is not in the cycle.
    pub fn next(self) -> Self {
        let i = Self::CHIPS
            .iter()
            .position(|(p, _)| *p == self)
            .unwrap_or(0);
        Self::CHIPS[(i + 1) % Self::CHIPS.len()].0
    }
}

/// Optional pane on the right of the always-visible issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    /// Parent and child tree.
    Tree,
    /// Related-issue hits.
    Related,
    /// Logbook notes.
    Notes,
    /// The working set: the plan above the issue and what its inputs produced.
    Recall,
}

impl DetailTab {
    /// Tab order cycled by Enter on a selected row.
    pub const ALL: [Self; 4] = [Self::Tree, Self::Related, Self::Notes, Self::Recall];

    /// Tab label drawn on the right-hand pane.
    pub fn label(self) -> &'static str {
        match self {
            Self::Tree => "tree",
            Self::Related => "related",
            Self::Notes => "notes",
            Self::Recall => "recall",
        }
    }

    /// Next tab in [`Self::ALL`] order.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// Which field owns typing. List is the default so j/k move rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// Row list (or the home project list).
    List,
    /// Search query field.
    Search,
    /// Add-task field.
    Add,
    /// Logbook note field.
    Note,
    /// Deed accession field.
    Deed,
    /// Home project list.
    Project,
    /// Help overlay.
    Help,
}

/// One selectable palette row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudItem {
    /// Issue id shown in the first column.
    pub id: String,
    /// Heading title.
    pub title: String,
    /// Project name.
    pub project: String,
    /// Org TODO state (`TODO`, `STARTED`, ...).
    pub state: String,
    /// Priority letter (`A`, `B`, or `C`).
    pub priority: String,
    /// Pane that produced this row.
    pub source: ItemSource,
    /// Identity holding the issue, when claimed.
    pub claimed_by: Option<String>,
    /// Agenda date, when the row came from Agenda.
    pub due: Option<String>,
    /// Ids listed in `:BLOCKED_BY:`.
    pub blocked_by: Vec<String>,
    /// Pane-specific suffix: holder, agenda date, or search snippet.
    pub extra: String,
    /// `:PARENT:` id, when set.
    pub parent: Option<String>,
    /// Forest indent; `0` for a root heading.
    pub depth: usize,
}

/// One row on the home project list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectCard {
    /// Project directory name.
    pub name: String,
    /// Ready-queue count for this project.
    pub ready: usize,
    /// Ready count as list meta (`caught up`, `1 ready`, `N ready`).
    pub blurb: String,
}

impl ProjectCard {
    fn new(name: String, ready: usize) -> Self {
        let blurb = match ready {
            0 => "caught up".to_string(),
            1 => "1 ready".to_string(),
            n => format!("{n} ready"),
        };
        Self { name, ready, blurb }
    }
}

/// Home project cards, ready-count descending.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectList {
    cards: Vec<ProjectCard>,
}

impl ProjectList {
    fn from_cards(mut cards: Vec<ProjectCard>) -> Self {
        cards.sort_by(|a, b| b.ready.cmp(&a.ready).then_with(|| a.name.cmp(&b.name)));
        Self { cards }
    }

    fn as_slice(&self) -> &[ProjectCard] {
        &self.cards
    }

    fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    fn len(&self) -> usize {
        self.cards.len()
    }
}

impl icedtea::collection::ListModel for ProjectList {
    fn len(&self) -> usize {
        self.cards.len()
    }

    fn id(&self, index: usize) -> u64 {
        index as u64
    }

    fn title(&self, index: usize) -> &str {
        &self.cards[index].name
    }

    fn meta(&self, index: usize) -> Option<&str> {
        Some(&self.cards[index].blurb)
    }
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
            extra: row.claimed_by.clone().unwrap_or_default(),
            parent: row.parent,
            depth: 0,
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
            parent: None,
            depth: 0,
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
            parent: None,
            depth: 0,
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
            parent: None,
            depth: 0,
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
    /// Printable character, including mapped chords.
    Char(char),
    /// Enter / Return.
    Enter,
    /// Escape.
    Esc,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Backspace.
    Backspace,
    /// Space.
    Space,
    /// Tab.
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
    deed_draft: Option<String>,
    add_draft: String,
    filter: BoardFilter,
    focus: Focus,
    detail_tab: DetailTab,
    detail_body: String,
    /// Issue id last passed to [`Self::fill_detail`]. The header follows this,
    /// not only the list cursor.
    viewing: Option<String>,
    /// When true, keep [`Self::viewing`] through reload and tab changes.
    viewing_held: bool,
    /// Rebuild the Tree tab on the next fill (list select or reload).
    tree_stale: bool,
    issue_tree: Option<IssueTreeNode>,
    tree_ids: Vec<String>,
    tree_focus: Option<String>,
    related_hits: Vec<vissue_core::views::RelatedHit>,
    recall: Option<vissue_core::views::Recall>,
    related_marks: std::collections::BTreeMap<String, (String, bool, bool)>,
    help_md: icedtea::widget::MarkdownDoc,
    project: Option<String>,
    projects: Vec<String>,
    confirm: Option<Confirm>,
    clipboard: String,
    ready_count: usize,
    list_count: usize,
    claims_count: usize,
    agenda_count: usize,
    search_count: usize,
    visible: bool,
    keymap: KeyMap,
    leader_armed: bool,
    leader_at: Option<std::time::Instant>,
    collapsed: std::collections::BTreeSet<String>,
    collapse_seeded: bool,
    project_cards: ProjectList,
    project_sel: usize,
    project_selection: icedtea::collection::Selection,
    project_window: icedtea::collection::VisibleWindow,
    task_window: icedtea::collection::VisibleWindow,
    task_heights: Vec<f32>,
    detail_split: icedtea::layout::SplitState,
    sash_drag: icedtea::layout::SashDrag,
    window_h: f32,
    selectables: icedtea::field::Selectables,
}

impl std::fmt::Debug for Palette {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Palette")
            .field("agent", &self.agent)
            .field("status", &self.status)
            .field("visible", &self.visible)
            .field("filter", &self.filter)
            .field("focus", &self.focus)
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}

/// Close or cancel confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmKind {
    /// Set state to DONE.
    Done,
    /// Set state to CANCELLED.
    Cancelled,
}

impl ConfirmKind {
    /// Org TODO keyword this confirmation applies.
    pub fn state(self) -> &'static str {
        match self {
            Self::Done => "DONE",
            Self::Cancelled => "CANCELLED",
        }
    }
}

/// Pending D/X confirm. The id and painted state are taken when confirm starts.
struct Confirm {
    kind: ConfirmKind,
    id: String,
    if_state: String,
}

impl Palette {
    /// Open a file-backed board and load the Ready pane.
    ///
    /// # Errors
    ///
    /// Returns an error if the vault cannot be parsed.
    pub fn open_core(layout: Layout, agent: String) -> anyhow::Result<Self> {
        let backend = CoreBackend::open(layout, agent.clone())?;
        Self::with_backend(Box::new(backend), agent, ServeStatus::Offline)
    }

    /// Build a board around an existing backend and load the Ready pane.
    ///
    /// # Errors
    ///
    /// Does not return `Err`. Pane load failures stay on the board message.
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
            deed_draft: None,
            add_draft: String::new(),
            filter: BoardFilter::Ready,
            focus: Focus::List,
            detail_tab: DetailTab::Tree,
            detail_body: String::new(),
            viewing: None,
            viewing_held: false,
            tree_stale: false,
            issue_tree: None,
            tree_ids: Vec::new(),
            tree_focus: None,
            related_hits: Vec::new(),
            recall: None,
            related_marks: std::collections::BTreeMap::new(),
            help_md: icedtea::widget::parse(HELP),
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
            keymap: KeyMap::from_defaults(),
            leader_armed: false,
            leader_at: None,
            collapsed: std::collections::BTreeSet::new(),
            collapse_seeded: false,
            project_cards: ProjectList::default(),
            project_sel: 0,
            project_selection: icedtea::collection::Selection::None,
            project_window: icedtea::collection::VisibleWindow::new(480.0),
            task_window: icedtea::collection::VisibleWindow::new(480.0),
            task_heights: Vec::new(),
            detail_split: icedtea::layout::SplitState::new(icedtea::layout::Axis::Vertical, 0.68),
            sash_drag: icedtea::layout::SashDrag::default(),
            window_h: 760.0,
            selectables: icedtea::field::Selectables::new(),
        };
        #[cfg(not(test))]
        {
            palette.keymap = KeyMap::load();
            if let Some(err) = palette.keymap.overlay_error.clone() {
                palette.message = err;
            }
        }
        palette.reload()?;
        Ok(palette)
    }

    /// How the status line labels the current store.
    pub fn serve_status(&self) -> ServeStatus {
        self.status
    }

    /// Identity used for claims and updates.
    pub fn agent(&self) -> &str {
        &self.agent
    }

    /// Whether the overlay window is mapped.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Search query text.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Status-line message, if any.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// On-disk excerpt for the selected issue, if loaded.
    pub fn excerpt(&self) -> Option<&Excerpt> {
        self.excerpt.as_ref()
    }

    /// Last loaded issue detail, if any.
    pub fn detail(&self) -> Option<&IssueDetail> {
        self.detail.as_ref()
    }

    /// Logbook note draft while the note field is open.
    pub fn note_draft(&self) -> Option<&str> {
        self.note_draft.as_deref()
    }

    /// The deed accession being typed, when the field is open.
    pub fn deed_draft(&self) -> Option<&str> {
        self.deed_draft.as_deref()
    }

    /// Add-task draft text.
    pub fn add_draft(&self) -> &str {
        &self.add_draft
    }

    /// Active list filter.
    pub fn filter(&self) -> BoardFilter {
        self.filter
    }

    /// Which field owns typing.
    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// Cached row count for a filter chip.
    pub fn count(&self, filter: BoardFilter) -> usize {
        match filter {
            BoardFilter::Ready => self.ready_count,
            BoardFilter::List => self.list_count,
            BoardFilter::Claims => self.claims_count,
            BoardFilter::Agenda => self.agenda_count,
            BoardFilter::Search => self.search_count,
        }
    }

    /// Active detail card tab.
    pub fn detail_tab(&self) -> DetailTab {
        self.detail_tab
    }

    /// Text drawn in the detail card.
    pub fn detail_body(&self) -> &str {
        &self.detail_body
    }

    /// Open project filter, if any.
    pub fn project(&self) -> Option<&str> {
        self.project.as_deref()
    }

    /// Pending DONE/CANCELLED confirmation.
    pub fn confirm(&self) -> Option<ConfirmKind> {
        self.confirm.as_ref().map(|confirm| confirm.kind)
    }

    /// Last id or URL copied with `y` or a markdown click.
    pub fn clipboard(&self) -> &str {
        &self.clipboard
    }

    /// Store `text` as the clipboard and report it on the status line.
    pub fn set_clipboard(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.message = format!("copied {text}");
        self.clipboard = text;
    }

    /// Issue the detail header paints: the id last loaded into the pane.
    pub fn header_issue(&self) -> Option<HeaderIssue<'_>> {
        let id = self.viewing.as_deref().or_else(|| self.selected_id())?;
        if let Some(item) = self.items.iter().find(|item| item.id == id) {
            return Some(HeaderIssue {
                priority: item.priority.as_str(),
                state: item.state.as_str(),
                title: item.title.as_str(),
                project: item.project.as_str(),
                blocked: !item.blocked_by.is_empty(),
                claimed: item.claimed_by.is_some(),
            });
        }
        let detail = self.detail.as_ref().filter(|d| d.id == id)?;
        Some(HeaderIssue {
            priority: detail.priority.as_str(),
            state: detail.state.as_str(),
            title: detail.title.as_str(),
            project: detail.project.as_str(),
            blocked: !detail.blocked_by.is_empty(),
            claimed: detail.claimed_by.is_some(),
        })
    }

    /// Help overlay body.
    pub fn help_text(&self) -> &'static str {
        HELP
    }

    /// Parsed help markdown. Same source as [`Self::help_text`].
    pub fn help_md(&self) -> &icedtea::widget::MarkdownDoc {
        &self.help_md
    }

    /// Related-tab hits for the focused issue.
    pub fn related_hits(&self) -> &[vissue_core::views::RelatedHit] {
        &self.related_hits
    }

    /// The working set for the painted issue, when the Recall tab loaded one.
    pub fn recall(&self) -> Option<&vissue_core::views::Recall> {
        self.recall.as_ref()
    }

    /// Priority, blocked, and claimed for a related hit, if loaded.
    pub fn related_marks(&self, id: &str) -> Option<(&str, bool, bool)> {
        self.related_marks
            .get(id)
            .map(|(p, b, c)| (p.as_str(), *b, *c))
    }

    /// Label and bind id for each Excerpt field row, in paint order.
    pub fn excerpt_form(&self) -> Vec<(String, String)> {
        let Some(detail) = self.detail.as_ref() else {
            return Vec::new();
        };
        excerpt_form(detail)
            .into_iter()
            .map(|row| (row.id, row.label))
            .collect()
    }

    /// Label gutter that fits the longest Excerpt field name.
    pub fn excerpt_label_width(&self) -> f32 {
        excerpt_columns(&self.detail.as_ref().map(excerpt_form).unwrap_or_default()).0
    }

    /// Pixel width that fits the Excerpt label/value table.
    pub fn excerpt_table_width(&self) -> f32 {
        excerpt_columns(&self.detail.as_ref().map(excerpt_form).unwrap_or_default()).1
    }

    /// Flattened Tree-tab rows, if a tree is loaded.
    pub fn tree_rows(&self) -> Vec<TreeRow<'_>> {
        let Some(root) = &self.issue_tree else {
            return Vec::new();
        };
        let mut out = Vec::new();
        flatten_issue_tree(root, 0, &mut out);
        out
    }

    /// Row in the current pane with this issue id, if loaded.
    pub fn item_by_id(&self, id: &str) -> Option<&HudItem> {
        self.items.iter().find(|item| item.id == id)
    }

    /// Node id for the focused issue, if that node is in the tree.
    pub fn tree_selected(&self) -> Option<u64> {
        let id = self.tree_focus.as_deref().or_else(|| self.selected_id())?;
        self.tree_ids
            .iter()
            .position(|row| row == id)
            .map(|i| i as u64)
    }

    /// Expand or collapse a Tree-tab node.
    pub fn toggle_tree_node(&mut self, id: u64) {
        if let Some(tree) = &mut self.issue_tree {
            toggle_issue_tree(tree, id);
        }
    }

    /// Expand or collapse every node that has children.
    pub fn set_tree_expanded(&mut self, expanded: bool) {
        if let Some(tree) = &mut self.issue_tree {
            set_issue_tree_expanded(tree, expanded);
        }
    }

    /// True when every parent in the loaded tree is expanded.
    pub fn tree_all_expanded(&self) -> bool {
        self.issue_tree.as_ref().is_some_and(tree_all_expanded)
    }

    /// Highlight a Tree-tab node without leaving the current tree.
    pub fn select_tree_node(&mut self, id: u64) {
        let Some(issue) = self.tree_ids.get(id as usize).cloned() else {
            return;
        };
        self.tree_focus = Some(issue.clone());
        self.viewing_held = true;
        self.fill_detail(Some(&issue));
    }

    /// Select `id` as the board row and load it into the issue pane.
    ///
    /// Call this when the user follows a Tree or Related hit. If `id` is
    /// already in the current list, only the selection changes. If it is
    /// not, the board opens that issue's project on List.
    pub fn open_issue(&mut self, id: &str) {
        self.viewing_held = true;
        self.tree_focus = Some(id.to_string());
        if self.selected_id() == Some(id) {
            self.viewing = Some(id.to_string());
            self.refresh_detail();
            return;
        }
        if self
            .filtered
            .iter()
            .any(|&i| self.items.get(i).is_some_and(|item| item.id == id))
        {
            if let Some(pos) = self
                .filtered
                .iter()
                .position(|&i| self.items.get(i).is_some_and(|item| item.id == id))
            {
                self.selected = pos;
                if let Some(p) = self.selected_item().map(|i| i.project.clone()) {
                    self.collapsed.remove(&p);
                }
            }
            self.viewing = Some(id.to_string());
            self.refresh_detail();
            return;
        }
        let project = match self.backend.get(id) {
            Ok(detail) => detail.project,
            Err(err) => {
                self.message = err.to_string();
                return;
            }
        };
        self.project = Some(project);
        self.query.clear();
        self.selected = 0;
        self.collapse_seeded = false;
        self.collapsed.clear();
        self.filter = BoardFilter::List;
        self.focus = Focus::List;
        self.backend.invalidate_since();
        let _ = self.reload();
        if let Some(pos) = self
            .filtered
            .iter()
            .position(|&i| self.items.get(i).is_some_and(|item| item.id == id))
        {
            self.selected = pos;
            if let Some(p) = self.selected_item().map(|i| i.project.clone()) {
                self.collapsed.remove(&p);
            }
            self.viewing = Some(id.to_string());
            self.refresh_detail();
        } else {
            self.fill_detail(Some(id));
        }
    }

    /// Selection the home project list paints.
    pub fn project_selection(&self) -> &icedtea::collection::Selection {
        &self.project_selection
    }

    /// Scroll window the home project list paints.
    pub fn project_window(&self) -> icedtea::collection::VisibleWindow {
        self.project_window
    }

    /// Store the home project list scroll window.
    pub fn set_project_window(&mut self, window: icedtea::collection::VisibleWindow) {
        self.project_window = window;
    }

    /// Scroll window the task board paints.
    pub fn task_window(&self) -> icedtea::collection::VisibleWindow {
        self.task_window
    }

    /// Store the task board scroll window.
    pub fn set_task_window(&mut self, window: icedtea::collection::VisibleWindow) {
        self.task_window = window;
    }

    /// Per-row heights for [`icedtea::widget::virtual_column`].
    pub fn task_heights(&self) -> &[f32] {
        &self.task_heights
    }

    /// Visible task-board rows: group headers, then the open cards.
    pub fn board_rows(&self) -> Vec<BoardRow<'_>> {
        let group = self.project.is_none();
        let searching = !self.query.is_empty();
        let mut out = Vec::new();
        for section in self.sections() {
            if group {
                out.push(BoardRow::Header {
                    project: section.project,
                    count: section.rows.len(),
                    open: !section.collapsed,
                    searching,
                });
            }
            if !section.collapsed || !group {
                for (index, item) in section.rows {
                    out.push(BoardRow::Task { index, item });
                }
            }
        }
        out
    }

    fn refresh_task_list(&mut self) {
        let rows = self.board_rows();
        let heights: Vec<f32> = rows.iter().map(|row| row.height()).collect();
        let cover = rows.iter().position(|row| match row {
            BoardRow::Task { index, .. } => *index == self.selected,
            BoardRow::Header { .. } => false,
        });
        let viewport = self.task_window.viewport.max(1.0);
        let mut scroll = self.task_window.scroll;
        if let Some(i) = cover {
            let top: f32 = heights.iter().take(i).sum();
            let h = heights.get(i).copied().unwrap_or(0.0);
            if top < scroll {
                scroll = top;
            } else if top + h > scroll + viewport {
                scroll = (top + h - viewport).max(0.0);
            }
        }
        self.task_window = icedtea::collection::window_after_scroll_var(
            self.task_window,
            scroll,
            viewport,
            &heights,
            2,
            None,
        );
        self.task_heights = heights;
    }

    /// List/detail split the board paints.
    pub fn detail_split(&self) -> icedtea::layout::SplitState {
        self.detail_split
    }

    /// Height the list/detail split is laid out against.
    pub fn split_total(&self) -> f32 {
        let mut chrome = 14.0 * 2.0 + 36.0 + 40.0 + 20.0;
        if self.note_draft.is_some() {
            chrome += 48.0;
        }
        if self.confirm.is_some() {
            chrome += 36.0;
        }
        if !self.message.is_empty() {
            chrome += 36.0;
        }
        (self.window_h - chrome).max(220.0)
    }

    /// Remember the window height so the sash drag matches the painted split.
    pub fn set_window_height(&mut self, height: f32) {
        self.window_h = height.max(1.0);
    }

    /// Drag the list/detail sash.
    pub fn apply_sash(&mut self, event: icedtea::layout::SashEvent) {
        let total = self.split_total();
        let _ = self.sash_drag.apply(
            &mut self.detail_split,
            event,
            total,
            icedtea::i18n::Direction::Ltr,
        );
    }

    /// Home screen: no project open, Ready/List would otherwise dump the vault.
    pub fn browsing(&self) -> bool {
        self.project.is_none() && matches!(self.filter, BoardFilter::Ready | BoardFilter::List)
    }

    /// Home project cards, ready-count descending.
    pub fn project_cards(&self) -> &[ProjectCard] {
        self.project_cards.as_slice()
    }

    /// Home list model for [`icedtea::widget::list_view`].
    pub fn project_list(&self) -> &ProjectList {
        &self.project_cards
    }

    /// Index into [`Self::project_cards`].
    pub fn selected_project_index(&self) -> usize {
        self.project_sel
    }

    /// Name of the selected home project card, if any.
    pub fn selected_project_name(&self) -> Option<&str> {
        self.project_cards
            .as_slice()
            .get(self.project_sel)
            .map(|c| c.name.as_str())
    }

    /// Open `name` and load its Ready pane.
    pub fn enter_project(&mut self, name: &str) {
        self.project = Some(name.to_string());
        self.query.clear();
        self.selected = 0;
        self.collapse_seeded = false;
        self.collapsed.clear();
        self.filter = BoardFilter::Ready;
        self.focus = Focus::List;
        self.viewing = None;
        self.viewing_held = false;
        self.backend.invalidate_since();
        let _ = self.reload();
    }

    /// Return to the home project list.
    pub fn leave_project(&mut self) {
        self.project = None;
        self.items.clear();
        self.filtered.clear();
        self.query.clear();
        self.filter = BoardFilter::Ready;
        self.focus = Focus::List;
        self.detail = None;
        self.excerpt = None;
        self.detail_body.clear();
        self.viewing = None;
        self.viewing_held = false;
        self.backend.invalidate_since();
        let _ = self.reload();
    }

    /// Project runs over the current filter, for collapsible headers.
    pub fn sections(&self) -> Vec<ProjectSection<'_>> {
        let mut out: Vec<ProjectSection<'_>> = Vec::new();
        for (i, item) in self.filtered_items().into_iter().enumerate() {
            match out.last_mut() {
                Some(sec) if sec.project == item.project => {
                    sec.end = i + 1;
                    sec.rows.push((i, item));
                }
                _ => out.push(ProjectSection {
                    project: item.project.as_str(),
                    start: i,
                    end: i + 1,
                    collapsed: self.collapsed.contains(&item.project),
                    rows: vec![(i, item)],
                }),
            }
        }
        out
    }

    /// Collapse or expand `project` in the current filter.
    pub fn toggle_project(&mut self, project: &str) {
        if !self.collapsed.remove(project) {
            self.collapsed.insert(project.to_string());
        }
        self.refresh_task_list();
    }

    fn seed_collapse(&mut self) {
        if self.collapse_seeded {
            return;
        }
        let open = self
            .selected_item()
            .map(|i| i.project.clone())
            .or_else(|| self.items.first().map(|i| i.project.clone()));
        let mut seen = std::collections::BTreeSet::new();
        for item in &self.items {
            seen.insert(item.project.clone());
        }
        if seen.len() <= 1 {
            self.collapse_seeded = true;
            return;
        }
        for p in seen {
            if open.as_deref() != Some(p.as_str()) {
                self.collapsed.insert(p);
            }
        }
        self.collapse_seeded = true;
    }

    /// The store this board is talking to.
    pub fn backend(&self) -> &dyn BoardBackend {
        self.backend.as_ref()
    }

    /// Catalog generation from the current backend.
    pub fn generation(&self) -> u64 {
        self.backend.generation()
    }

    /// Serve revision from the current backend. Core is always 0.
    pub fn revision(&self) -> u64 {
        self.backend.revision()
    }

    /// Rows that match the current query, in display order.
    pub fn filtered_items(&self) -> Vec<&HudItem> {
        self.filtered
            .iter()
            .filter_map(|&i| self.items.get(i))
            .collect()
    }

    /// Selected row, if the filter is not empty.
    pub fn selected_item(&self) -> Option<&HudItem> {
        self.filtered
            .get(self.selected)
            .and_then(|&i| self.items.get(i))
    }

    /// Id of the selected row, if the filter is not empty.
    pub fn selected_id(&self) -> Option<&str> {
        self.selected_item().map(|i| i.id.as_str())
    }

    /// Issue the board is acting on: a held Tree or Related view, else the list cursor.
    pub fn painted_id(&self) -> Option<&str> {
        if self.viewing_held {
            self.viewing.as_deref().or_else(|| self.selected_id())
        } else {
            self.selected_id()
        }
    }

    /// Index into [`Self::filtered_items`].
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// One-line `serve:` / gen / rev / agent / message summary.
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
    ///
    /// # Errors
    ///
    /// Does not return `Err`. Pane load failures stay on the board message.
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

    /// Replace the search query and reload matching rows.
    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.excerpt = None;
        let _ = self.reload();
    }

    /// Search field text. An empty home search returns to the project list.
    pub fn type_query(&mut self, query: impl Into<String>) {
        let query = query.into();
        if query.is_empty() && self.project.is_none() && self.filter == BoardFilter::Search {
            self.query.clear();
            self.set_filter(BoardFilter::Ready);
            return;
        }
        if self.browsing() && !query.is_empty() {
            self.focus_search();
        } else {
            self.focus_query();
        }
        self.set_query(query);
    }

    fn painted_state(&self, id: &str) -> Option<&str> {
        if let Some(item) = self.item_by_id(id) {
            return Some(item.state.as_str());
        }
        self.detail
            .as_ref()
            .filter(|detail| detail.id == id)
            .map(|detail| detail.state.as_str())
    }

    /// Map the overlay window.
    pub fn show(&mut self) {
        self.visible = true;
    }

    /// Unmap the overlay and drop drafts.
    pub fn hide(&mut self) {
        self.note_draft = None;
        self.deed_draft = None;
        self.add_draft.clear();
        self.confirm = None;
        self.focus = Focus::List;
        self.visible = false;
    }

    /// Invert mapped state.
    pub fn toggle(&mut self) {
        if self.visible {
            self.hide();
        } else {
            self.show();
        }
    }

    /// Apply a compositor summon verb.
    pub fn apply_summon(&mut self, req: &SummonRequest) {
        match req.action {
            SummonAction::Show => self.show(),
            SummonAction::Hide => self.hide(),
            SummonAction::Toggle => self.toggle(),
        }
    }

    /// Dispatch one overlay key. Ignored while hidden.
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
        if self.focus == Focus::Deed || self.deed_draft.is_some() {
            self.handle_deed_key(key);
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
        if self.leader_armed
            && let Some(at) = self.leader_at
            && at.elapsed().as_millis() as u64 > self.keymap.leader_timeout_ms
        {
            self.leader_armed = false;
        }
        if self.leader_armed {
            self.leader_armed = false;
            match key {
                PaletteKey::Esc => return,
                PaletteKey::Char(c) => {
                    if let Some(action) = self.keymap.get(&format!("leader+{c}")) {
                        self.dispatch(action);
                    }
                    return;
                }
                _ => {}
            }
        }
        if let PaletteKey::Char(c) = key
            && self.keymap.leader == Some(c)
        {
            self.leader_armed = true;
            self.leader_at = Some(std::time::Instant::now());
            return;
        }
        match key {
            PaletteKey::Esc => {
                if self.project.is_some() {
                    self.leave_project();
                } else if !matches!(self.filter, BoardFilter::Ready) {
                    self.set_filter(BoardFilter::Ready);
                } else {
                    self.hide();
                }
            }
            PaletteKey::Enter => self.dispatch(ActionId::DetailCycle),
            PaletteKey::Tab => self.dispatch(ActionId::PaneNext),
            PaletteKey::Up => self.move_sel(-1),
            PaletteKey::Down => self.move_sel(1),
            PaletteKey::Backspace => {}
            PaletteKey::Space => self.dispatch(ActionId::ListDone),
            PaletteKey::Char(c) => {
                if let Some(action) = self.keymap.get(&c.to_string()) {
                    self.dispatch(action);
                }
            }
        }
    }

    fn dispatch(&mut self, action: ActionId) {
        match action {
            ActionId::ListDown => self.move_sel(1),
            ActionId::ListUp => self.move_sel(-1),
            ActionId::ListSelect | ActionId::DetailCycle => {
                if self.browsing() {
                    if let Some(name) = self.selected_project_name().map(str::to_string) {
                        self.enter_project(&name);
                    }
                } else {
                    self.cycle_detail_tab();
                }
            }
            ActionId::ListDone => {
                if let Some(id) = self.painted_id().map(str::to_string) {
                    self.toggle_done(&id);
                }
            }
            ActionId::PaneReady => self.set_filter(BoardFilter::Ready),
            ActionId::PaneList => {
                if self.browsing()
                    && let Some(name) = self.selected_project_name().map(str::to_string)
                {
                    self.enter_project(&name);
                }
                self.set_filter(BoardFilter::List);
            }
            ActionId::PaneClaims => self.set_filter(BoardFilter::Claims),
            ActionId::PaneAgenda => self.set_filter(BoardFilter::Agenda),
            ActionId::PaneSearch => self.set_filter(BoardFilter::Search),
            ActionId::PaneNext => {
                if self.browsing() {
                    self.set_filter(BoardFilter::Claims);
                } else {
                    self.set_filter(self.filter.next());
                }
            }
            ActionId::ProjectCycle => self.cycle_project(),
            ActionId::Search => {
                if self.project.is_some() {
                    self.focus_query();
                } else {
                    self.set_filter(BoardFilter::Search);
                    self.focus_search();
                }
            }
            ActionId::Add => {
                if self.browsing()
                    && let Some(name) = self.selected_project_name().map(str::to_string)
                {
                    self.enter_project(&name);
                }
                self.focus_add();
            }
            ActionId::Claim => self.claim_selected(),
            ActionId::Deed => {
                if self.painted_id().is_some() {
                    // Onto the tab the citation will show up on, so the person
                    // typing sees it land.
                    self.detail_tab = DetailTab::Recall;
                    self.refresh_detail();
                    self.deed_draft = Some(String::new());
                    self.focus = Focus::Deed;
                }
            }
            ActionId::Note => {
                if self.painted_id().is_some() {
                    self.detail_tab = DetailTab::Notes;
                    self.refresh_detail();
                    self.note_draft = Some(String::new());
                    self.focus = Focus::Note;
                }
            }
            ActionId::StateCycle => self.cycle_state(),
            ActionId::ConfirmDone => self.start_confirm(ConfirmKind::Done),
            ActionId::ConfirmCancel => self.start_confirm(ConfirmKind::Cancelled),
            ActionId::Open => self.open_selected(),
            ActionId::CopyId => self.copy_selected(),
            ActionId::Reload => {
                self.backend.invalidate_since();
                let _ = self.reload();
            }
            ActionId::Help => self.focus = Focus::Help,
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
                if let Some(id) = self.painted_id().map(str::to_string) {
                    match self.backend.note(&id, &text) {
                        Ok(result) => {
                            self.message = result.report.trim().to_string();
                            self.focus = Focus::List;
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

    fn handle_deed_key(&mut self, key: PaletteKey) {
        let Some(mut text) = self.deed_draft.take() else {
            return;
        };
        match key {
            PaletteKey::Esc => {
                self.message.clear();
                self.focus = Focus::List;
            }
            PaletteKey::Enter => {
                if let Some(id) = self.painted_id().map(str::to_string) {
                    let accession = text.trim().to_string();
                    if accession.is_empty() {
                        self.message = "no deed cited".to_string();
                        self.focus = Focus::List;
                    } else {
                        match self.backend.deed(&id, &[accession]) {
                            Ok(result) => {
                                self.message = result.report.trim().to_string();
                                self.focus = Focus::List;
                                let _ = self.reload();
                            }
                            // The field stays open holding what was typed. A
                            // refused accession is usually a typo in a long
                            // hash, and clearing it would cost the whole line.
                            Err(err) => {
                                self.message = err.to_string();
                                self.deed_draft = Some(text);
                                self.focus = Focus::Deed;
                            }
                        }
                    }
                }
            }
            PaletteKey::Backspace => {
                text.pop();
                self.deed_draft = Some(text);
            }
            PaletteKey::Char(c) => {
                text.push(c);
                self.deed_draft = Some(text);
            }
            PaletteKey::Up | PaletteKey::Down | PaletteKey::Space | PaletteKey::Tab => {
                self.deed_draft = Some(text);
            }
        }
    }

    /// Open the deed field on the selected issue.
    pub fn focus_deed(&mut self) {
        if self.painted_id().is_some() {
            self.detail_tab = DetailTab::Recall;
            self.refresh_detail();
            self.deed_draft = Some(String::new());
            self.focus = Focus::Deed;
        }
    }

    /// Cite the deed draft on the selected issue.
    pub fn submit_deed(&mut self) {
        self.handle_deed_key(PaletteKey::Enter);
    }

    /// Replace the deed draft, for a text field that owns its own buffer.
    pub fn set_deed_draft(&mut self, text: impl Into<String>) {
        self.deed_draft = Some(text.into());
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

    /// Switch the list filter and reload rows.
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

    fn start_confirm(&mut self, kind: ConfirmKind) {
        let Some(id) = self.painted_id().map(str::to_string) else {
            return;
        };
        let Some(if_state) = self.painted_state(&id).map(str::to_string) else {
            return;
        };
        match (kind, if_state.as_str()) {
            (ConfirmKind::Done, "CANCELLED") | (ConfirmKind::Cancelled, "DONE") => {
                self.message = format!("{if_state}; D/X confirm a live heading");
                return;
            }
            (ConfirmKind::Done, "DONE") | (ConfirmKind::Cancelled, "CANCELLED") => return,
            _ => {}
        }
        self.confirm = Some(Confirm { kind, id, if_state });
    }

    fn handle_confirm_key(&mut self, key: PaletteKey) {
        match key {
            PaletteKey::Char('y') | PaletteKey::Char('Y') | PaletteKey::Enter => {
                if let Some(confirm) = self.confirm.take() {
                    self.apply_confirm(confirm);
                }
            }
            PaletteKey::Esc | PaletteKey::Char('n') | PaletteKey::Char('N') => {
                self.confirm = None;
            }
            _ => {}
        }
    }

    fn apply_confirm(&mut self, confirm: Confirm) {
        match self.backend.update(UpdateReq {
            id: confirm.id,
            state: Some(confirm.kind.state().to_string()),
            if_state: Some(confirm.if_state),
            ..UpdateReq::default()
        }) {
            Ok(result) => {
                self.message = result.report.trim().to_string();
                let _ = self.reload();
            }
            Err(err) => self.message = err.to_string(),
        }
    }

    /// Advance the detail card to the next tab.
    pub fn cycle_detail_tab(&mut self) {
        self.detail_tab = self.detail_tab.next();
        self.refresh_detail();
    }

    /// Open `tab` on the detail card.
    pub fn set_detail_tab(&mut self, tab: DetailTab) {
        self.detail_tab = tab;
        self.refresh_detail();
    }

    fn cycle_project(&mut self) {
        if self.projects.is_empty()
            && let Ok(list) = self.backend.projects()
        {
            self.projects = list;
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
        let Some(id) = self.painted_id().map(str::to_string) else {
            return;
        };
        let Some(state) = self.painted_state(&id).map(str::to_string) else {
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
        let Some(id) = self.painted_id().map(str::to_string) else {
            return;
        };
        match self.backend.update(UpdateReq {
            id: id.clone(),
            state: Some(state.to_string()),
            if_state: self.painted_state(&id).map(str::to_string),
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
        let Some(id) = self.painted_id().map(str::to_string) else {
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
        let Some(id) = self.painted_id().map(str::to_string) else {
            return;
        };
        self.set_clipboard(id);
    }

    fn refresh_detail(&mut self) {
        let id = if self.viewing_held {
            self.viewing.clone()
        } else {
            self.selected_id().map(str::to_string)
        };
        self.fill_detail(id.as_deref());
        if !self
            .tree_ids
            .iter()
            .any(|row| Some(row.as_str()) == self.tree_focus.as_deref())
        {
            self.tree_focus = id;
        }
    }

    fn follow_selection(&mut self) {
        self.viewing_held = false;
        self.tree_stale = true;
        self.viewing = self.selected_id().map(str::to_string);
        self.refresh_detail();
    }

    fn fill_detail(&mut self, id: Option<&str>) {
        self.viewing = id.map(str::to_string);
        let Some(id) = id else {
            self.detail = None;
            self.excerpt = None;
            self.detail_body.clear();
            self.issue_tree = None;
            self.tree_ids.clear();
            self.related_hits.clear();
            self.related_marks.clear();
            self.recall = None;
            self.recall = None;
            return;
        };
        match self.backend.excerpt(id) {
            Ok(excerpt) => {
                self.detail_body = excerpt.text.clone();
                self.excerpt = Some(excerpt);
                match self.backend.get(id) {
                    Ok(detail) => self.detail = Some(detail),
                    Err(err) => self.detail_body = err.to_string(),
                }
            }
            Err(err) => {
                self.excerpt = None;
                self.detail_body = err.to_string();
                self.detail = self.backend.get(id).ok();
            }
        }
        match self.detail_tab {
            DetailTab::Tree => {
                let keep = !self.tree_stale && self.tree_ids.iter().any(|row| row == id);
                self.tree_stale = false;
                if !keep {
                    self.issue_tree = None;
                    self.tree_ids.clear();
                    match self.backend.tree(id) {
                        Ok(node) => {
                            let mut ids = Vec::new();
                            self.issue_tree = Some(issue_tree_from(&node, &mut ids));
                            self.tree_ids = ids;
                        }
                        Err(err) => self.message = err.to_string(),
                    }
                }
            }
            DetailTab::Related => {
                self.related_hits.clear();
                self.related_marks.clear();
                match self.backend.related(id, 2, 20) {
                    Ok(hits) => {
                        for hit in &hits {
                            if let Ok(detail) = self.backend.get(&hit.id) {
                                self.related_marks.insert(
                                    hit.id.clone(),
                                    (
                                        detail.priority,
                                        !detail.blocked_by.is_empty(),
                                        detail.claimed_by.is_some(),
                                    ),
                                );
                            }
                        }
                        self.related_hits = hits;
                    }
                    Err(err) => self.message = err.to_string(),
                }
            }
            DetailTab::Recall => {
                self.recall = None;
                match self.backend.recall(id, 1) {
                    Ok(set) => self.recall = Some(set),
                    Err(err) => self.message = err.to_string(),
                }
            }
            DetailTab::Notes => {}
        }
        self.bind_selectables();
    }

    /// Drag-select buffer for a detail field, if bound.
    pub fn selectable(&self, id: &str) -> Option<&iced::widget::text_editor::Content> {
        self.selectables.get(id)
    }

    /// Apply a select-only editor action to a bound field.
    pub fn perform_select(&mut self, id: &str, action: iced::widget::text_editor::Action) {
        self.selectables.perform(id, action);
    }

    fn bind_selectables(&mut self) {
        self.selectables.retain(|id| {
            id.starts_with("excerpt-") || id.starts_with("note-") || id.starts_with("clock-")
        });
        let Some(detail) = self.detail.as_ref() else {
            return;
        };
        for row in excerpt_form(detail) {
            self.selectables.bind(row.id, row.value);
        }
        self.selectables.bind("excerpt-body", detail.body.trim());
        for (i, entry) in detail.logbook.iter().enumerate() {
            if let Some(note) = &entry.note {
                self.selectables.bind(format!("note-{i}"), note);
            }
            if let Some(raw) = &entry.raw {
                self.selectables.bind(format!("clock-{i}"), clock_text(raw));
            }
        }
    }

    /// Select the filtered row with this issue id, if present.
    pub fn select_id(&mut self, id: &str) {
        if let Some(pos) = self
            .filtered
            .iter()
            .position(|&i| self.items.get(i).is_some_and(|item| item.id == id))
        {
            self.selected = pos;
            if let Some(p) = self.selected_item().map(|i| i.project.clone()) {
                self.collapsed.remove(&p);
            }
            self.follow_selection();
            self.refresh_task_list();
        }
    }

    /// Flip `id` between DONE and TODO.
    pub fn toggle_done(&mut self, id: &str) {
        let Some(current) = self.painted_state(id).map(str::to_string) else {
            self.message = format!("{id} is not on the board");
            return;
        };
        let next = match current.as_str() {
            "TODO" => "DONE",
            "DONE" => "TODO",
            other => {
                self.message = format!("{other}; space toggles TODO/DONE");
                return;
            }
        };
        if let Some(pos) = self
            .filtered
            .iter()
            .position(|&i| self.items.get(i).is_some_and(|item| item.id == id))
        {
            self.selected = pos;
            if let Some(p) = self.selected_item().map(|i| i.project.clone()) {
                self.collapsed.remove(&p);
            }
        }
        self.viewing = Some(id.to_string());
        self.viewing_held = true;
        match self.backend.update(UpdateReq {
            id: id.to_string(),
            state: Some(next.to_string()),
            if_state: Some(current),
            ..UpdateReq::default()
        }) {
            Ok(result) => {
                self.message = result.report.trim().to_string();
                if !self.viewing_held {
                    self.tree_stale = true;
                }
                let _ = self.reload();
            }
            Err(err) => self.message = err.to_string(),
        }
    }

    /// Replace the add-task draft and focus the add field.
    pub fn set_add_draft(&mut self, text: impl Into<String>) {
        self.add_draft = text.into();
        self.focus = Focus::Add;
    }

    /// Create an issue from the add-task draft in the current project.
    pub fn submit_add(&mut self) {
        let title = self.add_draft.trim().to_string();
        if title.is_empty() {
            return;
        }
        let project = self.project.clone().or_else(|| {
            if self.browsing() {
                self.selected_project_name().map(str::to_string)
            } else {
                self.selected_item().map(|i| i.project.clone())
            }
        });
        let Some(project) = project else {
            self.message = "no project to add into".into();
            return;
        };
        match self.backend.create(&project, &title) {
            Ok(result) => {
                self.message = result.report.trim().to_string();
                self.add_draft.clear();
                self.focus = Focus::List;
                self.backend.invalidate_since();
                let _ = self.reload();
            }
            Err(err) => self.message = err.to_string(),
        }
    }

    /// Replace the logbook note draft and focus the note field.
    pub fn set_note_draft(&mut self, text: impl Into<String>) {
        self.note_draft = Some(text.into());
        self.focus = Focus::Note;
    }

    /// Append the note draft to the selected issue.
    pub fn submit_note(&mut self) {
        let Some(text) = self.note_draft.clone() else {
            return;
        };
        self.handle_note_key(PaletteKey::Enter);
        if self.note_draft.is_none() && text.is_empty() {
            self.focus = Focus::List;
        }
    }

    /// Focus the add-task field, entering the selected project from home.
    pub fn focus_add(&mut self) {
        if self.browsing()
            && let Some(name) = self.selected_project_name().map(str::to_string)
        {
            self.enter_project(&name);
        }
        self.focus = Focus::Add;
    }

    /// Switch to Search and focus the query field.
    pub fn focus_search(&mut self) {
        if self.filter != BoardFilter::Search {
            self.set_filter(BoardFilter::Search);
        }
        self.focus = Focus::Search;
    }

    /// Put typing in the query field without changing the pane.
    pub fn focus_query(&mut self) {
        self.focus = Focus::Search;
    }

    /// Return typing to the row list.
    pub fn focus_list(&mut self) {
        self.focus = Focus::List;
    }

    /// Load the on-disk excerpt for the painted issue.
    pub fn show_excerpt(&mut self) {
        let Some(id) = self.painted_id().map(str::to_string) else {
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

    /// Claim the painted issue for [`Self::agent`].
    pub fn claim_selected(&mut self) {
        let Some(id) = self.painted_id().map(str::to_string) else {
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

    /// Reload when serve revision or core generation advances.
    ///
    /// Peek only: a positive wait would sleep on the frame thread.
    pub fn poll_updates(&mut self) {
        let last = match self.backend.live() {
            vissue_tui::BackendKind::Control => self.backend.revision(),
            vissue_tui::BackendKind::Core => self.backend.generation(),
        };
        if let Ok(next) = self.backend.wait(last, 0)
            && next > last
        {
            let _ = self.reload();
        }
    }

    /// Fetch the current filter from the backend and refresh detail.
    ///
    /// # Errors
    ///
    /// Does not return `Err`. Backend pane failures stay on the board message.
    pub fn reload(&mut self) -> anyhow::Result<()> {
        if self.browsing() {
            self.reload_browser();
            self.refresh_chip_counts(None);
            self.items.clear();
            self.filtered.clear();
            self.selected = 0;
            self.detail = None;
            self.excerpt = None;
            self.detail_body.clear();
            self.issue_tree = None;
            self.tree_ids.clear();
            self.tree_focus = None;
            self.related_hits.clear();
            self.related_marks.clear();
            self.recall = None;
            self.refresh_task_list();
            return Ok(());
        }
        let project = self.project.clone();
        let project = project.as_deref();
        let keep = self.selected_id().map(str::to_string);
        match self.filter {
            BoardFilter::Ready => self.reload_ready(project),
            BoardFilter::List => self.reload_list(project),
            BoardFilter::Claims => self.reload_claims(project),
            BoardFilter::Agenda => self.reload_agenda(project),
            BoardFilter::Search => self.reload_search(),
        }
        if matches!(
            self.filter,
            BoardFilter::Ready | BoardFilter::List | BoardFilter::Search
        ) {
            self.items = apply_forest(std::mem::take(&mut self.items));
        }
        if !self.query.is_empty() && self.filter != BoardFilter::Search {
            self.filtered = rank_indices(&self.query, &self.items);
        } else {
            self.filtered = (0..self.items.len()).collect();
        }
        self.selected = match keep.as_deref() {
            Some(id) => self
                .filtered
                .iter()
                .position(|&i| self.items.get(i).is_some_and(|item| item.id == id))
                .unwrap_or_else(|| self.filtered.len().saturating_sub(1)),
            None => 0,
        };
        self.seed_collapse();
        if let Some(p) = self.selected_item().map(|i| i.project.clone()) {
            self.collapsed.remove(&p);
        }
        self.refresh_chip_counts(project);
        if !self.viewing_held {
            self.tree_stale = true;
        }
        self.refresh_detail();
        self.refresh_task_list();
        Ok(())
    }

    fn reload_browser(&mut self) {
        if let Ok(list) = self.backend.projects() {
            self.projects = list;
        }
        let mut counts: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        if let Ok(page) = self.backend.ready(None)
            && !page.unchanged
        {
            for row in &page.issues {
                *counts.entry(row.project.clone()).or_default() += 1;
            }
            self.ready_count = page.issues.len();
        }
        let cards: Vec<ProjectCard> = self
            .projects
            .iter()
            .map(|name| ProjectCard::new(name.clone(), counts.get(name).copied().unwrap_or(0)))
            .collect();
        self.project_cards = ProjectList::from_cards(cards);
        if self.project_sel >= self.project_cards.len() {
            self.project_sel = self.project_cards.len().saturating_sub(1);
        }
        self.sync_project_selection();
    }

    fn sync_project_selection(&mut self) {
        self.project_selection = if self.project_cards.is_empty() {
            icedtea::collection::Selection::None
        } else {
            icedtea::collection::Selection::Single(self.project_sel)
        };
    }

    fn refresh_chip_counts(&mut self, project: Option<&str>) {
        if let Ok(rows) = self.backend.claims(None, project) {
            self.claims_count = rows.len();
        }
        if let Ok(rows) = self.backend.agenda(14, project) {
            self.agenda_count = rows.len();
        }
    }

    fn reload_ready(&mut self, project: Option<&str>) {
        // Last full ready page stays when serve answers `{unchanged: true,
        // issues: []}`.
        self.items.retain(|item| item.source == ItemSource::Ready);
        if let Ok(page) = self.backend.ready(project)
            && !page.unchanged
        {
            self.items = page.issues.into_iter().map(HudItem::from_row).collect();
            self.ready_count = self.items.len();
        }
    }

    fn reload_list(&mut self, project: Option<&str>) {
        let q = ListQuery {
            project: project.map(str::to_string),
            limit: Some(200),
            ..ListQuery::default()
        };
        if let Ok(page) = self.backend.list(q)
            && !page.unchanged
        {
            self.items = page.issues.into_iter().map(HudItem::from_list).collect();
            self.list_count = self.items.len();
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
        let fetch = if self.project.is_some() { 200 } else { 50 };
        if let Ok(hits) = self.backend.search(&self.query, fetch) {
            let hits = match self.project.as_deref() {
                Some(project) => hits
                    .into_iter()
                    .filter(|hit| hit.project == project)
                    .take(50)
                    .collect(),
                None => hits,
            };
            self.items = hits.into_iter().map(HudItem::from_search).collect();
            self.search_count = self.items.len();
            self.attach_search_ancestors();
        }
    }

    fn attach_search_ancestors(&mut self) {
        let q = ListQuery {
            project: self.project.clone(),
            limit: Some(400),
            ..ListQuery::default()
        };
        let Ok(page) = self.backend.list(q) else {
            return;
        };
        let by_id: std::collections::HashMap<&str, &IssueRow> = page
            .issues
            .iter()
            .map(|row| (row.id.as_str(), row))
            .collect();
        for item in &mut self.items {
            if let Some(row) = by_id.get(item.id.as_str()) {
                item.parent = row.parent.clone();
                item.blocked_by = row.blocked_by.clone();
                item.claimed_by = row.claimed_by.clone();
            }
        }
        let mut want: std::collections::HashSet<String> =
            self.items.iter().map(|item| item.id.clone()).collect();
        let mut grew = true;
        while grew {
            grew = false;
            let snapshot: Vec<String> = want.iter().cloned().collect();
            for id in snapshot {
                if let Some(row) = by_id.get(id.as_str())
                    && let Some(parent) = row.parent.as_deref()
                    && want.insert(parent.to_string())
                {
                    grew = true;
                }
            }
        }
        for id in &want {
            if self.items.iter().any(|item| item.id == *id) {
                continue;
            }
            if let Some(row) = by_id.get(id.as_str()) {
                let mut item = HudItem::from_row((*row).clone());
                item.source = ItemSource::Search;
                self.items.push(item);
            }
        }
    }

    fn move_sel(&mut self, delta: i32) {
        if self.browsing() {
            if self.project_cards.is_empty() {
                return;
            }
            let next = (self.project_sel as i32 + delta)
                .clamp(0, self.project_cards.len() as i32 - 1) as usize;
            self.project_sel = next;
            self.sync_project_selection();
            return;
        }
        let vis = self.visible_indices();
        if vis.is_empty() {
            return;
        }
        let pos = vis.iter().position(|&i| i == self.selected).unwrap_or(0) as i32;
        let next = (pos + delta).clamp(0, vis.len() as i32 - 1) as usize;
        let next = vis[next];
        if next != self.selected {
            self.selected = next;
            self.follow_selection();
            self.refresh_task_list();
        }
    }

    fn visible_indices(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for sec in self.sections() {
            if sec.collapsed {
                continue;
            }
            out.extend(sec.start..sec.end);
        }
        out
    }
}

fn excerpt_columns(fields: &[ExcerptField]) -> (f32, f32) {
    let label_n = fields
        .iter()
        .map(|row| row.label.chars().count())
        .max()
        .unwrap_or(4);
    let value_n = fields
        .iter()
        .map(|row| row.value.chars().count())
        .max()
        .unwrap_or(8);
    let label_w = (label_n as f32 * 7.0 + 8.0).clamp(40.0, 88.0);
    let value_w = (value_n as f32 * 8.0).clamp(56.0, 168.0);
    (label_w, label_w + 8.0 + value_w)
}

fn field_label(key: &str) -> &str {
    match key {
        "DEADLINE" => "Due",
        "SCHEDULED" => "Scheduled",
        "CLOSED" => "Closed",
        "CREATED" => "Created",
        "CLAIMED_AT" => "Claimed",
        "CLAIMED_BY" => "Holder",
        "TYPE" => "Type",
        "PARENT" => "Parent",
        other => other,
    }
}

fn clock_text(raw: &str) -> String {
    format_org_stamps(raw)
        .trim()
        .trim_start_matches("CLOCK:")
        .trim()
        .to_string()
}

struct ExcerptField {
    id: String,
    label: String,
    value: String,
}

fn excerpt_form(d: &IssueDetail) -> Vec<ExcerptField> {
    let mut rows = Vec::new();
    let mut push = |key: &str, label: &str, value: String| {
        if value.is_empty() {
            return;
        }
        rows.push(ExcerptField {
            id: format!("excerpt-{key}"),
            label: label.to_string(),
            value,
        });
    };
    push("id", "Id", d.id.clone());
    for key in ["CLOSED", "SCHEDULED", "DEADLINE"] {
        if let Some(value) = d.properties.get(key) {
            push(key, field_label(key), format_org_stamps(value));
        }
    }
    for (key, value) in &d.properties {
        if matches!(key.as_str(), "ID" | "CLOSED" | "SCHEDULED" | "DEADLINE") {
            continue;
        }
        push(key, field_label(key), format_org_stamps(value));
    }
    let mut tags = d.tags.clone();
    for t in &d.org_tags {
        if !tags.iter().any(|x| x == t) {
            tags.push(t.clone());
        }
    }
    if !tags.is_empty() {
        push("tags", "Tags", tags.join("  ·  "));
    }
    rows
}

/// One mounted row on the task board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardRow<'a> {
    /// Project group header when more than one project is on the board.
    Header {
        /// Project directory name.
        project: &'a str,
        /// How many matching issues sit under this header.
        count: usize,
        /// Whether the group's cards are shown.
        open: bool,
        /// Search is narrowing the group.
        searching: bool,
    },
    /// Issue card. `index` is the filtered-list cursor.
    Task {
        /// Index into [`Palette::filtered_items`].
        index: usize,
        /// Issue the card paints.
        item: &'a HudItem,
    },
}

impl BoardRow<'_> {
    const HEADER_H: f32 = 40.0;
    const TASK_H: f32 = 56.0;

    fn height(self) -> f32 {
        match self {
            Self::Header { .. } => Self::HEADER_H,
            Self::Task { .. } => Self::TASK_H,
        }
    }
}

/// One project group in the current filter.
#[derive(Debug)]
pub struct ProjectSection<'a> {
    /// Project name shared by the rows.
    pub project: &'a str,
    /// Inclusive start index into the filtered list.
    pub start: usize,
    /// Exclusive end index into the filtered list.
    pub end: usize,
    /// Whether the group is collapsed.
    pub collapsed: bool,
    /// Filtered index plus row for each item in the group.
    pub rows: Vec<(usize, &'a HudItem)>,
}

/// Priority, state, and title the detail header paints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderIssue<'a> {
    /// Priority letter (`A`, `B`, or `C`).
    pub priority: &'a str,
    /// Org TODO state.
    pub state: &'a str,
    /// Heading title.
    pub title: &'a str,
    /// Project the heading lives in.
    pub project: &'a str,
    /// Whether `:BLOCKED_BY:` is set.
    pub blocked: bool,
    /// Whether a holder is set.
    pub claimed: bool,
}

/// One visible row in the Tree detail tab.
#[derive(Debug, Clone, Copy)]
pub struct TreeRow<'a> {
    /// Indent depth, `0` at the root.
    pub depth: u32,
    /// Node id used by toggle and pick messages.
    pub tea_id: u64,
    /// Issue id under this row.
    pub issue_id: &'a str,
    /// Org TODO state.
    pub state: &'a str,
    /// Heading title.
    pub title: &'a str,
    /// Whether children are shown.
    pub expanded: bool,
    /// Whether the row has children.
    pub has_children: bool,
}

#[derive(Debug, Clone)]
struct IssueTreeNode {
    tea_id: u64,
    issue_id: String,
    state: String,
    title: String,
    expanded: bool,
    children: Vec<IssueTreeNode>,
}

fn issue_tree_from(node: &vissue_core::views::TreeNode, ids: &mut Vec<String>) -> IssueTreeNode {
    let tea_id = ids.len() as u64;
    ids.push(node.id.clone());
    let children: Vec<_> = node
        .children
        .iter()
        .map(|child| issue_tree_from(child, ids))
        .collect();
    IssueTreeNode {
        tea_id,
        issue_id: node.id.clone(),
        state: node.state.clone(),
        title: node.title.clone(),
        expanded: !children.is_empty(),
        children,
    }
}

fn tree_row_ref(node: &IssueTreeNode, depth: u32) -> TreeRow<'_> {
    TreeRow {
        depth,
        tea_id: node.tea_id,
        issue_id: &node.issue_id,
        state: &node.state,
        title: &node.title,
        expanded: node.expanded,
        has_children: !node.children.is_empty(),
    }
}

fn flatten_issue_tree<'a>(node: &'a IssueTreeNode, depth: u32, out: &mut Vec<TreeRow<'a>>) {
    out.push(tree_row_ref(node, depth));
    if node.expanded {
        for child in &node.children {
            flatten_issue_tree(child, depth + 1, out);
        }
    }
}

fn toggle_issue_tree(node: &mut IssueTreeNode, id: u64) -> bool {
    if node.tea_id == id {
        node.expanded = !node.expanded;
        return true;
    }
    node.children
        .iter_mut()
        .any(|child| toggle_issue_tree(child, id))
}

fn set_issue_tree_expanded(node: &mut IssueTreeNode, expanded: bool) {
    if !node.children.is_empty() {
        node.expanded = expanded;
    }
    for child in &mut node.children {
        set_issue_tree_expanded(child, expanded);
    }
}

fn tree_all_expanded(node: &IssueTreeNode) -> bool {
    (node.children.is_empty() || node.expanded) && node.children.iter().all(tree_all_expanded)
}

fn apply_forest(items: Vec<HudItem>) -> Vec<HudItem> {
    use std::collections::{HashMap, HashSet};
    if items.len() < 2 {
        return items;
    }
    let mut order: Vec<String> = Vec::new();
    for item in &items {
        if !order.iter().any(|p| p == &item.project) {
            order.push(item.project.clone());
        }
    }
    if order.len() > 1 {
        let mut out = Vec::with_capacity(items.len());
        for project in order {
            let group: Vec<HudItem> = items
                .iter()
                .filter(|i| i.project == project)
                .cloned()
                .collect();
            out.extend(apply_forest(group));
        }
        return out;
    }
    if items.iter().all(|i| i.parent.is_none()) {
        return items;
    }
    let ids: HashSet<String> = items.iter().map(|i| i.id.clone()).collect();
    let mut kids: HashMap<Option<String>, Vec<usize>> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        let parent = item.parent.clone().filter(|p| ids.contains(p));
        kids.entry(parent).or_default().push(i);
    }
    let mut out = Vec::with_capacity(items.len());
    fn walk(
        items: &[HudItem],
        kids: &HashMap<Option<String>, Vec<usize>>,
        parent: Option<String>,
        depth: usize,
        out: &mut Vec<HudItem>,
    ) {
        let Some(ixs) = kids.get(&parent) else {
            return;
        };
        for &i in ixs {
            let mut row = items[i].clone();
            row.depth = depth;
            let id = row.id.clone();
            out.push(row);
            walk(items, kids, Some(id), depth + 1, out);
        }
    }
    walk(&items, &kids, None, 0, &mut out);
    if out.len() < items.len() {
        for item in &items {
            if !out.iter().any(|o| o.id == item.id) {
                out.push(item.clone());
            }
        }
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
    #[derive(Debug)]
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
            if n < 2 {
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
        fn create(
            &self,
            project: &str,
            title: &str,
        ) -> Result<vissue_tui::MutResult, vissue_core::error::Error> {
            self.inner.create(project, title)
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

    fn open_atlas(layout: Layout, agent: &str) -> Palette {
        let mut palette = Palette::open_core(layout, agent.into()).unwrap();
        palette.enter_project("atlas");
        palette
    }

    #[test]
    fn entering_a_project_selects_the_first_row() {
        let palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        assert_eq!(palette.selected_index(), 0);
        assert!(palette.selected_id().is_some());
    }

    #[test]
    fn search_in_a_project_drops_hits_from_other_projects() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        palette.set_filter(BoardFilter::Search);
        palette.set_query("retry");
        let titles: Vec<_> = palette
            .filtered_items()
            .iter()
            .map(|i| i.title.as_str())
            .collect();
        assert!(
            titles.iter().all(|t| !t.to_lowercase().contains("retry")),
            "{titles:?}"
        );
        palette.set_query("summary");
        let titles: Vec<_> = palette
            .filtered_items()
            .iter()
            .map(|i| i.title.as_str())
            .collect();
        assert!(
            titles.iter().any(|t| t.contains("Emit a summary")),
            "{titles:?}"
        );
        assert!(
            titles
                .iter()
                .any(|t| t.contains("Parse the manifest header")),
            "search should keep the parent in the tree: {titles:?}"
        );
        let child = palette
            .filtered_items()
            .into_iter()
            .find(|i| i.title.contains("Emit a summary"))
            .expect("child");
        assert!(child.depth > 0, "child should nest under its parent");
    }

    #[test]
    fn header_follows_the_opened_issue() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        palette.select_id("atlas-1a2b");
        assert_eq!(
            palette.header_issue().map(|h| h.title),
            Some("Parse the manifest header")
        );
        palette.set_detail_tab(DetailTab::Tree);
        assert_eq!(
            palette.header_issue().map(|h| h.title),
            Some("Parse the manifest header")
        );
        palette.open_issue("atlas-2c3d");
        assert_eq!(
            palette.header_issue().map(|h| h.title),
            Some("Emit a summary table")
        );
        assert_eq!(palette.header_issue().map(|h| h.project), Some("atlas"));
    }

    #[test]
    fn opened_issue_stays_when_it_leaves_the_list() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        palette.open_issue("atlas-2c3d");
        assert_eq!(palette.selected_id(), Some("atlas-2c3d"));
        palette.set_query("manifest");
        assert_ne!(
            palette.selected_id(),
            Some("atlas-2c3d"),
            "query should drop the opened row from the list"
        );
        palette.set_detail_tab(DetailTab::Notes);
        assert_eq!(
            palette.header_issue().map(|h| h.title),
            Some("Emit a summary table")
        );
        assert_eq!(palette.detail().map(|d| d.id.as_str()), Some("atlas-2c3d"));
        let _ = palette.reload();
        assert_eq!(palette.detail().map(|d| d.id.as_str()), Some("atlas-2c3d"));
        assert_ne!(palette.selected_id(), Some("atlas-2c3d"));
    }

    #[test]
    fn tree_tab_builds_rows() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        palette.set_query("atlas-2c3d");
        palette.set_detail_tab(DetailTab::Tree);
        assert!(!palette.tree_rows().is_empty());
        assert!(palette.tree_selected().is_some());
    }

    #[test]
    fn tree_tab_pick_highlights_without_leaving_the_tree() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        palette.select_id("atlas-1a2b");
        palette.set_detail_tab(DetailTab::Tree);
        let child_id = palette
            .tree_rows()
            .into_iter()
            .find(|row| row.issue_id == "atlas-2c3d")
            .expect("parent tree includes the child")
            .tea_id;
        palette.select_tree_node(child_id);
        assert_eq!(palette.selected_id(), Some("atlas-1a2b"));
        assert_eq!(
            palette.tree_rows()[0].issue_id,
            "atlas-1a2b",
            "pick must not re-root the tree"
        );
        assert!(
            palette.tree_rows()[0].has_children,
            "parent keeps its chevron after a child pick"
        );
        assert_eq!(palette.tree_selected(), Some(child_id));
        assert_eq!(
            palette.header_issue().map(|h| h.title),
            Some("Emit a summary table"),
            "pick should load that issue into the header"
        );
        assert_eq!(palette.detail_tab(), DetailTab::Tree);
        palette.set_detail_tab(DetailTab::Related);
        assert_eq!(
            palette.header_issue().map(|h| h.title),
            Some("Emit a summary table"),
            "a tree pick stays loaded on Related"
        );
        palette.set_detail_tab(DetailTab::Tree);
        assert_eq!(
            palette.tree_selected(),
            Some(child_id),
            "a tree pick should survive leaving the tree tab"
        );
        palette.cycle_detail_tab();
        palette.cycle_detail_tab();
        palette.cycle_detail_tab();
        assert_eq!(
            palette.tree_selected(),
            Some(child_id),
            "a tree pick should survive Enter cycling the side tabs"
        );
        let _ = palette.reload();
        assert_eq!(
            palette.tree_rows()[0].issue_id,
            "atlas-1a2b",
            "reload must keep the tree rooted on the list parent"
        );
        assert_eq!(palette.tree_selected(), Some(child_id));
    }

    fn pick_off_filter_child(palette: &mut Palette) -> u64 {
        palette.select_id("atlas-1a2b");
        palette.set_query("manifest");
        palette.set_detail_tab(DetailTab::Tree);
        assert!(
            palette
                .filtered_items()
                .iter()
                .all(|item| item.id != "atlas-2c3d"),
            "query should drop the child from the list"
        );
        let child_id = palette
            .tree_rows()
            .into_iter()
            .find(|row| row.issue_id == "atlas-2c3d")
            .expect("parent tree includes the child")
            .tea_id;
        palette.select_tree_node(child_id);
        assert_eq!(palette.selected_id(), Some("atlas-1a2b"));
        assert_eq!(palette.painted_id(), Some("atlas-2c3d"));
        child_id
    }

    #[test]
    fn tree_pick_mutations_use_the_painted_child() {
        let (_dir, layout) = writable();
        let mut palette = open_atlas(layout, "hud-test");
        pick_off_filter_child(&mut palette);
        palette.handle_key(PaletteKey::Char('y'));
        assert_eq!(palette.clipboard(), "atlas-2c3d");
        palette.handle_key(PaletteKey::Char('n'));
        for c in "from the child".chars() {
            palette.handle_key(PaletteKey::Char(c));
        }
        palette.handle_key(PaletteKey::Enter);
        let child = palette.backend().get("atlas-2c3d").unwrap();
        assert!(
            child
                .logbook
                .iter()
                .any(|entry| entry.note.as_deref() == Some("from the child")),
            "{:?}",
            child.logbook
        );
        let parent = palette.backend().get("atlas-1a2b").unwrap();
        assert!(
            parent
                .logbook
                .iter()
                .all(|entry| entry.note.as_deref() != Some("from the child")),
            "{:?}",
            parent.logbook
        );
        assert_eq!(palette.backend().get("atlas-2c3d").unwrap().state, "TODO");
        palette.handle_key(PaletteKey::Space);
        assert_eq!(palette.backend().get("atlas-2c3d").unwrap().state, "DONE");
        palette.handle_key(PaletteKey::Space);
        assert_eq!(palette.backend().get("atlas-2c3d").unwrap().state, "TODO");
        palette.handle_key(PaletteKey::Char('s'));
        assert_eq!(
            palette.backend().get("atlas-2c3d").unwrap().state,
            "STARTED"
        );
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
        assert_eq!(
            palette
                .backend()
                .get("atlas-1a2b")
                .unwrap()
                .claimed_by
                .as_deref(),
            Some("fixture-agent")
        );
        assert_eq!(
            palette.backend().get("atlas-1a2b").unwrap().state,
            "STARTED"
        );
        palette.handle_key(PaletteKey::Char('D'));
        palette.handle_key(PaletteKey::Char('y'));
        assert_eq!(palette.backend().get("atlas-2c3d").unwrap().state, "DONE");
        assert_eq!(
            palette.backend().get("atlas-1a2b").unwrap().state,
            "STARTED"
        );
        palette.handle_key(PaletteKey::Char('o'));
        assert!(
            palette.message().contains("atlas-2c3d"),
            "{}",
            palette.message()
        );
        assert_eq!(
            palette.tree_rows()[0].issue_id,
            "atlas-1a2b",
            "mutations must keep the tree rooted on the list parent"
        );
    }

    #[test]
    fn empty_filter_keeps_painted_detail() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        pick_off_filter_child(&mut palette);
        palette.set_query("zzzz-no-hit");
        assert!(palette.filtered_items().is_empty());
        assert!(palette.selected_id().is_none());
        assert_eq!(palette.painted_id(), Some("atlas-2c3d"));
        assert_eq!(
            palette.header_issue().map(|h| h.title),
            Some("Emit a summary table")
        );
        assert_eq!(palette.detail().map(|d| d.id.as_str()), Some("atlas-2c3d"));
    }

    #[test]
    fn opening_the_selected_row_after_a_tree_pick_loads_it() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        palette.select_id("atlas-1a2b");
        palette.set_detail_tab(DetailTab::Tree);
        let child_id = palette
            .tree_rows()
            .into_iter()
            .find(|row| row.issue_id == "atlas-2c3d")
            .expect("parent tree includes the child")
            .tea_id;
        palette.select_tree_node(child_id);
        assert_eq!(
            palette.header_issue().map(|h| h.title),
            Some("Emit a summary table")
        );
        palette.open_issue("atlas-1a2b");
        assert_eq!(
            palette.header_issue().map(|h| h.title),
            Some("Parse the manifest header"),
            "opening the list row must load that issue after a child pick"
        );
        assert_eq!(palette.selected_id(), Some("atlas-1a2b"));
        assert_eq!(palette.detail().map(|d| d.id.as_str()), Some("atlas-1a2b"));
    }

    #[test]
    fn opening_a_tree_node_selects_that_issue() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        palette.select_id("atlas-1a2b");
        palette.set_detail_tab(DetailTab::Tree);
        palette.open_issue("atlas-2c3d");
        assert_eq!(palette.selected_id(), Some("atlas-2c3d"));
        assert_eq!(palette.detail_tab(), DetailTab::Tree);
        assert_eq!(palette.project(), Some("atlas"));
        let heading = palette.header_issue().expect("header");
        assert_eq!(heading.title, "Emit a summary table");
        assert_eq!(heading.state, "TODO");
    }

    #[test]
    fn opening_an_issue_outside_the_filter_lists_it() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        palette.set_query("manifest");
        palette.select_id("atlas-1a2b");
        palette.set_detail_tab(DetailTab::Tree);
        assert!(
            palette
                .filtered_items()
                .iter()
                .all(|item| item.id != "atlas-2c3d"),
            "query should drop the child from the list"
        );
        palette.open_issue("atlas-2c3d");
        assert_eq!(palette.selected_id(), Some("atlas-2c3d"));
        assert_eq!(palette.detail_tab(), DetailTab::Tree);
        assert_eq!(palette.filter(), BoardFilter::List);
        assert!(palette.query().is_empty());
    }

    #[test]
    fn opening_an_issue_in_another_project_enters_that_project() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        palette.select_id("atlas-1a2b");
        palette.set_detail_tab(DetailTab::Related);
        palette.open_issue("beacon-5j6k");
        assert_eq!(palette.project(), Some("beacon"));
        assert_eq!(palette.selected_id(), Some("beacon-5j6k"));
        assert_eq!(palette.detail_tab(), DetailTab::Related);
        assert_eq!(palette.filter(), BoardFilter::List);
    }

    #[test]
    fn reload_keeps_the_selected_issue() {
        let (_dir, layout) = writable();
        let issues = layout.project_issues_path("atlas");
        let mut palette = open_atlas(layout, "hud-test");
        palette.set_filter(BoardFilter::List);
        palette.select_id("atlas-4g5h");
        assert_eq!(palette.selected_id(), Some("atlas-4g5h"));
        let body = std::fs::read_to_string(&issues).unwrap();
        let inserted =
            "* TODO [#C] Inserted first\n:PROPERTIES:\n:ID:         atlas-zzzz\n:END:\n\n";
        let body = match body.find("\n* ") {
            Some(at) => format!("{}{}{}", &body[..=at], inserted, &body[at + 1..]),
            None => format!("{body}\n{inserted}"),
        };
        std::fs::write(&issues, body).unwrap();
        palette.backend.refresh().unwrap();
        let _ = palette.reload();
        assert!(
            palette
                .filtered_items()
                .iter()
                .any(|item| item.id == "atlas-zzzz"),
            "the inserted heading should be on the board"
        );
        assert_eq!(
            palette.selected_id(),
            Some("atlas-4g5h"),
            "reload must keep the selected issue when a row is inserted above it"
        );
    }

    #[test]
    fn list_select_reloads_the_tree() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        palette.set_filter(BoardFilter::List);
        palette.select_id("atlas-1a2b");
        palette.set_detail_tab(DetailTab::Tree);
        assert_eq!(palette.tree_rows()[0].issue_id, "atlas-1a2b");
        palette.select_id("atlas-4g5h");
        assert_eq!(
            palette.tree_rows()[0].issue_id,
            "atlas-4g5h",
            "clicking a list row must load that issue's tree"
        );
        assert_eq!(
            palette.header_issue().map(|h| h.title),
            Some("Rename the config key")
        );
    }

    #[test]
    fn tree_tab_toggle_hides_children() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        palette.select_id("atlas-1a2b");
        palette.set_detail_tab(DetailTab::Tree);
        let before = palette.tree_rows().len();
        let root = palette.tree_rows()[0];
        assert!(root.has_children);
        palette.toggle_tree_node(root.tea_id);
        assert!(
            palette.tree_rows().len() < before,
            "collapse should hide children"
        );
        assert!(!palette.tree_rows()[0].expanded);
        palette.set_detail_tab(DetailTab::Related);
        palette.set_detail_tab(DetailTab::Tree);
        assert_eq!(
            palette.tree_rows().len(),
            1,
            "a collapsed parent must stay collapsed after leaving the tree tab"
        );
        assert!(!palette.tree_rows()[0].expanded);
        let tea_id = palette.tree_rows()[0].tea_id;
        palette.select_tree_node(tea_id);
        assert_eq!(
            palette.tree_rows().len(),
            1,
            "picking a collapsed parent must not rebuild the tree open"
        );
    }

    #[test]
    fn tree_tab_expand_and_collapse_all() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        palette.select_id("atlas-1a2b");
        palette.set_detail_tab(DetailTab::Tree);
        let open = palette.tree_rows().len();
        assert!(open > 1, "fixture parent should have children");
        palette.set_tree_expanded(false);
        assert_eq!(palette.tree_rows().len(), 1, "collapse all leaves the root");
        assert!(!palette.tree_rows()[0].expanded);
        palette.set_tree_expanded(true);
        assert_eq!(palette.tree_rows().len(), open);
        assert!(palette.tree_rows()[0].expanded);
        assert!(palette.tree_all_expanded());
        palette.set_tree_expanded(false);
        assert!(!palette.tree_all_expanded());
    }

    #[test]
    fn related_tab_keeps_structured_hits() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        palette.select_id("atlas-1a2b");
        palette.set_detail_tab(DetailTab::Related);
        assert!(
            !palette.related_hits().is_empty(),
            "parser heading should have neighbors"
        );
        assert!(
            palette
                .related_hits()
                .iter()
                .any(|hit| !hit.title.is_empty() && !hit.state.is_empty()),
            "{:?}",
            palette.related_hits()
        );
        assert!(
            palette
                .related_hits()
                .iter()
                .any(|hit| palette.related_marks(&hit.id).is_some()),
            "related rows should load the same badges as cards"
        );
    }

    #[test]
    fn excerpt_tab_keeps_the_org_heading() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        palette.select_id("atlas-1a2b");
        let excerpt = palette.excerpt().expect("heading range");
        assert!(excerpt.text.contains(":PROPERTIES:"), "{}", excerpt.text);
        assert!(
            excerpt.text.contains("Scope: read the header block"),
            "{}",
            excerpt.text
        );
        assert!(excerpt.line_start > 0);
        assert!(excerpt.line_end >= excerpt.line_start);
        let detail = palette.detail().expect("heading fields");
        assert_eq!(
            detail.properties.get("TYPE").map(String::as_str),
            Some("feature")
        );
        assert!(detail.body.contains("Scope: read the header block"));
        assert!(palette.selectable("excerpt-body").is_some());
        assert_eq!(detail.title, "Parse the manifest header");
        assert!(
            detail
                .body
                .contains("Done-when: a malformed header names the offending line number."),
            "{}",
            detail.body
        );
    }

    #[test]
    fn excerpt_table_fits_the_field_stack() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        palette.select_id("atlas-1a2b");
        let label = palette.excerpt_label_width();
        let table = palette.excerpt_table_width();
        assert!(
            label < icedtea::layout::FORM_LABEL,
            "label gutter {label} should follow the longest name, not the form column"
        );
        assert!(
            table < icedtea::layout::FORM_LABEL + 160.0,
            "table {table} should leave the body the wide pane"
        );
        assert!(table > label);
    }

    #[test]
    fn notes_tab_keeps_logbook_lines() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        palette.select_id("atlas-1a2b");
        palette.set_detail_tab(DetailTab::Notes);
        let detail = palette.detail().expect("detail");
        assert!(
            detail
                .logbook
                .iter()
                .any(|e| e.to_state.as_deref() == Some("STARTED")),
            "{:?}",
            detail.logbook
        );
        assert!(
            detail
                .logbook
                .iter()
                .any(|e| e.raw.as_deref().is_some_and(|r| r.contains("CLOCK"))),
            "{:?}",
            detail.logbook
        );
    }

    #[test]
    fn dragging_the_detail_sash_grows_the_detail_pane() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        let before = palette.detail_split().ratio;
        palette.apply_sash(icedtea::layout::SashEvent::Press);
        palette.apply_sash(icedtea::layout::SashEvent::Move(400.0));
        palette.apply_sash(icedtea::layout::SashEvent::Move(300.0));
        assert!(
            palette.detail_split().ratio < before,
            "dragging the sash up should shrink the list share"
        );
    }

    #[test]
    fn help_markdown_is_parsed_once() {
        let palette =
            Palette::open_core(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap".into()).unwrap();
        assert!(
            !palette.help_md().items.is_empty(),
            "help source must parse into markdown items"
        );
        assert!(palette.help_md().source.contains("claim"));
    }

    #[test]
    fn first_paint_lists_projects() {
        let palette =
            Palette::open_core(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap".into()).unwrap();
        assert!(palette.browsing());
        assert!(palette.filtered_items().is_empty());
        let names: Vec<_> = palette
            .project_cards()
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert!(names.contains(&"atlas"), "{names:?}");
        assert!(names.contains(&"beacon"), "{names:?}");
        assert!(palette.status_line().contains("serve:offline"));
        assert_eq!(palette.revision(), 0);
    }

    #[test]
    fn enter_project_lists_that_projects_ready() {
        let palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        assert!(!palette.browsing());
        assert_eq!(palette.project(), Some("atlas"));
        let ids: Vec<_> = palette
            .filtered_items()
            .into_iter()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(ids, ["atlas-1a2b", "atlas-2c3d"]);
    }

    #[test]
    fn selecting_a_row_loads_the_issue_body() {
        let (_dir, layout) = writable();
        let mut palette = open_atlas(layout, "hud-test");
        palette.handle_key(PaletteKey::Down);
        assert_eq!(palette.selected_id(), Some("atlas-2c3d"));
        assert_eq!(palette.detail_tab(), DetailTab::Tree);
        let text = palette.excerpt().map(|e| e.text.as_str()).unwrap_or("");
        assert!(text.contains("summary"), "{text}");
        assert!(
            palette
                .detail()
                .is_some_and(|d| d.body.contains("summary") || d.title.contains("summary")),
            "{:?}",
            palette.detail().map(|d| &d.body)
        );
    }

    #[test]
    fn clearing_home_search_returns_to_the_project_list() {
        let mut palette =
            Palette::open_core(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap".into()).unwrap();
        assert!(palette.browsing());
        palette.type_query("x");
        assert_eq!(palette.filter(), BoardFilter::Search);
        palette.type_query("");
        assert!(palette.browsing());
        assert_eq!(palette.filter(), BoardFilter::Ready);
        assert!(!palette.project_cards().is_empty());
    }

    #[test]
    fn add_writes_into_the_open_project() {
        let (_dir, layout) = writable();
        let mut palette = open_atlas(layout.clone(), "hud-test");
        palette.set_add_draft("only atlas");
        palette.submit_add();
        let atlas = std::fs::read_to_string(layout.project_issues_path("atlas")).unwrap();
        assert!(atlas.contains("only atlas"), "{atlas}");
        let beacon = std::fs::read_to_string(layout.project_issues_path("beacon")).unwrap();
        assert!(!beacon.contains("only atlas"), "{beacon}");
    }

    #[test]
    fn add_from_home_search_uses_the_hit_project() {
        let (_dir, layout) = writable();
        let mut palette = Palette::open_core(layout.clone(), "hud-test".into()).unwrap();
        assert!(palette.browsing());
        assert_eq!(palette.selected_project_name(), Some("atlas"));
        palette.type_query("retry");
        assert_eq!(palette.filter(), BoardFilter::Search);
        assert!(palette.project().is_none());
        let hit = palette.selected_item().expect("beacon hit");
        assert_eq!(hit.project, "beacon");
        assert_eq!(hit.id, "beacon-5j6k");
        palette.set_add_draft("from a beacon hit");
        palette.submit_add();
        let beacon = std::fs::read_to_string(layout.project_issues_path("beacon")).unwrap();
        assert!(beacon.contains("from a beacon hit"), "{beacon}");
        let atlas = std::fs::read_to_string(layout.project_issues_path("atlas")).unwrap();
        assert!(!atlas.contains("from a beacon hit"), "{atlas}");
    }

    #[test]
    fn toggle_done_flips_todo_to_done() {
        let (_dir, layout) = writable();
        let mut palette = open_atlas(layout, "hud-test");
        palette.set_filter(BoardFilter::List);
        palette.select_id("atlas-2c3d");
        assert_eq!(palette.backend().get("atlas-2c3d").unwrap().state, "TODO");
        palette.toggle_done("atlas-2c3d");
        assert_eq!(palette.backend().get("atlas-2c3d").unwrap().state, "DONE");
    }

    #[test]
    fn toggle_done_selects_the_clicked_row() {
        let (_dir, layout) = writable();
        let mut palette = open_atlas(layout, "hud-test");
        palette.set_filter(BoardFilter::List);
        palette.select_id("atlas-1a2b");
        palette.set_detail_tab(DetailTab::Notes);
        palette.toggle_done("atlas-2c3d");
        assert_eq!(palette.selected_id(), Some("atlas-2c3d"));
        assert_eq!(
            palette.header_issue().map(|h| h.title),
            Some("Emit a summary table"),
            "the checkbox must load the row it flipped"
        );
        assert_eq!(palette.backend().get("atlas-2c3d").unwrap().state, "DONE");
    }

    #[test]
    fn toggle_done_leaves_blocked_alone() {
        let (_dir, layout) = writable();
        let mut palette = open_atlas(layout, "hud-test");
        palette.set_filter(BoardFilter::List);
        palette.select_id("atlas-3e4f");
        assert_eq!(
            palette.backend().get("atlas-3e4f").unwrap().state,
            "BLOCKED"
        );
        palette.toggle_done("atlas-3e4f");
        assert_eq!(
            palette.backend().get("atlas-3e4f").unwrap().state,
            "BLOCKED"
        );
        assert!(
            palette.message().contains("BLOCKED"),
            "{}",
            palette.message()
        );
    }

    #[test]
    fn toggle_done_leaves_cancelled_alone() {
        let (_dir, layout) = writable();
        let mut palette = open_atlas(layout, "hud-test");
        let id = palette.selected_id().expect("a row").to_string();
        palette.handle_key(PaletteKey::Char('X'));
        palette.handle_key(PaletteKey::Enter);
        assert_eq!(palette.backend().get(&id).unwrap().state, "CANCELLED");
        palette.set_filter(BoardFilter::List);
        palette.select_id(&id);
        palette.toggle_done(&id);
        assert_eq!(palette.backend().get(&id).unwrap().state, "CANCELLED");
        assert!(
            palette.message().contains("CANCELLED"),
            "{}",
            palette.message()
        );
    }

    #[test]
    fn confirm_after_reject_on_ready_leaves_neighbors() {
        let (_dir, layout) = writable();
        let mut palette = open_atlas(layout, "hud-test");
        let first = palette.selected_id().expect("a row").to_string();
        assert_eq!(first, "atlas-1a2b");
        let first_state = palette.backend().get(&first).unwrap().state;
        assert_eq!(first_state, "STARTED");
        palette.handle_key(PaletteKey::Char('D'));
        assert_eq!(palette.confirm(), Some(ConfirmKind::Done));
        palette
            .backend()
            .update(UpdateReq {
                id: first.clone(),
                state: Some("CANCELLED".into()),
                if_state: Some(first_state),
                ..UpdateReq::default()
            })
            .expect("reject");
        let _ = palette.reload();
        assert!(
            palette.filtered_items().iter().all(|item| item.id != first),
            "cancelled heading should leave Ready"
        );
        let neighbor = palette.selected_id().expect("neighbor").to_string();
        assert_ne!(neighbor, first);
        let neighbor_state = palette.backend().get(&neighbor).unwrap().state;
        palette.handle_key(PaletteKey::Char('y'));
        assert_eq!(
            palette.backend().get(&neighbor).unwrap().state,
            neighbor_state,
            "yes must not close the Ready neighbor"
        );
        let rejected = palette.backend().get(&first).unwrap();
        assert_eq!(rejected.state, "CANCELLED");
        assert!(
            !rejected.properties.contains_key("SIBLING_TERMINAL"),
            "{:?}",
            rejected.properties
        );
    }

    #[test]
    fn confirm_done_on_cancelled_refuses() {
        let (_dir, layout) = writable();
        let mut palette = open_atlas(layout, "hud-test");
        let id = palette.selected_id().expect("a row").to_string();
        palette.handle_key(PaletteKey::Char('X'));
        palette.handle_key(PaletteKey::Enter);
        assert_eq!(palette.backend().get(&id).unwrap().state, "CANCELLED");
        palette.set_filter(BoardFilter::List);
        palette.select_id(&id);
        palette.handle_key(PaletteKey::Char('D'));
        assert!(palette.confirm().is_none());
        palette.handle_key(PaletteKey::Char('y'));
        let shown = palette.backend().get(&id).unwrap();
        assert_eq!(shown.state, "CANCELLED");
        assert!(
            !shown.properties.contains_key("SIBLING_TERMINAL"),
            "{:?}",
            shown.properties
        );
    }

    #[test]
    fn toggle_done_needs_a_painted_row() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "hud-test");
        let before = palette.backend().get("atlas-1a2b").unwrap().state;
        palette.toggle_done("atlas-zzzz");
        assert_eq!(palette.backend().get("atlas-1a2b").unwrap().state, before);
        assert!(
            palette.message().contains("atlas-zzzz"),
            "{}",
            palette.message()
        );
    }

    #[test]
    fn esc_hides_and_process_state_stays() {
        let palette_layout = Layout::new(fixture_root(), DEFAULT_PREFIX);
        let mut palette = Palette::open_core(palette_layout, "snap".into()).unwrap();
        assert!(palette.visible());
        assert!(palette.browsing());
        palette.handle_key(PaletteKey::Esc);
        assert!(!palette.visible());
    }

    #[test]
    fn esc_from_project_returns_to_browser() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        assert_eq!(palette.project(), Some("atlas"));
        palette.handle_key(PaletteKey::Esc);
        assert!(palette.visible());
        assert!(palette.browsing());
        assert!(palette.project().is_none());
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
        let mut palette = open_atlas(layout, "hud-test");
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
        let mut palette = open_atlas(layout, "hud-test");
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
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
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
    fn poll_updates_does_not_block_the_frame() {
        let mut palette = open_atlas(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap");
        let start = std::time::Instant::now();
        palette.poll_updates();
        assert!(
            start.elapsed() < std::time::Duration::from_millis(50),
            "poll must not sleep the 200ms wait interval on the frame thread"
        );
    }

    #[test]
    fn note_prompt_esc_and_empty_claim_are_noops() {
        let (_dir, layout) = writable();
        let mut palette = open_atlas(layout, "hud-test");
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

    /// The overlay can cite a deed, and a value that is not an accession is
    /// refused where the person typing it can see it, with the field left
    /// holding what was typed.
    ///
    /// A refused accession is usually a typo in a long hash, and clearing the
    /// field would cost the whole line.
    #[test]
    fn the_overlay_cites_a_deed_and_keeps_a_refused_one_in_the_field() {
        let (_dir, layout) = writable();
        let mut palette = open_atlas(layout, "hud-test");
        let id = palette.painted_id().expect("a painted row").to_string();

        palette.handle_key(PaletteKey::Char('d'));
        assert!(palette.deed_draft().is_some(), "d opens the deed field");
        assert_eq!(
            palette.detail_tab(),
            DetailTab::Recall,
            "onto the tab the citation lands on"
        );
        for c in "deed-file-thing".chars() {
            palette.handle_key(PaletteKey::Char(c));
        }
        palette.handle_key(PaletteKey::Enter);
        assert!(
            palette.message().contains("deeds += deed-file-thing"),
            "{}",
            palette.message()
        );
        assert!(palette.deed_draft().is_none(), "the field closed");

        palette.handle_key(PaletteKey::Char('d'));
        for c in "not-an-accession".chars() {
            palette.handle_key(PaletteKey::Char(c));
        }
        palette.handle_key(PaletteKey::Enter);
        assert!(
            palette.message().contains("not a deed accession"),
            "the refusal has to reach the overlay: {}",
            palette.message()
        );
        assert_eq!(
            palette.deed_draft(),
            Some("not-an-accession"),
            "the field keeps what was typed"
        );
        let _ = id;
    }

    #[test]
    fn unchanged_ready_does_not_wipe_rows() {
        let backend = UnchangedAfterFirst::new(
            CoreBackend::open(Layout::new(fixture_root(), DEFAULT_PREFIX), "snap").unwrap(),
        );
        let mut palette =
            Palette::with_backend(Box::new(backend), "snap".into(), ServeStatus::Live).unwrap();
        palette.enter_project("atlas");
        let first: Vec<_> = palette
            .filtered_items()
            .into_iter()
            .map(|i| i.id.clone())
            .collect();
        assert_eq!(first, ["atlas-1a2b", "atlas-2c3d"]);
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
        assert!(palette.browsing());
        assert_eq!(palette.filter(), BoardFilter::Ready);
        palette.handle_key(PaletteKey::Char('3'));
        assert_eq!(palette.filter(), BoardFilter::Claims);
        palette.handle_key(PaletteKey::Char('1'));
        assert!(palette.browsing());
        palette.enter_project("atlas");
        assert_eq!(palette.filter(), BoardFilter::Ready);
        let ids: Vec<_> = palette
            .filtered_items()
            .into_iter()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(ids, ["atlas-1a2b", "atlas-2c3d"]);
    }

    #[test]
    fn space_marks_selected_done() {
        let (_dir, layout) = writable();
        let mut palette = open_atlas(layout, "hud-test");
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
        let mut palette = open_atlas(layout, "hud-test");
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
    fn notes_tab_shows_a_written_note() {
        let (_dir, layout) = writable();
        let mut palette = open_atlas(layout, "hud-test");
        palette.set_query("atlas-2c3d");
        palette.focus_list();
        palette.handle_key(PaletteKey::Char('n'));
        assert_eq!(palette.detail_tab(), DetailTab::Notes);
        for c in "from the board".chars() {
            palette.handle_key(PaletteKey::Char(c));
        }
        palette.handle_key(PaletteKey::Enter);
        palette.set_detail_tab(DetailTab::Notes);
        assert!(
            palette.detail_body().contains("from the board"),
            "{}",
            palette.detail_body()
        );
    }

    #[test]
    fn list_forest_nests_a_child_under_its_parent() {
        let (_dir, layout) = writable();
        let parent = palette_create(&layout, "atlas", "root task");
        let _child = vissue_core::ops::create(
            &layout,
            "atlas",
            "child task",
            vissue_core::ops::CreateOpts {
                parent: Some(&parent),
                quiet: true,
                ..vissue_core::ops::CreateOpts::default()
            },
        )
        .unwrap()
        .trim()
        .to_string();
        let mut palette = Palette::open_core(layout, "hud-test".into()).unwrap();
        palette.enter_project("atlas");
        palette.set_filter(BoardFilter::List);
        let rows: Vec<_> = palette
            .filtered_items()
            .into_iter()
            .map(|i| (i.title.as_str(), i.depth, i.parent.clone()))
            .collect();
        let parent_pos = rows.iter().position(|(t, _, _)| *t == "root task");
        let child_pos = rows.iter().position(|(t, d, p)| {
            *t == "child task" && *d >= 1 && p.as_deref() == Some(parent.as_str())
        });
        assert!(parent_pos.is_some(), "{rows:?}");
        assert!(child_pos.is_some(), "{rows:?}");
        assert!(parent_pos.unwrap() < child_pos.unwrap(), "{rows:?}");
    }

    fn palette_create(layout: &Layout, project: &str, title: &str) -> String {
        vissue_core::ops::create(
            layout,
            project,
            title,
            vissue_core::ops::CreateOpts {
                quiet: true,
                ..vissue_core::ops::CreateOpts::default()
            },
        )
        .unwrap()
        .trim()
        .to_string()
    }
}
