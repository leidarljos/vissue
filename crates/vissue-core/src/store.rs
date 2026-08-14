//! The on-disk store: one `issues.org` per project, parsed and rewritten whole.

use anyhow::{anyhow, Context, Result};
use fs2::FileExt;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Layout;
use crate::model::TODO_KEYWORDS;
use crate::model::{parse_log_line, today_inactive_bracket, IssueHeading, LogEntry, TODO_HEADER};

/// Process-local mutex per path, so concurrent async handlers in one process
/// serialize even where an advisory file lock would not (same process, many
/// descriptors).
static PROCESS_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
static WRITE_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

const ID_ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

struct CrossProcessLock {
    file: fs::File,
}

impl CrossProcessLock {
    fn acquire(path: &Path) -> Result<Self> {
        let lock_path = issues_lock_path(path);
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create lock parent {}", parent.display()))?;
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&lock_path)
            .with_context(|| format!("open lock {}", lock_path.display()))?;
        file.lock_exclusive()
            .with_context(|| format!("lock {}", lock_path.display()))?;
        Ok(Self { file })
    }
}

impl Drop for CrossProcessLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

/// Write `bytes` and flush them to the device, so the caller may rename the
/// file knowing the contents are durable.
fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn issues_lock_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".lock");
    PathBuf::from(s)
}

fn process_mutex_for(path: &Path) -> Arc<Mutex<()>> {
    let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut map = PROCESS_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    map.entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// Serialize every read-modify-write cycle on one `issues.org`.
///
/// Without this, parallel creates race on the temporary file rename and lose
/// updates: the last writer wins and peers see a vanished temporary.
pub fn with_issues_lock<R, F>(path: &Path, f: F) -> Result<R>
where
    F: FnOnce() -> Result<R>,
{
    let mutex = process_mutex_for(path);
    let _proc = mutex.lock().unwrap_or_else(|p| p.into_inner());
    let _cross = CrossProcessLock::acquire(path)?;
    f()
}

/// Lock several files in sorted order, which makes a cross-project move
/// deadlock-free.
pub fn with_issues_locks<R, F>(paths: &[&Path], f: F) -> Result<R>
where
    F: FnOnce() -> Result<R>,
{
    let mut keys: Vec<PathBuf> = paths.iter().map(|p| (*p).to_path_buf()).collect();
    keys.sort();
    keys.dedup();
    let mutexes: Vec<Arc<Mutex<()>>> = keys.iter().map(|k| process_mutex_for(k)).collect();
    let mut proc_guards = Vec::with_capacity(mutexes.len());
    let mut cross_guards = Vec::with_capacity(keys.len());
    for (key, mutex) in keys.iter().zip(mutexes.iter()) {
        proc_guards.push(mutex.lock().unwrap_or_else(|p| p.into_inner()));
        cross_guards.push(CrossProcessLock::acquire(key)?);
    }
    f()
}

/// One project's `issues.org`: a preamble followed by top-level headings.
#[derive(Debug, Clone)]
pub struct IssueDoc {
    pub project: String,
    pub path: PathBuf,
    pub preamble: String,
    pub headings: Vec<IssueHeading>,
}

impl IssueDoc {
    pub fn empty(project: &str, path: PathBuf) -> Self {
        IssueDoc {
            project: project.to_string(),
            path,
            preamble: default_preamble(project),
            headings: Vec::new(),
        }
    }

    /// Parse `path`, or produce an empty document when the file is absent.
    pub fn parse_file(project: &str, path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::empty(project, path.to_path_buf()));
        }
        let content =
            fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        Self::parse(project, path.to_path_buf(), &content)
    }

    pub fn parse(project: &str, path: PathBuf, content: &str) -> Result<Self> {
        let lines: Vec<&str> = content.lines().collect();
        let first_heading = lines
            .iter()
            .position(|line| line.starts_with("* "))
            .unwrap_or(lines.len());
        let preamble = if first_heading == 0 {
            default_preamble(project)
        } else {
            lines[..first_heading].join("\n").trim_end().to_string()
        };
        let mut headings = Vec::new();
        let mut i = first_heading;
        while i < lines.len() {
            if lines[i].starts_with("* ") {
                let (heading, end_idx) = parse_heading(&lines, i)
                    .with_context(|| format!("at {}:{}", path.display(), i + 1))?;
                headings.push(heading);
                i = end_idx;
            } else {
                i += 1;
            }
        }
        Ok(IssueDoc {
            project: project.to_string(),
            path,
            preamble,
            headings,
        })
    }

    /// Render and replace the file through a uniquely named temporary. Callers
    /// hold [`with_issues_lock`] around the parse and this write.
    pub fn write(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = String::new();
        let preamble = if self.preamble.trim().is_empty() {
            default_preamble(&self.project)
        } else {
            self.preamble.clone()
        };
        out.push_str(preamble.trim_end());
        out.push_str("\n\n");
        for h in &self.headings {
            out.push_str(&h.render());
            out.push('\n');
        }
        // A shared temporary name races: a peer renames it out from under this
        // writer and the rename fails with ENOENT.
        let seq = WRITE_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let base = self
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("issues.org");
        let tmp = self
            .path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!(
                ".{}.tmp.{}-{}-{}",
                base,
                std::process::id(),
                nanos,
                seq
            ));
        // Flush the bytes to the device before the rename publishes them.
        // Rename is atomic against a concurrent reader, not against a crash:
        // an unsynced temporary can land as a truncated issues.org.
        if let Err(e) = write_synced(&tmp, out.as_bytes()) {
            let _ = fs::remove_file(&tmp);
            return Err(e).with_context(|| format!("write temp {}", tmp.display()));
        }
        if let Err(e) = fs::rename(&tmp, &self.path) {
            let _ = fs::remove_file(&tmp);
            return Err(e)
                .with_context(|| format!("rename {} -> {}", tmp.display(), self.path.display()));
        }
        self.announce_write();
        Ok(())
    }

    /// Tell pollers the file moved. The event files live beside the project
    /// directories, which is this file's grandparent. Failure here is not a
    /// failed write: the issue is already on disk.
    fn announce_write(&self) {
        if !crate::events::enabled() {
            return;
        }
        let Some(dir) = self.path.parent().and_then(|p| p.parent()) else {
            return;
        };
        let _ = crate::events::emit_issues_write(dir, &self.project, &self.path);
        let _ = crate::events::ensure_gitignore_hint(dir);
    }

    pub fn known_ids(&self) -> Vec<String> {
        self.headings.iter().map(|h| h.id.clone()).collect()
    }

    pub fn upsert(&mut self, heading: IssueHeading) {
        if let Some(slot) = self.headings.iter_mut().find(|h| h.id == heading.id) {
            *slot = heading;
        } else {
            self.headings.push(heading);
        }
    }

    pub fn remove(&mut self, id: &str) -> Option<IssueHeading> {
        let idx = self.headings.iter().position(|h| h.id == id)?;
        Some(self.headings.remove(idx))
    }
}

fn parse_heading(lines: &[&str], start: usize) -> Result<(IssueHeading, usize)> {
    let header = lines[start];
    let stripped = header
        .strip_prefix("* ")
        .ok_or_else(|| anyhow!("not a heading"))?;

    let trimmed = stripped.trim();
    let (state, after) = trimmed
        .split_once(' ')
        .map(|(s, a)| (s.to_string(), a.trim_start()))
        .unwrap_or((trimmed.to_string(), ""));
    if !TODO_KEYWORDS.contains(&state.as_str()) {
        return Err(anyhow!("unknown TODO keyword {:?}", state));
    }

    let (priority, heading_text) = match parse_priority_cookie(after) {
        Some((p, rest)) => (p, rest.trim()),
        None => ('C', after),
    };
    let (title, org_tags) = crate::model::split_headline_tags(heading_text);

    let mut properties = BTreeMap::new();
    let mut property_order = Vec::new();
    let mut logbook: Vec<LogEntry> = Vec::new();
    let mut i = start + 1;

    // Org writes DEADLINE, SCHEDULED, and CLOSED on a planning line between
    // the heading and the drawer. Reading it is what keeps `C-c C-d` in Emacs
    // from making the file unparseable here.
    while i < lines.len() {
        let found = parse_planning_line(lines[i]);
        if found.is_empty() {
            break;
        }
        for (key, value) in found {
            if !property_order.contains(&key) {
                property_order.push(key.clone());
            }
            properties.insert(key, value);
        }
        i += 1;
    }

    let mut body_start = i;
    if i < lines.len() && lines[i].trim() == ":PROPERTIES:" {
        i += 1;
        while i < lines.len() && lines[i].trim() != ":END:" {
            let line = lines[i].trim();
            if let Some(rest) = line.strip_prefix(':') {
                if let Some(idx) = rest.find(':') {
                    let key = rest[..idx].to_string();
                    let val = rest[idx + 1..].trim().to_string();
                    if !property_order.contains(&key) {
                        property_order.push(key.clone());
                    }
                    properties.insert(key, val);
                }
            }
            i += 1;
        }
        if i < lines.len() {
            i += 1;
        }
        body_start = i;
    }

    if i < lines.len() && lines[i].trim() == ":LOGBOOK:" {
        i += 1;
        while i < lines.len() && lines[i].trim() != ":END:" {
            if !lines[i].trim().is_empty() {
                logbook.push(parse_log_line(lines[i]));
            }
            i += 1;
        }
        if i < lines.len() {
            i += 1;
        }
        body_start = i;
    }

    let mut body_end = body_start;
    while body_end < lines.len() && !lines[body_end].starts_with("* ") {
        body_end += 1;
    }
    let body = lines[body_start..body_end]
        .join("\n")
        .trim_matches('\n')
        .trim_end()
        .to_string();

    // Carry a drawer written under the old name forward, so an existing
    // tracker reads the same and the next rewrite settles on the name Org
    // does not reserve.
    if let Some(legacy) = properties.remove(crate::model::LEGACY_TAGS_PROPERTY) {
        property_order.retain(|key| key != crate::model::LEGACY_TAGS_PROPERTY);
        properties
            .entry(crate::model::TAGS_PROPERTY.to_string())
            .or_insert(legacy);
    }

    let id = properties
        .get("ID")
        .cloned()
        .ok_or_else(|| anyhow!(":ID: property missing"))?;

    Ok((
        IssueHeading {
            id,
            title,
            state,
            priority,
            properties,
            org_tags,
            property_order,
            body,
            logbook,
            line_start: start + 1,
            line_end: if body_end == 0 { 1 } else { body_end },
        },
        body_end,
    ))
}

/// Read an Org planning line into its `KEY -> timestamp` pairs.
///
/// Org packs several onto one line, as `CLOSED: [...] SCHEDULED: <...>`, and
/// a line holding anything else is not a planning line at all.
fn parse_planning_line(line: &str) -> Vec<(String, String)> {
    let mut rest = line.trim();
    let mut found = Vec::new();
    while !rest.is_empty() {
        let Some(key) = crate::model::PLANNING_KEYS
            .iter()
            .find(|key| rest.starts_with(&format!("{key}:")))
        else {
            return Vec::new();
        };
        let after = rest[key.len() + 1..].trim_start();
        let close = match after.chars().next() {
            Some('<') => '>',
            Some('[') => ']',
            _ => return Vec::new(),
        };
        let Some(end) = after.find(close) else {
            return Vec::new();
        };
        found.push((key.to_string(), after[..=end].to_string()));
        rest = after[end + 1..].trim_start();
    }
    found
}

/// Split a leading `[#A]` cookie off a heading, returning the cookie character
/// and the rest of the line. Any other shape yields `None`, so a title that
/// merely opens with a bracket keeps its text.
///
/// The cookie character is taken as a character, not a byte: an issues.org is
/// hand-editable, and `[#<multibyte>]` is text a person can type.
fn parse_priority_cookie(after: &str) -> Option<(char, &str)> {
    let rest = after.strip_prefix("[#")?;
    let mut chars = rest.char_indices();
    let (_, priority) = chars.next()?;
    let (close, bracket) = chars.next()?;
    if bracket != ']' {
        return None;
    }
    Some((priority, &rest[close + 1..]))
}

/// The header a fresh project file gets.
///
/// `#+CATEGORY:` names the project, because Org otherwise takes the category
/// from the file name and every project's file is `issues.org`: an agenda
/// spanning several projects would label every row `issues`.
pub fn default_preamble(project: &str) -> String {
    format!(
        "#+TITLE: {project} issues\n#+CATEGORY: {project}\n#+FILETAGS: :issues:{project}:\n#+DATE: {}\n#+DESCRIPTION: Issue tracking file for {project} specs, plans, and implementation tasks.\n#+STATUS: Active\n{}",
        today_inactive_bracket(),
        TODO_HEADER
    )
}

/// Every project directory under the layout prefix that holds an `issues.org`.
pub fn list_projects(layout: &Layout) -> Result<Vec<String>> {
    let dir = layout.projects_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut projects = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("read dir {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && path.join("issues.org").exists() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                projects.push(name.to_string());
            }
        }
    }
    projects.sort();
    Ok(projects)
}

/// Map a project name onto the directory that already exists, ignoring case.
/// An unmatched name is returned unchanged so `create` can make it.
pub fn resolve_existing_project_case(layout: &Layout, project: &str) -> Result<String> {
    if project.is_empty() {
        return Ok(project.to_string());
    }
    if layout.project_issues_path(project).exists() {
        return Ok(project.to_string());
    }
    let project_lower = project.to_lowercase();
    let matches: Vec<String> = list_projects(layout)?
        .into_iter()
        .filter(|candidate| candidate.to_lowercase() == project_lower)
        .collect();
    match matches.as_slice() {
        [] => Ok(project.to_string()),
        [canonical] => Ok(canonical.clone()),
        _ => Err(anyhow!(
            "project {project:?} is ambiguous; case-insensitive matches: {}",
            matches.join(", ")
        )),
    }
}

/// Whether a project belongs to a `--project` selection.
///
/// Case folds, because the directory on disk is what names a project and
/// [`resolve_existing_project_case`] already folds case for every verb that
/// writes. A query that dropped `-p Atlas` on a tracker holding `atlas` would
/// answer "no issues" to a question that has issues.
pub fn project_selected(project: &str, filter: Option<&str>) -> bool {
    match filter {
        None => true,
        Some(p) => project.eq_ignore_ascii_case(p),
    }
}

/// Locate one issue by id across every project.
pub fn find_by_id(layout: &Layout, id: &str) -> Result<Option<(IssueHeading, PathBuf, String)>> {
    for project in list_projects(layout)? {
        let path = layout.project_issues_path(&project);
        let doc = IssueDoc::parse_file(&project, &path)?;
        for h in doc.headings {
            if h.id == id {
                return Ok(Some((h, path, project)));
            }
        }
    }
    Ok(None)
}

/// Snapshot every heading across every project, tagged with its project.
pub fn load_all(layout: &Layout) -> Result<Vec<(String, IssueHeading)>> {
    let mut all = Vec::new();
    for project in list_projects(layout)? {
        let path = layout.project_issues_path(&project);
        let doc = IssueDoc::parse_file(&project, &path)?;
        for h in doc.headings {
            all.push((project.clone(), h));
        }
    }
    Ok(all)
}

/// `<project>-<base36 suffix>`, retried until it does not collide.
pub fn generate_id(project: &str, existing: &[String], length: usize) -> String {
    let len = length.max(2);
    let mut counter: u128 = 0;
    let base = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(1);
    loop {
        let mut n = base
            .wrapping_add(counter.wrapping_mul(17))
            .wrapping_mul(2654435761);
        let mut suffix = String::new();
        for _ in 0..len {
            suffix.push(ID_ALPHABET[(n as usize) % 36] as char);
            n /= 36;
        }
        let id = format!("{}-{}", project, suffix);
        if !existing.iter().any(|e| e == &id) {
            return id;
        }
        counter += 1;
        if counter > 1_000_000 {
            panic!("could not generate unique issue id for {}", project);
        }
    }
}

/// Walk up from `start` for a `.project-ctx.toml` and read `[project].name`.
pub fn detect_project_from_ctx(start: &Path) -> Option<String> {
    let mut dir = start.canonicalize().ok()?;
    loop {
        let candidate = dir.join(".project-ctx.toml");
        if candidate.exists() {
            if let Ok(text) = fs::read_to_string(&candidate) {
                if let Ok(value) = text.parse::<toml::Value>() {
                    if let Some(name) = value
                        .get("project")
                        .and_then(|p| p.get("name"))
                        .and_then(|n| n.as_str())
                    {
                        return Some(name.to_string());
                    }
                }
            }
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

/// Every `:ID:` value in any org file under the layout prefix. A `:PARENT:`
/// may point at a design document, not only at another issue.
pub fn collect_org_ids(layout: &Layout) -> Result<std::collections::HashSet<String>> {
    let mut ids = std::collections::HashSet::new();
    let dir = layout.projects_dir();
    if !dir.exists() {
        return Ok(ids);
    }
    for entry in walkdir::WalkDir::new(&dir)
        .into_iter()
        .filter_entry(|e| !is_skipped_dir(e))
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|s| s.to_str()) != Some("org")
        {
            continue;
        }
        let content = fs::read_to_string(entry.path())
            .with_context(|| format!("read {}", entry.path().display()))?;
        for line in content.lines() {
            if let Some(id) = org_id_property_value(line) {
                ids.insert(id.to_string());
            }
        }
    }
    Ok(ids)
}

fn is_skipped_dir(entry: &walkdir::DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    let name = entry.file_name().to_string_lossy();
    matches!(
        name.as_ref(),
        "node_modules" | "target" | ".git" | ".cache" | "build"
    )
}

fn org_id_property_value(line: &str) -> Option<&str> {
    let value = line.trim_start().strip_prefix(":ID:")?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_PREFIX;

    fn sample_heading() -> IssueHeading {
        let mut props = BTreeMap::new();
        props.insert("ID".into(), "sample-abc1".into());
        props.insert("CREATED".into(), "[2026-04-25 Sat]".into());
        props.insert("TYPE".into(), "feature".into());
        IssueHeading {
            id: "sample-abc1".into(),
            title: "Add a thing".into(),
            state: "TODO".into(),
            priority: 'A',
            properties: props,
            org_tags: Vec::new(),
            property_order: vec!["ID".into(), "CREATED".into(), "TYPE".into()],
            body: "Some body lines.\nWith multiple lines.".into(),
            logbook: Vec::new(),
            line_start: 4,
            line_end: 12,
        }
    }

    #[test]
    fn render_then_parse_preserves_the_heading() {
        let mut content = String::from("#+TITLE: sample issues\n");
        content.push_str(TODO_HEADER);
        content.push_str("\n\n");
        content.push_str(&sample_heading().render());
        let parsed = IssueDoc::parse("sample", PathBuf::from("/tmp/x.org"), &content).unwrap();
        let h = &parsed.headings[0];
        let original = sample_heading();
        assert_eq!(h.id, original.id);
        assert_eq!(h.title, original.title);
        assert_eq!(h.state, original.state);
        assert_eq!(h.priority, original.priority);
        assert_eq!(h.body, original.body);
        assert_eq!(h.properties.get("TYPE"), original.properties.get("TYPE"));
    }

    #[test]
    fn heading_without_a_priority_cookie_defaults_to_c() {
        let content =
            "#+TITLE: x issues\n\n* TODO Just a title\n:PROPERTIES:\n:ID:         x-aaaa\n:END:\n";
        let doc = IssueDoc::parse("x", PathBuf::from("/tmp/x.org"), content).unwrap();
        assert_eq!(doc.headings[0].priority, 'C');
        assert_eq!(doc.headings[0].title, "Just a title");
    }

    #[test]
    fn a_multibyte_priority_cookie_parses_instead_of_panicking() {
        let content = "#+TITLE: x issues\n\n* TODO [#\u{2192}] Hand edited\n:PROPERTIES:\n:ID:         x-aaaa\n:END:\n";
        let doc = IssueDoc::parse("x", PathBuf::from("/tmp/x.org"), content).unwrap();
        assert_eq!(doc.headings[0].priority, '\u{2192}');
        assert_eq!(doc.headings[0].title, "Hand edited");
    }

    #[test]
    fn a_title_opening_with_a_bracket_keeps_its_text() {
        let content =
            "#+TITLE: x issues\n\n* TODO [#not a cookie] stays\n:PROPERTIES:\n:ID:         x-bbbb\n:END:\n";
        let doc = IssueDoc::parse("x", PathBuf::from("/tmp/x.org"), content).unwrap();
        assert_eq!(doc.headings[0].priority, 'C');
        assert_eq!(doc.headings[0].title, "[#not a cookie] stays");
    }

    #[test]
    fn an_org_planning_line_parses_instead_of_hiding_the_drawer() {
        // What `C-c C-d`, `C-c C-s`, and `org-log-done` write in Emacs. Before
        // the planning line was read, the drawer below it went unseen and the
        // whole file failed with ":ID: property missing".
        let content = "#+TITLE: x issues\n\n* DONE [#A] Ship it\nCLOSED: [2026-08-14 Fri 03:33] SCHEDULED: <2026-09-05 Sat> DEADLINE: <2026-09-01 Tue>\n:PROPERTIES:\n:ID:         x-aaaa\n:END:\n\nBody stays body.\n";
        let doc = IssueDoc::parse("x", PathBuf::from("/tmp/x.org"), content).unwrap();
        let h = &doc.headings[0];
        assert_eq!(h.id, "x-aaaa");
        assert_eq!(h.deadline(), Some("<2026-09-01 Tue>"));
        assert_eq!(h.scheduled(), Some("<2026-09-05 Sat>"));
        assert_eq!(
            h.properties.get("CLOSED").map(String::as_str),
            Some("[2026-08-14 Fri 03:33]")
        );
        assert_eq!(h.body, "Body stays body.");
    }

    #[test]
    fn a_planning_line_round_trips_in_orgs_own_order() {
        let content = "#+TITLE: x issues\n\n* DONE [#A] Ship it\nCLOSED: [2026-08-14 Fri 03:33] SCHEDULED: <2026-09-05 Sat> DEADLINE: <2026-09-01 Tue>\n:PROPERTIES:\n:ID:         x-aaaa\n:END:\n";
        let doc = IssueDoc::parse("x", PathBuf::from("/tmp/x.org"), content).unwrap();
        let rendered = doc.headings[0].render();
        assert!(
            rendered.contains(
                "\nCLOSED: [2026-08-14 Fri 03:33] SCHEDULED: <2026-09-05 Sat> DEADLINE: <2026-09-01 Tue>\n"
            ),
            "{rendered}"
        );
        assert!(!rendered.contains(":DEADLINE:"), "{rendered}");
    }

    #[test]
    fn a_legacy_date_property_is_promoted_to_a_planning_line() {
        // Trackers written before dates moved out of the drawer still parse,
        // and the next rewrite puts them where Org's agenda reads them.
        let content = "#+TITLE: x issues\n\n* TODO [#A] Ship it\n:PROPERTIES:\n:ID:         x-aaaa\n:DEADLINE:   <2026-09-01 Tue>\n:END:\n";
        let doc = IssueDoc::parse("x", PathBuf::from("/tmp/x.org"), content).unwrap();
        assert_eq!(doc.headings[0].deadline(), Some("<2026-09-01 Tue>"));
        let rendered = doc.headings[0].render();
        assert!(
            rendered.contains("\nDEADLINE: <2026-09-01 Tue>\n"),
            "{rendered}"
        );
        assert!(!rendered.contains(":DEADLINE:"), "{rendered}");
    }

    #[test]
    fn a_line_that_only_looks_like_planning_is_left_as_body() {
        let content = "#+TITLE: x issues\n\n* TODO [#A] Ship it\n:PROPERTIES:\n:ID:         x-aaaa\n:END:\n\nDEADLINE: is discussed in the design note.\n";
        let doc = IssueDoc::parse("x", PathBuf::from("/tmp/x.org"), content).unwrap();
        assert_eq!(doc.headings[0].deadline(), None);
        assert_eq!(
            doc.headings[0].body,
            "DEADLINE: is discussed in the design note."
        );
    }

    #[test]
    fn a_legacy_tags_property_moves_to_the_name_org_leaves_alone() {
        // `TAGS` is one of Org's own special property names, so a drawer that
        // claims it is what `org-lint` reports. Existing trackers still read.
        let content = "#+TITLE: x issues\n\n* TODO [#A] Ship it\n:PROPERTIES:\n:ID:         x-aaaa\n:TAGS:       needs-review,perf\n:END:\n";
        let doc = IssueDoc::parse("x", PathBuf::from("/tmp/x.org"), content).unwrap();
        let h = &doc.headings[0];
        assert_eq!(h.tags(), vec!["needs-review", "perf"]);
        let rendered = h.render();
        assert!(
            rendered.contains(":VISSUE_TAGS: needs-review,perf"),
            "{rendered}"
        );
        assert!(!rendered.contains(":TAGS:"), "{rendered}");
    }

    #[test]
    fn bodies_end_at_the_next_heading() {
        let content = "#+TITLE: x issues\n\n* TODO [#A] First\n:PROPERTIES:\n:ID:         x-1111\n:END:\n\nFirst body.\n\n* DONE [#C] Second\n:PROPERTIES:\n:ID:         x-2222\n:END:\n\nSecond body.\nMulti.\n";
        let doc = IssueDoc::parse("x", PathBuf::from("/tmp/x.org"), content).unwrap();
        assert_eq!(doc.headings.len(), 2);
        assert_eq!(doc.headings[0].body, "First body.");
        assert_eq!(doc.headings[1].body, "Second body.\nMulti.");
    }

    #[test]
    fn logbook_survives_a_document_round_trip() {
        let mut h = sample_heading();
        h.logbook = vec![LogEntry {
            timestamp: "[2026-04-26 Sun 09:15]".into(),
            from_state: Some("TODO".into()),
            to_state: Some("STARTED".into()),
            note: None,
            raw: None,
        }];
        let mut content = String::from("#+TITLE: sample issues\n\n");
        content.push_str(&h.render());
        let parsed = IssueDoc::parse("sample", PathBuf::from("/tmp/x.org"), &content).unwrap();
        assert_eq!(parsed.headings[0].logbook.len(), 1);
        assert_eq!(
            parsed.headings[0].logbook[0].from_state.as_deref(),
            Some("TODO")
        );
    }

    #[test]
    fn write_then_reread_finds_the_heading() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Software/sample/issues.org");
        IssueDoc {
            project: "sample".into(),
            path: path.clone(),
            preamble: default_preamble("sample"),
            headings: vec![sample_heading()],
        }
        .write()
        .unwrap();
        let parsed = IssueDoc::parse_file("sample", &path).unwrap();
        assert_eq!(parsed.headings[0].id, "sample-abc1");
    }

    #[test]
    fn write_preserves_the_existing_preamble_and_property_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Software/sample/issues.org");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "#+TITLE: sample issues\n#+FILETAGS: :issues:sample:\n#+STATUS: Active\n#+TODO: TODO STARTED BLOCKED | DONE CANCELLED\n\n* TODO [#A] Existing issue\n:PROPERTIES:\n:ID:         sample-abc1\n:CREATED:    [2026-04-26 Sun]\n:TYPE:       spec\n:PARENT:     sample-root\n:END:\n",
        )
        .unwrap();

        let mut doc = IssueDoc::parse_file("sample", &path).unwrap();
        doc.headings[0].priority = 'B';
        doc.write().unwrap();
        let written = fs::read_to_string(&path).unwrap();

        assert!(written.contains("#+FILETAGS: :issues:sample:"), "{written}");
        assert!(written.contains("#+STATUS: Active"), "{written}");
        assert!(
            written.find(":TYPE:").unwrap() < written.find(":PARENT:").unwrap(),
            "{written}"
        );
    }

    #[test]
    fn an_empty_document_writes_the_house_preamble() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Software/sample/issues.org");
        IssueDoc::empty("sample", path.clone()).write().unwrap();
        let written = fs::read_to_string(&path).unwrap();
        for expected in [
            "#+TITLE: sample issues",
            // Org takes the category from the file name otherwise, and every
            // project's file is issues.org.
            "#+CATEGORY: sample",
            "#+FILETAGS: :issues:sample:",
            "#+DATE:",
            "#+STATUS: Active",
            TODO_HEADER,
        ] {
            assert!(written.contains(expected), "missing {expected}: {written}");
        }
    }

    #[test]
    fn generated_ids_are_unique_and_sized() {
        let existing = vec!["p-aaaa".to_string()];
        let id = generate_id("p", &existing, 4);
        assert!(id.starts_with("p-"));
        assert!(!existing.contains(&id));
        assert_eq!(id.len(), 1 + 1 + 4);
        assert_eq!(generate_id("q", &[], 6).len(), 1 + 1 + 6);
    }

    #[test]
    fn projects_are_discovered_under_the_configured_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), "tracker");
        for project in ["beta", "alpha"] {
            IssueDoc::empty(project, layout.project_issues_path(project))
                .write()
                .unwrap();
        }
        assert_eq!(list_projects(&layout).unwrap(), vec!["alpha", "beta"]);
        assert!(list_projects(&Layout::new(dir.path(), DEFAULT_PREFIX))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn project_case_resolves_to_the_directory_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        IssueDoc::empty("MixedCase", layout.project_issues_path("MixedCase"))
            .write()
            .unwrap();
        assert_eq!(
            resolve_existing_project_case(&layout, "mixedcase").unwrap(),
            "MixedCase"
        );
        assert_eq!(
            resolve_existing_project_case(&layout, "brand-new").unwrap(),
            "brand-new"
        );
    }

    #[test]
    fn project_context_file_is_found_by_walking_up() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/c");
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            dir.path().join(".project-ctx.toml"),
            "[project]\nname = \"demoproj\"\n",
        )
        .unwrap();
        assert_eq!(
            detect_project_from_ctx(&nested).as_deref(),
            Some("demoproj")
        );
        let empty = tempfile::tempdir().unwrap();
        assert!(detect_project_from_ctx(empty.path()).is_none());
    }
}
