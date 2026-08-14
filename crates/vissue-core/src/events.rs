//! Append-only change log and generation counter, so a poller notices a write
//! without re-parsing every issues.org.
//!
//! Two files sit beside the project directories:
//!
//! - `.vault-events.gen`, a monotonic counter; comparing it against the last
//!   value seen is the cheap change check.
//! - `.vault-events.jsonl`, one event per line, read with a `since` sequence.
//!
//! The names are fixed rather than derived from the product name: existing
//! readers watch these paths, and renaming them would silence every poller.
//!
//! Emission is best-effort and never fails a write. Set `VISSUE_EVENTS=0` to
//! turn it off, which is what a caller wants when the tracker must stay
//! untouched.

use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex, OnceLock,
};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Layout;

static LAST_SEQ: AtomicU64 = AtomicU64::new(0);
static LAST_EMIT_MS_BY_DIR: OnceLock<Mutex<HashMap<PathBuf, u64>>> = OnceLock::new();
static PROCESS_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();

const LOCK_NAME: &str = ".vault-events.lock";

fn with_events_lock<R, F>(dir: &Path, f: F) -> Result<R>
where
    F: FnOnce() -> Result<R>,
{
    let key = dir.to_path_buf();
    let mutex = {
        let mut map = PROCESS_LOCKS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        map.entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _proc = mutex.lock().unwrap_or_else(|p| p.into_inner());
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let lock_path = dir.join(LOCK_NAME);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(&lock_path)
        .with_context(|| format!("open {}", lock_path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("lock {}", lock_path.display()))?;
    let result = f();
    let _ = FileExt::unlock(&file);
    let _ = file;
    result
}

/// Repeated writes inside this window bump the generation without appending a
/// line, so an editor or agent saving in a burst does not flood the log.
const DEBOUNCE_MS: u64 = 2000;

const LOG_NAME: &str = ".vault-events.jsonl";
const GEN_NAME: &str = ".vault-events.gen";

/// One entry in the change log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Monotonic sequence, mirrored in the generation file.
    pub seq: u64,
    /// Unix seconds.
    pub ts: u64,
    /// `issues_write`, `ping`, and so on.
    pub kind: String,
    pub project: Option<String>,
    pub id: Option<String>,
    pub path: Option<String>,
    pub detail: Option<String>,
}

/// Whether emission is enabled. `VISSUE_EVENTS=0` disables it.
pub fn enabled() -> bool {
    !matches!(
        std::env::var("VISSUE_EVENTS").as_deref(),
        Ok("0") | Ok("false") | Ok("off")
    )
}

pub fn log_path(dir: &Path) -> PathBuf {
    dir.join(LOG_NAME)
}

pub fn gen_path(dir: &Path) -> PathBuf {
    dir.join(GEN_NAME)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn read_gen(dir: &Path) -> u64 {
    fs::read_to_string(gen_path(dir))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn write_gen(dir: &Path, seq: u64) -> Result<()> {
    fs::create_dir_all(dir)?;
    let target = gen_path(dir);
    // A shared temporary name races: concurrent emitters would rename each
    // other's file out from under themselves.
    let tmp = dir.join(format!("{GEN_NAME}.tmp.{}-{}", std::process::id(), seq));
    fs::write(&tmp, format!("{seq}\n"))?;
    if let Err(e) = fs::rename(&tmp, &target) {
        let _ = fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("rename {} -> {}", tmp.display(), target.display()));
    }
    Ok(())
}

/// The current generation. A poller compares this against the last value it
/// saw; a difference means something changed.
pub fn generation_in(dir: &Path) -> u64 {
    let g = read_gen(dir);
    let _ = LAST_SEQ.fetch_max(g, Ordering::Relaxed);
    g
}

/// Append one event and return the new sequence.
pub fn emit_in(
    dir: &Path,
    kind: &str,
    project: Option<&str>,
    id: Option<&str>,
    path: Option<&Path>,
    detail: Option<&str>,
) -> Result<u64> {
    with_events_lock(dir, || {
        fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;

        let prev = read_gen(dir).max(LAST_SEQ.load(Ordering::Relaxed));
        let seq = prev.saturating_add(1);
        LAST_SEQ.store(seq, Ordering::Relaxed);

        let event = Event {
            seq,
            ts: now_secs(),
            kind: kind.to_string(),
            project: project.map(|s| s.to_string()),
            id: id.map(|s| s.to_string()),
            path: path.map(|p| p.display().to_string()),
            detail: detail.map(|s| s.to_string()),
        };

        if kind == "issues_write" {
            let now_ms = now_millis();
            let key = dir.to_path_buf();
            let mut last_by_dir = LAST_EMIT_MS_BY_DIR
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let previous = last_by_dir.get(&key).copied().unwrap_or(0);
            if previous > 0 && now_ms.saturating_sub(previous) < DEBOUNCE_MS {
                // Inside the window: advance the generation so pollers still wake,
                // but leave the log alone.
                write_gen(dir, seq)?;
                LAST_SEQ.store(seq, Ordering::Relaxed);
                last_by_dir.insert(key, now_ms);
                return Ok(seq);
            }
            last_by_dir.insert(key, now_ms);
        }

        let line = serde_json::to_string(&event)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path(dir))?;
        writeln!(file, "{line}")?;
        file.flush()?;
        write_gen(dir, seq)?;
        Ok(seq)
    })
}

/// Record that a project's issues.org was rewritten. Called by the store after
/// a successful write; failure here never fails the write.
pub fn emit_issues_write(dir: &Path, project: &str, path: &Path) -> Result<u64> {
    emit_in(
        dir,
        "issues_write",
        Some(project),
        None,
        Some(path),
        Some("issues.org updated"),
    )
}

/// Events with a sequence above `since_seq`, most recent last.
pub fn since_in(dir: &Path, since_seq: u64, limit: usize) -> Result<Vec<Event>> {
    let log = log_path(dir);
    if !log.is_file() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&log)?;
    let mut out = Vec::new();
    for line in text.lines().rev() {
        if line.trim().is_empty() {
            continue;
        }
        let event: Event = match serde_json::from_str(line) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if event.seq <= since_seq {
            break;
        }
        out.push(event);
        if out.len() >= limit {
            break;
        }
    }
    out.reverse();
    Ok(out)
}

/// As [`since_in`], narrowed to a project, a kind, or both.
pub fn since_filtered_in(
    dir: &Path,
    since_seq: u64,
    limit: usize,
    project: Option<&str>,
    kind: Option<&str>,
) -> Result<Vec<Event>> {
    let mut events = since_in(dir, since_seq, limit.saturating_mul(4).max(limit))?;
    if let Some(p) = project {
        events.retain(|e| e.project.as_deref() == Some(p));
    }
    if let Some(k) = kind {
        events.retain(|e| e.kind == k);
    }
    events.truncate(limit);
    Ok(events)
}

// --- Layout-facing wrappers, which is what the CLI and MCP server call. ---

/// The directory holding the event files: the same one holding the projects.
pub fn events_dir(layout: &Layout) -> PathBuf {
    layout.projects_dir()
}

pub fn generation(layout: &Layout) -> u64 {
    generation_in(&events_dir(layout))
}

pub fn since(layout: &Layout, since_seq: u64, limit: usize) -> Result<Vec<Event>> {
    since_in(&events_dir(layout), since_seq, limit)
}

/// Text plus a trailing JSON block, the shape existing readers parse.
pub fn since_report(layout: &Layout, since_seq: u64, limit: usize) -> Result<String> {
    let dir = events_dir(layout);
    let generation_now = generation_in(&dir);
    let events = since_in(&dir, since_seq, limit)?;
    let mut text = format!(
        "generation={} since={} count={}\n",
        generation_now,
        since_seq,
        events.len()
    );
    for e in &events {
        text.push_str(&format!(
            "{}\t{}\t{}\t{:?}\t{:?}\t{:?}\n",
            e.seq, e.ts, e.kind, e.project, e.id, e.path
        ));
    }
    let data = serde_json::json!({
        "generation": generation_now,
        "since": since_seq,
        "events": events,
        "log": log_path(&dir).display().to_string(),
        "gen_file": gen_path(&dir).display().to_string(),
    });
    text.push_str("\n---json---\n");
    text.push_str(&data.to_string());
    text.push('\n');
    Ok(text)
}

/// Append a manual event, waking pollers without touching an issues.org.
pub fn ping_report(layout: &Layout, detail: Option<&str>) -> Result<String> {
    let dir = events_dir(layout);
    let seq = emit_in(
        &dir,
        "ping",
        None,
        None,
        None,
        detail.or(Some("manual ping")),
    )?;
    Ok(format!(
        "ping seq={} generation={}\nlog={}\n",
        seq,
        generation_in(&dir),
        log_path(&dir).display()
    ))
}

/// Block until the generation passes `last`, or the timeout expires. Returns
/// the generation either way; the caller compares it against `last` to tell
/// which happened.
pub fn wait_generation(layout: &Layout, last: u64, poll_ms: u64, timeout_ms: u64) -> Result<u64> {
    let dir = events_dir(layout);
    let start = std::time::Instant::now();
    loop {
        let g = generation_in(&dir);
        if g > last {
            return Ok(g);
        }
        if start.elapsed().as_millis() as u64 >= timeout_ms {
            return Ok(g);
        }
        std::thread::sleep(std::time::Duration::from_millis(poll_ms.max(50)));
    }
}

/// The last `n` events, for a reader that only wants what is recent.
pub fn tail_report(layout: &Layout, n: usize) -> Result<String> {
    let generation_now = generation(layout);
    let since_seq = generation_now.saturating_sub(n as u64);
    since_report(layout, since_seq.saturating_sub(1), n)
}

/// Add the event files to a `.gitignore` that already exists beside them. Best
/// effort: a tracker without one is left alone.
pub fn ensure_gitignore_hint(dir: &Path) -> Result<()> {
    let gitignore = dir.join(".gitignore");
    if !gitignore.is_file() {
        return Ok(());
    }
    let current = fs::read_to_string(&gitignore)?;
    if current.contains(".vault-events") {
        return Ok(());
    }
    let mut file = OpenOptions::new().append(true).open(&gitignore)?;
    writeln!(
        file,
        "\n# agent ping stream (local)\n{LOG_NAME}\n{GEN_NAME}\n"
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_PREFIX;

    #[test]
    fn a_sequence_advances_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let first = emit_in(d, "ping", None, None, None, Some("a")).unwrap();
        let second = emit_in(
            d,
            "issues_write",
            Some("atlas"),
            Some("atlas-1a2b"),
            Some(Path::new("atlas/issues.org")),
            None,
        )
        .unwrap();
        assert!(second > first);
        assert_eq!(generation_in(d), second);

        let events = since_in(d, first, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "issues_write");
        assert_eq!(events[0].project.as_deref(), Some("atlas"));
    }

    #[test]
    fn filters_narrow_by_project_and_kind() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        emit_in(d, "ping", None, None, None, None).unwrap();
        emit_in(d, "manual", Some("atlas"), None, None, None).unwrap();
        emit_in(d, "manual", Some("beacon"), None, None, None).unwrap();

        let by_project = since_filtered_in(d, 0, 10, Some("atlas"), None).unwrap();
        assert_eq!(by_project.len(), 1);
        let by_kind = since_filtered_in(d, 0, 10, None, Some("ping")).unwrap();
        assert_eq!(by_kind.len(), 1);
        assert_eq!(by_kind[0].kind, "ping");
    }

    #[test]
    fn the_report_carries_a_json_block() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        ping_report(&layout, Some("hello")).unwrap();
        let text = since_report(&layout, 0, 10).unwrap();
        assert!(text.starts_with("generation="), "{text}");
        let (_, json) = text.split_once("---json---").expect("json block present");
        let parsed: serde_json::Value = serde_json::from_str(json.trim()).unwrap();
        assert_eq!(parsed["events"][0]["kind"], "ping");
        assert_eq!(parsed["events"][0]["detail"], "hello");
    }

    #[test]
    fn the_gitignore_hint_only_touches_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        ensure_gitignore_hint(dir.path()).unwrap();
        assert!(!dir.path().join(".gitignore").exists());

        fs::write(dir.path().join(".gitignore"), "target\n").unwrap();
        ensure_gitignore_hint(dir.path()).unwrap();
        let text = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(text.contains(LOG_NAME), "{text}");
        assert!(text.contains(GEN_NAME), "{text}");

        // Running again must not append a second copy.
        ensure_gitignore_hint(dir.path()).unwrap();
        let again = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(text, again);
    }

    #[test]
    fn concurrent_emits_assign_unique_sequences() {
        use std::sync::Arc;
        use std::thread;

        let dir = tempfile::tempdir().unwrap();
        let d = Arc::new(dir.path().to_path_buf());
        let handles: Vec<_> = (0..16)
            .map(|i| {
                let d = Arc::clone(&d);
                thread::spawn(move || emit_in(&d, "ping", None, None, None, Some(&format!("{i}"))))
            })
            .collect();
        let mut seqs = Vec::new();
        for handle in handles {
            seqs.push(handle.join().unwrap().unwrap());
        }
        seqs.sort_unstable();
        seqs.dedup();
        assert_eq!(seqs.len(), 16, "duplicate event sequences: {seqs:?}");
        assert_eq!(generation_in(&d), *seqs.last().unwrap());
    }

    #[test]
    fn waiting_returns_the_current_generation_on_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        let g = generation(&layout);
        let waited = wait_generation(&layout, g + 100, 50, 120).unwrap();
        assert!(waited <= g + 100, "timed out without advancing");
    }
}
