//! The issue heading and its logbook: parsing, accessors, and rendering.

use chrono::Local;
use serde::Serialize;
use std::collections::BTreeMap;

/// Planning-line keys (`CLOSED`, `SCHEDULED`, `DEADLINE`) and Org's tag
/// character class. The property map still holds the dates internally;
/// only the on-disk shape is Org's (manual 8.1, 6).
pub use crate::org::{PLANNING_KEYS, is_org_tag_char};

/// TODO keywords recognised on a heading, in org declaration order.
pub const TODO_KEYWORDS: &[&str] = &["TODO", "STARTED", "BLOCKED", "DONE", "CANCELLED"];
/// States an issue can be worked from once its blockers clear.
pub const READY_STATES: &[&str] = &["TODO", "STARTED"];
/// The `#+TODO:` line written into a fresh issues.org preamble.
pub const TODO_HEADER: &str = "#+TODO: TODO STARTED BLOCKED | DONE CANCELLED";

const PROPERTY_COLUMN: usize = 13;
const DEFAULT_PROPERTY_ORDER: &[&str] = crate::props::CANONICAL_ORDER;

/// Column org right-aligns headline tags to, matching the `org-tags-column`
/// default. Writing them anywhere else makes the next Emacs edit realign the
/// line and show up as a diff that changed nothing.
const TAG_COLUMN: usize = 77;

/// Split a trailing `:a:b:` tag run off a heading's text.
///
/// Returns the title and the tags. A title that merely ends in a colon, or
/// whose trailing run holds a character org would not accept in a tag, keeps
/// its text.
pub fn split_headline_tags(text: &str) -> (String, Vec<String>) {
    let trimmed = text.trim_end();
    let Some(run_start) = trimmed.rfind(char::is_whitespace).map(|i| i + 1) else {
        return (trimmed.to_string(), Vec::new());
    };
    let run = &trimmed[run_start..];
    if run.len() < 3 || !run.starts_with(':') || !run.ends_with(':') {
        return (trimmed.to_string(), Vec::new());
    }
    let tags: Vec<String> = run
        .trim_matches(':')
        .split(':')
        .map(str::to_string)
        .collect();
    if tags.is_empty()
        || tags
            .iter()
            .any(|tag| tag.is_empty() || !tag.chars().all(is_org_tag_char))
    {
        return (trimmed.to_string(), Vec::new());
    }
    (trimmed[..run_start].trim_end().to_string(), tags)
}

/// Property holding tags Org itself cannot carry on a heading.
///
/// Not `TAGS`: Org reserves that name for the headline tags it exposes as a
/// special property, and `org-lint` reports a drawer that claims it.
pub const TAGS_PROPERTY: &str = crate::props::TAGS;
/// The name this property had before the clash with Org was found. Read on
/// parse and rewritten under the current name.
pub const LEGACY_TAGS_PROPERTY: &str = "TAGS";

/// Property naming the identity that holds a STARTED issue.
pub const CLAIMED_BY: &str = crate::props::CLAIMED_BY;
/// Property recording when that claim was taken.
pub const CLAIMED_AT: &str = crate::props::CLAIMED_AT;

/// One line of an issue's `:LOGBOOK:` drawer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogEntry {
    /// Inactive org timestamp on the line, or empty for a raw CLOCK row.
    pub timestamp: String,
    /// Previous TODO keyword on a state flip.
    pub from_state: Option<String>,
    /// New TODO keyword on a state flip.
    pub to_state: Option<String>,
    /// Folded note text, when the line is a note rather than a state flip.
    pub note: Option<String>,
    /// Opaque logbook line (an org `CLOCK:` entry, say) preserved verbatim
    /// across rewrites.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

/// Opening of the one logbook note the tracker writes for itself.
pub const CLAIM_RELEASED_NOTE: &str = "claim released:";

impl LogEntry {
    /// Whether this line is the tracker's own bookkeeping rather than something
    /// a worker said.
    ///
    /// A reader looking for the last word on an issue wants what somebody wrote
    /// about the work. Releasing a claim files a note too, and it is the most
    /// recent one on almost every closed issue, so a reader that took the first
    /// note it found would nearly always be handed the same sentence about a
    /// claim rather than anything about the work.
    #[must_use]
    pub fn is_bookkeeping(&self) -> bool {
        self.note
            .as_deref()
            .is_some_and(|note| note.trim_start().starts_with(CLAIM_RELEASED_NOTE))
    }

    /// One logbook line, matching the Org drawer form this crate writes.
    pub fn render(&self) -> String {
        if let Some(raw) = &self.raw {
            return raw.clone();
        }
        if let (Some(to), Some(from)) = (&self.to_state, &self.from_state) {
            format!("- State \"{}\" from \"{}\" {}", to, from, self.timestamp)
        } else if let Some(to) = &self.to_state {
            format!("- State \"{}\" {}", to, self.timestamp)
        } else if let Some(note) = &self.note {
            format!("- Note: \"{}\" {}", note, self.timestamp)
        } else {
            format!("- {}", self.timestamp)
        }
    }

    /// An inactive org timestamp for the current local time.
    pub fn now() -> String {
        Local::now().format("[%Y-%m-%d %a %H:%M]").to_string()
    }

    fn raw_line(line: &str) -> Self {
        Self {
            timestamp: String::new(),
            from_state: None,
            to_state: None,
            note: None,
            raw: Some(line.trim_end_matches(['\r', '\n']).to_string()),
        }
    }
}

pub(crate) fn parse_log_line(s: &str) -> LogEntry {
    // Org CLOCK lines and other non-dash drawer content survive as raw text.
    let trimmed = s.trim();
    if trimmed.to_ascii_uppercase().starts_with("CLOCK:")
        || (!trimmed.starts_with('-') && !trimmed.is_empty())
    {
        return LogEntry::raw_line(s);
    }
    let Some(body) = trimmed.strip_prefix('-').map(str::trim_start) else {
        return LogEntry::raw_line(s);
    };
    let Some(bracket_idx) = body.rfind('[') else {
        return LogEntry::raw_line(s);
    };
    let Some(rel_end) = body[bracket_idx..].find(']') else {
        return LogEntry::raw_line(s);
    };
    let end = rel_end + bracket_idx;
    let timestamp = body[bracket_idx..=end].to_string();
    let prefix = body[..bracket_idx].trim().trim_end_matches(',');
    if let Some(rest) = prefix.strip_prefix("State ") {
        let mut chunks = rest.splitn(2, " from ");
        let to_quoted = chunks.next().unwrap_or("").trim();
        let from_quoted = chunks.next().map(str::trim);
        LogEntry {
            timestamp,
            from_state: from_quoted.map(|s| s.trim_matches('"').to_string()),
            to_state: Some(to_quoted.trim_matches('"').to_string()),
            note: None,
            raw: None,
        }
    } else if let Some(rest) = prefix.strip_prefix("Note:") {
        let note = rest.trim().trim_matches('"').to_string();
        LogEntry {
            timestamp,
            from_state: None,
            to_state: None,
            note: if note.is_empty() { None } else { Some(note) },
            raw: None,
        }
    } else {
        LogEntry {
            timestamp,
            from_state: None,
            to_state: None,
            note: if prefix.is_empty() {
                None
            } else {
                Some(prefix.to_string())
            },
            raw: None,
        }
    }
}

/// One issue: a top-level org heading with a properties drawer, an optional
/// logbook, and free-form body prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueHeading {
    /// `:ID:` value, `<project>-<suffix>`.
    pub id: String,
    /// Heading title, without a trailing tag run.
    pub title: String,
    /// TODO keyword on the heading.
    pub state: String,
    /// Priority cookie character (`A`, `B`, or `C`).
    pub priority: char,
    /// Property drawer, including planning keys held in the map.
    pub properties: BTreeMap<String, String>,
    /// Tags written on the heading itself, which is where Org's own tag
    /// search and agenda look. Kept apart from the `:TAGS:` property so each
    /// round-trips as it was written; [`IssueHeading::tags`] reads both.
    #[serde(default)]
    pub org_tags: Vec<String>,
    /// Trailing statistics cookies (`[2/5]`, `[40%]`), which Org keeps on
    /// the headline after the title (manual 5.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statistics: Option<String>,
    /// Drawer keys in on-disk order, so a rewrite leaves a hand-arranged
    /// drawer alone.
    #[serde(skip_serializing)]
    pub property_order: Vec<String>,
    /// Drawers other than `:PROPERTIES:` and `:LOGBOOK:` that sat at the
    /// drawer site. Written back after those two so a rewrite does not drop
    /// a `:NOTES:` drawer someone put there.
    #[serde(skip_serializing)]
    pub extra_drawers: Vec<String>,
    /// Prose under the heading, without the property drawer or logbook.
    #[serde(skip_serializing)]
    pub body: String,
    /// `:LOGBOOK:` lines, newest first.
    pub logbook: Vec<LogEntry>,
    /// 1-based first line of this heading in the file.
    pub line_start: usize,
    /// 1-based last line of this heading in the file.
    pub line_end: usize,
}

impl IssueHeading {
    /// Ids this heading waits on.
    ///
    /// `:BLOCKED_BY:` is the vissue form. `:BLOCKEDBY:` is a typo for that.
    /// `:BLOCKER:` is GNU ELPA org-edna (and org-depend in org-contrib):
    /// an `ids(...)` form or a bare id list both count; `prev-sibling` and
    /// the rest of the condition language do not.
    pub fn blocked_by(&self) -> Vec<String> {
        crate::org::blocker_ids_from_properties(&self.properties)
    }

    /// Deed accessions this issue's work produced.
    ///
    /// The tracker cites; it does not store. A deed is deedar's noun for what a
    /// unit of work made, and the id is the whole join: the next unit opens it
    /// with `deedar get` rather than rereading a transcript. Commas and
    /// whitespace both separate, as they do for `:BLOCKED_BY:`.
    pub fn deeds(&self) -> Vec<String> {
        crate::props::get(&self.properties, crate::props::DEEDS)
            .map(crate::org::split_id_list)
            .unwrap_or_default()
    }

    /// Org Effort estimate (`Effort` or `EFFORT`), when set.
    pub fn effort(&self) -> Option<&str> {
        crate::org::effort_from_properties(&self.properties)
    }

    /// Own tags plus inherited FILETAGS (Org ALLTAGS for a top-level heading).
    pub fn all_tags(&self, filetags: &[String]) -> Vec<String> {
        let mut tags = self.tags();
        for tag in filetags {
            if !tags.iter().any(|seen| seen == tag) {
                tags.push(tag.clone());
            }
        }
        tags
    }

    /// Every tag on the issue: the `:VISSUE_TAGS:` property and the heading's
    /// own Org tags, in that order and without duplicates.
    pub fn tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self
            .properties
            .get(TAGS_PROPERTY)
            .map(|s| {
                s.split([',', ':'])
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        for tag in &self.org_tags {
            if !tags.iter().any(|seen| seen == tag) {
                tags.push(tag.clone());
            }
        }
        tags
    }

    /// Deadline timestamp, when the heading carries one.
    pub fn deadline(&self) -> Option<&str> {
        self.properties.get("DEADLINE").map(|s| s.as_str())
    }

    /// Scheduled timestamp, when the heading carries one.
    pub fn scheduled(&self) -> Option<&str> {
        self.properties.get("SCHEDULED").map(|s| s.as_str())
    }

    /// `:PARENT:` id, when set.
    pub fn parent(&self) -> Option<&str> {
        crate::props::get(&self.properties, crate::props::PARENT)
    }

    /// The identity holding this issue, when one has claimed it.
    pub fn claimed_by(&self) -> Option<&str> {
        crate::props::get(&self.properties, CLAIMED_BY)
    }

    /// When the claim was taken, as the stored org timestamp.
    pub fn claimed_at(&self) -> Option<&str> {
        crate::props::get(&self.properties, CLAIMED_AT)
    }

    /// Whole days the claim has been held, when it parses as a date.
    pub fn claim_age_days(&self, today: chrono::NaiveDate) -> Option<i64> {
        let taken = parse_stamp_date(self.claimed_at()?)?;
        Some((today - taken).num_days())
    }

    /// Date of the holder's claim stamp. Logbook notes do not name an author,
    /// so they are not activity by this holder.
    pub fn last_activity_date(&self) -> Option<chrono::NaiveDate> {
        parse_stamp_date(self.claimed_at()?)
    }

    /// Whole days since [`Self::last_activity_date`], when that date parses.
    pub fn last_activity_age_days(&self, today: chrono::NaiveDate) -> Option<i64> {
        Some((today - self.last_activity_date()?).num_days())
    }

    /// Record the claim. Stores the identity verbatim: it is an opaque tag.
    pub fn set_claim(&mut self, identity: &str) {
        self.properties
            .insert(CLAIMED_BY.to_string(), identity.to_string());
        self.properties
            .insert(CLAIMED_AT.to_string(), LogEntry::now());
    }

    /// Drop the claim, leaving a logbook note so the history survives the
    /// properties being cleared.
    ///
    /// The note it writes is the one line in a logbook the tracker authors
    /// itself; [`LogEntry::is_bookkeeping`] is how a reader tells it from
    /// something a worker said.
    pub fn release_claim(&mut self) -> Option<(String, String)> {
        let who = self.properties.remove(CLAIMED_BY)?;
        let when = self.properties.remove(CLAIMED_AT).unwrap_or_default();
        self.logbook.insert(
            0,
            LogEntry {
                timestamp: LogEntry::now(),
                from_state: None,
                to_state: None,
                note: Some(format!("{CLAIM_RELEASED_NOTE} {who} held since {when}")),
                raw: None,
            },
        );
        Some((who, when))
    }

    /// Heading, planning line, drawers, and body as they are written to disk.
    pub fn render(&self) -> String {
        let mut org_tags = self.org_tags.clone();
        let mut properties = self.properties.clone();
        crate::props::settle(&mut org_tags, &mut properties);
        let mut out = render_heading_line(
            &self.state,
            self.priority,
            &self.title,
            self.statistics.as_deref(),
            &org_tags,
        );
        if let Some(planning) = self.render_planning_line() {
            out.push_str(&planning);
        }
        out.push_str(":PROPERTIES:\n");
        out.push_str(&render_property("ID", &self.id));
        for key in self.ordered_property_keys() {
            if key == "ID" || PLANNING_KEYS.contains(&key.as_str()) {
                continue;
            }
            if let Some(val) = properties.get(&key) {
                out.push_str(&render_property(&key, val));
            }
        }
        out.push_str(":END:\n");
        if !self.logbook.is_empty() {
            out.push_str(":LOGBOOK:\n");
            for entry in &self.logbook {
                out.push_str(&entry.render());
                out.push('\n');
            }
            out.push_str(":END:\n");
        }
        for drawer in &self.extra_drawers {
            out.push_str(drawer);
            if !drawer.ends_with('\n') {
                out.push('\n');
            }
        }
        if !self.body.is_empty() {
            let body = escape_body_headlines(&self.body);
            out.push('\n');
            out.push_str(&body);
            if !body.ends_with('\n') {
                out.push('\n');
            }
        }
        out
    }

    /// The `CLOSED: ... SCHEDULED: ... DEADLINE: ...` line org puts under a
    /// heading, or `None` when the issue carries no dates.
    ///
    /// Org's agenda reads this line and ignores a same-named property, so a
    /// deadline written into the drawer is a deadline Emacs cannot see.
    fn render_planning_line(&self) -> Option<String> {
        let parts: Vec<String> = PLANNING_KEYS
            .iter()
            .filter_map(|key| {
                let value = self.properties.get(*key)?.trim();
                (!value.is_empty()).then(|| format!("{key}: {value}"))
            })
            .collect();
        (!parts.is_empty()).then(|| format!("{}\n", parts.join(" ")))
    }

    /// Keys as they appeared on disk first, then the house order, then the rest.
    /// Rewriting an issue therefore leaves a hand-arranged drawer alone.
    fn ordered_property_keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        for key in &self.property_order {
            if self.properties.contains_key(key) && !keys.contains(key) {
                keys.push(key.clone());
            }
        }
        for key in DEFAULT_PROPERTY_ORDER {
            let key = key.to_string();
            if self.properties.contains_key(&key) && !keys.contains(&key) {
                keys.push(key);
            }
        }
        for key in self.properties.keys() {
            if !keys.contains(key) {
                keys.push(key.clone());
            }
        }
        keys
    }

    /// Move to `new_state` and prepend the transition to the logbook. A
    /// transition to the current state is not recorded.
    pub fn record_state_change(&mut self, new_state: &str) {
        if self.state == new_state {
            return;
        }
        let from = Some(self.state.clone());
        self.logbook.insert(
            0,
            LogEntry {
                timestamp: LogEntry::now(),
                from_state: from,
                to_state: Some(new_state.to_string()),
                note: None,
                raw: None,
            },
        );
        self.state = new_state.to_string();
    }
}

/// Append a `:tag:tag:` run to a heading, right-aligned the way Org aligns it,
/// so running `org-align-tags` in Emacs over the result changes nothing.
pub fn align_tags(stem: &str, org_tags: &[String]) -> String {
    if org_tags.is_empty() {
        return stem.to_string();
    }
    let run = format!(":{}:", org_tags.join(":"));
    let width = stem.chars().count() + run.chars().count();
    let pad = if width < TAG_COLUMN {
        TAG_COLUMN - width
    } else {
        1
    };
    format!("{stem}{}{run}", " ".repeat(pad))
}

/// `* STATE [#P] Title            :tag:tag:`.
/// Indent body lines that would end the issue.
///
/// The parser splits issues on a line starting with `* `, so a body carrying
/// one, which any markdown bullet list does, cuts the issue in two on the
/// next read: the tail becomes a heading with no `:ID:`, the file stops
/// parsing, and every issue in it drops out of `list`.
///
/// Deeper headings are left alone. `** Scope` under a level-one issue is a
/// child of it, which is structure the body is meant to be able to carry and
/// which [`crate::mirror`] demotes when it nests issues further down.
///
/// One leading space is enough, and reads the same to a person. Applying it
/// again changes nothing, so a file written, read, and written again is
/// stable.
fn escape_body_headlines(body: &str) -> String {
    if !body.lines().any(ends_the_issue) {
        return body.to_string();
    }
    let mut out = String::with_capacity(body.len() + 8);
    for (i, line) in body.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if ends_the_issue(line) {
            out.push(' ');
        }
        out.push_str(line);
    }
    out
}

/// Whether the parser would read this body line as the next issue heading.
fn ends_the_issue(line: &str) -> bool {
    line.starts_with("* ")
}

fn render_heading_line(
    state: &str,
    priority: char,
    title: &str,
    statistics: Option<&str>,
    org_tags: &[String],
) -> String {
    let mut stem = format!("* {} [#{}] {}", state, priority, title);
    if let Some(cookie) = statistics {
        stem.push(' ');
        stem.push_str(cookie);
    }
    format!("{}\n", align_tags(&stem, org_tags))
}

fn render_property(key: &str, val: &str) -> String {
    let key_part = format!(":{}:", key);
    let pad = if key_part.len() < PROPERTY_COLUMN {
        " ".repeat(PROPERTY_COLUMN - key_part.len())
    } else {
        " ".to_string()
    };
    format!("{}{}{}\n", key_part, pad, val)
}

/// Today as an inactive org timestamp.
pub fn today_inactive_bracket() -> String {
    Local::now().format("[%Y-%m-%d %a]").to_string()
}

/// The date inside an org timestamp, active or inactive, with or without a
/// time of day.
pub fn parse_stamp_date(s: &str) -> Option<chrono::NaiveDate> {
    let inner = s
        .trim()
        .trim_start_matches(['<', '['])
        .trim_end_matches(['>', ']']);
    let token = inner.split_whitespace().next()?;
    chrono::NaiveDate::parse_from_str(token, "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_heading() -> IssueHeading {
        let mut props = BTreeMap::new();
        props.insert("ID".into(), "sample-abc1".into());
        props.insert("CREATED".into(), "[2026-04-25 Sat]".into());
        IssueHeading {
            id: "sample-abc1".into(),
            title: "Add a thing".into(),
            state: "TODO".into(),
            priority: 'A',
            properties: props,
            org_tags: Vec::new(),
            statistics: None,
            property_order: vec!["ID".into(), "CREATED".into()],
            extra_drawers: Vec::new(),
            body: "Some body lines.\nWith multiple lines.".into(),
            logbook: Vec::new(),
            line_start: 4,
            line_end: 12,
        }
    }

    #[test]
    fn last_activity_is_the_claim_stamp_even_when_someone_else_noted() {
        let mut h = sample_heading();
        h.properties.insert(CLAIMED_BY.to_string(), "worker".into());
        h.properties
            .insert(CLAIMED_AT.to_string(), "[2026-01-01 Thu]".into());
        h.logbook.push(LogEntry {
            timestamp: "[2026-03-15 Sun]".into(),
            from_state: None,
            to_state: None,
            note: Some("still going".into()),
            raw: None,
        });
        assert_eq!(
            h.last_activity_date(),
            Some(chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap())
        );
        let today = chrono::NaiveDate::from_ymd_opt(2026, 3, 22).unwrap();
        assert_eq!(h.last_activity_age_days(today), Some(80));
        assert_eq!(h.claim_age_days(today), Some(80));
    }

    #[test]
    fn blocked_by_accepts_commas_spaces_and_both() {
        let mut h = sample_heading();
        for raw in [" A-1, B-2 ,, C-3 ", "A-1 B-2  C-3", "A-1, B-2 C-3"] {
            h.properties.insert("BLOCKED_BY".into(), raw.into());
            assert_eq!(h.blocked_by(), vec!["A-1", "B-2", "C-3"], "raw = {raw:?}");
        }
    }

    #[test]
    fn blocked_by_reads_a_blocker_id_list_and_edna_ids() {
        let mut h = sample_heading();
        h.properties.insert("BLOCKER".into(), "A-1 B-2".into());
        assert_eq!(h.blocked_by(), vec!["A-1", "B-2"]);
        h.properties
            .insert("BLOCKER".into(), "ids(A-1) prev-sibling".into());
        assert_eq!(h.blocked_by(), vec!["A-1"]);
        h.properties.insert("BLOCKER".into(), "prev-sibling".into());
        assert!(h.blocked_by().is_empty());
    }

    #[test]
    fn tags_split_on_commas_and_colons() {
        let mut h = sample_heading();
        h.properties
            .insert(TAGS_PROPERTY.into(), "rust:perf, scaling".into());
        assert_eq!(h.tags(), vec!["rust", "perf", "scaling"]);
    }

    #[test]
    fn accessors_read_optional_properties() {
        let mut h = sample_heading();
        assert!(h.parent().is_none());
        h.properties.insert("PARENT".into(), "sample-q3xa".into());
        h.properties
            .insert("DEADLINE".into(), "<2026-05-15 Fri>".into());
        h.properties
            .insert("SCHEDULED".into(), "<2026-04-28 Mon>".into());
        assert_eq!(h.parent(), Some("sample-q3xa"));
        assert_eq!(h.deadline(), Some("<2026-05-15 Fri>"));
        assert_eq!(h.scheduled(), Some("<2026-04-28 Mon>"));
    }

    #[test]
    fn state_change_prepends_and_skips_no_ops() {
        let mut h = sample_heading();
        h.record_state_change("STARTED");
        assert_eq!(h.state, "STARTED");
        assert_eq!(h.logbook[0].from_state.as_deref(), Some("TODO"));
        h.record_state_change("DONE");
        assert_eq!(h.logbook.len(), 2);
        assert_eq!(h.logbook[0].to_state.as_deref(), Some("DONE"));
        h.record_state_change("DONE");
        assert_eq!(h.logbook.len(), 2, "no-op transition is not logged");
    }

    #[test]
    fn logbook_renders_state_transitions() {
        let entry = LogEntry {
            timestamp: "[2026-04-26 Sun 14:22]".into(),
            from_state: Some("STARTED".into()),
            to_state: Some("DONE".into()),
            note: None,
            raw: None,
        };
        assert_eq!(
            entry.render(),
            "- State \"DONE\" from \"STARTED\" [2026-04-26 Sun 14:22]"
        );
    }

    #[test]
    fn clock_lines_survive_a_state_rewrite() {
        let clock = "   CLOCK: [2026-07-26 Sun 17:40]";
        let closed = "   CLOCK: [2026-07-26 Sun 10:00]--[2026-07-26 Sun 11:00] =>  1:00";
        let state = "- State \"STARTED\" from \"TODO\" [2026-07-26 Sun 17:39]";
        assert_eq!(parse_log_line(clock).render(), clock);
        assert_eq!(parse_log_line(closed).render(), closed);
        let parsed_state = parse_log_line(state);
        assert_eq!(parsed_state.to_state.as_deref(), Some("STARTED"));
        assert!(parsed_state.raw.is_none());

        let mut h = sample_heading();
        h.logbook = vec![parsed_state, parse_log_line(clock)];
        h.record_state_change("DONE");
        let rendered = h.render();
        assert!(
            rendered.contains("CLOCK: [2026-07-26 Sun 17:40]"),
            "{rendered}"
        );
        assert!(rendered.contains("State \"DONE\""), "{rendered}");
    }

    #[test]
    fn headline_tags_split_off_the_title() {
        assert_eq!(
            split_headline_tags("Document the retry policy   :docs:retry:"),
            (
                "Document the retry policy".to_string(),
                vec!["docs".to_string(), "retry".to_string()]
            )
        );
    }

    #[test]
    fn a_title_is_not_mistaken_for_a_tag_run() {
        // Org tags are `[[:alnum:]_@#%]+`, so neither of these is one and the
        // text has to survive intact.
        for title in [
            "Scope: the header block",
            "Rename the key :needs-review:",
            "A ratio of 3:1",
            "Trailing colon:",
        ] {
            assert_eq!(
                split_headline_tags(title),
                (title.to_string(), Vec::new()),
                "{title:?}"
            );
        }
    }

    #[test]
    fn tags_read_the_property_and_the_heading_together() {
        let mut h = sample_heading();
        h.properties
            .insert(TAGS_PROPERTY.into(), "needs-review, perf".into());
        h.org_tags = vec!["docs".into(), "perf".into()];
        assert_eq!(h.tags(), vec!["needs-review", "perf", "docs"]);
    }

    #[test]
    fn a_heading_renders_its_tags_where_org_aligns_them() {
        let mut h = sample_heading();
        h.state = "TODO".into();
        h.priority = 'B';
        h.title = "Document the retry policy".into();
        h.org_tags = vec!["docs".into(), "retry".into()];
        let line = h.render().lines().next().unwrap().to_string();
        assert_eq!(line.chars().count(), TAG_COLUMN, "{line:?}");
        assert!(line.ends_with(":docs:retry:"), "{line:?}");
    }

    #[test]
    fn a_long_title_keeps_one_space_before_its_tags() {
        let mut h = sample_heading();
        h.title = "t".repeat(TAG_COLUMN);
        h.org_tags = vec!["docs".into()];
        let line = h.render().lines().next().unwrap().to_string();
        assert!(line.ends_with(" :docs:"), "{line:?}");
    }

    #[test]
    fn note_lines_round_trip() {
        let parsed = parse_log_line("- Note: \"picked up after review\" [2026-04-26 Sun 09:15]");
        assert_eq!(parsed.note.as_deref(), Some("picked up after review"));
        assert_eq!(
            parsed.render(),
            "- Note: \"picked up after review\" [2026-04-26 Sun 09:15]"
        );
    }
}
