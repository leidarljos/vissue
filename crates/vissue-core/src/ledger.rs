//! Append-only per-issue files.
//!
//! `migrate-ledger` copies each heading out of `issues.org` once. Later notes
//! and field changes append a record to `PROJECT/issues/ID.org` and leave the
//! project file untouched. A read folds the records in order; a repeated field
//! keeps the last value.

use anyhow::{Context, anyhow};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::model::{IssueHeading, LogEntry, parse_log_line};
use crate::store::IssueDoc;

const LEDGER_MARK: &str = "#+VISSUE_LEDGER:";
const LINES_MARK: &str = "#+VISSUE_LINES:";
const LOG_MARK: &str = "#+VISSUE_LEDGER_LOG:";

/// Whether `issues.org` at `path` has been split into per-issue files.
#[must_use]
pub fn is_ledger(path: &Path) -> bool {
    marker_path(path).is_file()
}

/// Split every project under `layout`.
///
/// A project whose board still matches the digest stored at copy time is
/// skipped. A board that changed after the copy is folded into the
/// per-issue files: a log line, field, or new heading that exists only on
/// the board is appended, and a field the ledger already moved is left
/// alone. The same field changed on both sides is an error. `issues.org`
/// is read and not written. `--dry-run` (`dry_run`) reports the split and
/// creates nothing.
///
/// # Errors
///
/// Returns an error if a project file cannot be read, a heading has no usable
/// line range, or a ledger file cannot be created.
pub fn migrate(layout: &crate::config::Layout, dry_run: bool) -> Result<String> {
    let projects = crate::store::list_projects(layout)?;
    let mut report = String::new();
    for project in projects {
        let path = layout.project_issues_path(&project);
        if dry_run {
            report.push_str(&describe(&project, &path)?);
            continue;
        }
        let piece = crate::store::with_issues_lock(&path, || migrate_locked(&project, &path))?;
        report.push_str(&piece);
    }
    if dry_run {
        report.push_str("dry-run: wrote nothing\n");
    }
    Ok(report)
}

/// Copy every project in `layouts` while holding their locks.
///
/// `show` reads without the lock, so a note that lands after one project's
/// show and before that project's copy changes the later show. This holds
/// every project lock first, writes `dir/before.txt`, runs a dry-run, copies,
/// folds nothing (a board move under the lock is an error), and writes
/// `dir/after.txt`.
///
/// # Errors
///
/// Returns an error if a lock cannot be acquired, a project appears or
/// disappears while the locks are acquired, a board changes while its lock
/// is held, or a show or ledger file cannot be written.
pub fn migrate_frozen(layouts: &[&crate::config::Layout], dir: &Path) -> Result<String> {
    let mut projects: Vec<(String, PathBuf)> = Vec::new();
    for layout in layouts {
        for project in crate::store::list_projects(layout)? {
            let path = layout.project_issues_path(&project);
            projects.push((project, path));
        }
    }
    let paths: Vec<PathBuf> = projects.iter().map(|(_, path)| path.clone()).collect();
    let path_refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
    crate::store::with_issues_locks(&path_refs, || {
        let mut current = Vec::new();
        for layout in layouts {
            for project in crate::store::list_projects(layout)? {
                current.push((project, layout.project_issues_path(&project)));
            }
        }
        if current != projects {
            return Err(anyhow!("project set changed while acquiring locks").into());
        }
        fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let mut before = String::new();
        for layout in layouts {
            before.push_str(&crate::report::show_all(layout)?);
        }
        write_proof(dir, "before.txt", &before)?;
        let mut dry = String::new();
        for (project, path) in &projects {
            dry.push_str(&describe(project, path)?);
        }
        dry.push_str("dry-run: wrote nothing\n");
        write_proof(dir, "dry-run.txt", &dry)?;
        let mut migrated = String::new();
        for (project, path) in &projects {
            migrated.push_str(&migrate_locked(project, path)?);
        }
        write_proof(dir, "migrate.txt", &migrated)?;
        let mut catchup = String::new();
        for (project, path) in &projects {
            catchup.push_str(&migrate_locked(project, path)?);
        }
        if catchup
            .lines()
            .any(|line| !line.is_empty() && !line.contains("skipped"))
        {
            return Err(anyhow!("a board changed while its lock was held\n{catchup}").into());
        }
        write_proof(dir, "catchup.txt", &catchup)?;
        let mut after = String::new();
        for layout in layouts {
            after.push_str(&crate::report::show_all(layout)?);
        }
        write_proof(dir, "after.txt", &after)?;
        Ok(format!(
            "frozen: {} project(s), show bytes {} -> {}\n",
            projects.len(),
            before.len(),
            after.len()
        ))
    })
}

fn write_proof(dir: &Path, name: &str, text: &str) -> Result<()> {
    let path = dir.join(name);
    fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Fold the ledger for the project whose `issues.org` is `path`.
///
/// # Errors
///
/// Returns an error if a ledger file cannot be read or a record does not parse.
pub fn load(project: &str, path: &Path) -> Result<IssueDoc> {
    let loaded = read_dir(path)?;
    let preamble = loaded
        .iter()
        .find_map(|item| copied_preamble(&item.source))
        .filter(|text| !text.is_empty())
        .unwrap_or_default();
    let mut shell = if preamble.is_empty() {
        IssueDoc::empty(project, path.to_path_buf())
    } else {
        IssueDoc::parse(project, path.to_path_buf(), &format!("{preamble}\n"))?
    };
    let mut ledger_files = BTreeMap::new();
    let mut headings = Vec::new();
    for item in loaded {
        if item.tombstoned {
            continue;
        }
        ledger_files.insert(item.heading.id.clone(), item.file);
        headings.push(item.heading);
    }
    headings.sort_by(|a, b| {
        a.line_start
            .cmp(&b.line_start)
            .then_with(|| a.id.cmp(&b.id))
    });
    shell.after = vec![String::new(); headings.len()];
    shell.headings = headings;
    shell.ledger_files = ledger_files;
    shell.preamble = if preamble.is_empty() {
        shell.preamble
    } else {
        preamble
    };
    if !shell.preamble.is_empty() {
        shell.tag_settings = crate::org::tag_settings_from_preamble(&shell.preamble);
    }
    Ok(shell)
}

/// Append the difference between the on-disk ledger and `doc`.
///
/// Creates a header file for an id the ledger does not have. Does not open
/// `issues.org` for writing.
///
/// # Errors
///
/// Returns an error if the ledger cannot be read or a record cannot be appended.
pub fn commit(doc: &IssueDoc) -> Result<()> {
    let disk = read_dir(&doc.path)?;
    let mut by_id: BTreeMap<String, Loaded> = BTreeMap::new();
    for item in disk {
        let id = item.heading.id.clone();
        if by_id.insert(id.clone(), item).is_some() {
            return Err(anyhow!("duplicate ledger file for {id}").into());
        }
    }
    for heading in &doc.headings {
        match by_id.get(&heading.id) {
            None => write_header(doc, heading)?,
            Some(existing) if existing.tombstoned => {
                let mut buf = Vec::new();
                push_op(&mut buf, "replace", &heading.render());
                append_record(&existing.file, &buf)?;
            }
            Some(existing) => {
                let mut buf = Vec::new();
                push_diff(&mut buf, &existing.heading, heading);
                if !buf.is_empty() {
                    append_record(&existing.file, &buf)?;
                }
            }
        }
    }
    for (id, existing) in &by_id {
        if existing.tombstoned {
            continue;
        }
        if doc.headings.iter().any(|heading| heading.id == *id) {
            continue;
        }
        let mut buf = Vec::new();
        push_op(&mut buf, "tombstone", "");
        append_record(&existing.file, &buf)?;
    }
    Ok(())
}

struct Loaded {
    file: PathBuf,
    source: String,
    heading: IssueHeading,
    tombstoned: bool,
}

fn describe(project: &str, path: &Path) -> Result<String> {
    if is_ledger(path) {
        let content =
            fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let digest = board_digest(content.as_bytes());
        if marker_digest(path)?.as_deref() == Some(digest.as_str()) {
            return Ok(format!("{project}: already a ledger\n"));
        }
        return Ok(format!("{project}: board moved since the copy\n"));
    }
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let doc = IssueDoc::parse(project, path.to_path_buf(), &content)?;
    Ok(format!("{project}: {} issue(s)\n", doc.headings.len()))
}

fn migrate_locked(project: &str, path: &Path) -> Result<String> {
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let digest = board_digest(content.as_bytes());
    if is_ledger(path) {
        return catch_up_locked(project, path, &content, &digest);
    }
    let doc = IssueDoc::parse(project, path.to_path_buf(), &content)?;
    let dir = issues_dir(path);
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    for heading in &doc.headings {
        let slice = heading_bytes(&content, heading)?;
        if slice.contains(LOG_MARK) {
            return Err(anyhow!("{} heading contains the ledger sentinel", heading.id).into());
        }
        let file = dir.join(file_name(&heading.id));
        if file.exists() {
            let existing =
                fs::read_to_string(&file).with_context(|| format!("read {}", file.display()))?;
            if existing.contains(LOG_MARK) {
                continue;
            }
            return Err(anyhow!("partial ledger file {}", file.display()).into());
        }
        let bytes = header_bytes(&doc.preamble, heading.line_start, heading.line_end, &slice);
        write_new(&file, bytes.as_bytes())?;
    }
    write_marker(path, &digest)?;
    Ok(format!(
        "{project}: {} issue(s) -> {}\n",
        doc.headings.len(),
        dir.display()
    ))
}

/// Fold board writes that landed after the copy. The stored digest is the
/// bytes `migrate` read under the lock, so a note that landed before the
/// copy is already in the ledger and a later note is the only mismatch.
fn catch_up_locked(project: &str, path: &Path, content: &str, digest: &str) -> Result<String> {
    if marker_digest(path)?.as_deref() == Some(digest) {
        return Ok(format!("{project}: already a ledger, skipped\n"));
    }
    let board = IssueDoc::parse(project, path.to_path_buf(), content)?;
    let loaded = read_dir(path)?;
    let mut changed = 0usize;
    for heading in &board.headings {
        if catch_up_one(path, &board, &loaded, heading)? {
            changed += 1;
        }
    }
    for item in &loaded {
        if item.tombstoned {
            continue;
        }
        if board
            .headings
            .iter()
            .any(|heading| heading.id == item.heading.id)
        {
            continue;
        }
        return Err(anyhow!(
            "{project}: {} is in the ledger and absent from the board",
            item.heading.id
        )
        .into());
    }
    write_marker(path, digest)?;
    if changed == 0 {
        return Ok(format!(
            "{project}: board digest recorded, no heading change\n"
        ));
    }
    Ok(format!(
        "{project}: caught up {changed} issue(s) from the board\n"
    ))
}

fn catch_up_one(
    issues_org: &Path,
    board: &IssueDoc,
    loaded: &[Loaded],
    heading: &IssueHeading,
) -> Result<bool> {
    let Some(existing) = loaded.iter().find(|item| item.heading.id == heading.id) else {
        write_header(board, heading)?;
        return Ok(true);
    };
    if existing.tombstoned {
        return Err(anyhow!(
            "{} is tombstoned in the ledger and still on the board",
            heading.id
        )
        .into());
    }
    let snapshot = snapshot_heading(issues_org, &existing.source)?;
    let mut buf = Vec::new();
    queue_merge(&mut buf, &snapshot, &existing.heading, heading)?;
    if buf.is_empty() {
        return Ok(false);
    }
    append_record(&existing.file, &buf)?;
    Ok(true)
}

fn snapshot_heading(issues_org: &Path, source: &str) -> Result<IssueHeading> {
    let (left, _) = split_log(source);
    let doc = IssueDoc::parse("ledger", issues_org.to_path_buf(), left)?;
    doc.headings
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("ledger snapshot has no heading").into())
}

fn queue_merge(
    buf: &mut Vec<u8>,
    snapshot: &IssueHeading,
    folded: &IssueHeading,
    board: &IssueHeading,
) -> Result<()> {
    queue_str(
        buf,
        &board.id,
        "state",
        "state",
        &snapshot.state,
        &folded.state,
        &board.state,
    )?;
    if let Some(priority) = board_decision(
        &board.id,
        "priority",
        &snapshot.priority,
        &folded.priority,
        &board.priority,
    )? {
        push_op(buf, "priority", &priority.to_string());
    }
    queue_str(
        buf,
        &board.id,
        "title",
        "title",
        &snapshot.title,
        &folded.title,
        &board.title,
    )?;
    if let Some(statistics) = board_decision(
        &board.id,
        "statistics",
        &snapshot.statistics,
        &folded.statistics,
        &board.statistics,
    )? {
        push_op(buf, "statistics", statistics.as_deref().unwrap_or(""));
    }
    if let Some(tags) = board_decision(
        &board.id,
        "tags",
        &snapshot.org_tags,
        &folded.org_tags,
        &board.org_tags,
    )? {
        push_op(buf, "org-tags", &tags.join("\n"));
    }
    queue_props(buf, snapshot, folded, board)?;
    if let Some(drawers) = board_decision(
        &board.id,
        "drawers",
        &snapshot.extra_drawers,
        &folded.extra_drawers,
        &board.extra_drawers,
    )? {
        push_op(buf, "drawers", &drawers.concat());
    }
    queue_str(
        buf,
        &board.id,
        "body",
        "body",
        &snapshot.body,
        &folded.body,
        &board.body,
    )?;
    queue_log(buf, folded, board);
    Ok(())
}

fn queue_str(
    buf: &mut Vec<u8>,
    id: &str,
    field: &str,
    op: &str,
    snapshot: &str,
    folded: &str,
    board: &str,
) -> Result<()> {
    if let Some(value) = board_decision(id, field, snapshot, folded, board)? {
        push_op(buf, op, value);
    }
    Ok(())
}

fn queue_props(
    buf: &mut Vec<u8>,
    snapshot: &IssueHeading,
    folded: &IssueHeading,
    board: &IssueHeading,
) -> Result<()> {
    let mut keys: BTreeSet<&str> = BTreeSet::new();
    for key in snapshot
        .properties
        .keys()
        .chain(folded.properties.keys())
        .chain(board.properties.keys())
    {
        if key != "ID" {
            keys.insert(key.as_str());
        }
    }
    for key in keys {
        let previous = snapshot.properties.get(key);
        let current = folded.properties.get(key);
        let seen = board.properties.get(key);
        if seen == current || seen == previous {
            continue;
        }
        if current == previous {
            match seen {
                Some(value) => push_op(buf, "prop", &format!("{key}\n{value}")),
                None => push_op(buf, "prop-del", key),
            }
            continue;
        }
        return Err(anyhow!(
            "{}: property {key} changed on the board and in the ledger",
            board.id
        )
        .into());
    }
    Ok(())
}

fn queue_log(buf: &mut Vec<u8>, folded: &IssueHeading, board: &IssueHeading) {
    let missing: Vec<&LogEntry> = board
        .logbook
        .iter()
        .filter(|entry| !folded.logbook.contains(entry))
        .collect();
    for entry in missing.into_iter().rev() {
        push_op(buf, "log", &entry.render());
    }
}

/// `Some(board)` when the board moved this value and the ledger still has
/// the copied one. `None` when the ledger already has the board value, or
/// the ledger moved and the board did not. Both moved apart is an error.
fn board_decision<'a, T: PartialEq + ?Sized>(
    id: &str,
    field: &str,
    snapshot: &'a T,
    folded: &'a T,
    board: &'a T,
) -> Result<Option<&'a T>> {
    if board == folded || board == snapshot {
        return Ok(None);
    }
    if folded == snapshot {
        return Ok(Some(board));
    }
    Err(anyhow!("{id}: {field} changed on the board and in the ledger").into())
}

fn board_digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn marker_digest(issues_org: &Path) -> Result<Option<String>> {
    let path = marker_path(issues_org);
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let mut lines = text.lines();
    if lines.next() != Some("1") {
        return Ok(None);
    }
    Ok(lines
        .next()
        .filter(|line| line.len() == 64)
        .map(str::to_string))
}

fn write_marker(issues_org: &Path, digest: &str) -> Result<()> {
    let path = marker_path(issues_org);
    let body = format!("1\n{digest}\n");
    if path.is_file() {
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .with_context(|| format!("open {}", path.display()))?;
        file.write_all(body.as_bytes())
            .with_context(|| format!("write {}", path.display()))?;
        file.sync_all()
            .with_context(|| format!("sync {}", path.display()))?;
        return Ok(());
    }
    write_new(&path, body.as_bytes())
}

fn header_bytes(preamble: &str, line_start: usize, line_end: usize, slice: &str) -> String {
    let mut out = String::new();
    let preamble = preamble.trim_end();
    if !preamble.is_empty() {
        out.push_str(preamble);
        out.push_str("\n\n");
    }
    out.push_str(LEDGER_MARK);
    out.push('\n');
    out.push_str(&format!("{LINES_MARK} {line_start} {line_end}\n"));
    out.push_str(slice);
    if !slice.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(LOG_MARK);
    out.push('\n');
    out
}

fn write_header(doc: &IssueDoc, heading: &IssueHeading) -> Result<()> {
    let rendered = heading.render();
    let (start, end) = rendered_span(&doc.preamble, &rendered);
    let bytes = header_bytes(&doc.preamble, start, end, &rendered);
    let file = issues_dir(&doc.path).join(file_name(&heading.id));
    write_new(&file, bytes.as_bytes())
}

fn rendered_span(preamble: &str, rendered: &str) -> (usize, usize) {
    let mut prefix_lines = 0usize;
    if !preamble.trim().is_empty() {
        prefix_lines += preamble.trim_end().lines().count() + 1;
    }
    // `#+VISSUE_LEDGER:` and `#+VISSUE_LINES:` sit above the heading.
    let start = prefix_lines + 3;
    let count = rendered.lines().count().max(1);
    (start, start + count - 1)
}

fn heading_bytes(content: &str, heading: &IssueHeading) -> Result<String> {
    if heading.line_start == 0 || heading.line_end < heading.line_start {
        return Err(anyhow!("issue {} has no line range", heading.id).into());
    }
    let mut out = String::new();
    for (index, line) in content.lines().enumerate() {
        let number = index + 1;
        if number < heading.line_start {
            continue;
        }
        if number > heading.line_end {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !out.starts_with("* ") {
        return Err(anyhow!(
            "issue {} slice at {}-{} does not start with a heading",
            heading.id,
            heading.line_start,
            heading.line_end
        )
        .into());
    }
    Ok(out)
}

fn read_dir(issues_org: &Path) -> Result<Vec<Loaded>> {
    let dir = issues_dir(issues_org);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut loaded = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') || !name.ends_with(".org") {
            continue;
        }
        loaded.push(read_file(issues_org, &path)?);
    }
    Ok(loaded)
}

fn read_file(issues_org: &Path, file: &Path) -> Result<Loaded> {
    let source = fs::read_to_string(file).with_context(|| format!("read {}", file.display()))?;
    let (left, log) = split_log(&source);
    let doc = IssueDoc::parse("ledger", issues_org.to_path_buf(), left)
        .with_context(|| format!("parse {}", file.display()))?;
    let mut heading = doc
        .headings
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no heading in {}", file.display()))?;
    // `#+VISSUE_LINES:` records the old board span. The heading's own
    // line numbers are where it sits in this file, which is what `show`
    // prints next to the path.
    let tombstoned = apply_records(&mut heading, log, &doc.preamble)?;
    Ok(Loaded {
        file: file.to_path_buf(),
        source,
        heading,
        tombstoned,
    })
}

fn split_log(source: &str) -> (&str, &str) {
    let marker = format!("\n{LOG_MARK}\n");
    match source.split_once(&marker) {
        Some((left, log)) => (left, log),
        None => (source, ""),
    }
}

fn copied_preamble(source: &str) -> Option<String> {
    let index = source.find(LEDGER_MARK)?;
    let text = source[..index].trim_end();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn apply_records(heading: &mut IssueHeading, log: &str, preamble: &str) -> Result<bool> {
    let bytes = log.as_bytes();
    let mut index = 0usize;
    let mut tombstoned = false;
    while index < bytes.len() {
        while index < bytes.len() && (bytes[index] == b'\n' || bytes[index] == b'\r') {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        if !bytes[index..].starts_with(b"%% ") {
            break;
        }
        let Some(relative_end) = bytes[index..].iter().position(|byte| *byte == b'\n') else {
            break;
        };
        let header_end = index + relative_end;
        let header = std::str::from_utf8(&bytes[index..header_end])
            .map_err(|err| anyhow!("ledger record header: {err}"))?;
        let mut parts = header[3..].split_whitespace();
        let Some(op) = parts.next() else {
            break;
        };
        let Some(len) = parts.next().and_then(|raw| raw.parse::<usize>().ok()) else {
            break;
        };
        let payload_start = header_end + 1;
        let payload_end = payload_start.saturating_add(len);
        if payload_end > bytes.len() {
            break;
        }
        let payload = std::str::from_utf8(&bytes[payload_start..payload_end])
            .map_err(|err| anyhow!("ledger record: {err}"))?;
        index = payload_end;
        if index < bytes.len() && bytes[index] == b'\n' {
            index += 1;
        }
        let op = op.to_string();
        apply_one(heading, &op, payload, preamble, &mut tombstoned)?;
    }
    Ok(tombstoned)
}

fn apply_one(
    heading: &mut IssueHeading,
    op: &str,
    payload: &str,
    preamble: &str,
    tombstoned: &mut bool,
) -> Result<()> {
    match op {
        "tombstone" => *tombstoned = true,
        "replace" => {
            *tombstoned = false;
            let parsed = heading_from_rendered(preamble, payload)?;
            let line_start = heading.line_start;
            let line_end = heading.line_end;
            *heading = parsed;
            heading.line_start = line_start;
            heading.line_end = line_end;
        }
        "state" => heading.state = payload.to_string(),
        "priority" => {
            if let Some(cookie) = payload.chars().next() {
                heading.priority = cookie;
            }
        }
        "title" => heading.title = payload.to_string(),
        "statistics" => {
            heading.statistics = if payload.is_empty() {
                None
            } else {
                Some(payload.to_string())
            };
        }
        "org-tags" => {
            heading.org_tags = if payload.is_empty() {
                Vec::new()
            } else {
                payload.lines().map(str::to_string).collect()
            };
        }
        "prop" => {
            let Some((key, value)) = payload.split_once('\n') else {
                return Err(anyhow!("prop record is missing a value").into());
            };
            if !heading
                .property_order
                .iter()
                .any(|existing| existing == key)
            {
                heading.property_order.push(key.to_string());
            }
            heading
                .properties
                .insert(key.to_string(), value.to_string());
        }
        "prop-del" => {
            heading.properties.remove(payload);
            heading.property_order.retain(|key| key != payload);
        }
        "body" => heading.body = payload.to_string(),
        "drawers" => heading.extra_drawers = split_drawers(payload),
        "log" => heading.logbook.insert(0, parse_log_line(payload)),
        "log-set" => {
            heading.logbook = payload
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(parse_log_line)
                .collect();
        }
        other => return Err(anyhow!("unknown ledger op {other}").into()),
    }
    Ok(())
}

fn heading_from_rendered(preamble: &str, rendered: &str) -> Result<IssueHeading> {
    let text = format!("{}\n{rendered}", preamble.trim_end());
    let doc = IssueDoc::parse("ledger", PathBuf::new(), &text)?;
    doc.headings
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("replace record has no heading").into())
}

fn split_drawers(payload: &str) -> Vec<String> {
    let mut drawers = Vec::new();
    let mut current = String::new();
    for line in payload.lines() {
        current.push_str(line);
        current.push('\n');
        if line.trim().eq_ignore_ascii_case(":END:") {
            drawers.push(std::mem::take(&mut current));
        }
    }
    if !current.trim().is_empty() {
        drawers.push(current);
    }
    drawers
}

fn push_diff(buf: &mut Vec<u8>, old: &IssueHeading, new: &IssueHeading) {
    if old.state != new.state {
        push_op(buf, "state", &new.state);
    }
    if old.priority != new.priority {
        push_op(buf, "priority", &new.priority.to_string());
    }
    if old.title != new.title {
        push_op(buf, "title", &new.title);
    }
    if old.statistics != new.statistics {
        push_op(buf, "statistics", new.statistics.as_deref().unwrap_or(""));
    }
    if old.org_tags != new.org_tags {
        push_op(buf, "org-tags", &new.org_tags.join("\n"));
    }
    push_props(buf, &old.properties, &new.properties);
    if old.extra_drawers != new.extra_drawers {
        push_op(buf, "drawers", &new.extra_drawers.concat());
    }
    if old.body != new.body {
        push_op(buf, "body", &new.body);
    }
    push_log_diff(buf, &old.logbook, &new.logbook);
}

fn push_props(buf: &mut Vec<u8>, old: &BTreeMap<String, String>, new: &BTreeMap<String, String>) {
    for (key, value) in new {
        if key == "ID" {
            continue;
        }
        if old.get(key) != Some(value) {
            push_op(buf, "prop", &format!("{key}\n{value}"));
        }
    }
    for key in old.keys() {
        if key != "ID" && !new.contains_key(key) {
            push_op(buf, "prop-del", key);
        }
    }
}

fn push_log_diff(buf: &mut Vec<u8>, old: &[LogEntry], new: &[LogEntry]) {
    if old == new {
        return;
    }
    if new.len() >= old.len() && new[new.len() - old.len()..] == *old {
        let added = &new[..new.len() - old.len()];
        for entry in added.iter().rev() {
            push_op(buf, "log", &entry.render());
        }
        return;
    }
    let mut block = String::new();
    for entry in new {
        block.push_str(&entry.render());
        block.push('\n');
    }
    push_op(buf, "log-set", &block);
}

fn push_op(buf: &mut Vec<u8>, op: &str, payload: &str) {
    let header = format!("%% {op} {}\n", payload.len());
    buf.extend_from_slice(header.as_bytes());
    buf.extend_from_slice(payload.as_bytes());
    buf.push(b'\n');
}

fn append_record(path: &Path, bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    let wrote = file
        .write(bytes)
        .with_context(|| format!("append {}", path.display()))?;
    if wrote != bytes.len() {
        return Err(anyhow!(
            "short append to {}: wrote {wrote} of {} bytes",
            path.display(),
            bytes.len()
        )
        .into());
    }
    file.sync_data()
        .with_context(|| format!("sync {}", path.display()))?;
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    let wrote = file
        .write(bytes)
        .with_context(|| format!("write {}", path.display()))?;
    if wrote != bytes.len() {
        return Err(anyhow!(
            "short write to {}: wrote {wrote} of {} bytes",
            path.display(),
            bytes.len()
        )
        .into());
    }
    file.sync_all()
        .with_context(|| format!("sync {}", path.display()))?;
    Ok(())
}

fn issues_dir(issues_org: &Path) -> PathBuf {
    issues_org
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("issues")
}

fn marker_path(issues_org: &Path) -> PathBuf {
    issues_dir(issues_org).join(".ledger")
}

fn file_name(id: &str) -> String {
    let mut name = String::new();
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            name.push(byte as char);
        } else {
            name.push_str(&format!("%{byte:02X}"));
        }
    }
    name.push_str(".org");
    name
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Layout;
    use crate::ops;
    use crate::report;
    use std::sync::{Arc, Barrier};
    use std::thread;

    const FIXTURE: &str = "\
#+TITLE: sample issues
#+VISSUE: 1
#+FILETAGS: :issues:sample:noexport:
#+TODO: TODO STARTED BLOCKED | DONE CANCELLED

* TODO [#C] First
:PROPERTIES:
:ID:         sample-aaaa
:CREATED:    [2026-10-04 Sun]
:END:
:LOGBOOK:
- Note: \"kept\" [2026-10-04 Sun 01:00]
:END:

Body one.

* STARTED [#A] Second
:PROPERTIES:
:ID:         sample-bbbb
:CREATED:    [2026-10-04 Sun]
:BLOCKED_BY: sample-aaaa
:END:

Body two.
";

    fn layout_with_fixture() -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), "Software");
        let path = layout.project_issues_path("sample");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, FIXTURE).unwrap();
        (dir, layout)
    }

    #[test]
    fn migrate_show_reads_the_per_issue_file_and_a_note_appends_to_one_file() {
        let (_dir, layout) = layout_with_fixture();
        let path = layout.project_issues_path("sample");
        let before_a = report::show(&layout, "sample-aaaa").unwrap();
        let before_b = report::show(&layout, "sample-bbbb").unwrap();
        let original = fs::read(&path).unwrap();

        let planned = migrate(&layout, true).unwrap();
        assert!(planned.contains("dry-run: wrote nothing"), "{planned}");
        assert!(!issues_dir(&path).exists());
        assert_eq!(fs::read(&path).unwrap(), original);

        let done = migrate(&layout, false).unwrap();
        assert!(done.contains("2 issue"), "{done}");
        assert_eq!(fs::read(&path).unwrap(), original);
        let after_a = report::show(&layout, "sample-aaaa").unwrap();
        let after_b = report::show(&layout, "sample-bbbb").unwrap();
        assert_eq!(without_file(&after_a), without_file(&before_a));
        assert_eq!(without_file(&after_b), without_file(&before_b));
        assert_per_issue_file(&after_a, "sample-aaaa");
        assert_per_issue_file(&after_b, "sample-bbbb");

        let again = migrate(&layout, false).unwrap();
        assert!(again.contains("skipped"), "{again}");

        let files_before = snapshot(&issues_dir(&path));
        ops::note(&layout, "sample-aaaa", "ledger note").unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
        let files_after = snapshot(&issues_dir(&path));
        let mut changed = Vec::new();
        for (file, bytes) in &files_after {
            let previous = files_before.get(file).map(Vec::as_slice).unwrap_or(b"");
            if previous != bytes.as_slice() {
                assert!(
                    bytes.starts_with(previous),
                    "{} rewrote existing bytes",
                    file.display()
                );
                changed.push(file.clone());
            }
        }
        assert_eq!(changed.len(), 1, "note touched {changed:?}");
        let noted = report::show(&layout, "sample-aaaa").unwrap();
        assert_eq!(without_file(&noted), without_file(&after_a));
        assert_per_issue_file(&noted, "sample-aaaa");

        let (heading, _, _) = crate::store::find_by_id(&layout, "sample-aaaa")
            .unwrap()
            .unwrap();
        assert!(
            heading
                .logbook
                .iter()
                .any(|entry| entry.note.as_deref() == Some("ledger note"))
        );
        assert!(
            heading
                .logbook
                .iter()
                .any(|entry| entry.note.as_deref() == Some("kept"))
        );
        assert_eq!(heading.state, "TODO");

        ops::update(
            &layout,
            "sample-bbbb",
            Some("BLOCKED"),
            Some('B'),
            None,
            None,
        )
        .unwrap();
        ops::update(&layout, "sample-bbbb", Some("TODO"), Some('C'), None, None).unwrap();
        let (second, _, _) = crate::store::find_by_id(&layout, "sample-bbbb")
            .unwrap()
            .unwrap();
        assert_eq!(second.state, "TODO");
        assert_eq!(second.priority, 'C');
        assert_eq!(fs::read(&path).unwrap(), original);
        let grown = fs::read(issues_dir(&path).join(file_name("sample-bbbb"))).unwrap();
        let text = String::from_utf8(grown).unwrap();
        assert!(text.contains("BLOCKED"), "{text}");
        assert!(text.contains("TODO"), "{text}");
        assert!(text.contains("%% priority 1\nB"), "{text}");
        assert!(text.contains("%% priority 1\nC"), "{text}");
    }

    #[test]
    fn close_appends_to_the_per_issue_file() {
        let (_dir, layout) = layout_with_fixture();
        let board = layout.project_issues_path("sample");
        let original = fs::read(&board).unwrap();
        migrate(&layout, false).unwrap();
        let store = issues_dir(&board).join(file_name("sample-aaaa"));
        let before = fs::read(&store).unwrap();
        ops::update(&layout, "sample-aaaa", Some("DONE"), None, None, None).unwrap();
        assert_eq!(fs::read(&board).unwrap(), original);
        let after = fs::read(&store).unwrap();
        assert!(after.starts_with(&before));
        assert!(after.len() > before.len());
        let shown = report::show(&layout, "sample-aaaa").unwrap();
        assert!(shown.contains("State:    DONE"), "{shown}");
        assert_per_issue_file(&shown, "sample-aaaa");
    }

    #[test]
    fn a_board_note_after_the_copy_is_folded_and_a_second_run_skips() {
        let (_dir, layout) = layout_with_fixture();
        let board = layout.project_issues_path("sample");
        migrate(&layout, false).unwrap();
        let marker = marker_path(&board);
        let sealed = fs::read_to_string(&marker).unwrap();
        let copied = board_digest(&fs::read(&board).unwrap());
        assert_eq!(sealed.lines().next(), Some("1"));
        assert_eq!(sealed.lines().nth(1), Some(copied.as_str()));

        ops::note(&layout, "sample-aaaa", "ledger note").unwrap();
        insert_log_line(&board, "- Note: \"from the board\" [2026-10-04 Sun 02:00]");
        let moved = fs::read(&board).unwrap();
        let store = issues_dir(&board).join(file_name("sample-aaaa"));
        let before = fs::read(&store).unwrap();

        let caught = migrate(&layout, false).unwrap();
        assert!(caught.contains("caught up 1 issue"), "{caught}");
        assert_eq!(fs::read(&board).unwrap(), moved);
        let after = fs::read(&store).unwrap();
        assert!(after.starts_with(&before));
        assert!(after.len() > before.len());
        let (heading, _, _) = crate::store::find_by_id(&layout, "sample-aaaa")
            .unwrap()
            .unwrap();
        let notes: Vec<_> = heading
            .logbook
            .iter()
            .filter_map(|entry| entry.note.clone())
            .collect();
        assert!(notes.iter().any(|note| note == "ledger note"), "{notes:?}");
        assert!(
            notes.iter().any(|note| note == "from the board"),
            "{notes:?}"
        );
        assert_eq!(
            notes.iter().filter(|note| note.as_str() == "kept").count(),
            1,
            "{notes:?}"
        );

        let again = migrate(&layout, false).unwrap();
        assert!(again.contains("skipped"), "{again}");
        assert_eq!(fs::read(&store).unwrap(), after);
        let sealed_after = board_digest(&moved);
        assert_eq!(
            fs::read_to_string(&marker).unwrap().lines().nth(1),
            Some(sealed_after.as_str())
        );
    }

    #[test]
    fn a_field_changed_on_the_board_and_in_the_ledger_is_not_sealed() {
        let (_dir, layout) = layout_with_fixture();
        let board = layout.project_issues_path("sample");
        migrate(&layout, false).unwrap();
        let marker = fs::read_to_string(marker_path(&board)).unwrap();
        ops::update(&layout, "sample-bbbb", Some("DONE"), None, None, None).unwrap();
        let text = fs::read_to_string(&board).unwrap();
        let text = text.replace("* STARTED [#A] Second", "* CANCELLED [#A] Second");
        fs::write(&board, &text).unwrap();

        let err = migrate(&layout, false).unwrap_err();
        assert!(
            err.to_string()
                .contains("state changed on the board and in the ledger"),
            "{err}"
        );
        assert_eq!(fs::read_to_string(marker_path(&board)).unwrap(), marker);
        assert_eq!(fs::read_to_string(&board).unwrap(), text);
        let shown = report::show(&layout, "sample-bbbb").unwrap();
        assert!(shown.contains("State:    DONE"), "{shown}");
    }

    #[test]
    fn show_under_a_held_lock_matches_apart_from_the_file() {
        let (dir, layout) = layout_with_fixture();
        let proof = dir.path().join("proof");
        let report = migrate_frozen(&[&layout], &proof).unwrap();
        assert!(report.contains("frozen:"), "{report}");
        let before = fs::read_to_string(proof.join("before.txt")).unwrap();
        let after = fs::read_to_string(proof.join("after.txt")).unwrap();
        assert_eq!(without_file(&after), without_file(&before));
        assert!(before.contains("issues.org"), "{before}");
        assert_per_issue_file(&after, "sample-aaaa");
        assert_per_issue_file(&after, "sample-bbbb");
        let dry = fs::read_to_string(proof.join("dry-run.txt")).unwrap();
        assert!(dry.contains("dry-run: wrote nothing"), "{dry}");
        assert!(
            issues_dir(&layout.project_issues_path("sample"))
                .join(".ledger")
                .is_file()
        );
        let catchup = fs::read_to_string(proof.join("catchup.txt")).unwrap();
        assert!(catchup.contains("skipped"), "{catchup}");
        assert!(!catchup.contains("caught up"), "{catchup}");
    }

    fn without_file(text: &str) -> String {
        text.lines()
            .filter(|line| !line.starts_with("File:"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn assert_per_issue_file(text: &str, id: &str) {
        let file = text
            .lines()
            .find(|line| line.starts_with("File:"))
            .unwrap_or("");
        assert!(
            file.contains(&format!("issues/{id}.org")),
            "show did not name the per-issue file: {file}"
        );
        assert!(
            !file.contains("issues.org"),
            "show still names the project board: {file}"
        );
    }

    #[test]
    fn two_threads_appending_notes_both_land() {
        let (_dir, layout) = layout_with_fixture();
        migrate(&layout, false).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for text in ["alpha note", "beta note"] {
            let layout = layout.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                ops::note(&layout, "sample-aaaa", text).unwrap();
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        let (heading, _, _) = crate::store::find_by_id(&layout, "sample-aaaa")
            .unwrap()
            .unwrap();
        let notes: Vec<_> = heading
            .logbook
            .iter()
            .filter_map(|entry| entry.note.clone())
            .collect();
        assert!(notes.iter().any(|note| note == "alpha note"), "{notes:?}");
        assert!(notes.iter().any(|note| note == "beta note"), "{notes:?}");
    }

    fn insert_log_line(path: &Path, line: &str) {
        let text = fs::read_to_string(path).unwrap();
        let needle = ":LOGBOOK:\n";
        let pos = text.find(needle).unwrap() + needle.len();
        let mut next = String::new();
        next.push_str(&text[..pos]);
        next.push_str(line);
        next.push('\n');
        next.push_str(&text[pos..]);
        fs::write(path, next).unwrap();
    }

    fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut out = BTreeMap::new();
        let entries = fs::read_dir(dir).unwrap();
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_file() {
                out.insert(path.clone(), fs::read(&path).unwrap());
            }
        }
        out
    }
}
