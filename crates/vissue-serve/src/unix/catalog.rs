//! In-memory catalog. The hot path never calls `load_all`.

use std::collections::HashMap;
use std::path::Path;

use crate::error::Result;
use vissue_core::catalog::load_recs;
use vissue_core::config::Layout;
use vissue_core::events;
use vissue_core::store::{IssueDoc, list_projects};
use vissue_core::views::IssueRec;

/// First serve-local revision after a successful load.
pub const INITIAL_REVISION: u64 = 1;

/// Parsed corpus for one owner process.
#[derive(Debug, Clone)]
pub struct Catalog {
    pub revision: u64,
    pub generation: u64,
    pub issues: Vec<IssueRec>,
    pub by_id: HashMap<String, usize>,
    pub projects: Vec<String>,
    pub dirty_projects: Vec<String>,
    pub dirty_ids: Option<Vec<String>>,
}

impl Catalog {
    pub fn load(layout: &Layout) -> Result<Self> {
        let issues = load_recs(layout)?;
        let mut cat = Self {
            revision: INITIAL_REVISION,
            generation: events::generation(layout),
            issues,
            by_id: HashMap::new(),
            projects: list_projects(layout).unwrap_or_default(),
            dirty_projects: Vec::new(),
            dirty_ids: None,
        };
        cat.reindex();
        Ok(cat)
    }

    pub fn reindex(&mut self) {
        self.by_id.clear();
        self.by_id.reserve(self.issues.len());
        for (idx, rec) in self.issues.iter().enumerate() {
            self.by_id.insert(rec.heading.id.clone(), idx);
        }
    }

    pub fn apply_full(
        &mut self,
        layout: &Layout,
        issues: Vec<IssueRec>,
        dirty_projects: Vec<String>,
        dirty_ids: Option<Vec<String>>,
    ) {
        self.issues = issues;
        self.projects = list_projects(layout).unwrap_or_default();
        self.generation = events::generation(layout);
        self.revision = self.revision.saturating_add(1);
        self.dirty_projects = dirty_projects;
        self.dirty_ids = dirty_ids;
        self.reindex();
    }

    /// Swap recs for every dirty project in one invalidation: one revision
    /// tick, unioned `dirty_projects` / `dirty_ids`.
    pub fn replace_projects(
        &mut self,
        layout: &Layout,
        parts: impl IntoIterator<Item = (String, Vec<IssueRec>)>,
    ) {
        let mut dirty_projects = Vec::new();
        let mut dirty_ids = Vec::new();
        for (project, fresh) in parts {
            self.issues.retain(|rec| rec.project != project);
            dirty_ids.extend(fresh.iter().map(|rec| rec.heading.id.clone()));
            self.issues.extend(fresh);
            if !self.projects.iter().any(|p| p == &project) {
                if layout.project_issues_path(&project).exists() {
                    self.projects.push(project.clone());
                    self.projects.sort();
                }
            } else if !layout.project_issues_path(&project).exists() {
                self.projects.retain(|p| p != &project);
            }
            dirty_projects.push(project);
        }
        dirty_projects.sort();
        dirty_projects.dedup();
        dirty_ids.sort();
        dirty_ids.dedup();
        self.generation = events::generation(layout);
        self.revision = self.revision.saturating_add(1);
        self.dirty_projects = dirty_projects;
        self.dirty_ids = Some(dirty_ids);
        self.reindex();
    }
}

/// Parse one project's `issues.org` into recs. Missing file is an empty list.
pub fn load_project_recs(layout: &Layout, project: &str) -> Result<Vec<IssueRec>> {
    let path = layout.project_issues_path(project);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let doc = IssueDoc::parse_file(project, &path)?;
    let tag_settings = doc.tag_settings.clone();
    let files: Vec<std::path::PathBuf> = doc
        .headings
        .iter()
        .map(|heading| doc.heading_path(&heading.id).to_path_buf())
        .collect();
    Ok(doc
        .headings
        .into_iter()
        .zip(files)
        .map(|(heading, file)| IssueRec {
            project: project.to_string(),
            heading,
            path: file,
            tag_settings: tag_settings.clone(),
        })
        .collect())
}

/// Project directory name under the prefix, if `path` sits inside it.
pub fn project_from_path(layout: &Layout, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(layout.projects_dir()).ok()?;
    rel.components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use vissue_core::config::Layout;

    #[test]
    fn empty_layout_loads_revision_one() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), "Software");
        let cat = Catalog::load(&layout).unwrap();
        assert_eq!(cat.revision, INITIAL_REVISION);
        assert!(cat.issues.is_empty());
        assert!(cat.projects.is_empty());
    }

    #[test]
    fn apply_full_increments_revision() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), "Software");
        let mut cat = Catalog::load(&layout).unwrap();
        cat.apply_full(&layout, Vec::new(), vec!["atlas".into()], None);
        assert_eq!(cat.revision, 2);
        assert_eq!(cat.dirty_projects, ["atlas"]);
    }

    #[test]
    fn project_from_path_reads_first_component() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), "Software");
        let path = layout.projects_dir().join("atlas/issues.org");
        assert_eq!(project_from_path(&layout, &path).as_deref(), Some("atlas"));
        assert!(project_from_path(&layout, dir.path()).is_none());
    }

    #[test]
    fn replace_project_swaps_recs() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), "Software");
        let project = layout.projects_dir().join("atlas");
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join("issues.org"),
            "* TODO [#C] One\n:PROPERTIES:\n:ID:         atlas-aaaa\n:END:\n",
        )
        .unwrap();
        let mut cat = Catalog::load(&layout).unwrap();
        assert_eq!(cat.issues.len(), 1);
        fs::write(
            project.join("issues.org"),
            "* TODO [#C] Two\n:PROPERTIES:\n:ID:         atlas-bbbb\n:END:\n",
        )
        .unwrap();
        let fresh = load_project_recs(&layout, "atlas").unwrap();
        cat.replace_projects(&layout, [("atlas".into(), fresh)]);
        assert_eq!(cat.revision, 2);
        assert_eq!(cat.issues.len(), 1);
        assert_eq!(cat.issues[0].heading.id, "atlas-bbbb");
        assert_eq!(cat.dirty_projects, ["atlas"]);
    }

    #[test]
    fn replace_projects_unions_dirty_and_bumps_revision_once() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), "Software");
        for (name, id) in [("atlas", "atlas-aaaa"), ("beacon", "beacon-bbbb")] {
            let project = layout.projects_dir().join(name);
            fs::create_dir_all(&project).unwrap();
            fs::write(
                project.join("issues.org"),
                format!("* TODO [#C] One\n:PROPERTIES:\n:ID:         {id}\n:END:\n"),
            )
            .unwrap();
        }
        let mut cat = Catalog::load(&layout).unwrap();
        assert_eq!(cat.revision, INITIAL_REVISION);
        assert_eq!(cat.issues.len(), 2);

        fs::write(
            layout.projects_dir().join("atlas/issues.org"),
            "* TODO [#C] Two\n:PROPERTIES:\n:ID:         atlas-cccc\n:END:\n",
        )
        .unwrap();
        fs::write(
            layout.projects_dir().join("beacon/issues.org"),
            "* TODO [#C] Two\n:PROPERTIES:\n:ID:         beacon-dddd\n:END:\n",
        )
        .unwrap();
        let parts = vec![
            ("atlas".into(), load_project_recs(&layout, "atlas").unwrap()),
            (
                "beacon".into(),
                load_project_recs(&layout, "beacon").unwrap(),
            ),
        ];
        cat.replace_projects(&layout, parts);
        assert_eq!(cat.revision, 2);
        assert_eq!(cat.dirty_projects, ["atlas", "beacon"]);
        let mut ids = cat.dirty_ids.clone().unwrap();
        ids.sort();
        assert_eq!(ids, ["atlas-cccc", "beacon-dddd"]);
        assert_eq!(cat.issues.len(), 2);
    }
}
