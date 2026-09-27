//! Three-way merge of one `issues.org` by heading, for git's merge driver.
//!
//! Git merges a tracker as lines, so two notes on one issue, two ballots
//! on one issue, or a state change beside a note conflict although they
//! commute as issue edits. This merge takes each side's headings by `:ID:`
//! and merges field by field against the common base:
//!
//! - a field one side changed takes that side's value;
//! - a heading one side added is kept, theirs after ours where both added;
//! - the logbook is the union of both sides' lines, newest first, less a
//!   line one side removed;
//! - ballots merge per voter;
//! - tags and properties merge per tag and per key.
//!
//! When both sides changed one field to different values, the merge keeps
//! ours and writes the clash on the heading as a logbook note, with theirs
//! kept in full in a `:MERGE_CONFLICT:` drawer: a clash is a record, as it
//! is in Tardigrade, not markers in the file. A side that does not parse,
//! or a result that does not parse back, is an error; the caller then
//! leaves git's ordinary conflict, never a file vissue cannot read.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::anyhow;

use crate::error::Result;
use crate::model::{CLAIM_RELEASED_NOTE, CLAIMED_AT, CLAIMED_BY, IssueHeading, LogEntry};
use crate::ops::{VOTES_DRAWER, drawer_name_is, parse_ballot};
use crate::store::IssueDoc;

/// The drawer that keeps the other side's text of a clashing field.
pub const CONFLICT_DRAWER: &str = "MERGE_CONFLICT";

/// A merged tracker and how many clashes it recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    /// The merged file.
    pub text: String,
    /// Clashes written as notes on their headings.
    pub conflicts: usize,
}

/// One side of a three-way pick.
fn pick<T: PartialEq + Clone>(base: &T, ours: &T, theirs: &T) -> (T, bool) {
    if ours == theirs || theirs == base {
        (ours.clone(), false)
    } else if ours == base {
        (theirs.clone(), false)
    } else {
        (ours.clone(), true)
    }
}

/// Merge three versions of one tracker file.
///
/// # Errors
///
/// A side that does not parse as a tracker, or a merged file that does not
/// parse back.
pub fn merge_tracker(project: &str, base: &str, ours: &str, theirs: &str) -> Result<Merged> {
    let parse = |label: &str, text: &str| {
        IssueDoc::parse(project, PathBuf::from("issues.org"), text)
            .map_err(|e| anyhow!("merge: the {label} side does not parse: {e}"))
    };
    let o = parse("base", base)?;
    let a = parse("ours", ours)?;
    let b = parse("theirs", theirs)?;
    let mut conflicts = 0;

    let (preamble, clash) = pick(&o.preamble, &a.preamble, &b.preamble);
    conflicts += usize::from(clash);

    let index = |doc: &IssueDoc| -> BTreeMap<String, (IssueHeading, String)> {
        doc.headings
            .iter()
            .enumerate()
            .map(|(i, h)| {
                (
                    h.id.clone(),
                    (h.clone(), doc.after.get(i).cloned().unwrap_or_default()),
                )
            })
            .collect()
    };
    let (oi, ai, bi) = (index(&o), index(&a), index(&b));

    // Ours in its order. A heading only theirs has goes after the heading
    // theirs puts it after, past any headings ours added there, since both
    // sides append new issues; with no such heading, at the end.
    let mut order: Vec<String> = a.headings.iter().map(|h| h.id.clone()).collect();
    let ours_new = |id: &String| !oi.contains_key(id) && !bi.contains_key(id);
    let mut previous: Option<String> = None;
    for h in &b.headings {
        if !order.contains(&h.id) {
            let mut at = previous
                .as_ref()
                .and_then(|p| order.iter().position(|x| x == p))
                .map_or(order.len(), |i| i + 1);
            while at < order.len() && ours_new(&order[at]) {
                at += 1;
            }
            order.insert(at, h.id.clone());
        }
        previous = Some(h.id.clone());
    }

    let mut headings = Vec::new();
    let mut after = Vec::new();
    for id in order {
        let merged = match (oi.get(&id), ai.get(&id), bi.get(&id)) {
            (_, Some(x), None) | (_, None, Some(x)) if !oi.contains_key(&id) => Some(x.clone()),
            (Some(base), Some(ours), Some(theirs)) => {
                Some(merge_heading(base, ours, theirs, &mut conflicts))
            }
            (None, Some(ours), Some(theirs)) => {
                // One new id on both sides is one issue that reached both,
                // one copy older than the other. The older stands in for the
                // base, so the newer side's fields win and both logbooks and
                // ballots are kept; a tie keeps ours.
                let base = if newest(&theirs.0) > newest(&ours.0) {
                    ours
                } else {
                    theirs
                };
                Some(merge_heading(base, ours, theirs, &mut conflicts))
            }
            // Removed on one side: gone if the other left it alone, kept
            // with a note if the other changed it.
            (Some(base), Some(kept), None) | (Some(base), None, Some(kept)) => {
                if kept.0.render() == base.0.render() && kept.1 == base.1 {
                    None
                } else {
                    let mut k = kept.clone();
                    conflict_note(
                        &mut k.0,
                        "the heading was removed on one side and changed on the other; kept it",
                    );
                    conflicts += 1;
                    Some(k)
                }
            }
            _ => None,
        };
        if let Some((h, tail)) = merged {
            headings.push(h);
            after.push(tail);
        }
    }

    let doc = IssueDoc {
        preamble,
        headings,
        after,
        ..a
    };
    let text = doc.render_string();
    IssueDoc::parse(project, PathBuf::from("issues.org"), &text)
        .map_err(|e| anyhow!("merge: the merged file does not parse back: {e}"))?;
    Ok(Merged { text, conflicts })
}

/// The newest logbook stamp on a heading; org stamps sort as text.
fn newest(h: &IssueHeading) -> String {
    h.logbook
        .iter()
        .map(|e| e.timestamp.clone())
        .max()
        .unwrap_or_default()
}

fn conflict_note(h: &mut IssueHeading, what: &str) {
    h.logbook.insert(
        0,
        LogEntry {
            timestamp: LogEntry::now(),
            from_state: None,
            to_state: None,
            note: Some(format!("merge conflict: {what}")),
            raw: None,
        },
    );
}

fn merge_heading(
    base: &(IssueHeading, String),
    ours: &(IssueHeading, String),
    theirs: &(IssueHeading, String),
    conflicts: &mut usize,
) -> (IssueHeading, String) {
    let (o, a, b) = (&base.0, &ours.0, &theirs.0);
    let mut h = a.clone();
    let mut clashes: Vec<(String, String)> = Vec::new();
    let mut scalar = |field: &str, bv: &str, av: &str, tv: &str| -> String {
        let (v, clash) = pick(&bv.to_string(), &av.to_string(), &tv.to_string());
        if clash {
            clashes.push((field.to_string(), tv.to_string()));
        }
        v
    };
    h.title = scalar("title", &o.title, &a.title, &b.title);
    // Every state change is logged with its time, so two sides that both
    // moved the state are ordered: the later move wins. Only moves no
    // logged time orders are a clash.
    let moved = |x: &IssueHeading| {
        x.logbook
            .iter()
            .filter(|e| e.to_state.as_deref() == Some(x.state.as_str()))
            .map(|e| e.timestamp.clone())
            .max()
            .unwrap_or_default()
    };
    h.state = if a.state != b.state && a.state != o.state && b.state != o.state {
        match moved(a).cmp(&moved(b)) {
            std::cmp::Ordering::Greater if !moved(a).is_empty() => a.state.clone(),
            std::cmp::Ordering::Less if !moved(b).is_empty() => b.state.clone(),
            _ => scalar("state", &o.state, &a.state, &b.state),
        }
    } else {
        scalar("state", &o.state, &a.state, &b.state)
    };
    h.body = scalar("body", &o.body, &a.body, &b.body);
    let pri = scalar(
        "priority",
        &o.priority.to_string(),
        &a.priority.to_string(),
        &b.priority.to_string(),
    );
    h.priority = pri.chars().next().unwrap_or(a.priority);
    let stats = scalar(
        "statistics",
        o.statistics.as_deref().unwrap_or(""),
        a.statistics.as_deref().unwrap_or(""),
        b.statistics.as_deref().unwrap_or(""),
    );
    h.statistics = (!stats.is_empty()).then_some(stats);

    // Tags: whatever either side added, less whatever either side removed.
    let removed: BTreeSet<&String> = o
        .org_tags
        .iter()
        .filter(|t| !a.org_tags.contains(t) || !b.org_tags.contains(t))
        .collect();
    let mut tags: Vec<String> = a
        .org_tags
        .iter()
        .filter(|t| !removed.contains(t))
        .cloned()
        .collect();
    for t in &b.org_tags {
        if !removed.contains(t) && !a.org_tags.contains(t) {
            tags.push(t.clone());
        }
    }
    h.org_tags = tags;

    // Properties, one key at a time.
    let keys: BTreeSet<&String> = o
        .properties
        .keys()
        .chain(a.properties.keys())
        .chain(b.properties.keys())
        .collect();
    let mut props = BTreeMap::new();
    for k in keys {
        if LIST_PROPERTIES.contains(&k.as_str()) {
            let merged = merge_tokens(
                o.properties.get(k).map(String::as_str),
                a.properties.get(k).map(String::as_str),
                b.properties.get(k).map(String::as_str),
            );
            if let Some(v) = merged {
                props.insert(k.clone(), v);
            }
            continue;
        }
        let (v, clash) = pick(
            &o.properties.get(k).cloned(),
            &a.properties.get(k).cloned(),
            &b.properties.get(k).cloned(),
        );
        if clash {
            clashes.push((
                format!("property {k}"),
                b.properties.get(k).cloned().unwrap_or_default(),
            ));
        }
        if let Some(v) = v {
            props.insert(k.clone(), v);
        }
    }
    // A claim one side released stays released when the release note is
    // later than the claim the other side still carries.
    let released_after = |x: &IssueHeading, claimed_at: &str| {
        x.logbook.iter().any(|e| {
            e.note
                .as_deref()
                .is_some_and(|n| n.trim_start().starts_with(CLAIM_RELEASED_NOTE))
                && e.timestamp.as_str() >= claimed_at
        })
    };
    if a.properties.contains_key(CLAIMED_BY) != b.properties.contains_key(CLAIMED_BY) {
        let (held, releaser) = if a.properties.contains_key(CLAIMED_BY) {
            (a, b)
        } else {
            (b, a)
        };
        if let Some(at) = held.properties.get(CLAIMED_AT)
            && released_after(releaser, at)
        {
            props.remove(CLAIMED_BY);
            props.remove(CLAIMED_AT);
        }
    }
    h.properties = props;
    for k in &b.property_order {
        if !h.property_order.contains(k) {
            h.property_order.push(k.clone());
        }
    }
    h.property_order.retain(|k| h.properties.contains_key(k));

    h.logbook = merge_logbook(&o.logbook, &a.logbook, &b.logbook);
    h.extra_drawers = merge_drawers(
        &o.extra_drawers,
        &a.extra_drawers,
        &b.extra_drawers,
        &mut clashes,
    );
    let (tail, clash) = pick(&base.1, &ours.1, &theirs.1);
    if clash {
        clashes.push(("the org after the heading".into(), theirs.1.clone()));
    }

    if !clashes.is_empty() {
        let mut kept = format!(":{CONFLICT_DRAWER}:\n");
        for (field, theirs_value) in &clashes {
            conflict_note(
                &mut h,
                &format!(
                    "{field} changed on both sides; kept ours, theirs is in the {CONFLICT_DRAWER} drawer"
                ),
            );
            kept.push_str(&format!("{field}:\n{}\n", theirs_value.trim_end()));
        }
        kept.push_str(":END:");
        h.extra_drawers
            .retain(|d| !drawer_name_is(d, CONFLICT_DRAWER));
        h.extra_drawers.push(kept);
        *conflicts += clashes.len();
    }
    (h, tail)
}

/// Properties that hold a space-separated set: two sides that each added
/// to one commute, so they merge by token rather than clash.
const LIST_PROPERTIES: &[&str] = &[
    crate::props::DEEDS,
    crate::props::BLOCKED_BY,
    crate::props::FILES,
    crate::props::TAGS,
];

/// A set property merged by token: ours as written, less what either side
/// removed, then what theirs added.
fn merge_tokens(base: Option<&str>, ours: Option<&str>, theirs: Option<&str>) -> Option<String> {
    let split = |v: Option<&str>| -> Vec<String> {
        v.unwrap_or("")
            .split_whitespace()
            .map(str::to_string)
            .collect()
    };
    let (o, a, b) = (split(base), split(ours), split(theirs));
    let removed: BTreeSet<&String> = o
        .iter()
        .filter(|t| !a.contains(t) || !b.contains(t))
        .collect();
    let mut out: Vec<String> = a.iter().filter(|t| !removed.contains(t)).cloned().collect();
    for t in &b {
        if !removed.contains(t) && !out.contains(t) {
            out.push(t.clone());
        }
    }
    if out.is_empty() && ours.is_none() && theirs.is_none() {
        return None;
    }
    (!out.is_empty()).then(|| out.join(" "))
}

/// Both sides' logbook lines, newest first, less a line one side removed.
fn merge_logbook(base: &[LogEntry], ours: &[LogEntry], theirs: &[LogEntry]) -> Vec<LogEntry> {
    let line = |e: &LogEntry| e.render();
    let ours_lines: BTreeSet<String> = ours.iter().map(line).collect();
    let theirs_lines: BTreeSet<String> = theirs.iter().map(line).collect();
    let dropped: BTreeSet<String> = base
        .iter()
        .map(line)
        .filter(|l| !ours_lines.contains(l) || !theirs_lines.contains(l))
        .collect();
    let mut out: Vec<LogEntry> = ours
        .iter()
        .filter(|e| !dropped.contains(&line(e)))
        .cloned()
        .collect();
    for e in theirs {
        let l = line(e);
        if dropped.contains(&l) || out.iter().any(|x| line(x) == l) {
            continue;
        }
        // Before the first line older than it; org stamps sort as text.
        let at = if e.timestamp.is_empty() {
            out.len()
        } else {
            out.iter()
                .position(|x| !x.timestamp.is_empty() && x.timestamp < e.timestamp)
                .unwrap_or(out.len())
        };
        out.insert(at, e.clone());
    }
    out
}

fn drawer_name(d: &str) -> String {
    d.lines()
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches(':')
        .to_uppercase()
}

/// Drawers by name: the votes drawer per voter, any other as a whole.
fn merge_drawers(
    base: &[String],
    ours: &[String],
    theirs: &[String],
    clashes: &mut Vec<(String, String)>,
) -> Vec<String> {
    let find = |set: &[String], name: &str| set.iter().find(|d| drawer_name(d) == name).cloned();
    let mut names: Vec<String> = Vec::new();
    for d in ours.iter().chain(theirs) {
        let n = drawer_name(d);
        if n != CONFLICT_DRAWER && !names.contains(&n) {
            names.push(n);
        }
    }
    let mut out = Vec::new();
    for name in names {
        let (o, a, b) = (find(base, &name), find(ours, &name), find(theirs, &name));
        if name == VOTES_DRAWER {
            if let Some(d) = merge_votes(o.as_deref(), a.as_deref(), b.as_deref(), clashes) {
                out.push(d);
            }
            continue;
        }
        let (v, clash) = pick(&o, &a, &b);
        if clash {
            clashes.push((format!("drawer {name}"), b.clone().unwrap_or_default()));
        }
        if let Some(v) = v {
            out.push(v);
        }
    }
    // A clash drawer ours already carried stays until someone settles it.
    if let Some(d) = find(ours, CONFLICT_DRAWER) {
        out.push(d);
    }
    out
}

/// The votes drawer merged per voter: a ballot one side changed takes that
/// side's; both changed it, the later stamp wins and a same-day tie keeps
/// ours and says so.
fn merge_votes(
    base: Option<&str>,
    ours: Option<&str>,
    theirs: Option<&str>,
    clashes: &mut Vec<(String, String)>,
) -> Option<String> {
    let lines = |d: Option<&str>| -> Vec<(String, String)> {
        let mut m: Vec<(String, String)> = Vec::new();
        for l in d.unwrap_or("").lines() {
            let t = l.trim();
            if t.is_empty()
                || t.eq_ignore_ascii_case(":END:")
                || t.eq_ignore_ascii_case(&format!(":{VOTES_DRAWER}:"))
            {
                continue;
            }
            let key = parse_ballot(t)
                .map_or_else(|| format!("line {t}"), |b| format!("voter {}", b.agent));
            match m.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => slot.1 = t.to_string(),
                None => m.push((key, t.to_string())),
            }
        }
        m
    };
    let as_map =
        |v: &[(String, String)]| -> BTreeMap<String, String> { v.iter().cloned().collect() };
    let (ov, av, bv) = (lines(base), lines(ours), lines(theirs));
    let (o, a, b) = (as_map(&ov), as_map(&av), as_map(&bv));
    // Ours' order, then theirs' new voters, then any the base alone held.
    let mut keys: Vec<&String> = Vec::new();
    for (k, _) in av.iter().chain(&bv).chain(&ov) {
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    let mut kept = Vec::new();
    for k in keys {
        let (v, clash) = pick(&o.get(k).cloned(), &a.get(k).cloned(), &b.get(k).cloned());
        let v = if clash {
            let stamp = |s: &Option<String>| s.as_deref().and_then(parse_ballot).map(|x| x.stamp);
            match (stamp(&a.get(k).cloned()), stamp(&b.get(k).cloned())) {
                (Some(sa), Some(sb)) if sb > sa => b.get(k).cloned(),
                (Some(sa), Some(sb)) if sa > sb => a.get(k).cloned(),
                _ => {
                    clashes.push((
                        format!("ballot of {}", k.trim_start_matches("voter ")),
                        b.get(k).cloned().unwrap_or_default(),
                    ));
                    a.get(k).cloned()
                }
            }
        } else {
            v
        };
        if let Some(v) = v {
            kept.push(v);
        }
    }
    if kept.is_empty() {
        return None;
    }
    Some(format!(":{VOTES_DRAWER}:\n{}\n:END:", kept.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "#+VISSUE: 1\n#+TODO: TODO STARTED BLOCKED | DONE CANCELLED\n\n";

    fn issue(id: &str, state: &str, logbook: &[&str], extra: &str) -> String {
        let mut s = format!("* {state} [#A] Work on {id}\n:PROPERTIES:\n:ID:       {id}\n:END:\n");
        if !logbook.is_empty() {
            s.push_str(":LOGBOOK:\n");
            for l in logbook {
                s.push_str(l);
                s.push('\n');
            }
            s.push_str(":END:\n");
        }
        s.push_str(extra);
        s
    }

    fn file(issues: &[String]) -> String {
        format!("{HEAD}{}", issues.join("\n"))
    }

    fn heading(text: &str, id: &str) -> IssueHeading {
        let doc = IssueDoc::parse("acme", PathBuf::from("issues.org"), text).unwrap();
        doc.headings.into_iter().find(|h| h.id == id).unwrap()
    }

    #[test]
    fn two_notes_on_one_issue_merge_without_a_clash() {
        let n0 = "- Note: \"started\" [2026-09-20 Sun 10:00]";
        let na = "- Note: \"ours saw the build go green\" [2026-09-27 Sun 11:00]";
        let nb = "- Note: \"theirs filed the log\" [2026-09-27 Sun 12:00]";
        let base = file(&[issue("acme-1a2b", "TODO", &[n0], "")]);
        let ours = file(&[issue("acme-1a2b", "TODO", &[na, n0], "")]);
        let theirs = file(&[issue("acme-1a2b", "TODO", &[nb, n0], "")]);
        let m = merge_tracker("acme", &base, &ours, &theirs).unwrap();
        assert_eq!(m.conflicts, 0);
        let notes: Vec<String> = heading(&m.text, "acme-1a2b")
            .logbook
            .iter()
            .map(LogEntry::render)
            .collect();
        assert_eq!(notes, [nb, na, n0]);
    }

    #[test]
    fn a_state_change_beside_a_note_takes_both() {
        let n0 = "- Note: \"started\" [2026-09-20 Sun 10:00]";
        let nb = "- Note: \"theirs note\" [2026-09-27 Sun 12:00]";
        let base = file(&[issue("acme-1a2b", "TODO", &[n0], "")]);
        let ours = file(&[issue("acme-1a2b", "DONE", &[n0], "")]);
        let theirs = file(&[issue("acme-1a2b", "TODO", &[nb, n0], "")]);
        let m = merge_tracker("acme", &base, &ours, &theirs).unwrap();
        assert_eq!(m.conflicts, 0);
        let h = heading(&m.text, "acme-1a2b");
        assert_eq!(h.state, "DONE");
        assert_eq!(h.logbook.len(), 2);
    }

    #[test]
    fn ballots_merge_per_voter_and_new_issues_from_both_sides_stay() {
        let votes = |lines: &[&str]| format!(":VOTES:\n{}\n:END:\n", lines.join("\n"));
        let base = file(&[issue("acme-1a2b", "TODO", &[], "")]);
        let ours = file(&[
            issue(
                "acme-1a2b",
                "TODO",
                &[],
                &votes(&["[2026-09-27 Sun] alice: yes"]),
            ),
            issue("acme-3c4d", "TODO", &[], ""),
        ]);
        let theirs = file(&[
            issue(
                "acme-1a2b",
                "TODO",
                &[],
                &votes(&["[2026-09-27 Sun] bob: no"]),
            ),
            issue("acme-5e6f", "TODO", &[], ""),
        ]);
        let m = merge_tracker("acme", &base, &ours, &theirs).unwrap();
        assert_eq!(m.conflicts, 0);
        let h = heading(&m.text, "acme-1a2b");
        let drawer = h
            .extra_drawers
            .iter()
            .find(|d| drawer_name_is(d, VOTES_DRAWER))
            .unwrap();
        assert!(
            drawer.contains("alice: yes") && drawer.contains("bob: no"),
            "{drawer}"
        );
        let doc = IssueDoc::parse("acme", PathBuf::from("issues.org"), &m.text).unwrap();
        let ids: Vec<&str> = doc.headings.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, ["acme-1a2b", "acme-3c4d", "acme-5e6f"]);
    }

    #[test]
    fn a_real_clash_is_a_note_and_a_drawer_not_markers() {
        let base = file(&[issue("acme-1a2b", "TODO", &[], "")]);
        let ours = file(&[issue("acme-1a2b", "DONE", &[], "")]);
        let theirs = file(&[issue("acme-1a2b", "CANCELLED", &[], "")]);
        let m = merge_tracker("acme", &base, &ours, &theirs).unwrap();
        assert_eq!(m.conflicts, 1);
        assert!(!m.text.contains("<<<<<<<"));
        let h = heading(&m.text, "acme-1a2b");
        assert_eq!(h.state, "DONE", "ours kept");
        assert!(
            h.logbook[0]
                .render()
                .contains("merge conflict: state changed on both sides")
        );
        let kept = h
            .extra_drawers
            .iter()
            .find(|d| drawer_name_is(d, CONFLICT_DRAWER))
            .unwrap();
        assert!(kept.contains("CANCELLED"), "{kept}");
    }

    #[test]
    fn a_heading_removed_on_one_side_goes_unless_the_other_changed_it() {
        let base = file(&[
            issue("acme-1a2b", "TODO", &[], ""),
            issue("acme-3c4d", "TODO", &[], ""),
        ]);
        let ours = file(&[issue("acme-3c4d", "TODO", &[], "")]);
        let theirs = base.clone();
        let m = merge_tracker("acme", &base, &ours, &theirs).unwrap();
        assert!(!m.text.contains("acme-1a2b"));
        let changed = file(&[
            issue("acme-1a2b", "DONE", &[], ""),
            issue("acme-3c4d", "TODO", &[], ""),
        ]);
        let kept = merge_tracker("acme", &base, &ours, &changed).unwrap();
        assert!(kept.text.contains("acme-1a2b") && kept.conflicts == 1);
    }

    #[test]
    fn one_new_id_on_both_sides_takes_the_newer_copy_without_a_clash() {
        let older = "- State \"STARTED\" from \"TODO\" [2026-09-20 Sun 10:00]";
        let newer = "- State \"DONE\" from \"STARTED\" [2026-09-27 Sun 10:00]";
        let base = file(&[issue("acme-1a2b", "TODO", &[], "")]);
        let ours = file(&[
            issue("acme-1a2b", "TODO", &[], ""),
            issue("acme-3c4d", "STARTED", &[older], ""),
        ]);
        let theirs = file(&[
            issue("acme-1a2b", "TODO", &[], ""),
            issue("acme-3c4d", "DONE", &[newer, older], ""),
        ]);
        let m = merge_tracker("acme", &base, &ours, &theirs).unwrap();
        assert_eq!(m.conflicts, 0);
        let h = heading(&m.text, "acme-3c4d");
        assert_eq!(h.state, "DONE");
        assert_eq!(h.logbook.len(), 2);
    }

    #[test]
    fn merging_in_an_ancestor_of_ours_gives_ours() {
        let n0 = "- Note: \"one\" [2026-09-20 Sun 10:00]";
        let s1 = "- State \"STARTED\" from \"TODO\" [2026-09-21 Mon 10:00]";
        let s2 = "- State \"DONE\" from \"STARTED\" [2026-09-22 Tue 10:00]";
        let base = file(&[issue("acme-1a2b", "TODO", &[n0], "")]);
        let mid = file(&[
            issue("acme-1a2b", "STARTED", &[s1, n0], ""),
            issue("acme-3c4d", "TODO", &[], ""),
        ]);
        let ours = file(&[
            issue("acme-1a2b", "DONE", &[s2, s1, n0], ""),
            issue("acme-3c4d", "STARTED", &[s1], ""),
        ]);
        let m = merge_tracker("acme", &base, &ours, &mid).unwrap();
        assert_eq!(m.conflicts, 0);
        let rendered = IssueDoc::parse("acme", PathBuf::from("issues.org"), &ours)
            .unwrap()
            .render_string();
        assert_eq!(m.text, rendered);
    }

    #[test]
    fn a_claim_released_after_it_was_taken_stays_released() {
        let claimed = ":PROPERTIES:\n:ID:       acme-1a2b\n:CLAIMED_BY: brio\n:CLAIMED_AT: [2026-09-25 Fri 10:47]\n:END:\n";
        let base = format!("{HEAD}* TODO [#A] Work\n:PROPERTIES:\n:ID:       acme-1a2b\n:END:\n");
        let theirs = format!("{HEAD}* STARTED [#A] Work\n{claimed}");
        let ours = format!(
            "{HEAD}* STARTED [#A] Work\n:PROPERTIES:\n:ID:       acme-1a2b\n:END:\n:LOGBOOK:\n- Note: \"claim released: brio held since [2026-09-25 Fri 10:47]\" [2026-09-26 Sat 09:00]\n:END:\n"
        );
        let m = merge_tracker("acme", &base, &ours, &theirs).unwrap();
        assert!(
            !heading(&m.text, "acme-1a2b")
                .properties
                .contains_key(CLAIMED_BY),
            "{}",
            m.text
        );
    }

    #[test]
    fn deeds_added_on_both_sides_merge_by_token() {
        let with = |deeds: &str| {
            format!(
                "{HEAD}* TODO [#A] Work\n:PROPERTIES:\n:ID:       acme-1a2b\n:DEEDS:    {deeds}\n:END:\n"
            )
        };
        let m = merge_tracker(
            "acme",
            &with("deed-a"),
            &with("deed-a deed-b"),
            &with("deed-a deed-c"),
        )
        .unwrap();
        assert_eq!(m.conflicts, 0);
        assert_eq!(
            heading(&m.text, "acme-1a2b").properties["DEEDS"],
            "deed-a deed-b deed-c"
        );
    }

    #[test]
    fn a_side_that_does_not_parse_is_an_error() {
        let base = file(&[issue("acme-1a2b", "TODO", &[], "")]);
        let broken = format!("{HEAD}* TODO [#A] no id here\n");
        assert!(merge_tracker("acme", &base, &broken, &base).is_err());
    }
}
