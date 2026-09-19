//! Projection: the boards a shared repository shows from other trackers,
//! and the way work goes back to them.
//!
//! A tracker is a directory of org files, so sharing one across machines
//! has meant copying files by hand and a shell script per repository that
//! knew every project by name. This module makes that one declaration and
//! one verb. The repository's `vissue.toml` names its boards:
//!
//! ```toml
//! [[projection.board]]
//! project = "ljos"                        # the project's name on its source
//! source  = "vault"                        # a [layouts.*] name from the user's
//!                                          # config, "self", or a path
//! mirror  = "Software/ljos/issues-mirror.org"
//! inbox   = "Software/ljos/inbox.org"      # optional: `* TODO` headings become issues
//! claims  = "Software/ljos/claims.org"     # optional: `* TODO claim ID as AGENT` lines
//! ```
//!
//! `vissue project` folds each inbox into its source, applies each claims
//! file, and rewrites each mirror; `--check` says which mirrors are stale.
//! A source that is not on this machine is reported and its mirror left as
//! projected, so a delegate can run the same verb and learn what it can do
//! here. `vissue show ID` on an id that lives only in a mirror answers from
//! the mirror and names the inbox to write to.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

use crate::config::Layout;
use crate::error::Result;
use crate::mirror::{self, Format};
use crate::ops;
use crate::router::Router;
use crate::store;

/// One projected board, as `[[projection.board]]` declares it.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Board {
    /// The project's name on its source tracker.
    pub project: String,
    /// `self`, a `[layouts.*]` name from the user's config, or a path.
    #[serde(default = "self_source")]
    pub source: String,
    /// The mirror file, relative to the repository root.
    pub mirror: PathBuf,
    /// An inbox whose unstamped `* TODO` headings fold into the source.
    #[serde(default)]
    pub inbox: Option<PathBuf>,
    /// A file of `* TODO claim ID as AGENT` and `release` lines to apply.
    #[serde(default)]
    pub claims: Option<PathBuf>,
}

fn self_source() -> String {
    "self".to_string()
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Projection {
    board: Vec<Board>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct File {
    projection: Projection,
}

/// The boards `<root>/vissue.toml` declares; none when it declares none.
///
/// # Errors
///
/// A `vissue.toml` that cannot be read or parsed.
pub fn boards(root: &Path) -> Result<Vec<Board>> {
    let path = root.join("vissue.toml");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let parsed: File = toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    Ok(parsed.projection.board)
}

/// The tracker a board's `source` names, when it is on this machine: the
/// repository's own tracker for `self`, a `[layouts.*]` name from the
/// user's config, else a path that holds a tracker.
#[must_use]
pub fn resolve_source(router: &Router, source: &str) -> Option<Layout> {
    if source == "self" {
        return Some(router.default_layout().clone());
    }
    if let Some(named) = router.named_layout(source) {
        return Some(named.clone());
    }
    // A path source is one on this machine that holds a tracker; a bare
    // layout name the user's config does not know is not a directory here.
    let expanded = if let Some(rest) = source.strip_prefix("~/") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest))?
    } else {
        PathBuf::from(source)
    };
    if !expanded.is_dir() {
        return None;
    }
    let layout = crate::config::layout_at(&expanded).ok()?;
    if !layout.root().join("vissue.toml").is_file() && !layout.projects_dir().is_dir() {
        return None;
    }
    Some(layout)
}

/// What one run of `vissue project` did or found.
#[derive(Debug, Default)]
pub struct Outcome {
    /// One line per step, in order.
    pub lines: Vec<String>,
    /// Files this run rewrote, for the caller to commit.
    pub touched: Vec<PathBuf>,
    /// Mirrors found stale under `--check`.
    pub stale: usize,
    /// Boards whose source is not on this machine.
    pub skipped: usize,
}

/// Run the projection declared under `repo`: fold, claim, mirror, or with
/// `check` only compare each mirror's stamp against its source.
///
/// # Errors
///
/// A board's fold, claim, mirror or check failing; a missing source is not
/// an error, it is reported and skipped.
pub fn project(router: &Router, repo: &Path, check: bool) -> Result<Outcome> {
    let mut out = Outcome::default();
    let boards = boards(repo)?;
    if boards.is_empty() {
        out.lines.push(format!(
            "no [[projection.board]] in {}; nothing to project",
            repo.join("vissue.toml").display()
        ));
        return Ok(out);
    }
    for board in &boards {
        let mirror_path = repo.join(&board.mirror);
        let Some(source) = resolve_source(router, &board.source) else {
            out.skipped += 1;
            out.lines.push(format!(
                "{}: source {} is not on this seat; {} stays as projected",
                board.project,
                board.source,
                board.mirror.display()
            ));
            continue;
        };
        let projects = vec![board.project.clone()];
        if check {
            if mirror_path.exists() {
                let verdict = mirror::check(&source, &mirror_path, &projects)?;
                if !verdict.fresh {
                    out.stale += 1;
                }
                out.lines
                    .push(format!("{}: {}", board.project, verdict.report.trim_end()));
            } else {
                out.stale += 1;
                out.lines.push(format!(
                    "{}: {} does not exist yet",
                    board.project,
                    board.mirror.display()
                ));
            }
            continue;
        }
        if let Some(inbox) = &board.inbox {
            let inbox_path = repo.join(inbox);
            if inbox_path.exists() {
                let folded = ops::fold(&source, &inbox_path, &board.project)?;
                out.lines
                    .push(format!("{}: {}", board.project, folded.trim_end()));
                out.touched.push(inbox_path);
            }
        }
        if let Some(claims) = &board.claims {
            let claims_path = repo.join(claims);
            if claims_path.exists() && apply_claims(&source, &claims_path, &mut out.lines)? {
                out.touched.push(claims_path);
            }
        }
        let text = mirror::render(&source, &projects, Format::Org, None)?;
        if let Some(parent) = mirror_path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        store::replace_file_atomically(&mirror_path, &text)?;
        out.lines.push(format!(
            "{}: wrote {}",
            board.project,
            board.mirror.display()
        ));
        out.touched.push(mirror_path);
    }
    Ok(out)
}

/// Apply the unstamped lines of a claims file to the source and stamp them
/// in place. Returns whether the file changed.
fn apply_claims(source: &Layout, path: &Path, lines: &mut Vec<String>) -> Result<bool> {
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut changed = false;
    let mut rewritten = Vec::new();
    for line in text.lines() {
        let stamped = if let Some(rest) = line.strip_prefix("* TODO claim ") {
            match rest.rsplit_once(" as ") {
                Some((issue, agent)) => {
                    let result = ops::claim_as(source, issue.trim(), false, agent.trim())
                        .map_or_else(|e| format!("FAILED {e}"), |ok| ok.trim().to_string());
                    lines.push(format!("claim {issue} as {agent}: {result}"));
                    Some(format!(
                        "* DONE claim {} as {} :: {result}",
                        issue.trim(),
                        agent.trim()
                    ))
                }
                None => None,
            }
        } else if let Some(rest) = line.strip_prefix("* TODO release ") {
            match rest.rsplit_once(" as ") {
                Some((issue, agent)) => {
                    let result = ops::update(source, issue.trim(), Some("TODO"), None, None, None)
                        .map_or_else(|e| format!("FAILED {e}"), |ok| ok.report);
                    lines.push(format!("release {issue} as {agent}: {result}"));
                    Some(format!(
                        "* DONE release {} as {} :: {result}",
                        issue.trim(),
                        agent.trim()
                    ))
                }
                None => None,
            }
        } else if let Some(rest) = line.strip_prefix("* TODO done ") {
            // A seat without the source closes its ticket through the file;
            // the state moves and the claim releases as an update would.
            match rest.rsplit_once(" as ") {
                Some((issue, agent)) => {
                    let result = ops::update(source, issue.trim(), Some("DONE"), None, None, None)
                        .map_or_else(|e| format!("FAILED {e}"), |ok| ok.report);
                    lines.push(format!("done {issue} as {agent}: {result}"));
                    Some(format!(
                        "* DONE done {} as {} :: {result}",
                        issue.trim(),
                        agent.trim()
                    ))
                }
                None => None,
            }
        } else {
            None
        };
        match stamped {
            Some(s) => {
                changed = true;
                rewritten.push(s);
            }
            None => rewritten.push(line.to_string()),
        }
    }
    if changed {
        let mut body = rewritten.join("\n");
        body.push('\n');
        store::replace_file_atomically(path, &body)?;
    }
    Ok(changed)
}

/// An id that lives in one of the repository's mirrors: the board and the
/// heading's text as the mirror carries it, so a reader on a machine
/// without the source still gets an answer and is told where to write.
#[must_use]
pub fn find_in_mirrors(repo: &Path, id: &str) -> Option<(Board, String)> {
    for board in boards(repo).ok()? {
        let path = repo.join(&board.mirror);
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        let at = lines.iter().position(|l| {
            let t = l.trim();
            t.starts_with(":ID:") && t[4..].trim() == id
        })?;
        let start = lines[..at]
            .iter()
            .rposition(|l| l.starts_with("** "))
            .unwrap_or(at);
        let end = lines[at..]
            .iter()
            .position(|l| l.starts_with("** ") || l.starts_with("* "))
            .map_or(lines.len(), |n| at + n);
        let block: Vec<String> = lines[start..end]
            .iter()
            .map(|l| l.strip_prefix('*').unwrap_or(l).to_string())
            .collect();
        return Some((board, block.join("\n").trim_end().to_string() + "\n"));
    }
    None
}

/// The one line a reader of a projected issue needs beside the heading.
#[must_use]
pub fn projected_note(board: &Board) -> String {
    match &board.inbox {
        Some(inbox) => format!(
            "read-only projection of {} from {}; write discovered work to {}",
            board.project,
            board.source,
            inbox.display()
        ),
        None => format!(
            "read-only projection of {} from {}; it takes no work back",
            board.project, board.source
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_board_is_read_from_the_repository_config() {
        let dir = std::env::temp_dir().join(format!("vissue-proj-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("vissue.toml"),
            "prefix = \"Issues\"\n\n[[projection.board]]\nproject = \"ljos\"\nsource = \"vault\"\nmirror = \"Software/ljos/issues-mirror.org\"\ninbox = \"Software/ljos/inbox.org\"\n\n[[projection.board]]\nproject = \"surf\"\nmirror = \"Software/surf/issues-mirror.org\"\n",
        )
        .unwrap();
        let boards = boards(&dir).unwrap();
        assert_eq!(boards.len(), 2);
        assert_eq!(boards[0].source, "vault");
        assert_eq!(
            boards[0].inbox.as_deref(),
            Some(Path::new("Software/ljos/inbox.org"))
        );
        assert_eq!(
            boards[1].source, "self",
            "a board with no source is this tracker's"
        );
        // A projected id answers from the mirror and names the inbox.
        fs::create_dir_all(dir.join("Software/ljos")).unwrap();
        fs::write(
            dir.join("Software/ljos/issues-mirror.org"),
            "#+TITLE: vissue mirror\n# SYNC: digest=1 generation=1\n\n* ljos\n** TODO [#A] The seat under a herd :task:\n:PROPERTIES:\n:ID:         ljos-kcd6\n:END:\n\nBody line.\n** DONE [#C] Another\n:PROPERTIES:\n:ID:         ljos-zzzz\n:END:\n",
        )
        .unwrap();
        let (board, text) = find_in_mirrors(&dir, "ljos-kcd6").expect("found in the mirror");
        assert_eq!(board.project, "ljos");
        assert!(
            text.starts_with("* TODO [#A] The seat under a herd"),
            "{text}"
        );
        assert!(text.contains("Body line."), "{text}");
        assert!(!text.contains("Another"), "{text}");
        assert!(projected_note(&board).contains("Software/ljos/inbox.org"));
        assert!(find_in_mirrors(&dir, "ljos-none").is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
