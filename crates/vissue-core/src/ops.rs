//! Mutating verbs: create, update, and move issues between projects.

use anyhow::{Context, anyhow};

use crate::error::Result;
use chrono::NaiveDate;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::config::{Layout, VissueConfig};
use crate::error::Error;
use crate::graph::DependencyGraph;
use crate::model::{IssueHeading, LogEntry, today_inactive_bracket};
use crate::store::{
    IssueDoc, collect_org_ids, detect_project_from_ctx, find_by_id, generate_id, load_all,
    resolve_existing_project_case, with_issues_lock, with_issues_locks,
};

/// Resolve the project to act on. An explicit name wins; then a current
/// directory inside `<root>/<prefix>/<name>/` means `<name>`; otherwise walk
/// up for `.project-ctx.toml` and read `[project].name`. None available is an
/// error, so nothing is ever guessed silently.
///
/// # Errors
///
/// Returns an error if the explicit project name is empty, no name can be
/// resolved, the current directory cannot be read, or the name matches more
/// than one project directory.
pub fn resolve_project(layout: &Layout, explicit: Option<&str>) -> Result<String> {
    if let Some(p) = explicit {
        if p.is_empty() {
            return Err(anyhow!("--project given but empty").into());
        }
        return resolve_existing_project_case(layout, p);
    }
    let cwd = std::env::current_dir()?;
    if let Some(name) = project_from_tracker_path(&layout.projects_dir(), &cwd) {
        return resolve_existing_project_case(layout, &name);
    }
    let detected = detect_project_from_ctx(&cwd).ok_or_else(|| {
        anyhow!(
            "no --project given and no .project-ctx.toml found walking up from {}",
            cwd.display()
        )
    })?;
    resolve_existing_project_case(layout, &detected)
}

/// The project directory `cwd` stands in, when it is under `projects_dir`.
/// Both paths are compared as given and canonicalized, so a symlinked vault
/// still matches.
fn project_from_tracker_path(projects_dir: &Path, cwd: &Path) -> Option<String> {
    let first_under = |base: &Path, here: &Path| {
        here.strip_prefix(base)
            .ok()
            .and_then(|rest| rest.components().next())
            .and_then(|c| match c {
                std::path::Component::Normal(name) => name.to_str().map(str::to_string),
                _ => None,
            })
    };
    first_under(projects_dir, cwd).or_else(|| {
        let base = projects_dir.canonicalize().ok()?;
        let here = cwd.canonicalize().ok()?;
        first_under(&base, &here)
    })
}

/// Optional fields on a new issue.
#[derive(Debug, Default, Clone, Copy)]
pub struct CreateOpts<'a> {
    /// Priority cookie; the configured default is used when `None`.
    pub priority: Option<char>,
    /// `:TYPE:` property.
    pub issue_type: Option<&'a str>,
    /// Deadline as an org timestamp.
    pub deadline: Option<&'a str>,
    /// Scheduled date as an org timestamp.
    pub scheduled: Option<&'a str>,
    /// Comma- or colon-separated tags.
    pub tags: Option<&'a str>,
    /// `:PARENT:` id; must already exist somewhere under the prefix.
    pub parent: Option<&'a str>,
    /// Print only the new id.
    pub quiet: bool,
    /// Body prose written under the properties drawer.
    pub body: Option<&'a str>,
    /// Extra ids treated as taken when minting, so a twin file on another
    /// layout cannot share a suffix with this create.
    pub extra_ids: &'a [String],
    /// Twin files whose ids are read inside the lock and treated as taken, so
    /// two creates in two roots cannot mint one suffix twice.
    pub extra_id_paths: &'a [PathBuf],
    /// Keep this id instead of minting one. It must be `{project}-` plus a
    /// suffix of `0-9a-z`, and it must be free in this file and in the twins.
    pub id: Option<&'a str>,
}

/// Append a new TODO issue to the project's file and return the status text.
///
/// The first `[[id:XXX]]` in `body` that names a heading already in the
/// corpus becomes `:DISCOVERED_FROM:`, unless that property is already set.
/// Prose never writes `:BLOCKED_BY:`.
///
/// # Errors
///
/// Returns an error if the priority is not `A`/`B`/`C`, a date does not parse,
/// `parent` is not a known org id, the id space is exhausted, or the file
/// cannot be locked or rewritten.
pub fn create(layout: &Layout, project: &str, title: &str, opts: CreateOpts<'_>) -> Result<String> {
    // A board this tracker projects from another holds its issues there; a
    // heading written here lands in a stub no seat reads.
    if let Some(board) = crate::projection::boards(layout.root())
        .unwrap_or_default()
        .into_iter()
        .find(|b| b.source != "self" && b.project.eq_ignore_ascii_case(project))
    {
        return Err(anyhow!(
            "{project} is not created here: {}",
            crate::projection::projected_note(&board)
        )
        .into());
    }
    let project = resolve_existing_project_case(layout, project)?;
    let cfg = VissueConfig::load(layout)?;
    let path = layout.project_issues_path(&project);
    let (spec, named) = match IssueDoc::parse_file(&project, &path) {
        Ok(doc) => (doc.priority_spec(), doc.priorities_are_named()),
        Err(_) => (crate::org::PrioritySpec::default(), false),
    };
    let house_new = !path.exists();
    let priority = opts.priority.unwrap_or(if named || house_new {
        spec.default
    } else {
        cfg.issues.default_priority
    });
    if !spec.contains(priority) {
        return Err(anyhow!(
            "invalid priority {priority:?}; file allows [#{}]..[#{}]",
            spec.highest,
            spec.lowest
        )
        .into());
    }

    // Parent and body [[id:]] both need the corpus id set; scan once.
    let known_ids = if opts.parent.is_some() || opts.body.is_some() {
        collect_org_ids(layout)?
    } else {
        std::collections::HashSet::new()
    };
    if let Some(p) = opts.parent
        && !known_ids.contains(p)
    {
        return Err(anyhow!("--parent {p} does not refer to any known id").into());
    }

    // Every file the mint consults is locked; with_issues_locks dedups paths.
    let mut lock_paths: Vec<PathBuf> = vec![path.clone()];
    lock_paths.extend(opts.extra_id_paths.iter().cloned());
    let lock_refs: Vec<&Path> = lock_paths.iter().map(PathBuf::as_path).collect();
    with_issues_locks(&lock_refs, || {
        let mut doc = IssueDoc::parse_file(&project, &path)?;
        let mut taken = doc.known_ids();
        taken.extend(opts.extra_ids.iter().cloned());
        for twin in opts.extra_id_paths {
            if twin == &path {
                continue;
            }
            if let Ok(doc) = IssueDoc::parse_file(&project, twin) {
                taken.extend(doc.known_ids());
            }
        }
        let id = if let Some(want) = opts.id {
            validate_explicit_id(&project, want)?;
            if taken.iter().any(|seen| seen == want) {
                return Err(anyhow!("--id {want} already exists").into());
            }
            want.to_string()
        } else {
            generate_id(&project, title, &taken, cfg.issues.id_length)?
        };

        let mut props = BTreeMap::new();
        props.insert("ID".into(), id.clone());
        props.insert("CREATED".into(), today_inactive_bracket());
        if crate::props::get(&props, crate::props::DISCOVERED_FROM).is_none()
            && let Some(body) = opts.body
            && let Some(origin) = first_existing_id_link(body, &known_ids)
        {
            crate::props::insert(&mut props, crate::props::DISCOVERED_FROM, origin);
        }
        let mut org_tags: Vec<String> = Vec::new();
        if let Some(t) = opts.issue_type {
            crate::props::insert(&mut props, crate::props::TYPE, t.into());
            // Type is an Org tag when the character class allows it, so
            // agenda tag search and C-c \ see `bug` / `feature` / `task`.
            if t.chars().all(crate::model::is_org_tag_char)
                && !t.is_empty()
                && !org_tags.iter().any(|seen| seen == t)
            {
                org_tags.push(t.to_string());
            }
        }
        if let Some(d) = opts.deadline {
            validate_org_date(d)?;
            props.insert("DEADLINE".into(), d.into());
        }
        if let Some(s) = opts.scheduled {
            validate_org_date(s)?;
            props.insert("SCHEDULED".into(), s.into());
        }
        // A tag Org can hold goes on the heading, where Org's own tag search
        // and agenda read it. One Org would not accept, `needs-review` say,
        // stays in the property so it survives instead of becoming title text.
        if let Some(tags) = opts.tags {
            let mut property_tags: Vec<String> = Vec::new();
            for tag in tags.split([',', ':']).map(str::trim) {
                if tag.is_empty() {
                    continue;
                }
                if tag.chars().all(crate::model::is_org_tag_char) {
                    if !org_tags.iter().any(|seen| seen == tag) {
                        org_tags.push(tag.to_string());
                    }
                } else if !property_tags.iter().any(|seen| seen == tag) {
                    property_tags.push(tag.to_string());
                }
            }
            if !property_tags.is_empty() {
                props.insert(crate::model::TAGS_PROPERTY.into(), property_tags.join(","));
            }
        }
        if let Some(p) = opts.parent {
            crate::props::insert(&mut props, crate::props::PARENT, p.into());
        }

        doc.headings.push(IssueHeading {
            id: id.clone(),
            title: title.to_string(),
            state: "TODO".into(),
            priority,
            properties: props,
            org_tags,
            statistics: None,
            property_order: Vec::new(),
            extra_drawers: Vec::new(),
            body: match opts.body {
                Some(b) if !b.trim().is_empty() => format!("{}\n", b.trim_end()),
                _ => String::new(),
            },
            logbook: Vec::new(),
            line_start: 0,
            line_end: 0,
        });
        doc.write()?;

        if opts.quiet {
            Ok(format!("{id}\n"))
        } else {
            Ok(format!(
                "{id}  TODO  [#{priority}]  {title}\nfile: {}\n",
                path.display()
            ))
        }
    })
}

/// An explicit create id is `{project}-` plus one or more `0-9a-z`.
///
/// # Errors
///
/// Returns an error when the id is not in that form.
pub(crate) fn validate_explicit_id(project: &str, id: &str) -> Result<()> {
    let prefix = format!("{project}-");
    let Some(suffix) = id.strip_prefix(&prefix) else {
        return Err(anyhow!("--id {id} is not {project}-<suffix>").into());
    };
    if suffix.is_empty()
        || !suffix
            .bytes()
            .all(|b| b.is_ascii_digit() || (b.is_ascii_lowercase() && b.is_ascii_alphanumeric()))
    {
        return Err(anyhow!("--id {id} suffix must be one or more 0-9a-z").into());
    }
    Ok(())
}

pub(crate) fn validate_org_date(s: &str) -> Result<()> {
    let inner = s
        .trim_start_matches(['<', '['])
        .trim_end_matches(['>', ']']);
    let token = inner.split_whitespace().next().unwrap_or("");
    NaiveDate::parse_from_str(token, "%Y-%m-%d").with_context(|| {
        format!("expected org date like <YYYY-MM-DD> or [YYYY-MM-DD], got {s:?}")
    })?;
    Ok(())
}

/// Change state, priority, or blocker edges. Adding a blocker to an open issue
/// moves it to BLOCKED; clearing the last blocker moves it back to TODO.
///
/// # Errors
///
/// Returns an error if `id` is not in the corpus, the state or priority is
/// invalid, adding the blocker would cycle, or the file cannot be rewritten.
pub fn update(
    layout: &Layout,
    id: &str,
    new_state: Option<&str>,
    new_priority: Option<char>,
    block_add: Option<&str>,
    block_clear: Option<&str>,
) -> Result<UpdateOutcome> {
    let identity = crate::config::identity(layout);
    update_as(
        layout,
        id,
        new_state,
        new_priority,
        block_add,
        block_clear,
        &identity,
    )
}

/// Last-seen state or generation a write must still match.
///
/// This is the causal context on a PUT: the caller read the heading, then
/// writes only if nothing else closed or rewrote it.
#[derive(Debug, Default, Clone, Copy)]
pub struct UpdatePred<'a> {
    /// Refuse unless the heading is still this state.
    pub if_state: Option<&'a str>,
    /// Refuse unless the corpus generation is still this value.
    pub if_gen: Option<u64>,
}

/// [`update`] with a last-seen predicate.
///
/// # Errors
///
/// Same as [`update`], plus [`Error::StaleWrite`] when the predicate fails.
pub fn update_pred(
    layout: &Layout,
    id: &str,
    new_state: Option<&str>,
    new_priority: Option<char>,
    block_add: Option<&str>,
    block_clear: Option<&str>,
    pred: UpdatePred<'_>,
) -> Result<UpdateOutcome> {
    let identity = crate::config::identity(layout);
    update_as_pred(
        layout,
        id,
        new_state,
        new_priority,
        block_add,
        block_clear,
        &identity,
        pred,
    )
}

/// [`update`] with an explicit identity instead of [`crate::config::identity`].
///
/// # Errors
///
/// Returns an error if `id` is not in the corpus, the state or priority is
/// invalid, adding the blocker would cycle, or the file cannot be rewritten.
pub fn update_as(
    layout: &Layout,
    id: &str,
    new_state: Option<&str>,
    new_priority: Option<char>,
    block_add: Option<&str>,
    block_clear: Option<&str>,
    identity: &str,
) -> Result<UpdateOutcome> {
    update_as_pred(
        layout,
        id,
        new_state,
        new_priority,
        block_add,
        block_clear,
        identity,
        UpdatePred::default(),
    )
}

/// [`update_as`] with a last-seen predicate.
///
/// # Errors
///
/// Same as [`update_as`], plus [`Error::StaleWrite`] when the predicate fails.
#[allow(clippy::too_many_arguments)]
pub fn update_as_pred(
    layout: &Layout,
    id: &str,
    new_state: Option<&str>,
    new_priority: Option<char>,
    block_add: Option<&str>,
    block_clear: Option<&str>,
    identity: &str,
    pred: UpdatePred<'_>,
) -> Result<UpdateOutcome> {
    let (_h0, path, project) =
        find_by_id(layout, id)?.ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;

    let (transition, changed) = with_issues_lock(&path, || {
        // Read the graph inside the lock. Built before it, the check answers
        // for a corpus a peer may already have moved on from.
        let graph = if block_add.is_some() {
            Some(DependencyGraph::from_issues(&load_all(layout)?)?)
        } else {
            None
        };
        let mut doc = IssueDoc::parse_file(&project, &path)?;
        let spec = doc.priority_spec();
        let keywords = doc.keywords.clone();
        let h = doc
            .headings
            .iter_mut()
            .find(|x| x.id == id)
            .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;

        let original = h.state.clone();
        let mut changed = Vec::new();

        if pred.if_state.is_some() || pred.if_gen.is_some() {
            let seen = crate::events::generation(layout);
            if let Some(want) = pred.if_state {
                if !keywords.knows(want) {
                    return Err(anyhow!(
                        "invalid --if-state {want:?}; allowed: {:?}",
                        keywords.all()
                    )
                    .into());
                }
                if h.state != want {
                    return Err(Error::StaleWrite {
                        id: id.to_string(),
                        expected_state: Some(want.to_string()),
                        actual_state: h.state.clone(),
                        expected_gen: pred.if_gen,
                        actual_gen: Some(seen),
                    });
                }
            }
            if let Some(want_gen) = pred.if_gen
                && seen != want_gen
            {
                return Err(Error::StaleWrite {
                    id: id.to_string(),
                    expected_state: pred.if_state.map(str::to_string),
                    actual_state: h.state.clone(),
                    expected_gen: Some(want_gen),
                    actual_gen: Some(seen),
                });
            }
        }

        if let Some(s) = new_state {
            // The file's own sequence is the law: a keyword its `#+TODO:`
            // declares is as legal as the house five, and the bar says
            // which side it closes on.
            if !keywords.knows(s) {
                return Err(anyhow!(
                    "invalid state {s:?}; allowed: {:?} (the file's #+TODO: line adds to these)",
                    keywords.all()
                )
                .into());
            }
            if h.state != s {
                if keywords.is_done(&h.state) && keywords.is_done(s) {
                    record_sibling_terminal(h, s);
                    changed.push(format!("sibling terminal {s} (held {})", h.state));
                } else {
                    let from = h.state.clone();
                    h.record_state_change(s);
                    changed.push(format!("state {from} -> {s}"));
                    for note in settle_claim(h, &from, s, identity) {
                        changed.push(note);
                    }
                }
            }
        }
        // Closing: Org stamps `CLOSED:` on the planning line when a task is
        // done and clears it when the task reopens (org-log-done). A
        // repeating task does not close; its dates move one interval on,
        // `:LAST_REPEAT:` records the time, and the state returns to the
        // file's first open keyword, or `:REPEAT_TO_STATE:` (manual 8.3.3).
        let now_done = keywords.is_done(&h.state);
        let was_done = keywords.is_done(&original);
        if now_done && !was_done {
            let today = chrono::Local::now().date_naive();
            let mut repeated = Vec::new();
            for key in ["SCHEDULED", "DEADLINE"] {
                if let Some(value) = h.properties.get(key).cloned()
                    && let Some(next) = crate::org::shift_repeating_timestamp(&value, today)
                {
                    crate::props::insert(&mut h.properties, key, next.clone());
                    repeated.push(format!("{key} -> {next}"));
                }
            }
            if repeated.is_empty() {
                crate::props::insert(&mut h.properties, "CLOSED", LogEntry::now());
                changed.push("CLOSED stamped".to_string());
            } else {
                crate::props::insert(&mut h.properties, "LAST_REPEAT", LogEntry::now());
                let back = h
                    .properties
                    .get("REPEAT_TO_STATE")
                    .map(|s| s.trim().to_string())
                    .filter(|s| keywords.knows(s) && !keywords.is_done(s))
                    .or_else(|| keywords.open.first().cloned())
                    .unwrap_or_else(|| "TODO".to_string());
                let closed_as = h.state.clone();
                h.record_state_change(&back);
                for note in settle_claim(h, &closed_as, &back, identity) {
                    changed.push(note);
                }
                changed.push(format!(
                    "repeats: {}; state {closed_as} -> {back}",
                    repeated.join(", ")
                ));
            }
        } else if was_done && !now_done && h.properties.contains_key("CLOSED") {
            crate::props::remove(&mut h.properties, "CLOSED");
            changed.push("CLOSED cleared".to_string());
        }

        if let Some(p) = new_priority {
            if !spec.contains(p) {
                return Err(anyhow!(
                    "invalid priority {p:?}; file allows [#{}]..[#{}]",
                    spec.highest,
                    spec.lowest
                )
                .into());
            }
            if h.priority != p {
                h.priority = p;
                changed.push(format!("priority -> [#{p}]"));
            }
        }

        if let Some(blk) = block_add {
            let mut current = h.blocked_by();
            if !current.iter().any(|x| x == blk) {
                if let Some(graph) = &graph {
                    graph.accepts_edge(blk, id)?;
                }
                current.push(blk.to_string());
                crate::props::insert(
                    &mut h.properties,
                    crate::props::BLOCKED_BY,
                    current.join(" "),
                );
                if h.state == "TODO" || h.state == "STARTED" {
                    let from = h.state.clone();
                    h.record_state_change("BLOCKED");
                    changed.push(format!("state {from} -> BLOCKED (auto on block)"));
                }
                changed.push(format!("blocked_by += {blk}"));
            }
        }

        if let Some(blk) = block_clear {
            let mut current = h.blocked_by();
            let before = current.len();
            current.retain(|x| x != blk);
            if current.len() < before {
                if current.is_empty() {
                    crate::props::remove(&mut h.properties, crate::props::BLOCKED_BY);
                    if h.state == "BLOCKED" {
                        let from = h.state.clone();
                        h.record_state_change("TODO");
                        changed.push("state BLOCKED -> TODO (auto on unblock)".to_string());
                        for note in settle_claim(h, &from, "TODO", identity) {
                            changed.push(note);
                        }
                    }
                } else {
                    crate::props::insert(
                        &mut h.properties,
                        crate::props::BLOCKED_BY,
                        current.join(" "),
                    );
                }
                changed.push(format!("blocked_by -= {blk}"));
            }
        }

        if changed.is_empty() {
            return Ok((None, Vec::new()));
        }

        let final_state = h.state.clone();
        doc.write()?;
        let transition = (original != final_state).then_some((original, final_state));
        Ok((transition, changed))
    })?;

    if changed.is_empty() {
        return Ok(UpdateOutcome {
            report: format!("{id}: no change\n"),
            hints: Vec::new(),
        });
    }

    if let Some((from, to)) = &transition {
        let _ = crate::events::emit_state_change(layout, &project, id, from, to);
    }

    let mut hints = Vec::new();
    if matches!(
        transition.as_ref().map(|(_, to)| to.as_str()),
        Some("DONE") | Some("CANCELLED")
    ) {
        for (other_project, other) in load_all(layout)? {
            if !other.blocked_by().iter().any(|b| b == id) {
                continue;
            }
            if other.state == "DONE" || other.state == "CANCELLED" {
                continue;
            }
            hints.push(format!(
                "{} (in {}) lists this as a blocker; clear with `vissue update {} --unblock {}`",
                other.id, other_project, other.id, id
            ));
        }
    }
    Ok(UpdateOutcome {
        report: format!("{id}: {}\n", changed.join(", ")),
        hints,
    })
}

/// States that keep a claim: someone still holds the issue even when it is
/// waiting on something else. Leaving for TODO, DONE, or CANCELLED gives it up.
fn keeps_claim(state: &str) -> bool {
    matches!(state, "STARTED" | "BLOCKED")
}

fn is_terminal(state: &str) -> bool {
    matches!(state, "DONE" | "CANCELLED")
}

fn record_sibling_terminal(h: &mut IssueHeading, attempted: &str) {
    crate::props::insert(
        &mut h.properties,
        crate::props::SIBLING_TERMINAL,
        attempted.to_string(),
    );
}

/// Pick one terminal after a sibling close. Clears `:SIBLING_TERMINAL:`.
///
/// # Errors
///
/// Returns an error if `id` is missing, `state` is not DONE or CANCELLED, or
/// the file cannot be rewritten.
pub fn resolve_terminal(layout: &Layout, id: &str, state: &str) -> Result<String> {
    if !is_terminal(state) {
        return Err(anyhow!("resolve state must be DONE or CANCELLED, got {state:?}").into());
    }
    let identity = crate::config::identity(layout);
    let (_h0, path, project) =
        find_by_id(layout, id)?.ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
    let from = with_issues_lock(&path, || {
        let mut doc = IssueDoc::parse_file(&project, &path)?;
        let h = doc
            .headings
            .iter_mut()
            .find(|x| x.id == id)
            .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
        let from = h.state.clone();
        if from != state {
            h.record_state_change(state);
            settle_claim(h, &from, state, &identity);
        }
        crate::props::remove(&mut h.properties, crate::props::SIBLING_TERMINAL);
        doc.write()?;
        Ok(from)
    })?;
    if from != state {
        let _ = crate::events::emit_state_change(layout, &project, id, &from, state);
    }
    Ok(format!("resolved {id} -> {state}\n"))
}

/// Take or give up the claim as the state moves.
///
/// Entering STARTED unclaimed stamps the identity; leaving for a state that
/// holds no claim releases it, and the logbook keeps who held it and since
/// when.
fn settle_claim(h: &mut IssueHeading, from: &str, to: &str, identity: &str) -> Vec<String> {
    let mut notes = Vec::new();
    if to == "STARTED" && h.claimed_by().is_none() {
        h.set_claim(identity);
        notes.push(format!("claimed by {identity}"));
    } else if keeps_claim(from)
        && !keeps_claim(to)
        && let Some((who, _when)) = h.release_claim()
    {
        notes.push(format!("claim released ({who})"));
    }
    notes
}

/// Take an issue: move it to STARTED and stamp the claim.
///
/// A claim held by another identity is refused unless `force`, which records
/// the takeover in the logbook rather than losing it.
///
/// # Errors
///
/// Returns an error if `id` is not in the corpus, the issue is DONE or
/// CANCELLED, another identity holds it and `force` is false, or the file
/// cannot be rewritten.
pub fn claim(layout: &Layout, id: &str, force: bool) -> Result<String> {
    let identity = crate::config::identity(layout);
    claim_as(layout, id, force, &identity)
}

/// [`claim`] with an explicit identity instead of [`crate::config::identity`].
///
/// # Errors
///
/// Returns an error if `id` is not in the corpus, the issue is DONE or
/// CANCELLED, another identity holds it and `force` is false, or the file
/// cannot be rewritten.
pub fn claim_as(layout: &Layout, id: &str, force: bool, identity: &str) -> Result<String> {
    let (_h0, path, project) =
        find_by_id(layout, id)?.ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;

    let report = with_issues_lock(&path, || {
        let mut doc = IssueDoc::parse_file(&project, &path)?;
        let h = doc
            .headings
            .iter_mut()
            .find(|x| x.id == id)
            .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;

        if h.state == "DONE" || h.state == "CANCELLED" {
            return Err(Error::InvalidState {
                id: id.to_string(),
                state: h.state.clone(),
            });
        }
        if let Some(holder) = h.claimed_by() {
            if holder != identity && !force {
                return Err(Error::ClaimConflict {
                    id: id.to_string(),
                    holder: holder.to_string(),
                    claimed_at: h.claimed_at().map(str::to_string),
                });
            }
            if holder != identity {
                let previous = holder.to_string();
                let from = h.state.clone();
                h.release_claim();
                h.set_claim(identity);
                h.record_state_change("STARTED");
                doc.write()?;
                if from != "STARTED" {
                    let _ =
                        crate::events::emit_state_change(layout, &project, id, &from, "STARTED");
                }
                return Ok(format!("claimed {id} (taken over from {previous})\n"));
            }
        }

        let was = h.state.clone();
        h.record_state_change("STARTED");
        if h.claimed_by().is_none() {
            h.set_claim(identity);
        }
        // Read off the heading before the write releases the borrow on it.
        let standing = standing_on(h);
        doc.write()?;
        if was != "STARTED" {
            let _ = crate::events::emit_state_change(layout, &project, id, &was, "STARTED");
        }
        let mut out = if was == "STARTED" {
            format!("claimed {id} by {identity}\n")
        } else {
            format!("claimed {id} by {identity} ({was} -> STARTED)\n")
        };
        out.push_str(&standing);
        Ok(out)
    })?;
    Ok(report)
}

/// The line a claim adds when the issue has declared inputs, from the heading
/// in hand; `recall` does the corpus walk.
fn standing_on(h: &IssueHeading) -> String {
    let blockers = h.blocked_by().len();
    let bounced = crate::props::get(&h.properties, crate::props::DISCOVERED_FROM).is_some();
    if blockers == 0 && !bounced && h.parent().is_none() {
        return String::new();
    }
    let mut parts: Vec<String> = Vec::new();
    if blockers > 0 {
        parts.push(format!(
            "{blockers} declared input{}",
            if blockers == 1 { "" } else { "s" }
        ));
    }
    if bounced {
        parts.push("an origin it was bounced from".to_string());
    }
    if h.parent().is_some() {
        parts.push("a plan above it".to_string());
    }
    format!("  `recall {}` for {}\n", h.id, parts.join(", "))
}

/// Fold a logbook note the same way [`note`] does: one line, single quotes.
fn fold_note_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('"', "'")
}

/// Drop every live claim held by `holder`. State stays STARTED or BLOCKED.
///
/// Each released heading gets the usual claim-released bookkeeping line plus a
/// note naming who ran the verb and why. `--older-than` keeps claims whose
/// claim stamp is still inside that many days. `dry_run` prints the
/// same report without writing.
///
/// # Errors
///
/// Returns an error if `holder` is empty or a file cannot be rewritten.
pub fn release_holder(
    layout: &Layout,
    holder: &str,
    older_than: Option<i64>,
    why: Option<&str>,
    dry_run: bool,
) -> Result<String> {
    let identity = crate::config::identity(layout);
    release_holder_as(layout, holder, older_than, why, dry_run, &identity)
}

/// [`release_holder`] with the releasing identity passed in.
///
/// # Errors
///
/// Returns an error if `holder` is empty or a file cannot be rewritten.
pub fn release_holder_as(
    layout: &Layout,
    holder: &str,
    older_than: Option<i64>,
    why: Option<&str>,
    dry_run: bool,
    identity: &str,
) -> Result<String> {
    if holder.trim().is_empty() {
        return Err(anyhow!("--holder given but empty").into());
    }
    let today = chrono::Local::now().date_naive();
    let why_text = {
        let folded = why.map(fold_note_text).unwrap_or_default();
        if folded.is_empty() {
            match older_than {
                Some(days) => {
                    format!("bulk release of holder {holder}: last activity older than {days}d")
                }
                None => format!("bulk release of holder {holder}"),
            }
        } else {
            folded
        }
    };
    let note_line = format!("released by {identity}: {why_text}");

    #[derive(Clone)]
    struct Target {
        id: String,
        project: String,
        last: Option<NaiveDate>,
        age: Option<i64>,
        state: String,
    }

    let mut targets: Vec<Target> = Vec::new();
    for (project, h) in load_all(layout)? {
        let Some(who) = h.claimed_by() else {
            continue;
        };
        if who != holder {
            continue;
        }
        if h.state != "STARTED" && h.state != "BLOCKED" {
            continue;
        }
        let age = h.last_activity_age_days(today);
        if let Some(limit) = older_than {
            match age {
                Some(d) if d > limit => {}
                _ => continue,
            }
        }
        targets.push(Target {
            id: h.id.clone(),
            project,
            last: h.last_activity_date(),
            age,
            state: h.state.clone(),
        });
    }
    targets.sort_by(|a, b| a.id.cmp(&b.id));

    let mut out = String::new();
    if targets.is_empty() {
        let _ = writeln!(out, "no live claims held by {holder}");
        return Ok(out);
    }
    let prefix = if dry_run {
        "dry-run: would release"
    } else {
        "released"
    };
    let _ = writeln!(
        out,
        "{prefix} {} claim{} held by {holder}",
        targets.len(),
        if targets.len() == 1 { "" } else { "s" }
    );
    for t in &targets {
        let age_txt = t
            .age
            .map(|d| format!("{d}d"))
            .unwrap_or_else(|| "?d".into());
        let last_txt = t
            .last
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "?".into());
        let _ = writeln!(
            out,
            "  {}  {}  last {last_txt}  {age_txt}  {} ({})",
            t.id, t.state, holder, t.project
        );
    }
    let _ = writeln!(out, "note: {note_line}");
    if dry_run {
        return Ok(out);
    }

    let mut by_path: BTreeMap<PathBuf, (String, Vec<String>)> = BTreeMap::new();
    for t in &targets {
        let path = layout.project_issues_path(&t.project);
        by_path
            .entry(path)
            .or_insert_with(|| (t.project.clone(), Vec::new()))
            .1
            .push(t.id.clone());
    }
    let paths: Vec<PathBuf> = by_path.keys().cloned().collect();
    let path_refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
    with_issues_locks(&path_refs, || {
        for (path, (project, ids)) in &by_path {
            let mut doc = IssueDoc::parse_file(project, path)?;
            for id in ids {
                let h = doc
                    .headings
                    .iter_mut()
                    .find(|x| x.id == *id)
                    .ok_or_else(|| Error::IssueNotFound { id: id.clone() })?;
                if h.claimed_by() != Some(holder) {
                    continue;
                }
                h.release_claim();
                h.logbook.insert(
                    0,
                    LogEntry {
                        timestamp: LogEntry::now(),
                        from_state: None,
                        to_state: None,
                        note: Some(note_line.clone()),
                        raw: None,
                    },
                );
            }
            doc.write()?;
        }
        Ok(())
    })?;
    Ok(out)
}

/// What an update changed, plus advice about issues left dangling by it.
#[derive(Debug, Clone)]
pub struct UpdateOutcome {
    /// One-line change summary, or `{id}: no change`.
    pub report: String,
    /// Issues that still list this one as a blocker after it closed.
    pub hints: Vec<String>,
}

/// Add a dated note to the top of an issue's logbook. State, claim, and
/// properties stay untouched, so an agent can record progress without owning
/// the issue.
///
/// # Errors
///
/// Returns an error if `text` is empty, `id` is not in the corpus, or the
/// file cannot be rewritten.
pub fn note(layout: &Layout, id: &str, text: &str) -> Result<String> {
    // One line in the drawer: fold internal whitespace, and swap double
    // quotes for singles so the rendered `- Note: "..."` line re-parses.
    let text = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('"', "'");
    if text.is_empty() {
        return Err(anyhow!("note text is empty").into());
    }
    let (_h0, path, project) =
        find_by_id(layout, id)?.ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
    with_issues_lock(&path, || {
        let mut doc = IssueDoc::parse_file(&project, &path)?;
        let h = doc
            .headings
            .iter_mut()
            .find(|x| x.id == id)
            .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
        // Newest first, matching state transitions and claim releases. A
        // drawer written from both ends reads as sorted by neither.
        h.logbook.insert(
            0,
            LogEntry {
                timestamp: LogEntry::now(),
                from_state: None,
                to_state: None,
                note: Some(text.clone()),
                raw: None,
            },
        );
        doc.write()?;
        Ok(format!("{id}: noted\n"))
    })
}

/// Append prose to an issue's body, stamped with the date and identity. Lines
/// that would end the issue are indented, so markdown is safe to append.
///
/// # Errors
///
/// Returns an error if `text` is empty, `id` is not in the corpus, or the
/// file cannot be rewritten.
pub fn append_body(layout: &Layout, id: &str, text: &str) -> Result<String> {
    append_body_as(layout, id, text, &crate::config::identity(layout))
}

/// [`append_body`] with the recorded identity passed in.
///
/// # Errors
///
/// Returns an error if `text` is empty, `id` is not in the corpus, or the
/// file cannot be rewritten.
pub fn append_body_as(layout: &Layout, id: &str, text: &str, identity: &str) -> Result<String> {
    let text = text.trim_end();
    if text.trim().is_empty() {
        return Err(anyhow!("append text is empty").into());
    }
    let (_h0, path, project) =
        find_by_id(layout, id)?.ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
    with_issues_lock(&path, || {
        let mut doc = IssueDoc::parse_file(&project, &path)?;
        let h = doc
            .headings
            .iter_mut()
            .find(|x| x.id == id)
            .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
        let stamp = format!("{} {identity}", today_inactive_bracket());
        if !h.body.trim().is_empty() {
            h.body = h.body.trim_end().to_string();
            h.body.push_str("\n\n");
        } else {
            h.body.clear();
        }
        h.body.push_str(&stamp);
        h.body.push('\n');
        h.body.push_str(text);
        h.body.push('\n');
        doc.write()?;
        let lines = text.lines().count();
        Ok(format!("{id}: appended {lines} line(s)\n"))
    })
}

/// Name of the drawer votes live in.
const VOTES_DRAWER: &str = "VOTES";

/// One agent's ballot on one issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ballot {
    /// Identity that cast it, as [`crate::config::identity`] reports.
    pub agent: String,
    /// What was voted for, verbatim.
    pub choice: String,
    /// Inactive org date the vote was cast or last changed.
    pub stamp: String,
    /// Deed accessions the ballot drew on, or `none`.
    pub used: Option<String>,
    /// Stated probability that `choice` is the outcome, as written.
    pub confidence: Option<String>,
}

/// Cast or change one agent's vote, or read the tally when `choice` is `None`.
/// One ballot per identity, a recast replaces it, and the read-modify-write
/// runs under the file lock. Stored as a `:VOTES:` drawer on the heading.
///
/// # Errors
///
/// Returns an error if `id` is not in the corpus, `choice` is blank, or the file
/// cannot be rewritten.
pub fn vote(layout: &Layout, id: &str, choice: Option<&str>, identity: &str) -> Result<String> {
    vote_with(layout, id, choice, identity, None, None)
}

/// [`vote`], plus the deeds the ballot used and a stated probability.
///
/// `used` is `none` or accession ids. `confidence` is a decimal in `(0, 1]`.
/// Both are kept on the ballot line and are not part of the choice.
///
/// # Errors
///
/// Returns an error when `used` or `confidence` is not a single token, or
/// `confidence` is outside `(0, 1]`. The errors from [`vote`] apply as well.
pub fn vote_with(
    layout: &Layout,
    id: &str,
    choice: Option<&str>,
    identity: &str,
    used: Option<&str>,
    confidence: Option<&str>,
) -> Result<String> {
    let used = clean_used(used)?;
    let confidence = clean_confidence(confidence)?;
    let (_h, path, project) =
        find_by_id(layout, id)?.ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
    let Some(choice) = choice else {
        let doc = IssueDoc::parse_file(&project, &path)?;
        let h = doc
            .headings
            .iter()
            .find(|x| x.id == id)
            .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
        let (ballots, _) = read_ballots(h);
        return Ok(tally_text(id, &ballots));
    };
    let choice = choice.trim();
    if choice.is_empty() {
        return Err(anyhow!("vote needs something to vote for").into());
    }
    if choice.contains('\n') {
        return Err(anyhow!("a vote is one line").into());
    }
    // A ballot line is `[date] agent: choice`, split at the first ": ", so an
    // identity holding ": " would be misfiled; refused instead.
    if identity.contains(": ") {
        return Err(anyhow!(
            "the identity {identity:?} contains a colon and a space, which a ballot line \
             cannot hold unambiguously; set VISSUE_AGENT or `agent` in the config to a \
             name without one"
        )
        .into());
    }
    if identity.trim().is_empty() {
        return Err(anyhow!("a ballot needs an identity to file it under").into());
    }
    with_issues_lock(&path, || {
        let mut doc = IssueDoc::parse_file(&project, &path)?;
        let h = doc
            .headings
            .iter_mut()
            .find(|x| x.id == id)
            .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
        let (mut ballots, foreign) = read_ballots(h);
        let stamp = today_inactive_bracket();
        let previous = ballots.iter().position(|b| b.agent == identity);
        let changed_from = previous.map(|i| ballots[i].choice.clone());
        let ballot = Ballot {
            agent: identity.to_string(),
            choice: choice.to_string(),
            stamp,
            used,
            confidence,
        };
        match previous {
            Some(i) => ballots[i] = ballot,
            None => ballots.push(ballot),
        }
        write_ballots(h, &ballots, &foreign);
        doc.write()?;
        let mut out = match changed_from {
            Some(old) if old == choice => format!("{id}: {identity} already voted {choice}\n"),
            Some(old) => format!("{id}: {identity} changed {old} to {choice}\n"),
            None => format!("{id}: {identity} voted {choice}\n"),
        };
        out.push_str(&tally_text(id, &ballots));
        Ok(out)
    })
}

/// The ballots cast on one issue, in drawer order; [`crate::consensus`] weighs them.
///
/// # Errors
///
/// Returns an error if `id` is not in the corpus or the file cannot be read.
pub fn ballots(layout: &Layout, id: &str) -> Result<Vec<Ballot>> {
    let (h, _path, _project) =
        find_by_id(layout, id)?.ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
    Ok(read_ballots(&h).0)
}

/// Ballots on a heading, plus the drawer lines this does not parse, which a
/// rewrite carries rather than drops.
fn read_ballots(h: &IssueHeading) -> (Vec<Ballot>, Vec<String>) {
    let Some(drawer) = h
        .extra_drawers
        .iter()
        .find(|d| drawer_name_is(d, VOTES_DRAWER))
    else {
        return (Vec::new(), Vec::new());
    };
    let mut ballots: Vec<Ballot> = Vec::new();
    let mut foreign: Vec<String> = Vec::new();
    for line in drawer.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // The drawer's own delimiters are structure rather than content.
        if trimmed.eq_ignore_ascii_case(&format!(":{VOTES_DRAWER}:"))
            || trimmed.eq_ignore_ascii_case(":END:")
        {
            continue;
        }
        match parse_ballot(trimmed) {
            // One ballot per agent: a hand-edited duplicate collapses on read,
            // last line winning.
            Some(b) => match ballots.iter_mut().find(|x| x.agent == b.agent) {
                Some(existing) => *existing = b,
                None => ballots.push(b),
            },
            None => foreign.push(trimmed.to_string()),
        }
    }
    (ballots, foreign)
}

/// `[date] agent: choice`. The choice may hold ": ", so the first one delimits
/// and the agent may not contain it; [`vote`] refuses an identity that does.
fn parse_ballot(line: &str) -> Option<Ballot> {
    let (stamp, rest) = line.strip_prefix('[')?.split_once("] ")?;
    let (agent, choice) = rest.split_once(": ")?;
    let agent = agent.trim();
    let choice = choice.trim();
    if agent.is_empty() || choice.is_empty() {
        return None;
    }
    let (choice, used, confidence) = split_ballot_tail(choice);
    Some(Ballot {
        agent: agent.to_string(),
        choice: choice.to_string(),
        stamp: format!("[{stamp}]"),
        used,
        confidence,
    })
}

/// `used` is one token: `none` or accession ids. A blank is the same as absent.
fn clean_used(used: Option<&str>) -> Result<Option<String>> {
    let Some(used) = used.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if used.contains(char::is_whitespace) {
        return Err(anyhow!("--used is one token, `none` or accession ids").into());
    }
    Ok(Some(used.to_string()))
}

/// A stated probability in `(0, 1]`, kept as written after the range check.
fn clean_confidence(confidence: Option<&str>) -> Result<Option<String>> {
    let Some(confidence) = confidence.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let p: f64 = confidence
        .parse()
        .map_err(|_| anyhow!("--confidence must be a number in (0, 1]"))?;
    if !p.is_finite() || p <= 0.0 || p > 1.0 {
        return Err(anyhow!("--confidence must be a number in (0, 1]").into());
    }
    Ok(Some(confidence.to_string()))
}

/// Split `ship used=none confidence=0.5` into the choice and the two tails.
fn split_ballot_tail(choice: &str) -> (&str, Option<String>, Option<String>) {
    if let Some((choice, tail)) = choice.split_once(" used=") {
        let (used, confidence) = match tail.split_once(" confidence=") {
            Some((used, confidence)) => (used, Some(confidence.to_string())),
            None => (tail, None),
        };
        let used = if used.is_empty() {
            None
        } else {
            Some(used.to_string())
        };
        return (choice.trim(), used, confidence);
    }
    if let Some((choice, confidence)) = choice.split_once(" confidence=") {
        return (choice.trim(), None, Some(confidence.to_string()));
    }
    (choice, None, None)
}

fn drawer_name_is(drawer: &str, name: &str) -> bool {
    drawer
        .lines()
        .next()
        .map(str::trim)
        .and_then(|first| first.strip_prefix(':'))
        .and_then(|rest| rest.strip_suffix(':'))
        .is_some_and(|n| n.eq_ignore_ascii_case(name))
}

/// Replace the heading's votes drawer in place, so other drawers keep their
/// order; dropped when it would be empty.
fn write_ballots(h: &mut IssueHeading, ballots: &[Ballot], foreign: &[String]) {
    let at = h
        .extra_drawers
        .iter()
        .position(|d| drawer_name_is(d, VOTES_DRAWER));
    if ballots.is_empty() && foreign.is_empty() {
        if let Some(i) = at {
            h.extra_drawers.remove(i);
        }
        return;
    }
    let mut drawer = format!(":{VOTES_DRAWER}:\n");
    for b in ballots {
        drawer.push_str(&format!("{} {}: {}", b.stamp, b.agent, b.choice));
        if let Some(used) = &b.used {
            drawer.push_str(&format!(" used={used}"));
        }
        if let Some(confidence) = &b.confidence {
            drawer.push_str(&format!(" confidence={confidence}"));
        }
        drawer.push('\n');
    }
    for line in foreign {
        drawer.push_str(line);
        drawer.push('\n');
    }
    drawer.push_str(":END:\n");
    match at {
        Some(i) => h.extra_drawers[i] = drawer,
        None => h.extra_drawers.push(drawer),
    }
}

/// The tally, and whether it is a consensus; a plurality is reported as one.
fn tally_text(id: &str, ballots: &[Ballot]) -> String {
    if ballots.is_empty() {
        return format!("{id}: no votes\n");
    }
    let mut counts: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for b in ballots {
        counts
            .entry(b.choice.as_str())
            .or_default()
            .push(b.agent.as_str());
    }
    let total = ballots.len();
    let mut rows: Vec<(&&str, &Vec<&str>)> = counts.iter().collect();
    rows.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));
    let mut out = format!(
        "{id}: {total} vote{} from {} option{}\n",
        if total == 1 { "" } else { "s" },
        counts.len(),
        if counts.len() == 1 { "" } else { "s" }
    );
    for (choice, who) in &rows {
        let _ = writeln!(out, "  {:<24} {} ({})", choice, who.len(), who.join(", "));
    }
    let top = rows[0].1.len();
    let tied = rows.iter().filter(|(_, who)| who.len() == top).count();
    if tied > 1 {
        let _ = writeln!(out, "  no consensus: {tied} options tied at {top}");
    } else if total < 2 {
        // One agent agreeing with itself is not a consensus, and calling it one
        // is how a single unreviewed opinion gets acted on as though it had been
        // checked. This is the whole failure the tally exists to prevent.
        let _ = writeln!(
            out,
            "  one ballot only: {}, which nobody has agreed with yet",
            rows[0].0
        );
    } else if top * 2 > total {
        let _ = writeln!(out, "  consensus: {} ({top} of {total})", rows[0].0);
    } else {
        let _ = writeln!(
            out,
            "  plurality only: {} ({top} of {total}), which is not a majority",
            rows[0].0
        );
    }
    out
}

/// Prefixes a deed accession can open with: `deed-<kind>-<slug>` or a `sha256:`.
const DEED_PREFIXES: &[&str] = &["deed-", "sha256:"];

/// Whether `value` has the shape of a deed accession; the store is not asked.
#[must_use]
pub fn is_deed_accession(value: &str) -> bool {
    let value = value.trim();
    if value.contains(|c: char| c.is_whitespace() || c == ',') {
        return false;
    }
    DEED_PREFIXES.iter().any(|prefix| {
        value
            .strip_prefix(*prefix)
            .is_some_and(|rest| !rest.is_empty())
    })
}

/// Cite, drop, or list the deeds an issue's work produced. The tracker stores
/// the accession only; the deed store owns the bytes. With neither `add` nor
/// `remove`, this reads the citations in the order they were cited.
///
/// # Errors
///
/// Returns an error if `id` is not in the corpus, an added value is not a deed
/// accession, or the file cannot be rewritten.
pub fn deed(layout: &Layout, id: &str, add: &[String], remove: &[String]) -> Result<String> {
    let (h, path, project) =
        find_by_id(layout, id)?.ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
    if add.is_empty() && remove.is_empty() {
        return Ok(deed_list_text(id, &h.deeds()));
    }
    for value in add {
        if !is_deed_accession(value) {
            return Err(anyhow!(
                "{value:?} is not a deed accession; deedar mints `deed-<kind>-<slug>` \
                 and answers `get` for a `sha256:` of the deed or of one product path"
            )
            .into());
        }
    }
    with_issues_lock(&path, || {
        let mut doc = IssueDoc::parse_file(&project, &path)?;
        let h = doc
            .headings
            .iter_mut()
            .find(|x| x.id == id)
            .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
        let mut cited = h.deeds();
        let mut changed: Vec<String> = Vec::new();
        for value in add {
            let value = value.trim();
            // Citing twice is what a retried step does, and a second copy of the
            // id would make `trail` walk the same deed twice for no reason.
            if cited.iter().any(|x| x == value) {
                continue;
            }
            cited.push(value.to_string());
            changed.push(format!("deeds += {value}"));
        }
        for value in remove {
            let value = value.trim();
            let before = cited.len();
            cited.retain(|x| x != value);
            if cited.len() != before {
                changed.push(format!("deeds -= {value}"));
            }
        }
        if changed.is_empty() {
            return Ok(format!("{id}: no change\n{}", deed_list_text(id, &cited)));
        }
        if cited.is_empty() {
            crate::props::remove(&mut h.properties, crate::props::DEEDS);
        } else {
            crate::props::insert(&mut h.properties, crate::props::DEEDS, cited.join(" "));
        }
        doc.write()?;
        Ok(format!(
            "{id}: {}\n{}",
            changed.join(", "),
            deed_list_text(id, &cited)
        ))
    })
}

/// The citations on one heading, one per line.
fn deed_list_text(id: &str, cited: &[String]) -> String {
    if cited.is_empty() {
        return format!("{id}: no deeds cited\n");
    }
    let mut out = format!(
        "{id}: {} deed{}\n",
        cited.len(),
        if cited.len() == 1 { "" } else { "s" }
    );
    for value in cited {
        let _ = writeln!(out, "  {value}");
    }
    out
}

/// Fold an inbox-convention org file into tracked issues.
///
/// Each top-level `* TODO <title>` heading that does not already carry a
/// `:VISSUE_ID:` line becomes an issue in `project` (body = the heading's
/// text up to the next heading). The heading is then flipped to DONE and
/// stamped with the assigned id in place, so a second run is a no-op:
/// stamped headings are skipped, and folding is idempotent.
///
/// # Errors
///
/// Returns an error if the inbox cannot be read or written, `project` cannot
/// be resolved, or creating a folded issue fails. Headings already stamped
/// before a failure stay stamped.
pub fn fold(layout: &Layout, inbox: &std::path::Path, project: &str) -> Result<String> {
    let project = resolve_existing_project_case(layout, project)?;
    let text = std::fs::read_to_string(inbox)
        .with_context(|| format!("read inbox {}", inbox.display()))?;
    let lines: Vec<String> = text.lines().map(str::to_string).collect();

    struct Entry {
        line: usize,
        title: String,
        body: String,
        stamped: bool,
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut i = 0;
    let mut nest = crate::org::OrgScan::new();
    while i < lines.len() {
        if nest.observe(&lines[i]) {
            i += 1;
            continue;
        }
        if let Some(title) = lines[i].strip_prefix("* TODO ") {
            let start = i + 1;
            let mut end_nest = crate::org::OrgScan::new();
            let end = {
                let mut j = start;
                while j < lines.len() {
                    if !end_nest.observe(&lines[j]) && lines[j].starts_with("* ") {
                        break;
                    }
                    j += 1;
                }
                j
            };
            let stamped = lines[start..end]
                .iter()
                .any(|l| l.trim_start().starts_with(":VISSUE_ID:"));
            let body = lines[start..end].join("\n").trim().to_string();
            entries.push(Entry {
                line: i,
                title: title.trim().to_string(),
                body,
                stamped,
            });
            i = end;
        } else {
            i += 1;
        }
    }

    // Stamping inserts lines, so rewrite from the bottom up to keep the
    // recorded line numbers valid.
    let mut out = lines.clone();
    let mut created: Vec<String> = Vec::new();
    let mut failure = None;
    for e in entries.iter().rev() {
        if e.stamped {
            continue;
        }
        let printed = create(
            layout,
            &project,
            &e.title,
            CreateOpts {
                quiet: true,
                body: if e.body.is_empty() {
                    None
                } else {
                    Some(&e.body)
                },
                ..CreateOpts::default()
            },
        );
        let id = match printed {
            Ok(printed) => printed.trim().to_string(),
            Err(e) => {
                // Stop, but stamp what already exists below. Returning here
                // with the inbox untouched would leave every issue created so
                // far unstamped, and the next run would create them again.
                failure = Some(e);
                break;
            }
        };
        out[e.line] = format!("* DONE {}", e.title);
        out.insert(e.line + 1, format!(":VISSUE_ID: {id}"));
        created.push(id);
    }
    created.reverse();

    if !created.is_empty() {
        let mut rendered = out.join("\n");
        if text.ends_with('\n') {
            rendered.push('\n');
        }
        std::fs::write(inbox, rendered)
            .with_context(|| format!("write inbox {}", inbox.display()))?;
    }
    if let Some(error) = failure {
        return Err(crate::error::Error::Other(
            anyhow::Error::from(error).context(format!(
                "folded {} before failing: {}",
                created.len(),
                created.join(" ")
            )),
        ));
    }
    if created.is_empty() {
        return Ok("folded 0 (nothing unstamped)\n".into());
    }
    Ok(format!("folded {}: {}\n", created.len(), created.join(" ")))
}

/// Move one issue's heading to another project's file. The id is not
/// regenerated, so cross-project blocker edges keep resolving.
///
/// # Errors
///
/// Returns an error if `id` is not in the corpus, `to_project` cannot be
/// resolved, or either file cannot be locked or rewritten.
pub fn refile(layout: &Layout, id: &str, to_project: &str) -> Result<String> {
    refile_to(layout, id, layout, to_project)
}

/// [`refile`] onto a destination the router resolved, which may be another
/// tracker layout.
///
/// # Errors
///
/// Same as [`refile`].
pub fn refile_to(
    layout: &Layout,
    id: &str,
    dst_layout: &Layout,
    to_project: &str,
) -> Result<String> {
    let to_project = resolve_existing_project_case(dst_layout, to_project)?;
    let target_path = dst_layout.project_issues_path(&to_project);
    let (_heading, src_path, src_project) =
        find_by_id(layout, id)?.ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
    if src_path == target_path {
        return Ok(format!("{id} already in {to_project}; nothing to do\n"));
    }
    with_issues_locks(&[&src_path, &target_path], || {
        let mut src_doc = IssueDoc::parse_file(&src_project, &src_path)?;
        let heading = src_doc
            .remove(id)
            .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;

        // Target first: a failure then duplicates the id, which `check`
        // reports, rather than deleting the issue.
        let mut tgt_doc = IssueDoc::parse_file(&to_project, &target_path)?;
        tgt_doc.upsert(heading);
        tgt_doc.write()?;
        src_doc.write()?;
        Ok(())
    })?;
    Ok(format!("{id}: {src_project} -> {to_project}\n"))
}

/// Optional fields on [`reject`].
#[derive(Debug, Default, Clone, Copy)]
pub struct RejectOpts<'a> {
    /// Existing destination id. When set, that heading is the successor.
    pub to: Option<&'a str>,
    /// Project to create the destination in when [`Self::to`] is absent.
    pub project: Option<&'a str>,
    /// Title of a created destination. The source title is used when omitted.
    pub title: Option<&'a str>,
    /// Prose appended to the cancelled source.
    pub reason: Option<&'a str>,
    /// Tracker that holds the destination. `None` keeps the source's.
    pub dst_layout: Option<&'a Layout>,
    /// Twin files read under the lock when minting a successor, so a twin on
    /// another layout cannot share a suffix with it. Paths and not ids, because
    /// ids the caller read before the lock can be stale by the time it is held.
    pub dst_extra_id_paths: &'a [PathBuf],
}

/// Cancel `src` and point it at a successor in one graph edit.
///
/// Writes `src` to CANCELLED, sets `:PIVOTED_TO:` to the destination, and
/// settles any claim on `src`. A created destination, or an existing one
/// whose `:DISCOVERED_FROM:` is empty, records `src` as its origin. A
/// non-empty `:DISCOVERED_FROM:` is left alone.
///
/// # Errors
///
/// Returns an error if `src` is not in the corpus, `--to` names no heading,
/// neither a destination nor a create project is given, or a file cannot be
/// rewritten.
pub fn reject(layout: &Layout, src: &str, opts: RejectOpts<'_>) -> Result<String> {
    let identity = crate::config::identity(layout);
    let (src0, src_path, src_project) =
        find_by_id(layout, src)?.ok_or_else(|| Error::IssueNotFound {
            id: src.to_string(),
        })?;

    let dst_layout = opts.dst_layout.unwrap_or(layout);
    let existing_dst = if let Some(to) = opts.to {
        if to == src {
            return Err(anyhow!("reject destination cannot be the source {src}").into());
        }
        Some(
            find_by_id(dst_layout, to)?
                .ok_or_else(|| Error::IssueNotFound { id: to.to_string() })?,
        )
    } else {
        None
    };

    let creating = existing_dst.is_none();
    if creating && opts.project.is_none() {
        return Err(anyhow!("reject needs --to DST or --project to create a successor").into());
    }

    let dst_project = if let Some((_, _, ref project)) = existing_dst {
        project.clone()
    } else {
        resolve_existing_project_case(dst_layout, opts.project.unwrap_or(&src_project))?
    };
    let dst_path = dst_layout.project_issues_path(&dst_project);
    let dst_title = opts.title.unwrap_or(src0.title.as_str());
    let cfg = VissueConfig::load(layout)?;

    // The twins the mint consults are locked too, or the reservation is read
    // outside the lock that guards the write and a peer can mint the same id.
    let mut lock_paths: Vec<PathBuf> = vec![src_path.clone(), dst_path.clone()];
    lock_paths.extend(opts.dst_extra_id_paths.iter().cloned());
    let lock_refs: Vec<&Path> = lock_paths.iter().map(PathBuf::as_path).collect();
    let (dst_id, old_state, new_state) = with_issues_locks(&lock_refs, || {
        if src_path == dst_path {
            let mut doc = IssueDoc::parse_file(&src_project, &src_path)?;
            let dst_id = if creating {
                push_successor(
                    &mut doc,
                    &dst_project,
                    dst_title,
                    src,
                    &cfg,
                    opts.dst_extra_id_paths,
                )?
            } else {
                let to = reject_to(opts)?;
                set_discovered_from_if_empty(&mut doc, to, src)?;
                to.to_string()
            };
            let (old_state, new_state) =
                cancel_and_pivot(&mut doc, src, &dst_id, opts.reason, &identity)?;
            doc.write()?;
            Ok((dst_id, old_state, new_state))
        } else {
            let mut src_doc = IssueDoc::parse_file(&src_project, &src_path)?;
            let mut dst_doc = IssueDoc::parse_file(&dst_project, &dst_path)?;
            let dst_id = if creating {
                push_successor(
                    &mut dst_doc,
                    &dst_project,
                    dst_title,
                    src,
                    &cfg,
                    opts.dst_extra_id_paths,
                )?
            } else {
                let to = reject_to(opts)?;
                set_discovered_from_if_empty(&mut dst_doc, to, src)?;
                to.to_string()
            };
            let (old_state, new_state) =
                cancel_and_pivot(&mut src_doc, src, &dst_id, opts.reason, &identity)?;
            dst_doc.write()?;
            src_doc.write()?;
            Ok((dst_id, old_state, new_state))
        }
    })?;

    if old_state != new_state {
        let _ = crate::events::emit_state_change(layout, &src_project, src, &old_state, &new_state);
    }
    Ok(format!("rejected {src} -> {dst_id}\n"))
}

fn reject_to(opts: RejectOpts<'_>) -> Result<&str> {
    opts.to
        .ok_or_else(|| anyhow!("reject destination missing after --to was required").into())
}

fn push_successor(
    doc: &mut IssueDoc,
    project: &str,
    title: &str,
    src: &str,
    cfg: &VissueConfig,
    extra_id_paths: &[PathBuf],
) -> Result<String> {
    let mut taken = doc.known_ids();
    // Read here rather than by the caller, because here is inside the lock set.
    for twin in extra_id_paths {
        if twin == &doc.path {
            continue;
        }
        if let Ok(other) = IssueDoc::parse_file(project, twin) {
            taken.extend(other.known_ids());
        }
    }
    let id = generate_id(project, title, &taken, cfg.issues.id_length)?;
    let mut props = BTreeMap::new();
    props.insert("ID".into(), id.clone());
    props.insert("CREATED".into(), today_inactive_bracket());
    crate::props::insert(&mut props, crate::props::DISCOVERED_FROM, src.to_string());
    doc.headings.push(IssueHeading {
        id: id.clone(),
        title: title.to_string(),
        state: "TODO".into(),
        priority: doc.default_create_priority(cfg.issues.default_priority),
        properties: props,
        org_tags: Vec::new(),
        statistics: None,
        property_order: Vec::new(),
        extra_drawers: Vec::new(),
        body: String::new(),
        logbook: Vec::new(),
        line_start: 0,
        line_end: 0,
    });
    Ok(id)
}

fn set_discovered_from_if_empty(doc: &mut IssueDoc, id: &str, src: &str) -> Result<()> {
    let h = doc
        .headings
        .iter_mut()
        .find(|h| h.id == id)
        .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
    let empty = crate::props::get(&h.properties, crate::props::DISCOVERED_FROM)
        .is_none_or(|s| s.trim().is_empty());
    if empty {
        crate::props::insert(
            &mut h.properties,
            crate::props::DISCOVERED_FROM,
            src.to_string(),
        );
    }
    Ok(())
}

fn cancel_and_pivot(
    doc: &mut IssueDoc,
    src: &str,
    dst: &str,
    reason: Option<&str>,
    identity: &str,
) -> Result<(String, String)> {
    let h = doc
        .headings
        .iter_mut()
        .find(|h| h.id == src)
        .ok_or_else(|| Error::IssueNotFound {
            id: src.to_string(),
        })?;
    let old_state = h.state.clone();
    if is_terminal(&old_state) && old_state != "CANCELLED" {
        record_sibling_terminal(h, "CANCELLED");
    } else if old_state != "CANCELLED" {
        h.record_state_change("CANCELLED");
        settle_claim(h, &old_state, "CANCELLED", identity);
    }
    crate::props::insert(&mut h.properties, crate::props::PIVOTED_TO, dst.to_string());
    if let Some(reason) = reason {
        append_reason(h, reason, identity);
    }
    Ok((old_state, h.state.clone()))
}

fn append_reason(h: &mut IssueHeading, text: &str, identity: &str) {
    let text = text.trim_end();
    if text.trim().is_empty() {
        return;
    }
    let stamp = format!("{} {identity}", today_inactive_bracket());
    if !h.body.trim().is_empty() {
        h.body = h.body.trim_end().to_string();
        h.body.push_str("\n\n");
    } else {
        h.body.clear();
    }
    h.body.push_str(&stamp);
    h.body.push('\n');
    h.body.push_str(text);
    h.body.push('\n');
}

/// First `[[id:XXX]]` (optionally `[[id:XXX][label]]`) whose id is in `known`.
fn first_existing_id_link(body: &str, known: &std::collections::HashSet<String>) -> Option<String> {
    let mut rest = body;
    while let Some(start) = rest.find("[[") {
        let after_start = &rest[start + 2..];
        let end = after_start.find("]]")?;
        let raw = &after_start[..end];
        let target = raw.split_once("][").map_or(raw, |(target, _)| target);
        let target = target.trim();
        if let Some(id) = target.strip_prefix("id:") {
            let id = id.trim();
            if known.contains(id) {
                return Some(id.to_string());
            }
        }
        rest = &after_start[end + 2..];
    }
    None
}

/// Rewrite project files onto the Org / ELPA / vissue property split.
///
/// Folds typos (`BLOCKEDBY`, drawer `TAGS`) and a bare `:BLOCKER:` id
/// list into `:BLOCKED_BY:`. A real org-edna condition stays. Puts legal
/// types on the heading and inserts a missing `#+CATEGORY:`. Does not
/// mint `:BLOCKER: ids(...)`.
///
/// # Errors
///
/// Returns an error if a project file cannot be read or rewritten.
pub fn normalize(layout: &Layout, project: Option<&str>, dry_run: bool) -> Result<String> {
    let projects = match project {
        Some(name) => vec![resolve_existing_project_case(layout, name)?],
        None => crate::store::list_projects(layout)?,
    };
    let mut out = String::new();
    let mut files = 0usize;
    let mut headings = 0usize;
    let mut changed = 0usize;
    for project in projects {
        let path = layout.project_issues_path(&project);
        if !path.exists() {
            continue;
        }
        files += 1;
        let before =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let report = with_issues_lock(&path, || {
            let mut doc = IssueDoc::parse_file(&project, &path)?;
            let mut moved = 0usize;
            for h in &mut doc.headings {
                moved += crate::props::settle(&mut h.org_tags, &mut h.properties);
            }
            let after = doc.render_string();
            if after != before {
                if !dry_run {
                    doc.write()?;
                }
                Ok(Some((moved, after.len())))
            } else {
                Ok(None)
            }
        })?;
        headings += IssueDoc::parse(&project, path.clone(), &before)
            .map(|d| d.headings.len())
            .unwrap_or(0);
        if let Some((moved, _)) = report {
            changed += 1;
            let verb = if dry_run { "would rewrite" } else { "rewrote" };
            writeln!(out, "{verb} {project} ({moved} key move(s))")?;
        }
    }
    let mode = if dry_run { "dry-run" } else { "wrote" };
    writeln!(
        out,
        "normalize {mode}: {changed}/{files} file(s) changed, {headings} heading(s) scanned"
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_PREFIX;
    use std::fs;
    use std::path::Path;

    fn fresh_layout(dir: &Path) -> Layout {
        fs::create_dir_all(dir.join(DEFAULT_PREFIX)).unwrap();
        Layout::new(dir, DEFAULT_PREFIX)
    }

    fn issue_at(layout: &Layout, project: &str, id: &str) -> IssueHeading {
        IssueDoc::parse_file(project, &layout.project_issues_path(project))
            .unwrap()
            .headings
            .into_iter()
            .find(|h| h.id == id)
            .expect("issue not found")
    }

    fn only_id(layout: &Layout, project: &str) -> String {
        IssueDoc::parse_file(project, &layout.project_issues_path(project))
            .unwrap()
            .headings[0]
            .id
            .clone()
    }

    #[test]
    fn a_projected_board_refuses_a_create_and_names_its_inbox() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        fs::write(
            dir.path().join("vissue.toml"),
            "[[projection.board]]\nproject = \"surf\"\nmirror = \"m/surf.org\"\n\n[[projection.board]]\nproject = \"ljos\"\nsource = \"vault\"\nmirror = \"m/ljos.org\"\ninbox = \"Software/ljos/inbox.org\"\n",
        )
        .unwrap();
        let err = create(&layout, "ljos", "an audit", CreateOpts::default()).unwrap_err();
        assert!(err.to_string().contains("Software/ljos/inbox.org"), "{err}");
        assert!(!layout.project_issues_path("ljos").exists());
        // A board this tracker is the source of takes the create.
        create(&layout, "surf", "local work", CreateOpts::default()).unwrap();
        assert_eq!(
            IssueDoc::parse_file("surf", &layout.project_issues_path("surf"))
                .unwrap()
                .headings
                .len(),
            1
        );
    }

    /// A claim is where an agent starts working, so it is where the working set
    /// has to be findable from. A verb nothing points at is a verb nobody runs.
    #[test]
    fn a_claim_points_at_the_working_set_when_there_is_one() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "the groundwork", CreateOpts::default()).unwrap();
        let first = only_id(&layout, "sample");
        create(&layout, "sample", "the next step", CreateOpts::default()).unwrap();
        let second = IssueDoc::parse_file("sample", &layout.project_issues_path("sample"))
            .unwrap()
            .headings
            .into_iter()
            .find(|h| h.id != first)
            .unwrap()
            .id;
        update(&layout, &second, None, None, Some(&first), None).unwrap();

        let claimed = claim_as(&layout, &second, false, "impl").unwrap();
        assert!(
            claimed.contains(&format!("`recall {second}`")),
            "the claim has to say where the working set is: {claimed}"
        );
        assert!(claimed.contains("1 declared input"), "{claimed}");

        // A node that stands on nothing gets no line, because there is nothing
        // for recall to hand over and a pointer to an empty answer is noise.
        let alone = claim_as(&layout, &first, false, "impl").unwrap();
        assert!(!alone.contains("recall"), "{alone}");
    }

    #[test]
    fn closing_stamps_closed_and_reopening_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "close me", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");
        let out = update(&layout, &id, Some("DONE"), None, None, None).unwrap();
        assert!(out.report.contains("CLOSED stamped"), "{}", out.report);
        let text = fs::read_to_string(layout.project_issues_path("sample")).unwrap();
        assert!(text.contains("\nCLOSED: ["), "{text}");
        assert!(text.contains("- State \"DONE\" from \"TODO\""), "{text}");
        update(&layout, &id, Some("TODO"), None, None, None).unwrap();
        let text = fs::read_to_string(layout.project_issues_path("sample")).unwrap();
        assert!(!text.contains("CLOSED:"), "{text}");
    }

    #[test]
    fn a_keyword_the_file_declares_is_legal_and_its_side_decides_closing() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "wait on it", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");
        let path = layout.project_issues_path("sample");
        let text = fs::read_to_string(&path).unwrap();
        let text = text.replace(
            "#+TODO: TODO STARTED BLOCKED | DONE CANCELLED",
            "#+TODO: TODO STARTED BLOCKED WAITING | DONE CANCELLED WONTFIX",
        );
        assert!(
            text.contains("WONTFIX"),
            "the house TODO line is where expected: {text}"
        );
        fs::write(&path, text).unwrap();
        update(&layout, &id, Some("WAITING"), None, None, None).unwrap();
        assert_eq!(issue_at(&layout, "sample", &id).state, "WAITING");
        let out = update(&layout, &id, Some("WONTFIX"), None, None, None).unwrap();
        assert!(out.report.contains("CLOSED stamped"), "{}", out.report);
        let err = update(&layout, &id, Some("NOPE"), None, None, None).unwrap_err();
        assert!(err.to_string().contains("WAITING"), "{err}");
    }

    #[test]
    fn a_repeating_deadline_moves_on_instead_of_closing() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(
            &layout,
            "sample",
            "weekly report",
            CreateOpts {
                deadline: Some("<2026-09-22 Tue +1w>"),
                ..CreateOpts::default()
            },
        )
        .unwrap();
        let id = only_id(&layout, "sample");
        let out = update(&layout, &id, Some("DONE"), None, None, None).unwrap();
        assert!(
            out.report
                .contains("repeats: DEADLINE -> <2026-09-29 Tue +1w>"),
            "{}",
            out.report
        );
        let h = issue_at(&layout, "sample", &id);
        assert_eq!(h.state, "TODO");
        assert_eq!(
            h.properties.get("DEADLINE").map(String::as_str),
            Some("<2026-09-29 Tue +1w>")
        );
        assert!(h.properties.contains_key("LAST_REPEAT"));
        assert!(!h.properties.contains_key("CLOSED"));
        assert_eq!(h.logbook[0].to_state.as_deref(), Some("TODO"));
        assert_eq!(h.logbook[1].to_state.as_deref(), Some("DONE"));
    }

    #[test]
    fn a_parents_statistics_cookie_follows_its_children() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(
            &layout,
            "sample",
            "parent of two [/]",
            CreateOpts::default(),
        )
        .unwrap();
        let parent = only_id(&layout, "sample");
        create(
            &layout,
            "sample",
            "first child",
            CreateOpts {
                parent: Some(&parent),
                ..CreateOpts::default()
            },
        )
        .unwrap();
        create(
            &layout,
            "sample",
            "second child [%]",
            CreateOpts {
                parent: Some(&parent),
                ..CreateOpts::default()
            },
        )
        .unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let first = doc
            .headings
            .iter()
            .find(|h| h.title == "first child")
            .unwrap()
            .id
            .clone();
        assert_eq!(
            issue_at(&layout, "sample", &parent).statistics.as_deref(),
            Some("[0/2]"),
            "the empty cookie is filled on the first write after the children exist"
        );
        update(&layout, &first, Some("DONE"), None, None, None).unwrap();
        assert_eq!(
            issue_at(&layout, "sample", &parent).statistics.as_deref(),
            Some("[1/2]")
        );
        let second = doc
            .headings
            .iter()
            .find(|h| h.title == "second child")
            .unwrap()
            .id
            .clone();
        assert_eq!(
            issue_at(&layout, "sample", &second).statistics.as_deref(),
            Some("[0%]")
        );
    }

    /// The citation is the handoff, so it has to survive the round trip through
    /// the file rather than living in the process that wrote it.
    #[test]
    fn a_cited_deed_is_readable_back_off_the_heading() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "name the note", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");

        let out = deed(&layout, &id, &["deed-patch-note".to_string()], &[]).unwrap();
        assert!(out.contains("deeds += deed-patch-note"), "{out}");
        assert_eq!(
            issue_at(&layout, "sample", &id).deeds(),
            vec!["deed-patch-note".to_string()]
        );
    }

    /// Two citations, and the order they were cited in is the order they read
    /// back: a trail is walked from the first product to the last.
    #[test]
    fn citations_keep_the_order_they_were_added_in() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "two products", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");

        deed(&layout, &id, &["deed-file-note".to_string()], &[]).unwrap();
        deed(&layout, &id, &["deed-patch-note".to_string()], &[]).unwrap();
        assert_eq!(
            issue_at(&layout, "sample", &id).deeds(),
            vec!["deed-file-note".to_string(), "deed-patch-note".to_string()]
        );
    }

    /// A retried step cites the same deed twice. Two copies would make a trail
    /// walk one deed twice and say nothing by doing it.
    #[test]
    fn citing_the_same_deed_twice_leaves_one_citation() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "retried", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");

        deed(&layout, &id, &["deed-file-note".to_string()], &[]).unwrap();
        let again = deed(&layout, &id, &["deed-file-note".to_string()], &[]).unwrap();
        assert!(again.contains("no change"), "{again}");
        assert_eq!(issue_at(&layout, "sample", &id).deeds().len(), 1);
    }

    /// Dropping the last citation drops the property rather than leaving an
    /// empty one, which `normalize` would otherwise have to clean up.
    #[test]
    fn removing_the_last_citation_removes_the_property() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "mistaken", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");

        deed(&layout, &id, &["deed-file-oops".to_string()], &[]).unwrap();
        deed(&layout, &id, &[], &["deed-file-oops".to_string()]).unwrap();
        let h = issue_at(&layout, "sample", &id);
        assert!(h.deeds().is_empty());
        assert!(
            !h.properties.contains_key(crate::props::DEEDS),
            "an empty citation list is not a citation list: {:?}",
            h.properties
        );
    }

    /// A path, a title, or a sentence in this field is a citation that resolves
    /// to nothing, and the failure would only show up in whatever tried to open
    /// it much later.
    #[test]
    fn a_value_deedar_could_not_be_asked_for_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "bad citation", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");

        let err = deed(&layout, &id, &["/tmp/note.md".to_string()], &[]).unwrap_err();
        assert!(err.to_string().contains("not a deed accession"), "{err}");
        assert!(
            issue_at(&layout, "sample", &id).deeds().is_empty(),
            "a refused citation must not land"
        );
    }

    /// Both accession forms deedar answers `get` for.
    #[test]
    fn both_deed_forms_are_accessions() {
        assert!(is_deed_accession("deed-quote-rfc2094-nll"));
        assert!(is_deed_accession(
            "sha256:0e1f2a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f7"
        ));
        assert!(!is_deed_accession("deed-"), "a prefix alone names nothing");
        assert!(
            !is_deed_accession("sha256:"),
            "a prefix alone names nothing"
        );
        assert!(!is_deed_accession(""));
        // Whitespace and commas separate the list, so a value holding one would
        // read back as two citations neither of which was cited.
        assert!(!is_deed_accession("deed-file a"));
        assert!(!is_deed_accession("deed-file,a"));
    }

    /// Reading is a read: `deed` with nothing to add or drop must not rewrite.
    #[test]
    fn listing_citations_does_not_touch_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "read only", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");
        deed(&layout, &id, &["deed-file-note".to_string()], &[]).unwrap();

        let path = layout.project_issues_path("sample");
        let before = fs::read_to_string(&path).unwrap();
        let out = deed(&layout, &id, &[], &[]).unwrap();
        assert!(out.contains("deed-file-note"), "{out}");
        assert_eq!(before, fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn create_rejects_a_parent_that_does_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        let err = create(
            &layout,
            "sample",
            "child without parent",
            CreateOpts {
                parent: Some("sample-zzz9"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("does not refer to any known id"));
    }

    #[test]
    fn create_accepts_a_parent_defined_in_a_design_document() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        let parent_id = "sample-spec-20260615";
        let project_dir = layout.projects_dir().join("sample");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(
            project_dir.join("design.org"),
            format!("#+TITLE: sample design\n\n* Design\n:PROPERTIES:\n:ID:         {parent_id}\n:END:\n"),
        )
        .unwrap();

        create(
            &layout,
            "sample",
            "child under design",
            CreateOpts {
                parent: Some(parent_id),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(only_id(&layout, "sample").starts_with("sample-"));
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        assert_eq!(doc.headings[0].parent(), Some(parent_id));
    }

    #[test]
    fn a_state_update_writes_a_logbook_entry() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "first", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");
        update(&layout, &id, Some("STARTED"), None, None, None).unwrap();
        let h = issue_at(&layout, "sample", &id);
        assert_eq!(h.state, "STARTED");
        assert_eq!(h.logbook[0].from_state.as_deref(), Some("TODO"));
        assert_eq!(h.logbook[0].to_state.as_deref(), Some("STARTED"));
    }

    #[test]
    fn blocking_and_unblocking_drive_the_state() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "first", CreateOpts::default()).unwrap();
        create(&layout, "sample", "blocker", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let first = doc.headings[0].id.clone();
        let blocker = doc.headings[1].id.clone();

        update(&layout, &first, None, None, Some(&blocker), None).unwrap();
        let h = issue_at(&layout, "sample", &first);
        assert_eq!(h.state, "BLOCKED");
        assert!(h.blocked_by().contains(&blocker));

        update(&layout, &first, None, None, None, Some(&blocker)).unwrap();
        let h = issue_at(&layout, "sample", &first);
        assert_eq!(h.state, "TODO");
        assert!(h.blocked_by().is_empty());
    }

    #[test]
    fn auto_unblock_to_todo_releases_the_claim() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "first", CreateOpts::default()).unwrap();
        create(&layout, "sample", "blocker", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let first = doc.headings[0].id.clone();
        let blocker = doc.headings[1].id.clone();

        crate::agent::claim(&layout, &first, false).unwrap();
        update(&layout, &first, None, None, Some(&blocker), None).unwrap();
        assert!(issue_at(&layout, "sample", &first).claimed_by().is_some());

        update(&layout, &first, None, None, None, Some(&blocker)).unwrap();
        let h = issue_at(&layout, "sample", &first);
        assert_eq!(h.state, "TODO");
        assert!(h.claimed_by().is_none(), "claim stuck on TODO: {h:?}");
    }

    #[test]
    fn release_holder_drops_every_claim_and_leaves_state() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "first", CreateOpts::default()).unwrap();
        create(&layout, "sample", "second", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let first = doc.headings[0].id.clone();
        let second = doc.headings[1].id.clone();
        claim_as(&layout, &first, false, "dead-host").unwrap();
        claim_as(&layout, &second, false, "dead-host").unwrap();

        let preview = release_holder_as(
            &layout,
            "dead-host",
            None,
            Some("X1 laptop is gone"),
            true,
            "operator",
        )
        .unwrap();
        assert!(
            preview.starts_with("dry-run: would release 2 claims"),
            "{preview}"
        );
        assert!(
            preview.contains(&first) && preview.contains(&second),
            "{preview}"
        );
        assert!(
            issue_at(&layout, "sample", &first).claimed_by() == Some("dead-host"),
            "dry-run wrote"
        );

        let done = release_holder_as(
            &layout,
            "dead-host",
            None,
            Some("X1 laptop is gone"),
            false,
            "operator",
        )
        .unwrap();
        assert!(
            done.starts_with("released 2 claims held by dead-host"),
            "{done}"
        );
        for id in [&first, &second] {
            let h = issue_at(&layout, "sample", id);
            assert_eq!(h.state, "STARTED", "{id} left STARTED");
            assert_eq!(h.claimed_by(), None, "{id} still claimed");
            assert!(
                h.logbook.iter().any(|e| {
                    e.note
                        .as_deref()
                        .is_some_and(|n| n.contains("released by operator: X1 laptop is gone"))
                }),
                "no why note on {id}: {:?}",
                h.logbook
            );
        }
    }

    #[test]
    fn release_holder_older_than_keeps_a_fresh_claim() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "fresh", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");
        claim_as(&layout, &id, false, "still-here").unwrap();
        let text =
            release_holder_as(&layout, "still-here", Some(7), None, false, "operator").unwrap();
        assert!(text.contains("no live claims held by still-here"), "{text}");
        assert_eq!(
            issue_at(&layout, "sample", &id).claimed_by(),
            Some("still-here")
        );
    }

    #[test]
    fn blocker_cycle_is_rejected_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "first", CreateOpts::default()).unwrap();
        create(&layout, "sample", "second", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let first = doc.headings[0].id.clone();
        let second = doc.headings[1].id.clone();

        update(&layout, &first, None, None, Some(&second), None).unwrap();
        let err = update(&layout, &second, None, None, Some(&first), None).unwrap_err();
        assert!(err.to_string().contains("blocker cycle"), "{err}");
        assert!(issue_at(&layout, "sample", &second).blocked_by().is_empty());
    }

    #[test]
    fn closing_a_blocker_reports_the_issues_still_pointing_at_it() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "first", CreateOpts::default()).unwrap();
        create(&layout, "sample", "blocker", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let first = doc.headings[0].id.clone();
        let blocker = doc.headings[1].id.clone();
        update(&layout, &first, None, None, Some(&blocker), None).unwrap();

        let outcome = update(&layout, &blocker, Some("DONE"), None, None, None).unwrap();
        assert_eq!(outcome.hints.len(), 1, "{:?}", outcome.hints);
        assert!(outcome.hints[0].contains(&first), "{:?}", outcome.hints);
    }

    #[test]
    fn refile_moves_the_heading_between_projects() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "source", "the issue", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "source");
        refile(&layout, &id, "target").unwrap();

        let src = IssueDoc::parse_file("source", &layout.project_issues_path("source")).unwrap();
        let tgt = IssueDoc::parse_file("target", &layout.project_issues_path("target")).unwrap();
        assert!(src.headings.is_empty());
        assert_eq!(tgt.headings[0].id, id);
    }

    #[test]
    fn deadlines_must_parse_as_org_dates() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        let err = create(
            &layout,
            "sample",
            "bad date",
            CreateOpts {
                deadline: Some("not-a-date"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("expected org date"));

        for (i, d) in ["<2026-05-15 Fri>", "[2026-05-15]"].iter().enumerate() {
            create(
                &layout,
                "sample",
                &format!("issue {i}"),
                CreateOpts {
                    deadline: Some(d),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        assert_eq!(doc.headings.len(), 2);
        assert!(doc.headings.iter().all(|h| h.deadline().is_some()));
    }

    #[test]
    fn org_safe_tags_go_on_the_heading_and_the_rest_stay_in_the_property() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(
            &layout,
            "sample",
            "tagged",
            CreateOpts {
                tags: Some("rust: perf ,, scaling, needs-review"),
                ..Default::default()
            },
        )
        .unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let h = &doc.headings[0];
        assert_eq!(h.org_tags, vec!["rust", "perf", "scaling"]);
        assert_eq!(
            h.properties
                .get(crate::model::TAGS_PROPERTY)
                .map(|s| s.as_str()),
            Some("needs-review"),
            "a tag Org cannot hold keeps the property"
        );
        // Whichever half a tag landed in, a query sees all of them.
        assert_eq!(
            h.tags(),
            vec!["needs-review", "rust", "perf", "scaling"],
            "{h:?}"
        );
    }

    #[test]
    fn create_keeps_an_explicit_id_that_is_free() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(
            &layout,
            "sample",
            "imported from the other board",
            CreateOpts {
                id: Some("sample-ab12"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(only_id(&layout, "sample"), "sample-ab12");
    }

    #[test]
    fn create_rejects_an_explicit_id_that_is_taken() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        let first = create(&layout, "sample", "already here", CreateOpts::default()).unwrap();
        let id = first.split_whitespace().next().unwrap().to_string();
        let err = create(
            &layout,
            "sample",
            "second copy",
            CreateOpts {
                id: Some(&id),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            err.to_string().contains(&id),
            "taken id must be named: {err}"
        );
    }

    #[test]
    fn create_rejects_an_explicit_id_for_another_project() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        let err = create(
            &layout,
            "sample",
            "wrong prefix",
            CreateOpts {
                id: Some("other-ab12"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("sample-<suffix>"), "{err}");
    }

    #[test]
    fn create_puts_a_legal_type_on_the_heading() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(
            &layout,
            "sample",
            "a bug",
            CreateOpts {
                issue_type: Some("bug"),
                ..Default::default()
            },
        )
        .unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let h = &doc.headings[0];
        assert_eq!(
            crate::props::get(&h.properties, crate::props::TYPE),
            Some("bug")
        );
        assert_eq!(h.org_tags, vec!["bug"]);
        let written = std::fs::read_to_string(layout.project_issues_path("sample")).unwrap();
        assert!(written.contains("#+CATEGORY: sample"), "{written}");
        assert!(written.contains(":bug:"), "{written}");
    }

    #[test]
    fn resolve_project_needs_a_name_from_somewhere() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        assert_eq!(
            resolve_project(&layout, Some("fromcli")).unwrap(),
            "fromcli"
        );
        assert!(
            resolve_project(&layout, Some(""))
                .unwrap_err()
                .to_string()
                .contains("empty")
        );
    }

    /// Parallel creates must not lose headings or fail the temporary rename.
    #[test]
    fn concurrent_creates_preserve_every_heading() {
        use std::sync::Arc;
        use std::thread;

        let dir = tempfile::tempdir().unwrap();
        let layout = Arc::new(fresh_layout(dir.path()));
        let n = 24usize;
        let handles: Vec<_> = (0..n)
            .map(|i| {
                let layout = Arc::clone(&layout);
                thread::spawn(move || {
                    create(
                        &layout,
                        "sample",
                        &format!("parallel title {i}"),
                        CreateOpts {
                            quiet: true,
                            ..Default::default()
                        },
                    )
                })
            })
            .collect();
        let mut ids: Vec<String> = handles
            .into_iter()
            .map(|h| {
                h.join()
                    .expect("thread panicked")
                    .expect("create failed")
                    .trim()
                    .to_string()
            })
            .collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "expected {n} unique ids, got {ids:?}");

        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let mut on_disk: Vec<String> = doc.headings.iter().map(|h| h.id.clone()).collect();
        on_disk.sort();
        assert_eq!(on_disk, ids);
    }

    #[test]
    fn note_appends_to_the_logbook_and_leaves_state_alone() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "carries a note", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");

        let out = note(&layout, &id, "first pass done,\n  \"quoted\" bit next").unwrap();
        assert_eq!(out, format!("{id}: noted\n"));

        let h = issue_at(&layout, "sample", &id);
        assert_eq!(h.state, "TODO");
        assert!(h.claimed_by().is_none());
        let notes: Vec<&str> = h.logbook.iter().filter_map(|e| e.note.as_deref()).collect();
        // Whitespace collapses to single spaces; double quotes become single.
        assert_eq!(notes, vec!["first pass done, 'quoted' bit next"]);
    }

    #[test]
    fn the_logbook_reads_newest_first_however_an_entry_arrived() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "ordered", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");

        note(&layout, &id, "first note").unwrap();
        update(&layout, &id, Some("STARTED"), None, None, None).unwrap();
        note(&layout, &id, "second note").unwrap();

        let h = issue_at(&layout, "sample", &id);
        let summary: Vec<String> = h
            .logbook
            .iter()
            .map(|e| match (&e.note, &e.to_state) {
                (Some(note), _) => note.clone(),
                (_, Some(to)) => format!("state:{to}"),
                _ => "?".into(),
            })
            .collect();
        assert_eq!(
            summary,
            vec!["second note", "state:STARTED", "first note"],
            "{h:?}"
        );
    }

    #[test]
    fn note_rejects_empty_text_and_unknown_ids() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "target", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");
        assert!(note(&layout, &id, "   ").is_err());
        assert!(note(&layout, "sample-zzz9", "text").is_err());
    }

    #[test]
    fn fold_creates_issues_and_stamps_the_inbox_idempotently() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "seed", CreateOpts::default()).unwrap();

        let inbox = dir.path().join("inbox.org");
        fs::write(
            &inbox,
            "#+TITLE: inbox\n\n\
             * TODO first discovered thing\nSome body line.\nAnother line.\n\
             * DONE already handled elsewhere\n\
             * TODO second discovered thing\n",
        )
        .unwrap();

        let out = fold(&layout, &inbox, "sample").unwrap();
        assert!(out.starts_with("folded 2: "), "got: {out}");

        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let titles: Vec<&str> = doc.headings.iter().map(|h| h.title.as_str()).collect();
        assert!(titles.contains(&"first discovered thing"));
        assert!(titles.contains(&"second discovered thing"));
        let folded = doc
            .headings
            .iter()
            .find(|h| h.title == "first discovered thing")
            .unwrap();
        assert!(folded.body.contains("Some body line."));

        // Headings flipped to DONE and stamped with the assigned id.
        let stamped = fs::read_to_string(&inbox).unwrap();
        assert_eq!(stamped.matches("* DONE ").count(), 3);
        assert_eq!(stamped.matches(":VISSUE_ID: sample-").count(), 2);
        assert!(!stamped.contains("* TODO "));

        // Second fold finds nothing unstamped and creates nothing.
        let again = fold(&layout, &inbox, "sample").unwrap();
        assert_eq!(again, "folded 0 (nothing unstamped)\n");
        let doc2 = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        assert_eq!(doc2.headings.len(), doc.headings.len());
    }

    #[test]
    fn refile_to_moves_across_two_layouts_and_leaves_no_shadow() {
        let src_dir = tempfile::tempdir().unwrap();
        let dst_dir = tempfile::tempdir().unwrap();
        let src_layout = fresh_layout(src_dir.path());
        let dst_layout = fresh_layout(dst_dir.path());
        create(&src_layout, "misc", "wrong board", CreateOpts::default()).unwrap();
        let id = IssueDoc::parse_file("misc", &src_layout.project_issues_path("misc"))
            .unwrap()
            .headings[0]
            .id
            .clone();

        let out = refile_to(&src_layout, &id, &dst_layout, "surf").unwrap();
        assert!(out.contains("misc -> surf"), "{out}");

        // The heading is on the destination tracker, and the source root has
        // no `surf` directory standing in for it.
        let moved = IssueDoc::parse_file("surf", &dst_layout.project_issues_path("surf")).unwrap();
        assert_eq!(moved.headings.len(), 1);
        assert_eq!(moved.headings[0].id, id);
        assert!(!src_layout.project_issues_path("surf").exists());
        let left = IssueDoc::parse_file("misc", &src_layout.project_issues_path("misc")).unwrap();
        assert!(left.headings.is_empty());
    }

    #[test]
    fn reject_creates_the_successor_on_the_destination_layout() {
        let src_dir = tempfile::tempdir().unwrap();
        let dst_dir = tempfile::tempdir().unwrap();
        let src_layout = fresh_layout(src_dir.path());
        let dst_layout = fresh_layout(dst_dir.path());
        create(&src_layout, "misc", "old approach", CreateOpts::default()).unwrap();
        let src = IssueDoc::parse_file("misc", &src_layout.project_issues_path("misc"))
            .unwrap()
            .headings[0]
            .id
            .clone();

        // The routed board's ids, handed over as the file so they are read
        // under the write lock.
        let twin_dir = tempfile::tempdir().unwrap();
        let twin_layout = fresh_layout(twin_dir.path());
        let twin_path = twin_layout.project_issues_path("surf");
        std::fs::create_dir_all(twin_path.parent().unwrap()).unwrap();
        std::fs::write(
            &twin_path,
            "#+TITLE: surf issues\n\n* TODO taken elsewhere\n:PROPERTIES:\n             :ID:         surf-aaaa\n:END:\n",
        )
        .unwrap();
        let twins = vec![twin_path.clone()];
        let out = reject(
            &src_layout,
            &src,
            RejectOpts {
                project: Some("surf"),
                title: Some("new approach"),
                dst_layout: Some(&dst_layout),
                dst_extra_id_paths: &twins,
                ..Default::default()
            },
        )
        .unwrap();

        assert!(!src_layout.project_issues_path("surf").exists());
        let made = IssueDoc::parse_file("surf", &dst_layout.project_issues_path("surf")).unwrap();
        assert_eq!(made.headings.len(), 1);
        assert_ne!(made.headings[0].id, "surf-aaaa");
        assert!(out.contains(&made.headings[0].id), "{out}");
        assert_eq!(issue_at(&src_layout, "misc", &src).state, "CANCELLED");
    }

    #[test]
    fn reject_to_an_existing_issue_cancels_and_wires_the_pair() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "old approach", CreateOpts::default()).unwrap();
        create(&layout, "sample", "new approach", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let src = doc.headings[0].id.clone();
        let dst = doc.headings[1].id.clone();

        let out = reject(
            &layout,
            &src,
            RejectOpts {
                to: Some(&dst),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(out.contains(&src) && out.contains(&dst), "{out}");

        let src_h = issue_at(&layout, "sample", &src);
        assert_eq!(src_h.state, "CANCELLED");
        assert_eq!(
            src_h.properties.get("PIVOTED_TO").map(String::as_str),
            Some(dst.as_str())
        );
        let dst_h = issue_at(&layout, "sample", &dst);
        assert_eq!(
            dst_h.properties.get("DISCOVERED_FROM").map(String::as_str),
            Some(src.as_str())
        );
    }

    #[test]
    fn reject_creates_the_destination_in_another_project() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "old approach", CreateOpts::default()).unwrap();
        let src = only_id(&layout, "sample");

        let out = reject(
            &layout,
            &src,
            RejectOpts {
                project: Some("other"),
                title: Some("new approach"),
                ..Default::default()
            },
        )
        .unwrap();

        let dst_doc = IssueDoc::parse_file("other", &layout.project_issues_path("other")).unwrap();
        assert_eq!(dst_doc.headings.len(), 1);
        let dst = &dst_doc.headings[0];
        assert_eq!(dst.title, "new approach");
        assert_eq!(
            dst.properties.get("DISCOVERED_FROM").map(String::as_str),
            Some(src.as_str())
        );
        assert!(out.contains(&src) && out.contains(&dst.id), "{out}");

        let src_h = issue_at(&layout, "sample", &src);
        assert_eq!(src_h.state, "CANCELLED");
        assert_eq!(
            src_h.properties.get("PIVOTED_TO").map(String::as_str),
            Some(dst.id.as_str())
        );
    }

    #[test]
    fn reject_refuses_an_unknown_source_or_destination() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "only", CreateOpts::default()).unwrap();
        let src = only_id(&layout, "sample");

        let missing_src = reject(
            &layout,
            "sample-zzzz",
            RejectOpts {
                to: Some(&src),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            matches!(missing_src, Error::IssueNotFound { .. }),
            "{missing_src}"
        );

        let missing_dst = reject(
            &layout,
            &src,
            RejectOpts {
                to: Some("sample-zzzz"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            matches!(missing_dst, Error::IssueNotFound { .. }),
            "{missing_dst}"
        );
    }

    #[test]
    fn reject_does_not_overwrite_a_nonempty_discovered_from() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "origin", CreateOpts::default()).unwrap();
        create(&layout, "sample", "old approach", CreateOpts::default()).unwrap();
        create(&layout, "sample", "already sourced", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let origin = doc.headings[0].id.clone();
        let src = doc.headings[1].id.clone();
        let dst = doc.headings[2].id.clone();

        let path = layout.project_issues_path("sample");
        let mut doc = IssueDoc::parse_file("sample", &path).unwrap();
        doc.headings
            .iter_mut()
            .find(|h| h.id == dst)
            .unwrap()
            .properties
            .insert("DISCOVERED_FROM".into(), origin.clone());
        doc.write().unwrap();

        reject(
            &layout,
            &src,
            RejectOpts {
                to: Some(&dst),
                ..Default::default()
            },
        )
        .unwrap();
        let dst_h = issue_at(&layout, "sample", &dst);
        assert_eq!(
            dst_h.properties.get("DISCOVERED_FROM").map(String::as_str),
            Some(origin.as_str()),
            "a filled DISCOVERED_FROM stays put"
        );
    }

    #[test]
    fn create_sets_discovered_from_from_the_first_known_id_link() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "source", CreateOpts::default()).unwrap();
        let known = only_id(&layout, "sample");
        create(
            &layout,
            "sample",
            "fell out of it",
            CreateOpts {
                body: Some(&format!("See [[id:{known}]] for the parent finding.")),
                ..Default::default()
            },
        )
        .unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let child = doc
            .headings
            .iter()
            .find(|h| h.title == "fell out of it")
            .unwrap();
        assert_eq!(
            child.properties.get("DISCOVERED_FROM").map(String::as_str),
            Some(known.as_str())
        );
    }

    #[test]
    fn create_ignores_an_id_link_that_is_not_in_the_corpus() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(
            &layout,
            "sample",
            "orphan mention",
            CreateOpts {
                body: Some("See [[id:sample-zzzz]] which does not exist."),
                ..Default::default()
            },
        )
        .unwrap();
        let h = issue_at(&layout, "sample", &only_id(&layout, "sample"));
        assert!(
            !h.properties.contains_key("DISCOVERED_FROM"),
            "unknown [[id:]] must not mint DISCOVERED_FROM: {h:?}"
        );
        assert!(
            !h.properties.contains_key("BLOCKED_BY"),
            "prose must not mint BLOCKED_BY: {h:?}"
        );
    }

    #[test]
    fn related_after_reject_names_the_successor_without_a_body_link() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "old approach", CreateOpts::default()).unwrap();
        create(&layout, "sample", "new approach", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let src = doc.headings[0].id.clone();
        let dst = doc.headings[1].id.clone();
        reject(
            &layout,
            &src,
            RejectOpts {
                to: Some(&dst),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(
            !issue_at(&layout, "sample", &src).body.contains(&dst),
            "the pair is wired by PIVOTED_TO, not prose"
        );
        let from_src = crate::related::related(&layout, &src, 1, 10, "text").unwrap();
        assert!(from_src.contains(&dst), "{from_src}");
        assert!(from_src.contains("pivoted_to"), "{from_src}");

        let from_dst = crate::related::related(&layout, &dst, 1, 10, "text").unwrap();
        assert!(from_dst.contains(&src), "{from_dst}");
        assert!(from_dst.contains("successor_of"), "{from_dst}");

        let waiting = crate::report::backlinks(&layout, &dst).unwrap();
        assert!(waiting.contains(&src), "{waiting}");
    }

    #[test]
    fn update_to_cancelled_emits_state_change_with_the_id() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "first", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");
        let before = crate::events::generation(&layout);
        update(&layout, &id, Some("CANCELLED"), None, None, None).unwrap();
        let events = crate::events::since(&layout, before, 50).unwrap();
        assert!(
            events.iter().any(|e| {
                e.kind == "state_change"
                    && e.id.as_deref() == Some(id.as_str())
                    && e.detail.as_deref() == Some("TODO->CANCELLED")
            }),
            "{events:?}"
        );
    }

    #[test]
    fn a_stale_done_after_reject_is_refused_and_the_source_stays_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "old plan", CreateOpts::default()).unwrap();
        create(&layout, "sample", "rewrite", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let src = doc.headings[0].id.clone();
        let dst = doc.headings[1].id.clone();
        reject(
            &layout,
            &src,
            RejectOpts {
                to: Some(&dst),
                ..Default::default()
            },
        )
        .unwrap();

        let err = update_pred(
            &layout,
            &src,
            Some("DONE"),
            None,
            None,
            None,
            UpdatePred {
                if_state: Some("STARTED"),
                if_gen: None,
            },
        )
        .unwrap_err();
        assert!(
            matches!(
                err,
                Error::StaleWrite {
                    ref actual_state,
                    ref expected_state,
                    ..
                } if actual_state == "CANCELLED" && expected_state.as_deref() == Some("STARTED")
            ),
            "{err:?}"
        );
        assert_eq!(issue_at(&layout, "sample", &src).state, "CANCELLED");
    }

    #[test]
    fn if_gen_refuses_when_the_corpus_moved() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "first", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");
        let seen = crate::events::generation(&layout);
        update(&layout, &id, Some("STARTED"), None, None, None).unwrap();
        let err = update_pred(
            &layout,
            &id,
            Some("DONE"),
            None,
            None,
            None,
            UpdatePred {
                if_state: None,
                if_gen: Some(seen),
            },
        )
        .unwrap_err();
        assert!(matches!(err, Error::StaleWrite { .. }), "{err:?}");
        assert_eq!(issue_at(&layout, "sample", &id).state, "STARTED");
    }

    #[test]
    fn a_second_terminal_does_not_drop_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "first", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");
        update(&layout, &id, Some("DONE"), None, None, None).unwrap();
        update(&layout, &id, Some("CANCELLED"), None, None, None).unwrap();
        let h = issue_at(&layout, "sample", &id);
        assert_eq!(h.state, "DONE", "first terminal must stay");
        assert_eq!(
            crate::props::get(&h.properties, crate::props::SIBLING_TERMINAL),
            Some("CANCELLED")
        );

        resolve_terminal(&layout, &id, "CANCELLED").unwrap();
        let h = issue_at(&layout, "sample", &id);
        assert_eq!(h.state, "CANCELLED");
        assert!(crate::props::get(&h.properties, crate::props::SIBLING_TERMINAL).is_none());
    }

    #[test]
    fn check_warns_on_reject_prose_done_and_a_mention_without_an_edge() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "shipped", CreateOpts::default()).unwrap();
        create(&layout, "sample", "other", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let shipped = doc.headings[0].id.clone();
        let other = doc.headings[1].id.clone();
        update(&layout, &shipped, Some("DONE"), None, None, None).unwrap();
        append_body(&layout, &shipped, "superseded by the other one, bounced").unwrap();
        append_body(
            &layout,
            &other,
            &format!("discovered while reading [[id:{shipped}]]"),
        )
        .unwrap();

        let report = crate::report::check(&layout).unwrap();
        assert!(
            report.text.contains(&shipped)
                && report.text.contains("DONE but the body reads as a reject"),
            "{}",
            report.text
        );
        assert!(
            report.text.contains(&other)
                && report
                    .text
                    .contains("as discovered or pivoted with no edge"),
            "{}",
            report.text
        );
        assert!(report.warnings >= 2, "{}", report.text);
    }

    // The word is not the finding. Every bug about input validation says
    // "rejected", and three issues in one corpus were flagged for sentences
    // about what the software does to bad input.
    #[test]
    fn check_is_quiet_about_a_done_issue_that_merely_uses_the_word_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "validation", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let id = doc.headings[0].id.clone();
        update(&layout, &id, Some("DONE"), None, None, None).unwrap();
        append_body(
            &layout,
            &id,
            "A compound spec is silently corrupted rather than rejected, and the \
             alternative parser was rejected as strictly dominated.",
        )
        .unwrap();

        let report = crate::report::check(&layout).unwrap();
        assert!(
            !report.text.contains("reads as a reject"),
            "the word alone was read as an outcome: {}",
            report.text
        );
    }

    // A "Supersedes" section rolls up issues this one did not close, which is the
    // opposite of being superseded, and the two differ by one letter.
    #[test]
    fn check_reads_supersedes_as_a_roll_up_and_superseded_by_as_an_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "umbrella", CreateOpts::default()).unwrap();
        create(&layout, "sample", "replaced", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let rollup = doc.headings[0].id.clone();
        let replaced = doc.headings[1].id.clone();
        update(&layout, &rollup, Some("DONE"), None, None, None).unwrap();
        update(&layout, &replaced, Some("DONE"), None, None, None).unwrap();
        append_body(&layout, &rollup, "** Supersedes\nrolls up the pieces").unwrap();
        append_body(&layout, &replaced, "superseded by the umbrella").unwrap();

        let report = crate::report::check(&layout).unwrap();
        let flagged: Vec<&str> = report
            .text
            .lines()
            .filter(|l| l.contains("reads as a reject"))
            .collect();

        assert!(
            flagged.iter().any(|l| l.contains(&replaced)),
            "an issue that says it was superseded was not flagged: {}",
            report.text
        );
        assert!(
            !flagged.iter().any(|l| l.contains(&rollup)),
            "a Supersedes roll-up was read as its own rejection: {}",
            report.text
        );
    }

    // A body links other issues for every reason there is. Only the reason the
    // properties name is a finding.
    #[test]
    fn check_is_quiet_about_a_mention_that_claims_no_relation() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "umbrella", CreateOpts::default()).unwrap();
        create(&layout, "sample", "piece", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let umbrella = doc.headings[0].id.clone();
        let piece = doc.headings[1].id.clone();
        append_body(
            &layout,
            &umbrella,
            &format!("** Supersedes\nRolls up [[id:{piece}]], which it does not close."),
        )
        .unwrap();

        let report = crate::report::check(&layout).unwrap();
        assert!(
            !report.text.contains("as discovered or pivoted"),
            "a roll-up was read as a discovery: {}",
            report.text
        );
    }

    // And the claim has to be near the link: a long issue says many things.
    #[test]
    fn check_reads_a_discovery_claim_only_near_the_link_it_belongs_to() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "long", CreateOpts::default()).unwrap();
        create(&layout, "sample", "elsewhere", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let long = doc.headings[0].id.clone();
        let elsewhere = doc.headings[1].id.clone();
        let filler = "prose ".repeat(120);
        append_body(
            &layout,
            &long,
            &format!("discovered while auditing the loader.\n{filler}\nsee [[id:{elsewhere}]]"),
        )
        .unwrap();

        let report = crate::report::check(&layout).unwrap();
        assert!(
            !report.text.contains("as discovered or pivoted"),
            "a claim in another section was attached to this link: {}",
            report.text
        );
    }

    // A parent naming its child is a stated relation the tracker already holds.
    #[test]
    fn check_is_quiet_about_a_mention_that_a_parent_edge_already_explains() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "umbrella", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let parent = doc.headings[0].id.clone();
        create(
            &layout,
            "sample",
            "piece",
            CreateOpts {
                parent: Some(parent.as_str()),
                ..CreateOpts::default()
            },
        )
        .unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let child = doc
            .headings
            .iter()
            .find(|h| h.id != parent)
            .map(|h| h.id.clone())
            .unwrap();
        // The prose claims a discovery, so the warning fires unless the parent
        // edge is recognised.
        append_body(
            &layout,
            &parent,
            &format!("discovered while reading [[id:{child}]]"),
        )
        .unwrap();
        create(&layout, "sample", "unrelated", CreateOpts::default()).unwrap();
        let doc = IssueDoc::parse_file("sample", &layout.project_issues_path("sample")).unwrap();
        let stranger = doc
            .headings
            .iter()
            .find(|h| h.id != parent && h.id != child)
            .map(|h| h.id.clone())
            .unwrap();
        append_body(
            &layout,
            &stranger,
            &format!("discovered while reading [[id:{parent}]]"),
        )
        .unwrap();

        let report = crate::report::check(&layout).unwrap();
        let flagged: Vec<&str> = report
            .text
            .lines()
            .filter(|l| l.contains("as discovered or pivoted"))
            .collect();
        assert!(
            flagged.iter().any(|l| l.contains(&stranger)),
            "the control pair with no edge was not flagged, so this test proves nothing: {}",
            report.text
        );
        assert!(
            !flagged
                .iter()
                .any(|l| l.contains(&parent) && l.contains(&child)),
            "a parent edge did not count as a relation: {}",
            report.text
        );
    }

    #[test]
    fn check_names_a_file_missing_category_and_a_type_not_on_the_heading() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        let path = layout.project_issues_path("sample");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "#+TITLE: sample issues\n#+TODO: TODO STARTED BLOCKED | DONE CANCELLED\n\n* TODO [#A] Untagged type\n:PROPERTIES:\n:ID:         sample-aaaa\n:TYPE:       bug\n:END:\n",
        )
        .unwrap();
        let report = crate::report::check(&layout).unwrap();
        assert!(
            report.text.contains("sample: preamble has no #+CATEGORY:"),
            "{}",
            report.text
        );
        assert!(
            report
                .text
                .contains("have :TYPE: that is a legal Org tag but is not on the heading"),
            "{}",
            report.text
        );
        assert!(
            report
                .text
                .contains("preamble has no #+VISSUE: protocol stamp"),
            "{}",
            report.text
        );
        assert!(
            report.text.contains("preamble has no #+PRIORITIES:"),
            "{}",
            report.text
        );
    }

    #[test]
    fn check_errors_on_a_newer_protocol_stamp() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        let path = layout.project_issues_path("sample");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "#+TITLE: sample issues\n#+VISSUE: 99\n#+CATEGORY: sample\n#+FILETAGS: :issues:sample:noexport:\n#+TAGS: docs\n#+TODO: TODO | DONE\n\n* TODO [#A] Future\n:PROPERTIES:\n:ID:         sample-aaaa\n:END:\n",
        )
        .unwrap();
        let report = crate::report::check(&layout).unwrap();
        assert!(report.errors >= 1, "{}", report.text);
        assert!(
            report
                .text
                .contains("#+VISSUE: 99 is newer than this vissue"),
            "{}",
            report.text
        );
    }

    #[test]
    fn normalize_rewrites_legacy_keys_and_keeps_edna() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        let path = layout.project_issues_path("sample");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "#+TITLE: sample issues\n#+TODO: TODO STARTED BLOCKED | DONE CANCELLED\n\n* TODO [#A] Legacy\n:PROPERTIES:\n:ID:         sample-aaaa\n:TYPE:       bug\n:PARENT:     sample-root\n:BLOCKEDBY:  sample-bbbb\n:END:\n\n* TODO [#A] Edna condition\n:PROPERTIES:\n:ID:         sample-cccc\n:BLOCKER:    prev-sibling\n:END:\n",
        )
        .unwrap();
        let dry = normalize(&layout, Some("sample"), true).unwrap();
        assert!(dry.contains("would rewrite"), "{dry}");
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains(":TYPE:"), "{on_disk}");
        let wrote = normalize(&layout, Some("sample"), false).unwrap();
        assert!(wrote.contains("rewrote"), "{wrote}");
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("#+CATEGORY: sample"), "{after}");
        assert!(after.contains("#+PRIORITIES: A C C"), "{after}");
        assert!(after.contains(":TYPE:       bug"), "{after}");
        assert!(after.contains(":PARENT:"), "{after}");
        assert!(after.contains(":BLOCKED_BY:"), "{after}");
        assert!(
            !after.contains("ids(sample-bbbb)"),
            "normalize must not mint edna ids(): {after}"
        );
        assert!(after.contains("prev-sibling"), "{after}");
    }
    /// The reservation is read under the lock: with `id_length = 2` the twin
    /// holds 1295 of 1296 suffixes, so a mint that reads it returns the one
    /// free suffix and a mint that trusts the caller's snapshot does not.
    #[test]
    fn the_reservation_is_read_after_the_lock_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let own_root = dir.path().join("own");
        let twin_root = dir.path().join("twin");
        std::fs::create_dir_all(&own_root).unwrap();
        std::fs::create_dir_all(&twin_root).unwrap();
        std::fs::write(own_root.join("vissue.toml"), "[issues]\nid_length = 2\n").unwrap();
        let own = fresh_layout(&own_root);
        let twin = fresh_layout(&twin_root);

        // Every suffix but "zz", written straight to the twin file.
        let mut body = String::from("#+TITLE: sample issues\n\n");
        let alphabet = b"0123456789abcdefghijklmnopqrstuvwxyz";
        for a in alphabet {
            for b in alphabet {
                if *a == b'z' && *b == b'z' {
                    continue;
                }
                let id = format!("sample-{}{}", *a as char, *b as char);
                body.push_str(&format!(
                    "* TODO filler {id}\n:PROPERTIES:\n:ID:         {id}\n:END:\n\n"
                ));
            }
        }
        let twin_path = twin.project_issues_path("sample");
        std::fs::create_dir_all(twin_path.parent().unwrap()).unwrap();
        std::fs::write(&twin_path, body).unwrap();

        let twins = vec![twin_path.clone()];
        let id = create(
            &own,
            "sample",
            "the only suffix left",
            CreateOpts {
                quiet: true,
                extra_id_paths: &twins,
                ..Default::default()
            },
        )
        .expect("create failed")
        .trim()
        .to_string();

        assert_eq!(
            id, "sample-zz",
            "the mint did not treat the twin file as taken, so it read the reservation \
             before the lock rather than after"
        );
    }

    /// The write path in its own reservation list is not a deadlock.
    #[test]
    fn the_written_file_appearing_in_its_own_reservation_is_not_a_deadlock() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        let own_path = layout.project_issues_path("sample");
        let twins = vec![own_path.clone(), own_path.clone()];
        let id = create(
            &layout,
            "sample",
            "self referential reservation",
            CreateOpts {
                quiet: true,
                extra_id_paths: &twins,
                ..Default::default()
            },
        )
        .expect("create deadlocked or failed")
        .trim()
        .to_string();
        assert!(id.starts_with("sample-"), "{id}");
    }
    // ------------------------------------------------------------------ votes

    fn voted(layout: &Layout, id: &str, who: &str, choice: &str) -> String {
        vote(layout, id, Some(choice), who).expect("vote failed")
    }

    #[test]
    fn one_agent_one_ballot_and_a_recast_replaces_it() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "what to do", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");

        voted(&layout, &id, "agent-a", "ship");
        let out = voted(&layout, &id, "agent-a", "hold");
        assert!(out.contains("changed ship to hold"), "{out}");

        let tally = vote(&layout, &id, None, "reader").unwrap();
        assert!(tally.contains("1 vote from 1 option"), "{tally}");
        assert!(tally.contains("hold"), "{tally}");
        assert!(!tally.contains("ship"), "{tally}");
    }

    #[test]
    fn two_agents_do_not_overwrite_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "what to do", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");

        voted(&layout, &id, "agent-a", "ship");
        voted(&layout, &id, "agent-b", "ship");
        let out = voted(&layout, &id, "agent-c", "hold");

        assert!(out.contains("3 votes from 2 options"), "{out}");
        assert!(out.contains("consensus: ship (2 of 3)"), "{out}");
    }

    /// A tie is the case a tally exists to surface, so it must not report the
    /// first option as though the agents agreed.
    #[test]
    fn a_tie_is_reported_as_no_consensus() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "what to do", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");

        voted(&layout, &id, "agent-a", "ship");
        let out = voted(&layout, &id, "agent-b", "hold");

        assert!(out.contains("no consensus: 2 options tied at 1"), "{out}");
        assert!(!out.contains("consensus: ship"), "{out}");
    }

    /// And a lead that is not a majority is a plurality, which is a different
    /// claim from agreement.
    #[test]
    fn a_lead_short_of_a_majority_is_not_called_consensus() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "what to do", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");

        voted(&layout, &id, "agent-a", "ship");
        voted(&layout, &id, "agent-b", "ship");
        voted(&layout, &id, "agent-c", "hold");
        let out = voted(&layout, &id, "agent-d", "rework");

        // 2 of 4 leads but does not carry.
        assert!(out.contains("plurality only: ship (2 of 4)"), "{out}");
        assert!(!out.contains("consensus: ship"), "{out}");
    }

    #[test]
    fn votes_survive_a_rewrite_and_are_readable_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "what to do", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");
        voted(&layout, &id, "agent-a", "ship");

        // An unrelated edit rewrites the file; the drawer has to come back.
        append_body(&layout, &id, "some prose").unwrap();
        let text = std::fs::read_to_string(layout.project_issues_path("sample")).unwrap();
        assert!(text.contains(":VOTES:"), "{text}");
        assert!(text.contains("agent-a: ship"), "{text}");
        let recorded = vote_with(
            &layout,
            &id,
            Some("hold"),
            "agent-b",
            Some("none"),
            Some("0.5"),
        )
        .unwrap();
        assert!(recorded.contains("agent-b voted hold"), "{recorded}");
        let text = std::fs::read_to_string(layout.project_issues_path("sample")).unwrap();
        assert!(
            text.contains("agent-b: hold used=none confidence=0.5"),
            "{text}"
        );
        let ballots = ballots(&layout, &id).unwrap();
        let b = ballots.iter().find(|b| b.agent == "agent-b").unwrap();
        assert_eq!(b.choice, "hold");
        assert_eq!(b.used.as_deref(), Some("none"));
        assert_eq!(b.confidence.as_deref(), Some("0.5"));

        let tally = vote(&layout, &id, None, "reader").unwrap();
        assert!(tally.contains("agent-a"), "{tally}");
    }

    #[test]
    fn an_issue_with_no_votes_says_so_rather_than_showing_an_empty_table() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "what to do", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");
        assert!(
            vote(&layout, &id, None, "reader")
                .unwrap()
                .contains("no votes")
        );
    }

    #[test]
    fn a_blank_or_multiline_vote_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "what to do", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");
        assert!(vote(&layout, &id, Some("   "), "agent-a").is_err());
        assert!(vote(&layout, &id, Some("ship\nhold"), "agent-a").is_err());
    }

    /// A choice may hold a colon, because "ship: after the audit" is a thing an
    /// agent will vote for and the line format has to survive it.
    #[test]
    fn a_choice_containing_a_colon_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "what to do", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");
        voted(&layout, &id, "agent-a", "ship: after the audit");
        let tally = vote(&layout, &id, None, "reader").unwrap();
        assert!(tally.contains("ship: after the audit"), "{tally}");
    }

    /// Concurrent voters are the point of the feature, so they are tested the
    /// way the id reservation is: every ballot has to land.
    #[test]
    fn concurrent_voters_all_land() {
        use std::sync::Arc;
        use std::thread;

        let dir = tempfile::tempdir().unwrap();
        let layout = Arc::new(fresh_layout(dir.path()));
        create(&layout, "sample", "what to do", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");

        let n = 16usize;
        let handles: Vec<_> = (0..n)
            .map(|i| {
                let layout = Arc::clone(&layout);
                let id = id.clone();
                thread::spawn(move || vote(&layout, &id, Some("ship"), &format!("agent-{i:02}")))
            })
            .collect();
        for h in handles {
            h.join().expect("thread panicked").expect("vote failed");
        }

        let tally = vote(&layout, &id, None, "reader").unwrap();
        assert!(
            tally.contains(&format!("{n} votes from 1 option")),
            "a ballot was lost: {tally}"
        );
    }
    /// One agent agreeing with itself is not a consensus. Calling it one is how a
    /// single unreviewed opinion gets acted on as though it had been checked.
    #[test]
    fn a_single_ballot_is_not_called_a_consensus() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "what to do", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");

        let out = voted(&layout, &id, "agent-a", "ship");
        assert!(out.contains("one ballot only: ship"), "{out}");
        assert!(!out.contains("consensus: ship"), "{out}");

        // A second agent agreeing makes it one.
        let out = voted(&layout, &id, "agent-b", "ship");
        assert!(out.contains("consensus: ship (2 of 2)"), "{out}");
    }

    /// An identity holding ": " is refused: the ballot line splits there.
    #[test]
    fn an_identity_that_the_line_format_cannot_hold_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "what to do", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");

        let err = vote(&layout, &id, Some("ship"), "team: alpha").unwrap_err();
        assert!(err.to_string().contains("colon"), "{err}");
        assert!(vote(&layout, &id, Some("ship"), "   ").is_err());

        // And the tally is untouched by the refusal.
        assert!(
            vote(&layout, &id, None, "reader")
                .unwrap()
                .contains("no votes")
        );
    }

    /// The drawer is org a person can edit. A rewrite that kept only the lines
    /// this parser understands would eat a comment left there, on the next vote,
    /// without saying anything.
    #[test]
    fn a_hand_written_line_in_the_drawer_survives_a_vote() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "what to do", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");
        voted(&layout, &id, "agent-a", "ship");

        // Someone edits the drawer by hand.
        let path = layout.project_issues_path("sample");
        let text = std::fs::read_to_string(&path).unwrap();
        let edited = text.replace(
            ":VOTES:\n",
            ":VOTES:\n# decided at the Tuesday review, do not clear\n",
        );
        std::fs::write(&path, edited).unwrap();

        voted(&layout, &id, "agent-b", "hold");

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(
            after.contains("# decided at the Tuesday review, do not clear"),
            "the hand-written line was eaten: {after}"
        );
        assert!(after.contains("agent-a: ship"), "{after}");
        assert!(after.contains("agent-b: hold"), "{after}");
    }

    /// Two spellings of one file, through a symlink, lock it once; `Path`
    /// component comparison alone would not catch this.
    #[cfg(unix)]
    #[test]
    fn one_file_named_two_ways_is_locked_once() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        let direct = layout.project_issues_path("sample");
        create(&layout, "sample", "first", CreateOpts::default()).unwrap();

        // A second name for the same tree, the way two configured roots can be.
        let link = dir.path().join("linked");
        std::os::unix::fs::symlink(dir.path().join(DEFAULT_PREFIX), &link).unwrap();
        let indirect = link.join("sample").join("issues.org");
        assert!(indirect.exists(), "the link does not reach the file");
        assert_ne!(
            direct.components().count(),
            0,
            "the two paths must differ by components or this proves nothing"
        );
        assert!(
            direct != indirect,
            "the two paths compare equal, so the plain dedup would already collapse them"
        );

        let twins = vec![direct.clone(), indirect];
        let id = create(
            &layout,
            "sample",
            "second",
            CreateOpts {
                quiet: true,
                extra_id_paths: &twins,
                ..Default::default()
            },
        )
        .expect("create hung or failed on an aliased lock path")
        .trim()
        .to_string();
        assert!(id.starts_with("sample-"), "{id}");
    }

    /// A drawer edited by hand can hold two lines for one agent. The tally counts
    /// on one ballot per agent, so the duplicate has to collapse rather than let
    /// one voter count twice.
    #[test]
    fn two_hand_written_lines_for_one_agent_collapse_to_the_last() {
        let dir = tempfile::tempdir().unwrap();
        let layout = fresh_layout(dir.path());
        create(&layout, "sample", "what to do", CreateOpts::default()).unwrap();
        let id = only_id(&layout, "sample");
        voted(&layout, &id, "agent-b", "hold");

        let path = layout.project_issues_path("sample");
        let text = std::fs::read_to_string(&path).unwrap();
        let edited = text.replace(
            ":VOTES:\n",
            ":VOTES:\n[2026-01-01 Thu] agent-a: ship\n[2026-02-02 Mon] agent-a: rework\n",
        );
        std::fs::write(&path, edited).unwrap();

        let tally = vote(&layout, &id, None, "reader").unwrap();
        // agent-a counts once, as rework, so two agents and two options.
        assert!(tally.contains("2 votes from 2 options"), "{tally}");
        assert!(tally.contains("rework"), "{tally}");
        assert!(!tally.contains("ship"), "{tally}");

        // And the rewrite leaves one line for that agent, not two.
        voted(&layout, &id, "agent-c", "hold");
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after.matches("agent-a:").count(), 1, "{after}");
    }

    /// Standing in a project directory of the tracker names that project,
    /// ahead of a `.project-ctx.toml` at the vault root.
    #[test]
    fn a_cwd_under_the_prefix_names_its_project() {
        let projects = Path::new("/vault/Software");
        assert_eq!(
            project_from_tracker_path(projects, Path::new("/vault/Software/ljos")),
            Some("ljos".to_string())
        );
        assert_eq!(
            project_from_tracker_path(projects, Path::new("/vault/Software/ljos/notes/deep")),
            Some("ljos".to_string())
        );
        assert_eq!(
            project_from_tracker_path(projects, Path::new("/vault/Software")),
            None
        );
        assert_eq!(
            project_from_tracker_path(projects, Path::new("/vault")),
            None
        );
        assert_eq!(
            project_from_tracker_path(projects, Path::new("/elsewhere/ljos")),
            None
        );
    }
}
