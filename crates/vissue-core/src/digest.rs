//! A content digest of the corpus, so a consumer can tell whether a projection
//! it holds is still current.
//!
//! The digest is taken over the canonical JSONL export, which is already
//! deterministic and byte-for-byte stable. Hashing that rather than the org
//! files means formatting churn that does not change an issue does not change
//! the digest either. Every function here reads; none writes.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Write as _;
use xxhash_rust::xxh3::xxh3_64;

use crate::config::Layout;
use crate::events;
use crate::report;
use crate::store::list_projects;

/// One project's contribution to the digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProjectDigest {
    /// Project directory name.
    pub project: String,
    /// 16-digit hex of the xxh3 hash over that project's export.
    pub digest: String,
    /// Number of JSONL rows in that export.
    pub issues: usize,
}

/// The digest of a selected set of projects, plus what it was taken against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CorpusDigest {
    /// Hash over the per-project digests, so it moves exactly when one of them
    /// does.
    pub combined: String,
    /// Per-project digests, sorted by project name.
    pub projects: Vec<ProjectDigest>,
    /// Sum of per-project issue counts.
    pub issues: usize,
    /// Event-log generation at the time of the digest.
    pub generation: u64,
}

fn hex(value: u64) -> String {
    format!("{value:016x}")
}

/// Digest one project's export bytes.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read or exported.
pub fn project_digest(layout: &Layout, project: &str) -> Result<ProjectDigest> {
    let export = report::export(layout, Some(project))?;
    Ok(ProjectDigest {
        project: project.to_string(),
        digest: hex(xxh3_64(export.as_bytes())),
        issues: export.lines().filter(|l| !l.trim().is_empty()).count(),
    })
}

/// Digest the named projects, or every project when the list is empty.
///
/// # Errors
///
/// Returns an error if the project list or the corpus cannot be read.
pub fn corpus_digest(layout: &Layout, projects: &[String]) -> Result<CorpusDigest> {
    let mut selected: Vec<String> = if projects.is_empty() {
        list_projects(layout)?
    } else {
        projects.to_vec()
    };
    selected.sort();
    selected.dedup();

    // One read of the corpus, grouped, rather than one read per project.
    // `project_digest` filters a whole-corpus export down to a single
    // project, so calling it in a loop is quadratic in the project count.
    let grouped = report::export_by_project(layout)?;
    let per_project: Vec<ProjectDigest> = selected
        .iter()
        .map(|project| {
            let export = grouped.get(project).map(String::as_str).unwrap_or("");
            ProjectDigest {
                project: project.clone(),
                digest: hex(xxh3_64(export.as_bytes())),
                issues: export.lines().filter(|l| !l.trim().is_empty()).count(),
            }
        })
        .collect();

    // Combine over the sub-digests rather than over the raw bytes, so the
    // combined value is a pure function of the parts a reader can inspect.
    let mut material = String::new();
    for p in &per_project {
        let _ = writeln!(material, "{}={}", p.project, p.digest);
    }

    Ok(CorpusDigest {
        combined: hex(xxh3_64(material.as_bytes())),
        issues: per_project.iter().map(|p| p.issues).sum(),
        projects: per_project,
        generation: events::generation(layout),
    })
}

impl CorpusDigest {
    /// A summary line, then one line per project.
    ///
    /// The per-project lines are what turn "something moved" into "this moved",
    /// which is why they are not folded into the summary.
    pub fn render(&self) -> String {
        let mut out = format!(
            "combined={} issues={} generation={} projects={}\n",
            self.combined,
            self.issues,
            self.generation,
            self.projects.len()
        );
        for p in &self.projects {
            let _ = writeln!(out, "{}  {:>6}  {}", p.digest, p.issues, p.project);
        }
        out
    }

    /// The digest as a JSON object: combined hash, counts, and per-project rows.
    ///
    /// The type's own serialization rather than a second copy of its shape
    /// written out by hand, which is a second place for a field to be added and
    /// only one of them to get it.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// The per-project digest for `project`, when that name is in this corpus.
    pub fn digest_of(&self, project: &str) -> Option<&str> {
        self.projects
            .iter()
            .find(|p| p.project == project)
            .map(|p| p.digest.as_str())
    }

    /// Project names in this digest, in the same order as [`Self::projects`].
    pub fn project_names(&self) -> Vec<String> {
        self.projects.iter().map(|p| p.project.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_PREFIX;
    use crate::ops::{CreateOpts, create};
    use std::fs;

    /// A tracker with `projects` projects, two issues each.
    fn many_projects(projects: usize) -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        fs::create_dir_all(layout.projects_dir()).unwrap();
        for p in 0..projects {
            let project = format!("p{p:03}");
            for i in 0..2 {
                create(
                    &layout,
                    &project,
                    &format!("issue {i}"),
                    CreateOpts {
                        quiet: true,
                        body: Some("Body text, so a row is not trivially short."),
                        ..CreateOpts::default()
                    },
                )
                .unwrap();
            }
        }
        (dir, layout)
    }

    /// Each project's digest is taken over exactly what `export` returns for
    /// it. The grouped read exists for speed, and a digest that moved would
    /// mark every mirror ever stamped as stale.
    #[test]
    fn a_project_digest_matches_its_own_export() {
        let (_dir, layout) = many_projects(4);
        let grouped = crate::report::export_by_project(&layout).unwrap();
        for project in crate::store::list_projects(&layout).unwrap() {
            let alone = crate::report::export(&layout, Some(&project)).unwrap();
            assert_eq!(
                grouped.get(&project).map(String::as_str).unwrap_or(""),
                alone,
                "{project}: grouped export differs from its own"
            );
            let single = project_digest(&layout, &project).unwrap();
            let combined = corpus_digest(&layout, &[]).unwrap();
            assert_eq!(
                combined.digest_of(&project),
                Some(single.digest.as_str()),
                "{project}: corpus digest disagrees with the project's own"
            );
        }
    }

    /// Digesting the corpus reads it once, not once per project.
    ///
    /// Compared against `export`, which reads it once by definition, so the
    /// bound calibrates itself to the machine rather than to a clock. Taking
    /// one export per project costs one read each; the project count has to
    /// be high enough that the difference clears the fixed slack, or a
    /// quadratic version passes on a small corpus.
    #[test]
    fn the_corpus_digest_does_not_read_once_per_project() {
        let (_dir, layout) = many_projects(120);
        let started = std::time::Instant::now();
        let _ = crate::report::export(&layout, None).unwrap();
        let one_read = started.elapsed();

        let started = std::time::Instant::now();
        let digest = corpus_digest(&layout, &[]).unwrap();
        let whole = started.elapsed();

        assert_eq!(digest.projects.len(), 120);
        assert!(
            whole < one_read * 5 + std::time::Duration::from_millis(25),
            "digest took {whole:?} against a single export of {one_read:?}, \
             which is the shape of one read per project"
        );
    }

    fn seeded() -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        fs::create_dir_all(layout.projects_dir()).unwrap();
        create(&layout, "alpha", "first", CreateOpts::default()).unwrap();
        create(&layout, "beta", "second", CreateOpts::default()).unwrap();
        (dir, layout)
    }

    #[test]
    fn a_digest_is_stable_across_runs() {
        let (_dir, layout) = seeded();
        let once = corpus_digest(&layout, &[]).unwrap();
        let twice = corpus_digest(&layout, &[]).unwrap();
        assert_eq!(once.combined, twice.combined);
        assert_eq!(once.projects, twice.projects);
        assert_eq!(once.combined.len(), 16, "{}", once.combined);
    }

    #[test]
    fn the_combined_digest_follows_the_parts() {
        let (_dir, layout) = seeded();
        let before = corpus_digest(&layout, &[]).unwrap();
        create(&layout, "alpha", "a third issue", CreateOpts::default()).unwrap();
        let after = corpus_digest(&layout, &[]).unwrap();

        assert_ne!(before.combined, after.combined, "the corpus moved");
        assert_ne!(
            before.digest_of("alpha"),
            after.digest_of("alpha"),
            "the changed project moved"
        );
        assert_eq!(
            before.digest_of("beta"),
            after.digest_of("beta"),
            "an untouched project must not move"
        );
    }

    #[test]
    fn selecting_projects_narrows_the_digest() {
        let (_dir, layout) = seeded();
        let all = corpus_digest(&layout, &[]).unwrap();
        let one = corpus_digest(&layout, &["alpha".to_string()]).unwrap();
        assert_eq!(one.projects.len(), 1);
        assert_ne!(all.combined, one.combined);
        assert_eq!(all.digest_of("alpha"), one.digest_of("alpha"));
    }

    #[test]
    fn selection_order_does_not_change_the_digest() {
        let (_dir, layout) = seeded();
        let forward = corpus_digest(&layout, &["alpha".into(), "beta".into()]).unwrap();
        let backward = corpus_digest(&layout, &["beta".into(), "alpha".into()]).unwrap();
        assert_eq!(forward.combined, backward.combined);
    }

    #[test]
    fn the_rendered_form_names_every_project() {
        let (_dir, layout) = seeded();
        let digest = corpus_digest(&layout, &[]).unwrap();
        let text = digest.render();
        assert!(
            text.starts_with(&format!("combined={}", digest.combined)),
            "{text}"
        );
        assert!(text.contains("alpha"), "{text}");
        assert!(text.contains("beta"), "{text}");
        assert_eq!(text.lines().count(), 3, "{text}");

        let value = digest.to_json();
        assert_eq!(value["combined"], digest.combined);
        assert_eq!(value["projects"].as_array().unwrap().len(), 2);
    }
}
