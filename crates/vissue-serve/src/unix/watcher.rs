//! Coalesce inotify and generation polls into one catalog rebuild.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crate::error::Result;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use vissue_control::rpc::{Notification, VaultChanged};
use vissue_core::catalog::load_recs;
use vissue_core::config::Layout;
use vissue_core::events;
use vissue_core::store::list_projects;

use super::catalog::{load_project_recs, project_from_path};
use super::owner::OwnerState;

/// Quiet time after the last signal before one rebuild takes them all.
///
/// It must outlast the longest gap between the events of one save, or that save is read twice.
/// It is also the delay every change waits before clients hear of it.
/// A change only the poll sees reaches clients after `GEN_POLL_MS` + the slowest tick + this + the slowest rebuild.
/// That has to stay under one second, how long a user waits without losing their train of thought (Nielsen, Usability Engineering, 1993, ch. 5).
/// So: longest gap < `COALESCE_MS` <= 1000 - `GEN_POLL_MS` - slowest tick - slowest rebuild.
/// Over six runs per platform on the 173-project tracker, as median, p95 and max in ms (`scripts/bench/watch_timing.md`):
/// the longest gap of one save was 0.23, 2.08 and 3.75 on Linux and 0.01, 0.03 and 109.69 on the macOS runner,
/// a full rebuild 23.417, 28.069 and 41.573 on Linux and 47.271, 64.091 and 132.728 on macOS,
/// and a tick as under `GEN_POLL_MS`, at most 8.356 on Linux and 50.190 on macOS.
/// That bounds it to 3.75 < it <= 700.071 ms on Linux and 109.69 < it <= 567.082 ms on macOS.
/// The data fixes only that range: the macOS floor is one save, so six runs do not bound the tail.
/// So 200 is a measured budget, not a derived value, checked against `scripts/bench/data/watch_timing/` and `scripts/bench/data/rebuild_timing/`.
/// It is 53.33 times the longest gap on Linux, but only 1.82 times the longest on macOS.
const COALESCE_MS: u64 = 200;
/// Interval of the generation and mtime poll, the path for changes the file watcher misses.
///
/// It must outlast the slowest tick, or ticks are skipped, and keep the slowest wait under the one second named at `COALESCE_MS`:
/// slowest tick < `GEN_POLL_MS` <= 1000 - `COALESCE_MS` - slowest tick - slowest rebuild.
/// A tick over 173 projects took 1.149, 1.571 and 8.356 ms (median, p95, max) on Linux and 7.533, 13.875 and 50.190 ms on the macOS runner.
/// That bounds it to 8.356 < it <= 750.071 ms on Linux and 50.190 < it <= 617.082 ms on macOS.
/// Inside that it trades idle cost against the wait of a change only the poll sees, and no standard sets the idle cost.
/// So 250 is a measured budget, not a derived value, checked against the same runs (`scripts/bench/watch_timing.md`).
/// At 250 ms the median tick costs 0.46% of a core on Linux and 3.01% on macOS.
/// The slowest wait is then 499.929 ms on Linux and 632.918 ms on macOS, 500.071 and 367.082 ms under the second.
const GEN_POLL_MS: u64 = 250;

struct Pending {
    last_signal: Instant,
    full: bool,
    dirty: HashSet<String>,
}

impl Pending {
    fn mark(&mut self, full: bool, dirty: HashSet<String>) {
        self.last_signal = Instant::now();
        self.full |= full;
        self.dirty.extend(dirty);
    }
}

pub async fn run(state: Arc<OwnerState>) -> Result<()> {
    let projects_dir = state.layout.projects_dir();
    std::fs::create_dir_all(&projects_dir)?;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<Event>| {
            let _ = tx.send(res);
        },
        notify::Config::default(),
    )?;
    watcher.watch(&projects_dir, RecursiveMode::Recursive)?;

    let mut last_gen = events::generation(&state.layout);
    let mut last_mtimes = collect_mtimes(&state.layout);
    let mut published = state
        .catalog
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .revision;
    let mut pending: Option<Pending> = None;
    let mut poll = tokio::time::interval(Duration::from_millis(GEN_POLL_MS));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        let wait = pending
            .as_ref()
            .map(|p| Duration::from_millis(COALESCE_MS).saturating_sub(p.last_signal.elapsed()));
        tokio::select! {
            ev = rx.recv() => {
                let Some(ev) = ev else {
                    return Ok(());
                };
                if let Ok(event) = ev {
                    let mut dirty = HashSet::new();
                    let mut full = false;
                    classify_event(&state.layout, &event, &mut dirty, &mut full);
                    // An event that named nothing schedules nothing. `rebuild`
                    // reads an empty dirty set as "reload everything", so
                    // queueing a job here regardless would turn any stray
                    // event into a full reload and a broadcast to every client.
                    if full || !dirty.is_empty() {
                        pending.get_or_insert_with(|| Pending {
                            last_signal: Instant::now(),
                            full: false,
                            dirty: HashSet::new(),
                        }).mark(full, dirty);
                    }
                }
            }
            _ = poll.tick() => {
                let generation = events::generation(&state.layout);
                let mtimes = collect_mtimes(&state.layout);
                let mut dirty = HashSet::new();
                let mut full = false;
                if generation != last_gen {
                    last_gen = generation;
                    full = true;
                }
                if mtimes != last_mtimes {
                    for (project, stamp) in &mtimes {
                        if last_mtimes.get(project) != Some(stamp) {
                            dirty.insert(project.clone());
                        }
                    }
                    if mtimes.len() != last_mtimes.len() {
                        full = true;
                    }
                    last_mtimes = mtimes;
                }
                if full || !dirty.is_empty() {
                    pending.get_or_insert_with(|| Pending {
                        last_signal: Instant::now(),
                        full: false,
                        dirty: HashSet::new(),
                    }).mark(full, dirty);
                }
            }
            _ = sleep_optional(wait) => {
                if let Some(job) = pending.take() {
                    rebuild(&state, job.dirty, job.full, &mut published).await;
                    last_gen = events::generation(&state.layout);
                    last_mtimes = collect_mtimes(&state.layout);
                }
            }
        }
    }
}

async fn sleep_optional(wait: Option<Duration>) {
    match wait {
        Some(d) => tokio::time::sleep(d).await,
        None => std::future::pending::<()>().await,
    }
}

fn classify_event(layout: &Layout, event: &Event, dirty: &mut HashSet<String>, full: &mut bool) {
    // Reading is not changing. The generation poll opens the projects
    // directory every tick, and inotify reports that open against the very
    // tree being watched, so counting it as signal makes the watcher feed
    // itself: poll, rebuild, broadcast, poll again.
    if matches!(event.kind, EventKind::Access(_)) {
        return;
    }
    for path in &event.paths {
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            if name.starts_with(".vault-events") {
                *full = true;
                continue;
            }
            if name == "issues.org" {
                if let Some(project) = project_from_path(layout, path) {
                    dirty.insert(project);
                } else {
                    *full = true;
                }
                continue;
            }
        }
        if matches!(event.kind, EventKind::Create(_) | EventKind::Remove(_)) {
            *full = true;
        }
        if let Some(project) = project_from_path(layout, path) {
            dirty.insert(project);
        }
    }
}

fn collect_mtimes(layout: &Layout) -> HashMap<String, Option<SystemTime>> {
    let mut out = HashMap::new();
    let Ok(projects) = list_projects(layout) else {
        return out;
    };
    for project in projects {
        let path = layout.project_issues_path(&project);
        let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        out.insert(project, stamp);
    }
    out
}

/// Reload what `dirty` and `full` name and broadcast what changed.
///
/// An event is not proof of a change: macOS can deliver writes made just
/// before the watch began, and a touch moves only the mtime. So a project
/// whose recs the catalog already holds is left out, and a reload that
/// changes nothing broadcasts nothing. The comparison is on parsed issues,
/// not bytes: new bytes that parse the same are no change, and issues that
/// only moved lines are one, since each carries its line span. That
/// comparison needs the catalog to
/// be what this loop last broadcast (`published`): a write through the
/// server refreshes it without a broadcast, and the rebuild its events cause
/// here is how other clients hear of that write.
async fn rebuild(state: &OwnerState, dirty: HashSet<String>, full: bool, published: &mut u64) {
    let Ok(permit) = state.reload_sem.acquire().await else {
        return;
    };
    let layout = state.layout.clone();
    let do_full = full || dirty.is_empty();
    let dirty_vec: Vec<String> = {
        let mut v: Vec<String> = dirty.into_iter().collect();
        v.sort();
        v
    };
    let loaded = tokio::task::spawn_blocking(move || -> Result<_> {
        if do_full {
            let recs = load_recs(&layout)?;
            Ok(Reload::Full {
                recs,
                dirty: dirty_vec,
            })
        } else {
            let mut parts = Vec::new();
            for project in &dirty_vec {
                parts.push((project.clone(), load_project_recs(&layout, project)?));
            }
            Ok(Reload::Partial(parts))
        }
    })
    .await;
    drop(permit);
    let Ok(Ok(reload)) = loaded else {
        return;
    };
    let note = {
        let mut cat = state.catalog.write().unwrap_or_else(|p| p.into_inner());
        let settled =
            cat.revision == *published && cat.generation == events::generation(&state.layout);
        match reload {
            Reload::Full { recs, dirty } => {
                let dirty = if settled {
                    cat.changed_projects(&state.layout, &recs)
                } else {
                    dirty
                };
                if settled && dirty.is_empty() {
                    return;
                }
                let ids = if dirty.is_empty() {
                    None
                } else {
                    Some(
                        recs.iter()
                            .filter(|r| dirty.iter().any(|p| p == &r.project))
                            .map(|r| r.heading.id.clone())
                            .collect(),
                    )
                };
                cat.apply_full(&state.layout, recs, dirty, ids);
            }
            Reload::Partial(parts) => {
                let parts: Vec<_> = parts
                    .into_iter()
                    .filter(|(project, fresh)| {
                        let listed = state.layout.project_issues_path(project).exists();
                        !settled || !cat.holds(project, listed, fresh)
                    })
                    .collect();
                if parts.is_empty() {
                    return;
                }
                cat.replace_projects(&state.layout, parts);
            }
        }
        *published = cat.revision;
        Notification::VaultChanged(VaultChanged {
            generation: cat.generation,
            revision: cat.revision,
            projects: cat.dirty_projects.clone(),
            ids: cat.dirty_ids.clone(),
        })
    };
    state.bus.broadcast(&note);
}

enum Reload {
    Full {
        recs: Vec<vissue_core::views::IssueRec>,
        dirty: Vec<String>,
    },
    Partial(Vec<(String, Vec<vissue_core::views::IssueRec>)>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn issues_org_marks_the_project() {
        let layout = Layout::new("/tmp/tracker", "Software");
        let event = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            paths: vec![PathBuf::from("/tmp/tracker/Software/atlas/issues.org")],
            attrs: notify::event::EventAttributes::new(),
        };
        let mut dirty = HashSet::new();
        let mut full = false;
        classify_event(&layout, &event, &mut dirty, &mut full);
        assert!(dirty.contains("atlas"));
        assert!(!full);
    }

    /// Opening a file is not editing it.
    ///
    /// The generation poll opens the projects directory on every tick, and
    /// inotify reports that open against the watched tree. Classified as a
    /// change it schedules a rebuild, whose own reads raise the next event:
    /// an idle tracker then reloads and broadcasts about four times a second
    /// forever.
    #[test]
    fn a_read_is_not_a_change() {
        let layout = Layout::new("/tmp/tracker", "Software");
        for path in [
            "/tmp/tracker/Software",
            "/tmp/tracker/Software/atlas/issues.org",
            "/tmp/tracker/Software/.vault-events.gen",
        ] {
            let event = Event {
                kind: EventKind::Access(notify::event::AccessKind::Open(
                    notify::event::AccessMode::Any,
                )),
                paths: vec![PathBuf::from(path)],
                attrs: notify::event::EventAttributes::new(),
            };
            let mut dirty = HashSet::new();
            let mut full = false;
            classify_event(&layout, &event, &mut dirty, &mut full);
            assert!(dirty.is_empty(), "{path} marked {dirty:?} dirty on a read");
            assert!(!full, "{path} forced a full reload on a read");
        }
    }

    /// An event about the watched root names no project and changes nothing.
    ///
    /// `rebuild` reads an empty dirty set as "reload everything", so the loop
    /// only queues work when classification actually marked something.
    #[test]
    fn an_event_on_the_root_marks_nothing() {
        let layout = Layout::new("/tmp/tracker", "Software");
        let event = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Metadata(
                notify::event::MetadataKind::Any,
            )),
            paths: vec![PathBuf::from("/tmp/tracker/Software")],
            attrs: notify::event::EventAttributes::new(),
        };
        let mut dirty = HashSet::new();
        let mut full = false;
        classify_event(&layout, &event, &mut dirty, &mut full);
        assert!(dirty.is_empty(), "{dirty:?}");
        assert!(!full);
    }

    #[test]
    fn gen_file_forces_full() {
        let layout = Layout::new("/tmp/tracker", "Software");
        let event = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            paths: vec![PathBuf::from("/tmp/tracker/Software/.vault-events.gen")],
            attrs: notify::event::EventAttributes::new(),
        };
        let mut dirty = HashSet::new();
        let mut full = false;
        classify_event(&layout, &event, &mut dirty, &mut full);
        assert!(full);
    }
}
