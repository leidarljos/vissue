//! Desktop notices: the transitions an operator should see with the board
//! out of view, posted on the desktop's notification bus. A claim, an
//! issue coming unblocked, and a move into BLOCKED, FAILED or CANCELLED.
//! `VISSUE_HUD_NOTIFY=0` turns them off.

use std::collections::BTreeMap;

use vissue_core::views::IssueRow;

/// What one issue looked like the last time the board looked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// TODO keyword.
    pub state: String,
    /// Identity holding it, when claimed.
    pub claimed_by: Option<String>,
    /// Whether `:BLOCKED_BY:` names anything.
    pub blocked: bool,
    /// Heading title, for the notice body.
    pub title: String,
}

/// One notice as it is posted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// The first line: the id and what happened.
    pub summary: String,
    /// The title, and who, when somebody did it.
    pub body: String,
}

/// States a move into is worth a notice.
const STOPPED: &[&str] = &["BLOCKED", "FAILED", "CANCELLED"];

/// Keyed by id.
#[must_use]
pub fn snapshot(rows: &[IssueRow]) -> BTreeMap<String, Snapshot> {
    rows.iter()
        .map(|row| {
            (
                row.id.clone(),
                Snapshot {
                    state: row.state.clone(),
                    claimed_by: row.claimed_by.clone(),
                    blocked: !row.blocked_by.is_empty(),
                    title: row.title.clone(),
                },
            )
        })
        .collect()
}

/// The notices between two looks at the board: only issues seen both times
/// count, so a first look and a new issue post nothing.
#[must_use]
pub fn transitions(
    prev: &BTreeMap<String, Snapshot>,
    now: &BTreeMap<String, Snapshot>,
) -> Vec<Notice> {
    let mut out = Vec::new();
    for (id, cur) in now {
        let Some(old) = prev.get(id) else {
            continue;
        };
        if old.claimed_by.is_none()
            && let Some(who) = cur.claimed_by.as_deref()
        {
            out.push(Notice {
                summary: format!("{id} claimed"),
                body: format!("{} by {who}", cur.title),
            });
        }
        if old.blocked && !cur.blocked && !STOPPED.contains(&cur.state.as_str()) {
            out.push(Notice {
                summary: format!("{id} unblocked"),
                body: cur.title.clone(),
            });
        }
        if cur.state != old.state && STOPPED.contains(&cur.state.as_str()) {
            out.push(Notice {
                summary: format!("{id} {}", cur.state),
                body: cur.title.clone(),
            });
        }
    }
    out
}

/// `VISSUE_HUD_NOTIFY=0` is the one way off.
#[must_use]
pub fn enabled() -> bool {
    match std::env::var("VISSUE_HUD_NOTIFY") {
        Ok(v) => v.trim() != "0",
        Err(_) => true,
    }
}

/// Post one notice on the desktop bus. A bus that does not answer is not
/// the board's problem; the notice is dropped.
pub fn post(notice: &Notice) {
    if !enabled() {
        return;
    }
    #[cfg(not(test))]
    {
        let _ = notify_rust::Notification::new()
            .appname("vissue")
            .summary(&notice.summary)
            .body(&notice.body)
            .show();
    }
    #[cfg(test)]
    {
        let _ = notice;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, state: &str, claimed_by: Option<&str>, blocked: &[&str]) -> IssueRow {
        IssueRow {
            id: id.into(),
            state: state.into(),
            priority: "B".into(),
            title: format!("title of {id}"),
            project: "atlas".into(),
            blocked_by: blocked.iter().map(|b| (*b).to_string()).collect(),
            claimed_by: claimed_by.map(str::to_string),
            claimed_at: None,
            parent: None,
        }
    }

    #[test]
    fn a_claim_an_unblock_and_a_stop_each_post_once() {
        let before = snapshot(&[
            row("atlas-1", "TODO", None, &[]),
            row("atlas-2", "TODO", None, &["atlas-1"]),
            row("atlas-3", "STARTED", Some("me"), &[]),
        ]);
        let after = snapshot(&[
            row("atlas-1", "STARTED", Some("seat"), &[]),
            row("atlas-2", "TODO", None, &[]),
            row("atlas-3", "BLOCKED", Some("me"), &[]),
            row("atlas-4", "STARTED", Some("new"), &[]),
        ]);
        let notices = transitions(&before, &after);
        let lines: Vec<String> = notices.iter().map(|n| n.summary.clone()).collect();
        assert_eq!(
            lines,
            ["atlas-1 claimed", "atlas-2 unblocked", "atlas-3 BLOCKED"]
        );
        assert_eq!(notices[0].body, "title of atlas-1 by seat");
        // The first look, and the same look twice, post nothing.
        assert!(transitions(&BTreeMap::new(), &after).is_empty());
        assert!(transitions(&after, &after).is_empty());
    }

    #[test]
    fn coming_unblocked_into_a_stopped_state_is_the_stop() {
        let before = snapshot(&[row("atlas-2", "TODO", None, &["atlas-1"])]);
        let after = snapshot(&[row("atlas-2", "CANCELLED", None, &[])]);
        let lines: Vec<String> = transitions(&before, &after)
            .iter()
            .map(|n| n.summary.clone())
            .collect();
        assert_eq!(lines, ["atlas-2 CANCELLED"]);
    }
}
