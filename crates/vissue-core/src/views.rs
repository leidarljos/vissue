//! Typed issue views shared by JSON output and later control clients.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::model::IssueHeading;

/// One parsed heading plus the `issues.org` it came from.
#[derive(Debug, Clone)]
pub struct IssueRec {
    pub project: String,
    pub heading: IssueHeading,
    pub path: PathBuf,
}

/// Filters for [`crate::catalog::CatalogService::issues_rows`].
#[derive(Debug, Clone, Default)]
pub struct ListQuery {
    pub project: Option<String>,
    pub state: Option<String>,
    pub ready: bool,
    /// Case-insensitive substring over id, title, tags, and properties.
    pub query: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueRow {
    pub id: String,
    pub state: String,
    pub priority: String,
    pub title: String,
    pub project: String,
    pub blocked_by: Vec<String>,
    pub claimed_by: Option<String>,
    pub claimed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueDetail {
    pub id: String,
    pub project: String,
    pub title: String,
    pub state: String,
    pub priority: String,
    pub properties: BTreeMap<String, String>,
    pub org_tags: Vec<String>,
    pub tags: Vec<String>,
    pub blocked_by: Vec<String>,
    pub parent: Option<String>,
    pub claimed_by: Option<String>,
    pub claimed_at: Option<String>,
    pub file: String,
    pub line_start: usize,
    pub line_end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClaimRow {
    pub id: String,
    pub project: String,
    pub state: String,
    pub priority: String,
    pub holder: Option<String>,
    pub claimed_at: Option<String>,
    pub age_days: i64,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Excerpt {
    pub id: String,
    pub file: String,
    pub line_start: usize,
    pub line_end: usize,
    pub text: String,
    pub suppressed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchHit {
    pub id: String,
    pub project: String,
    pub state: String,
    pub priority: String,
    pub title: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgendaRow {
    pub date: String,
    pub kind: String,
    pub overdue_days: i64,
    pub id: String,
    pub project: String,
    pub state: String,
    pub priority: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeNode {
    pub id: String,
    pub state: String,
    pub title: String,
    pub children: Vec<TreeNode>,
    pub blocked_by: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RelatedHit {
    pub id: String,
    pub project: String,
    pub state: String,
    pub title: String,
    pub score: f64,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalkHit {
    pub id: String,
    pub project: String,
    pub state: String,
    pub title: String,
    pub relation: String,
}
