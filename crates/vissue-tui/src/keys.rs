//! Key dispatch. Bindings are listed on `?`.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// What the event loop does after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Stay in the event loop.
    Continue,
    /// Leave the event loop.
    Quit,
}

/// One of the five list surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
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

impl Pane {
    /// Tab order, left to right.
    pub const ALL: [Pane; 5] = [
        Pane::Ready,
        Pane::List,
        Pane::Claims,
        Pane::Agenda,
        Pane::Search,
    ];

    /// Tab label drawn on the board.
    pub fn title(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::List => "List",
            Self::Claims => "Claims",
            Self::Agenda => "Agenda",
            Self::Search => "Search",
        }
    }

    /// Index into [`Self::ALL`].
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    /// Pane at `i` modulo the tab count.
    pub fn from_index(i: usize) -> Self {
        Self::ALL[i % Self::ALL.len()]
    }

    /// Next pane in tab order.
    pub fn next(self) -> Self {
        Self::from_index(self.index() + 1)
    }
}

/// Right-hand detail surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    /// Metadata from `issue/get`.
    Show,
    /// On-disk heading range.
    Excerpt,
    /// Parent and child tree.
    Tree,
    /// Related-issue hits.
    Related,
    /// The working set: plan, declared inputs, and what they produced.
    Recall,
}

impl DetailTab {
    /// Tab order cycled by Enter in the detail pane.
    pub const ALL: [DetailTab; 5] = [
        DetailTab::Show,
        DetailTab::Excerpt,
        DetailTab::Tree,
        DetailTab::Related,
        DetailTab::Recall,
    ];

    /// Tab label drawn on the detail border.
    pub fn title(self) -> &'static str {
        match self {
            Self::Show => "show",
            Self::Excerpt => "excerpt",
            Self::Tree => "tree",
            Self::Related => "related",
            Self::Recall => "recall",
        }
    }

    /// Next tab in cycle order.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// Which pane receives movement keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// Row list on the left.
    Rows,
    /// Detail pane on the right.
    Detail,
}

/// Line prompt opened by `/`, `n`, or `p`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// Search query for the Search pane.
    Search,
    /// Logbook note on the selected issue.
    Note,
    /// Deed accession to cite on the selected issue.
    Deed,
    /// Project filter. Empty clears it.
    Project,
}

/// Destructive state change waiting for `y`.
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

/// True for Press and Repeat; false for Release.
pub fn is_press(key: KeyEvent) -> bool {
    key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat
}

/// Printable character from `key`, including Shift. Other modifiers drop it.
pub fn char_of(key: KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
            Some(c)
        }
        _ => None,
    }
}

/// Overlay text shown on `?`.
pub const HELP: &str = "\
vissue tui

j/k, arrows   move
Tab, 1-5      pane (Ready List Claims Agenda Search)
Enter         focus detail / cycle detail tab
p             project filter
/             search
c             claim
n             note
s             cycle TODO / STARTED / BLOCKED
D             DONE (confirm)
X             CANCELLED (confirm)
o             open (shared selection)
y             copy id
R             reload
?             this help
q / Esc       quit / back

Body edits stay in the file.
body lives in file; open the range above
";
