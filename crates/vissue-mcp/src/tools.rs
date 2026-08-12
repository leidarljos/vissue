//! Argument schemas for the MCP tool surface.

use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
pub struct ProjectArgs {
    /// Project name. Omit to cover every project.
    pub project: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ListArgs {
    /// Project name. Omit to cover every project.
    pub project: Option<String>,
    /// State filter: TODO, STARTED, BLOCKED, DONE, or CANCELLED.
    pub state: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct IdArgs {
    /// Issue id.
    pub issue_id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct CreateArgs {
    /// Project name.
    pub project: String,
    /// One-line title.
    pub title: String,
    /// Priority cookie: A, B, or C.
    pub priority: Option<String>,
    /// Type tag such as feature, bug, task, chore, or plan.
    pub issue_type: Option<String>,
    /// Comma- or colon-separated tags.
    pub tags: Option<String>,
    /// Parent id for a plan or spec hierarchy.
    pub parent: Option<String>,
    /// Body prose written under the heading.
    pub body: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct UpdateArgs {
    /// Issue id to update.
    pub issue_id: String,
    /// New state: TODO, STARTED, BLOCKED, DONE, or CANCELLED.
    pub state: Option<String>,
    /// New priority cookie: A, B, or C.
    pub priority: Option<String>,
    /// Add a blocker edge.
    pub block: Option<String>,
    /// Remove a blocker edge.
    pub unblock: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct CountArgs {
    /// Project name. Omit to cover every project.
    pub project: Option<String>,
    /// State filter.
    pub state: Option<String>,
    /// Count only actionable issues.
    pub ready: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// Case-insensitive substring.
    pub query: String,
    /// Maximum rows returned.
    pub limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
pub struct RelatedArgs {
    /// Issue id used as the center of the derived relation query.
    pub issue_id: String,
    /// Maximum Org relation hops to traverse.
    pub depth: Option<usize>,
    /// Maximum rows returned.
    pub limit: Option<usize>,
    /// Output format: text or org.
    pub format: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TreeArgs {
    /// Root issue id.
    pub issue_id: String,
    /// Output format: ascii or dot.
    pub format: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct MirrorArgs {
    /// Projects to include. Omit to cover every project.
    pub projects: Option<Vec<String>>,
    /// Output format: org or markdown.
    pub format: Option<String>,
    /// Include only this state.
    pub state: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct DigestArgs {
    /// Projects to digest. Omit to cover every project.
    pub projects: Option<Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
pub struct MirrorCheckArgs {
    /// Path to the mirror file to check.
    pub path: String,
    /// Projects to compare. Omit to use the ones the stamp names.
    pub projects: Option<Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ClaimArgs {
    /// Issue id.
    pub issue_id: String,
    /// Take over a claim held by another identity.
    pub force: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct NoteArgs {
    /// Issue id.
    pub issue_id: String,
    /// Note text; appended to the logbook with a timestamp.
    pub text: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ClaimsArgs {
    /// Only claims held by this identity.
    pub holder: Option<String>,
    /// Only claims in this project.
    pub project: Option<String>,
    /// Machine-readable JSON array instead of text rows.
    pub json: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct AgendaArgs {
    /// Days ahead to include (default 14). Overdue items always appear.
    pub days: Option<i64>,
    /// Only issues in this project.
    pub project: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct FoldArgs {
    /// Absolute path of the inbox org file to fold.
    pub file: String,
    /// Project the folded issues are created in.
    pub project: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct HygieneArgs {
    /// Days a claim may be held before it counts as stale.
    pub stale_days: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct EventsArgs {
    /// Only events with a sequence above this value.
    pub since: Option<u64>,
    /// Maximum events returned.
    pub limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
pub struct PingArgs {
    /// Note recorded on the event.
    pub detail: Option<String>,
}

/// Parse a single-character priority cookie from the wire representation.
pub fn priority_char(raw: Option<&String>) -> Option<char> {
    raw.and_then(|s| s.trim().chars().next())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_takes_the_first_character() {
        assert_eq!(priority_char(Some(&"A".to_string())), Some('A'));
        assert_eq!(priority_char(Some(&" b".to_string())), Some('b'));
        assert_eq!(priority_char(Some(&String::new())), None);
        assert_eq!(priority_char(None), None);
    }
}
