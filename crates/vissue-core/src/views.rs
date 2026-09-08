//! Typed issue views shared by JSON output and later control clients.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::model::IssueHeading;

/// One parsed heading plus the `issues.org` it came from.
#[derive(Debug, Clone)]
pub struct IssueRec {
    /// Project directory name the heading lives under.
    pub project: String,
    /// Parsed heading, including body and logbook.
    pub heading: IssueHeading,
    /// Absolute path of the project's `issues.org`.
    pub path: PathBuf,
    /// File-level tags and `#+TAGS:` groups from the preamble.
    pub tag_settings: crate::org::TagSettings,
}

/// Filters for [`crate::catalog::CatalogService::issues_rows`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListQuery {
    /// Restrict to this project name (case-insensitive).
    pub project: Option<String>,
    /// Restrict to this TODO keyword.
    pub state: Option<String>,
    /// Keep only TODO or STARTED issues with no open blocker.
    pub ready: bool,
    /// Case-insensitive substring over id, title, tags, and properties.
    pub query: Option<String>,
    /// Cap the result after sorting.
    pub limit: Option<usize>,
    /// Drop this many leading rows after sorting.
    pub offset: Option<usize>,
}

/// One list/ready row: the fields a board or JSON client paints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueRow {
    /// Issue id, `<project>-<suffix>`.
    pub id: String,
    /// TODO keyword on the heading.
    pub state: String,
    /// Priority cookie as a one-character string.
    pub priority: String,
    /// Heading title, without tags.
    pub title: String,
    /// Project the heading lives in.
    pub project: String,
    /// Ids listed in `:BLOCKED_BY:`.
    pub blocked_by: Vec<String>,
    /// Identity holding the issue, when claimed.
    pub claimed_by: Option<String>,
    /// Org timestamp of the claim.
    pub claimed_at: Option<String>,
    /// `:PARENT:` id, when set.
    #[serde(default)]
    pub parent: Option<String>,
}

/// One issue as a detail card: properties, tags, file range, body, and logbook.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueDetail {
    /// Issue id, `<project>-<suffix>`.
    pub id: String,
    /// Project the heading lives in.
    pub project: String,
    /// Heading title, without tags.
    pub title: String,
    /// TODO keyword on the heading.
    pub state: String,
    /// Priority cookie as a one-character string.
    pub priority: String,
    /// Property drawer, including planning keys held in the map.
    pub properties: BTreeMap<String, String>,
    /// Tags written on the heading itself.
    pub org_tags: Vec<String>,
    /// Combined heading tags and `:VISSUE_TAGS:`.
    pub tags: Vec<String>,
    /// Ids listed in `:BLOCKED_BY:`.
    pub blocked_by: Vec<String>,
    /// Deed accessions listed in `:DEEDS:`.
    ///
    /// Typed beside `blocked_by` rather than left in `properties` for the same
    /// reason: a client that paints what an issue produced should not have to
    /// know how the drawer spells a list.
    #[serde(default)]
    pub deeds: Vec<String>,
    /// `:PARENT:` id, when set.
    pub parent: Option<String>,
    /// Identity holding the issue, when claimed.
    pub claimed_by: Option<String>,
    /// Org timestamp of the claim.
    pub claimed_at: Option<String>,
    /// `path:line_start-line_end` of the heading in its `issues.org`.
    pub file: String,
    /// 1-based first line of the heading in the file.
    pub line_start: usize,
    /// 1-based last line of the heading in the file.
    pub line_end: usize,
    /// Prose under the heading, without the property drawer or logbook.
    ///
    /// Carried here so a caller that fetched the detail has what the issue
    /// asks for, rather than a file path and a line range to go read.
    #[serde(default)]
    pub body: String,
    /// Logbook lines on the heading, newest first.
    #[serde(default)]
    pub logbook: Vec<LogbookLine>,
}

/// One logbook line on a detail card: note, state flip, or raw CLOCK.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogbookLine {
    /// Inactive org timestamp on the line, or empty for a raw CLOCK row.
    #[serde(default)]
    pub timestamp: String,
    /// Previous TODO keyword on a state flip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_state: Option<String>,
    /// New TODO keyword on a state flip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_state: Option<String>,
    /// Folded note text, when the line is a note rather than a state flip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Opaque drawer line preserved verbatim (a `CLOCK:` entry, say).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

/// One live claim: who holds the issue and for how long.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClaimRow {
    /// Issue id.
    pub id: String,
    /// Project the heading lives in.
    pub project: String,
    /// TODO keyword on the heading.
    pub state: String,
    /// Priority cookie as a one-character string.
    pub priority: String,
    /// Identity holding the issue.
    pub holder: Option<String>,
    /// Org timestamp of the claim.
    pub claimed_at: Option<String>,
    /// Whole days since the claim; `-1` when the stamp does not parse.
    pub age_days: i64,
    /// Heading title.
    pub title: String,
}

/// A capped, secret-screened slice of a heading's on-disk range.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Excerpt {
    /// Issue id.
    pub id: String,
    /// Path of the `issues.org` the heading lives in.
    pub file: String,
    /// 1-based first line of the heading.
    pub line_start: usize,
    /// 1-based last line of the heading.
    pub line_end: usize,
    /// Excerpt text, or a suppression notice when credential-shaped.
    pub text: String,
    /// Whether `text` is a suppression notice rather than the heading.
    pub suppressed: bool,
}

/// One search match: the heading plus a short snippet of the hit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchHit {
    /// Issue id.
    pub id: String,
    /// Project the heading lives in.
    pub project: String,
    /// TODO keyword on the heading.
    pub state: String,
    /// Priority cookie as a one-character string.
    pub priority: String,
    /// Heading title.
    pub title: String,
    /// First matching line, capped.
    pub snippet: String,
}

/// One dated row: a deadline or scheduled date on an open issue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgendaRow {
    /// Calendar date as `YYYY-MM-DD`.
    pub date: String,
    /// `deadline` or `scheduled`.
    pub kind: String,
    /// Days past the date; `0` when it is today or still upcoming.
    pub overdue_days: i64,
    /// Issue id.
    pub id: String,
    /// Project the heading lives in.
    pub project: String,
    /// TODO keyword on the heading.
    pub state: String,
    /// Priority cookie as a one-character string.
    pub priority: String,
    /// Heading title.
    pub title: String,
}

/// A parent/child subtree node, with the issue's own blockers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeNode {
    /// Issue id.
    pub id: String,
    /// TODO keyword on the heading.
    pub state: String,
    /// Heading title.
    pub title: String,
    /// Direct children by `:PARENT:`.
    pub children: Vec<TreeNode>,
    /// Ids listed in `:BLOCKED_BY:`.
    pub blocked_by: Vec<String>,
}

/// One ranked related-issue hit, with the evidence that produced the score.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RelatedHit {
    /// Issue id.
    pub id: String,
    /// Project the heading lives in.
    pub project: String,
    /// TODO keyword on the heading.
    pub state: String,
    /// Heading title.
    pub title: String,
    /// Combined evidence score; higher is a closer match.
    pub score: f64,
    /// Named reasons (`blocked_by`, `term:foo`, `org_distance:1`, ...).
    pub evidence: Vec<String>,
}

/// The working set for one issue: the plan it sits in, the products of the work
/// it waits on, and what it has produced so far.
///
/// Assembled from declared edges rather than from similarity, so the set is the
/// answer and not a ranked guess at it. Nothing here is scored, and nothing is
/// dropped for being far away: the partial order already said what this issue
/// needs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Recall {
    /// Issue the working set is for.
    pub id: String,
    /// Project the heading lives in.
    pub project: String,
    /// TODO keyword on the heading.
    pub state: String,
    /// Heading title.
    pub title: String,
    /// Parent chain, outermost plan first, without this issue.
    pub plan: Vec<WalkHit>,
    /// What this issue waits on and where it came from, each with its products.
    pub inputs: Vec<RecallInput>,
    /// Deed accessions this issue has already cited.
    pub produced: Vec<String>,
    /// Heading body: the dispatch note the work is done from.
    pub body: String,
}

/// What a plan's children hold, child by child.
///
/// A report rather than an average. Weighting children is a judgement the
/// tracker has no basis for, a child that settled split has no single position
/// to fold in, and a child nobody voted on is absent rather than neutral, so
/// there is no honest number to reduce these rows to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanConsensus {
    /// The plan the children hang under.
    pub plan: String,
    /// Heading title of the plan.
    pub title: String,
    /// One row per child, in the order `children` walks them.
    pub children: Vec<ChildConsensus>,
}

/// One child of a plan, and what its own ballots settled on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChildConsensus {
    /// Issue id.
    pub id: String,
    /// TODO keyword on the heading.
    pub state: String,
    /// Heading title.
    pub title: String,
    /// Ballots cast on this child.
    pub ballots: usize,
    /// How the child settled, when anyone voted on it.
    pub settling: Option<crate::consensus::Settling>,
    /// The choice the child holds and its share, when one leads.
    pub holds: Option<(String, f64)>,
}

impl PlanConsensus {
    /// Children nobody has voted on.
    #[must_use]
    pub fn unvoted(&self) -> Vec<&ChildConsensus> {
        self.children.iter().filter(|c| c.ballots == 0).collect()
    }

    /// Children whose own reviewers split into groups that do not listen to
    /// each other.
    #[must_use]
    pub fn split(&self) -> Vec<&ChildConsensus> {
        self.children
            .iter()
            .filter(|c| c.settling == Some(crate::consensus::Settling::Split))
            .collect()
    }

    /// The distinct choices the settled children hold.
    ///
    /// One entry means the children that were voted on point the same way.
    /// More than one means they disagree with each other, which is the case a
    /// per-child report exists to make visible and an average would hide.
    #[must_use]
    pub fn positions(&self) -> Vec<&str> {
        let mut seen: Vec<&str> = Vec::new();
        for child in &self.children {
            if let Some((choice, _)) = &child.holds
                && !seen.contains(&choice.as_str())
            {
                seen.push(choice.as_str());
            }
        }
        seen
    }
}

/// One declared input to an issue, and the deeds that input produced.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecallInput {
    /// Issue id.
    pub id: String,
    /// Project the heading lives in.
    pub project: String,
    /// TODO keyword on the heading.
    pub state: String,
    /// Heading title.
    pub title: String,
    /// Which declared edge made this an input (`blocked-by`, `discovered-from`).
    pub relation: String,
    /// Deed accessions cited on that heading.
    pub deeds: Vec<String>,
    /// A capped excerpt of that input's heading, when one was asked for.
    ///
    /// What the input concluded lives in its body: `append` writes the report
    /// there, and the deed names the product rather than the reasoning. Off
    /// unless asked, because the common case wants the accessions and a working
    /// set that pastes four screens of prose is one nobody reads.
    ///
    /// Screened and capped by the same path `body-excerpt` uses, so an input
    /// whose body looks like credential material is suppressed here too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    /// The most recent note in that input's logbook, when it has one.
    ///
    /// What happened to the input, for the case where it produced no deed. A
    /// blocker that closed without naming a product would otherwise hand the
    /// next unit its title and nothing else.
    pub last_note: Option<String>,
}

/// One related heading from a walk: children, ancestors, impact, or backlinks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalkHit {
    /// Issue id.
    pub id: String,
    /// Project the heading lives in.
    pub project: String,
    /// TODO keyword on the heading.
    pub state: String,
    /// Heading title.
    pub title: String,
    /// How this heading relates to the walk root (`child`, `ancestor`, ...).
    pub relation: String,
}
