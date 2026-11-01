//! Sync board facade shared by the file-backed and socket-backed clients.

use vissue_core::config::Layout;
use vissue_core::error::Error;
use vissue_core::views::{
    AgendaRow, ClaimRow, Excerpt, IssueDetail, ListQuery, Recall, RelatedHit, SearchHit, TreeNode,
};

/// Which store the board is talking to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    /// File-backed [`crate::CoreBackend`].
    Core,
    /// Socket-backed `ControlBackend`.
    Control,
}

/// One page of list/ready rows.
#[derive(Debug, Clone, Default)]
pub struct ListPage {
    /// Rows for this page. Empty when [`Self::unchanged`].
    pub issues: Vec<vissue_core::views::IssueRow>,
    /// Total issues in the catalog on serve; core repeats [`Self::matched`].
    pub total: u64,
    /// Rows that matched the query.
    pub matched: u64,
    /// Serve catalog revision. Core is 0.
    pub revision: u64,
    /// File-watcher generation.
    pub generation: u64,
    /// Serve says the catalog is unchanged since `since_revision`.
    pub unchanged: bool,
}

/// Outcome of claim, note, or update.
#[derive(Debug, Clone)]
pub struct MutResult {
    /// Mutation succeeded.
    pub ok: bool,
    /// Status text from the op. Core includes a trailing newline.
    pub report: String,
    /// Issue after the write, when the backend returns one.
    pub issue: Option<IssueDetail>,
    /// Serve revision after the write. Core is 0.
    pub revision: u64,
    /// File-watcher generation after the write.
    pub generation: u64,
}

/// Fields `issue/update` accepts.
#[derive(Debug, Clone, Default)]
pub struct UpdateReq {
    /// Issue to change.
    pub id: String,
    /// New org TODO state, if any.
    pub state: Option<String>,
    /// New priority letter, if any.
    pub priority: Option<char>,
    /// Blocker id to add.
    pub block: Option<String>,
    /// Blocker id to drop.
    pub unblock: Option<String>,
    /// Refuse unless the heading is still this state.
    pub if_state: Option<String>,
    /// Refuse unless the corpus generation is still this value.
    pub if_gen: Option<u64>,
}

/// Drops `since_revision` for one fetch after attach.
#[derive(Debug)]
pub struct SinceGate {
    skip_once: std::sync::atomic::AtomicBool,
}

impl SinceGate {
    /// After `initialize`, the next list must not send a core generation.
    pub fn after_attach() -> Self {
        Self {
            skip_once: std::sync::atomic::AtomicBool::new(true),
        }
    }

    /// Consume the skip flag, or return `Some(revision)` when `revision > 0`.
    pub fn next(&self, revision: u64) -> Option<u64> {
        if self
            .skip_once
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            None
        } else if revision > 0 {
            Some(revision)
        } else {
            None
        }
    }

    /// Next list/ready must omit `since_revision` (pane or query changed).
    pub fn invalidate(&self) {
        self.skip_once
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Read and mutate the board. Implementations are `CoreBackend` and
/// `ControlBackend`.
pub trait BoardBackend: Send + Sync + std::fmt::Debug {
    /// Vault this backend reads and writes.
    fn layout(&self) -> &Layout;
    /// File-watcher generation.
    fn generation(&self) -> u64;
    /// Serve catalog revision. Core is always 0.
    fn revision(&self) -> u64;
    /// Which store this backend is.
    fn live(&self) -> BackendKind;
    /// Claim and update identity (core: constructor; control: serve `initialize`).
    fn identity(&self) -> &str;

    /// Filtered issue list for the List pane.
    ///
    /// # Errors
    ///
    /// Returns an error if the list cannot be loaded.
    fn list(&self, q: ListQuery) -> Result<ListPage, Error>;
    /// Actionable ready queue, optionally scoped to `project`.
    ///
    /// # Errors
    ///
    /// Returns an error if the ready list cannot be loaded.
    fn ready(&self, project: Option<&str>) -> Result<ListPage, Error>;
    /// Full metadata for one issue.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue does not exist or cannot be fetched.
    fn get(&self, id: &str) -> Result<IssueDetail, Error>;
    /// On-disk heading range, capped and screened for secrets.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue does not exist or its file cannot be read.
    fn excerpt(&self, id: &str) -> Result<Excerpt, Error>;
    /// Title and body search hits, capped at `limit`.
    ///
    /// # Errors
    ///
    /// Returns an error if search cannot run.
    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, Error>;
    /// Open claims, optionally filtered by holder and project.
    ///
    /// # Errors
    ///
    /// Returns an error if claims cannot be listed.
    fn claims(&self, holder: Option<&str>, project: Option<&str>) -> Result<Vec<ClaimRow>, Error>;
    /// Deadlines and scheduled dates inside `days`.
    ///
    /// # Errors
    ///
    /// Returns an error if the agenda cannot be listed.
    fn agenda(&self, days: i64, project: Option<&str>) -> Result<Vec<AgendaRow>, Error>;
    /// Parent and child tree rooted at `id`.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue does not exist or the tree cannot be built.
    fn tree(&self, id: &str) -> Result<TreeNode, Error>;
    /// Related issues by graph walk and text overlap.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue does not exist or related hits cannot be scored.
    fn related(&self, id: &str, depth: usize, limit: usize) -> Result<Vec<RelatedHit>, Error>;
    /// The working set for `id`: plan, declared inputs and their deeds.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue does not exist or the blocker graph cannot
    /// be built.
    fn recall(&self, id: &str, depth: usize) -> Result<Recall, Error>;
    /// Cite deed accessions on `id`.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue does not exist, a value is not a deed
    /// accession, or the file cannot be rewritten.
    fn deed(&self, id: &str, add: &[String]) -> Result<MutResult, Error>;
    /// Project names under the layout prefix.
    ///
    /// # Errors
    ///
    /// Returns an error if the project list cannot be read.
    fn projects(&self) -> Result<Vec<String>, Error>;
    /// Claim `id` as [`Self::identity`]. `force` takes over another holder.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue does not exist, is DONE or CANCELLED, is
    /// held by another identity without `force`, or the write fails.
    fn claim(&self, id: &str, force: bool) -> Result<MutResult, Error>;
    /// Append a one-line logbook note.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue does not exist, the text is empty, or the
    /// write fails.
    fn note(&self, id: &str, text: &str) -> Result<MutResult, Error>;
    /// Change state, priority, or blocker edges.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue does not exist, the change is refused, or
    /// the write fails.
    fn update(&self, req: UpdateReq) -> Result<MutResult, Error>;
    /// Create a TODO in `project` with `title`.
    ///
    /// # Errors
    ///
    /// Returns an error if the project does not exist, the title is empty, or
    /// the write fails.
    fn create(&self, project: &str, title: &str) -> Result<MutResult, Error>;
    /// Same metadata as [`Self::get`]; control also marks the issue opened.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue does not exist or cannot be fetched.
    fn open(&self, id: &str) -> Result<IssueDetail, Error>;

    /// Core: wait on the file generation. Control: wait for `vault/changed`.
    ///
    /// # Errors
    ///
    /// Returns an error if the wait cannot be started or a newer catalog cannot
    /// be re-read.
    fn wait(&self, last: u64, timeout_ms: u64) -> Result<u64, Error>;

    /// Last `since_revision` sent on list/ready. `None` means the field was
    /// omitted. Default is "not recorded".
    fn last_since_revision(&self) -> Option<Option<u64>> {
        None
    }

    /// Drop `since_revision` on the next list/ready. Serve `unchanged` is
    /// catalog-wide, so a pane or project change must fetch a full page.
    fn invalidate_since(&self) {}

    /// Re-read the files. Core uses this after an out-of-band write. Control
    /// is a no-op; serve sees the file event.
    ///
    /// # Errors
    ///
    /// Returns an error if the catalog cannot be re-read. Control never fails.
    fn refresh(&self) -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SinceGate;

    #[test]
    fn after_attach_first_list_omits_since_revision() {
        let gate = SinceGate::after_attach();
        assert_eq!(gate.next(7), None);
        assert_eq!(gate.next(7), Some(7));
        assert_eq!(gate.next(8), Some(8));
    }

    #[test]
    fn a_zero_revision_never_sends_since() {
        let gate = SinceGate::after_attach();
        assert_eq!(gate.next(0), None);
        assert_eq!(gate.next(0), None);
    }

    #[test]
    fn invalidate_omits_the_next_since() {
        let gate = SinceGate::after_attach();
        assert_eq!(gate.next(7), None);
        assert_eq!(gate.next(7), Some(7));
        gate.invalidate();
        assert_eq!(gate.next(7), None);
        assert_eq!(gate.next(7), Some(7));
    }
}
