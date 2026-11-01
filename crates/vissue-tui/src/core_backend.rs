//! File-backed board: `CatalogService` plus `ops` with an explicit identity.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use vissue_core::catalog::{CatalogService, load_recs};
use vissue_core::config::Layout;
use vissue_core::error::Error;
use vissue_core::events;
use vissue_core::ops::{self, UpdateOutcome};
use vissue_core::store;
use vissue_core::views::{
    AgendaRow, ClaimRow, Excerpt, IssueDetail, IssueRec, ListQuery, Recall, RelatedHit, SearchHit,
    TreeNode,
};

use crate::backend::{BackendKind, BoardBackend, ListPage, MutResult, UpdateReq};

/// One-shot parse of the tracker, refreshed after mutations and `wait`.
#[derive(Debug)]
pub struct CoreBackend {
    layout: Layout,
    identity: String,
    recs: Mutex<Vec<IssueRec>>,
    generation: AtomicU64,
}

impl CoreBackend {
    /// Parse the vault once and stamp `identity` on later mutations.
    ///
    /// # Errors
    ///
    /// Returns an error if a project file cannot be read or parsed.
    pub fn open(layout: Layout, identity: impl Into<String>) -> Result<Self, Error> {
        let recs = load_recs(&layout)?;
        let generation = events::generation(&layout);
        Ok(Self {
            layout,
            identity: identity.into(),
            recs: Mutex::new(recs),
            generation: AtomicU64::new(generation),
        })
    }

    fn reload(&self) -> Result<(), Error> {
        let recs = load_recs(&self.layout)?;
        *self.recs.lock().expect("core recs") = recs;
        self.generation
            .store(events::generation(&self.layout), Ordering::SeqCst);
        Ok(())
    }

    fn with_service<T>(
        &self,
        f: impl FnOnce(&CatalogService<'_>) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let recs = self.recs.lock().expect("core recs");
        f(&CatalogService::from_recs(&recs))
    }

    fn mut_result(&self, report: String, id: &str) -> Result<MutResult, Error> {
        self.reload()?;
        let issue = self.with_service(|svc| svc.detail(id)).ok();
        Ok(MutResult {
            ok: true,
            report,
            issue,
            revision: 0,
            generation: self.generation.load(Ordering::SeqCst),
        })
    }
}

impl BoardBackend for CoreBackend {
    fn layout(&self) -> &Layout {
        &self.layout
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn revision(&self) -> u64 {
        0
    }

    fn live(&self) -> BackendKind {
        BackendKind::Core
    }

    fn identity(&self) -> &str {
        &self.identity
    }

    fn list(&self, q: ListQuery) -> Result<ListPage, Error> {
        self.with_service(|svc| {
            let issues = svc.issues_rows(q)?;
            let matched = issues.len() as u64;
            Ok(ListPage {
                issues,
                total: matched,
                matched,
                revision: 0,
                generation: self.generation.load(Ordering::SeqCst),
                unchanged: false,
            })
        })
    }

    fn ready(&self, project: Option<&str>) -> Result<ListPage, Error> {
        self.with_service(|svc| {
            let issues = svc.ready(project)?;
            let matched = issues.len() as u64;
            Ok(ListPage {
                issues,
                total: matched,
                matched,
                revision: 0,
                generation: self.generation.load(Ordering::SeqCst),
                unchanged: false,
            })
        })
    }

    fn get(&self, id: &str) -> Result<IssueDetail, Error> {
        self.with_service(|svc| svc.detail(id))
    }

    fn excerpt(&self, id: &str) -> Result<Excerpt, Error> {
        self.with_service(|svc| svc.excerpt(id))
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, Error> {
        self.with_service(|svc| svc.search(query, limit))
    }

    fn claims(&self, holder: Option<&str>, project: Option<&str>) -> Result<Vec<ClaimRow>, Error> {
        self.with_service(|svc| svc.claims(holder, project))
    }

    fn agenda(&self, days: i64, project: Option<&str>) -> Result<Vec<AgendaRow>, Error> {
        self.with_service(|svc| svc.agenda(days, project))
    }

    fn tree(&self, id: &str) -> Result<TreeNode, Error> {
        self.with_service(|svc| svc.tree(id))
    }

    fn related(&self, id: &str, depth: usize, limit: usize) -> Result<Vec<RelatedHit>, Error> {
        self.with_service(|svc| svc.related(id, depth, limit))
    }

    fn recall(&self, id: &str, depth: usize) -> Result<Recall, Error> {
        // The board paints the detail pane beside this, so it does not need the
        // inputs' prose spliced into the working set as well.
        self.with_service(|svc| svc.recall(id, depth, false))
    }

    fn deed(&self, id: &str, add: &[String]) -> Result<MutResult, Error> {
        let report = ops::deed(&self.layout, id, add, &[])?;
        self.mut_result(report, id)
    }

    fn projects(&self) -> Result<Vec<String>, Error> {
        store::list_projects(&self.layout)
    }

    fn claim(&self, id: &str, force: bool) -> Result<MutResult, Error> {
        let report = ops::claim_as(&self.layout, id, force, &self.identity)?;
        self.mut_result(report, id)
    }

    fn note(&self, id: &str, text: &str) -> Result<MutResult, Error> {
        let report = ops::note(&self.layout, id, text)?;
        self.mut_result(report, id)
    }

    fn update(&self, req: UpdateReq) -> Result<MutResult, Error> {
        let UpdateOutcome { report, .. } = ops::update_as_pred(
            &self.layout,
            &req.id,
            req.state.as_deref(),
            req.priority,
            req.block.as_deref(),
            req.unblock.as_deref(),
            &self.identity,
            vissue_core::UpdatePred {
                if_state: req.if_state.as_deref(),
                if_gen: req.if_gen,
            },
        )?;
        self.mut_result(report, &req.id)
    }

    fn create(&self, project: &str, title: &str) -> Result<MutResult, Error> {
        let report = ops::create(
            &self.layout,
            project,
            title,
            ops::CreateOpts {
                quiet: true,
                ..ops::CreateOpts::default()
            },
        )?;
        let id = report.split_whitespace().next().unwrap_or("").to_string();
        self.mut_result(report, &id)
    }

    fn open(&self, id: &str) -> Result<IssueDetail, Error> {
        self.get(id)
    }

    fn wait(&self, last: u64, timeout_ms: u64) -> Result<u64, Error> {
        let generation = events::wait_generation(&self.layout, last, 200, timeout_ms)?;
        if generation > last {
            self.reload()?;
        }
        Ok(generation)
    }

    fn refresh(&self) -> Result<(), Error> {
        self.reload()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BackendKind, BoardBackend, UpdateReq};
    use std::path::Path;
    use vissue_core::config::DEFAULT_PREFIX;
    use vissue_core::views::ListQuery;

    fn fixture() -> Layout {
        Layout::new(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixture_vault"),
            DEFAULT_PREFIX,
        )
    }

    #[test]
    fn core_revision_is_zero_and_ready_matches_fixture() {
        let backend = CoreBackend::open(fixture(), "snap").unwrap();
        assert_eq!(backend.revision(), 0);
        assert_eq!(backend.live(), BackendKind::Core);
        assert_eq!(backend.identity(), "snap");
        let page = backend.ready(None).unwrap();
        let ids: Vec<_> = page.issues.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["atlas-1a2b", "atlas-2c3d", "beacon-5j6k"]);
        assert_eq!(backend.wait(backend.generation(), 1).unwrap(), 0);
        assert!(!backend.projects().unwrap().is_empty());
        assert_eq!(
            backend
                .list(ListQuery {
                    project: Some("beacon".into()),
                    ..ListQuery::default()
                })
                .unwrap()
                .issues
                .len(),
            2
        );
        assert_eq!(backend.search("retry", 5).unwrap().len(), 1);
        assert_eq!(backend.claims(None, None).unwrap().len(), 1);
        assert!(!backend.agenda(400, None).unwrap().is_empty());
        let tree = backend.tree("atlas-1a2b").unwrap();
        assert_eq!(tree.id, "atlas-1a2b");
        assert!(!backend.related("atlas-1a2b", 2, 5).unwrap().is_empty());
        assert!(
            backend
                .excerpt("atlas-2c3d")
                .unwrap()
                .text
                .contains("summary")
        );
        assert_eq!(backend.open("atlas-2c3d").unwrap().id, "atlas-2c3d");
        assert!(backend.get("missing").is_err());
    }

    #[test]
    fn core_mutations_reload_the_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixture_vault");
        copy_tree(&src.join(DEFAULT_PREFIX), &dir.path().join(DEFAULT_PREFIX));
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        let backend = CoreBackend::open(layout, "tui-test").unwrap();
        assert!(backend.claim("atlas-2c3d", false).unwrap().ok);
        assert_eq!(
            backend.get("atlas-2c3d").unwrap().claimed_by.as_deref(),
            Some("tui-test")
        );
        assert!(backend.note("atlas-2c3d", "hi").unwrap().ok);
        assert!(
            backend
                .update(UpdateReq {
                    id: "atlas-2c3d".into(),
                    state: Some("BLOCKED".into()),
                    ..UpdateReq::default()
                })
                .unwrap()
                .ok
        );
        assert_eq!(backend.get("atlas-2c3d").unwrap().state, "BLOCKED");
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
}
