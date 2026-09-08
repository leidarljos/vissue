//! Read-only verbs. Every function returns its text instead of printing, so a
//! CLI, an MCP server, and a library caller share one implementation.

use anyhow::anyhow;

use crate::error::{Error, Result};
use chrono::{Local, NaiveDate};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

use crate::catalog::{CatalogService, load_recs};
use crate::config::Layout;
use crate::graph::DependencyGraph;
use crate::model::{IssueHeading, READY_STATES};
pub use crate::related::related;
use crate::store::{IssueDoc, find_by_id, find_org_ids, list_projects, load_all, project_selected};
use crate::views::{IssueRec, IssueRow, ListQuery};

struct GraphIndex<'a> {
    by_id: HashMap<&'a str, &'a IssueHeading>,
    children: HashMap<&'a str, Vec<&'a str>>,
    blockers: HashMap<&'a str, Vec<&'a str>>,
}

impl<'a> GraphIndex<'a> {
    fn new(all: &'a [(String, IssueHeading)]) -> Self {
        let mut index = Self {
            by_id: HashMap::with_capacity(all.len()),
            children: HashMap::new(),
            blockers: HashMap::new(),
        };
        for (_, h) in all {
            index.by_id.insert(h.id.as_str(), h);
        }
        for (_, h) in all {
            if let Some(parent) = h.parent() {
                index
                    .children
                    .entry(parent)
                    .or_default()
                    .push(h.id.as_str());
            }
            let blockers = blocker_ids(h);
            if !blockers.is_empty() {
                index.blockers.insert(h.id.as_str(), blockers);
            }
        }
        for children in index.children.values_mut() {
            children.sort_unstable();
        }
        index
    }
}

fn blocker_ids(h: &IssueHeading) -> Vec<&str> {
    let mut ids = Vec::new();
    if let Some(raw) = crate::props::get(&h.properties, crate::props::BLOCKED_BY) {
        ids.extend(
            raw.split(|c: char| c == ',' || c.is_whitespace())
                .map(str::trim)
                .filter(|id| !id.is_empty()),
        );
    }
    if let Some(raw) = h.properties.get("BLOCKER") {
        if crate::org::is_edna_blocker(raw) {
            ids.extend(crate::org::edna_blocker_id_refs(raw));
        } else {
            ids.extend(
                raw.split(|c: char| c == ',' || c.is_whitespace())
                    .map(str::trim)
                    .filter(|id| !id.is_empty()),
            );
        }
    }
    let mut unique = Vec::new();
    for id in ids {
        if !unique.contains(&id) {
            unique.push(id);
        }
    }
    unique
}

/// One row per issue: id, state, priority cookie, title.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
pub fn list(
    layout: &Layout,
    project_filter: Option<&str>,
    state_filter: Option<&str>,
    ready_only: bool,
) -> Result<String> {
    let recs = load_recs(layout)?;
    let rows = CatalogService::from_recs(&recs).issues_rows(ListQuery {
        project: project_filter.map(str::to_string),
        state: state_filter.map(str::to_string),
        ready: ready_only,
        ..ListQuery::default()
    })?;
    Ok(format_issue_rows(&recs, &rows))
}

fn format_issue_rows(recs: &[IssueRec], rows: &[IssueRow]) -> String {
    let mut out = String::new();
    for row in rows {
        let suffix = recs
            .iter()
            .find(|r| r.heading.id == row.id)
            .map(|r| claim_suffix(&r.heading))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "{:<22} {:<9} [#{}]  {}{}",
            row.id, row.state, row.priority, row.title, suffix
        );
    }
    out
}

/// ` (claimed 3d by <identity>)`, or nothing when no one holds the issue.
/// Only a claimed issue grows the suffix, so an unclaimed corpus renders
/// exactly as it did before claims existed.
pub(crate) fn claim_suffix(h: &IssueHeading) -> String {
    let Some(who) = h.claimed_by() else {
        return String::new();
    };
    match h.claim_age_days(Local::now().date_naive()) {
        Some(days) => format!("  (claimed {days}d by {who})"),
        None => format!("  (claimed by {who})"),
    }
}

/// Actionable issues: TODO or STARTED with no open blocker.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
pub fn ready(layout: &Layout, project_filter: Option<&str>) -> Result<String> {
    let recs = load_recs(layout)?;
    let rows = CatalogService::from_recs(&recs).ready(project_filter)?;
    Ok(format_issue_rows(&recs, &rows))
}

/// One issue's metadata, file range, and body text.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read, or `id` is not in it.
pub fn show(layout: &Layout, id: &str) -> Result<String> {
    let (h, path, project) =
        find_by_id(layout, id)?.ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
    let mut out = String::new();
    writeln!(out, "ID:       {}", h.id)?;
    writeln!(out, "Project:  {project}")?;
    writeln!(out, "Title:    {}", h.title)?;
    writeln!(out, "State:    {}", h.state)?;
    writeln!(out, "Priority: [#{}]", h.priority)?;
    if let Some(who) = h.claimed_by() {
        match h.claim_age_days(Local::now().date_naive()) {
            Some(days) => writeln!(
                out,
                "Claimed:  {who} since {} ({days}d)",
                h.claimed_at().unwrap_or("?")
            )?,
            None => writeln!(out, "Claimed:  {who}")?,
        }
    }
    let settings = crate::org::tag_settings_from_preamble(
        &IssueDoc::parse_file(&project, &path)
            .map(|d| d.preamble)
            .unwrap_or_default(),
    );
    let tags = settings.all_tags(&h.tags());
    if !tags.is_empty() {
        writeln!(out, "Tags:     {}", tags.join(", "))?;
    }
    if h.properties.iter().any(|(k, _)| k != "ID") {
        writeln!(out, "Properties:")?;
        for (k, v) in &h.properties {
            if k == "ID" {
                continue;
            }
            writeln!(out, "  {k}: {v}")?;
        }
    }
    writeln!(
        out,
        "File:     {}:{}-{}",
        path.display(),
        h.line_start,
        h.line_end
    )?;
    writeln!(out)?;
    // The body is what the issue actually asks for, so printing the file
    // range and stopping leaves every reader to go fetch it by hand.
    let body = h.body.trim_end();
    if body.is_empty() {
        writeln!(out, "(no body; edit the range above to add one)")?;
    } else {
        writeln!(out, "Body:")?;
        writeln!(out, "{body}")?;
    }
    Ok(out)
}

/// What a plan's children hold, child by child.
///
/// Deliberately not a number. The design note that settled this is in the
/// vault; the short version is that no weighting over children can be picked
/// without a judgement the tracker has no basis for, a child that settled split
/// has no single position to fold in, and a child nobody voted on is absent
/// rather than neutral. Rolling those into one figure would hide exactly the
/// rows a person has to go read.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read, `id` is not in it, or the
/// configuration names a weight the iteration cannot use.
pub fn plan_consensus(layout: &Layout, id: &str) -> Result<String> {
    let roll = crate::consensus::of_plan(layout, id)?;
    let mut out = String::new();
    writeln!(out, "{}  {}", roll.plan, roll.title)?;
    if roll.children.is_empty() {
        writeln!(out, "  no children: nothing to roll up")?;
        return Ok(out);
    }

    let voted = roll.children.iter().filter(|c| c.ballots > 0).count();
    writeln!(
        out,
        "  {} child{}, {voted} with ballots",
        roll.children.len(),
        if roll.children.len() == 1 { "" } else { "ren" }
    )?;
    for child in &roll.children {
        let held = match (&child.holds, child.settling) {
            (Some((choice, share)), _) => format!("{choice} {share:.3}"),
            (None, Some(crate::consensus::Settling::Split)) => "split".to_string(),
            (None, Some(crate::consensus::Settling::Oscillating)) => "never settles".to_string(),
            (None, Some(_)) => "no lead".to_string(),
            (None, None) => "no ballots".to_string(),
        };
        writeln!(
            out,
            "    {:<22} {:<9} {:<16} {}",
            child.id, child.state, held, child.title
        )?;
    }

    let positions = roll.positions();
    match positions.len() {
        0 => writeln!(out, "  nothing holds a position yet")?,
        1 => writeln!(
            out,
            "  the children that were voted on all hold {}",
            positions[0]
        )?,
        n => writeln!(
            out,
            "  the children disagree with each other: {n} positions ({})",
            positions.join(", ")
        )?,
    }
    let split = roll.split();
    if !split.is_empty() {
        writeln!(
            out,
            "  {} child(ren) settled split and need a person: {}",
            split.len(),
            split
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )?;
    }
    let unvoted = roll.unvoted();
    if !unvoted.is_empty() {
        // Named rather than counted into an average. Most work is done rather
        // than argued over, so this is the common row and folding it in as a
        // neutral vote would make the plan's position mostly fiction.
        writeln!(out, "  {} child(ren) carry no ballots", unvoted.len())?;
    }
    Ok(out)
}

/// The working set for one issue: the plan around it, the deeds its declared
/// inputs produced, and what it has produced itself.
///
/// This is the layer between the task graph and the work: the tracker already
/// records what a node waits on, so what an agent should open before starting is
/// derivable rather than searchable. Nothing is ranked and nothing is embedded.
/// A neighbourhood by resemblance is a different question and `related` answers
/// it.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read, `id` is not in it, or the
/// blocker graph cannot be built.
pub fn recall(layout: &Layout, id: &str, depth: usize, excerpts: bool) -> Result<String> {
    let set = CatalogService::from_recs(&load_recs(layout)?).recall(id, depth, excerpts)?;
    let mut out = String::new();
    writeln!(
        out,
        "{:<22} {:<9} {}  ({})",
        set.id, set.state, set.title, set.project
    )?;

    if !set.plan.is_empty() {
        writeln!(out, "\nPlan")?;
        for step in &set.plan {
            writeln!(out, "  {:<22} {:<9} {}", step.id, step.state, step.title)?;
        }
    }

    writeln!(out, "\nInputs")?;
    if set.inputs.is_empty() {
        writeln!(
            out,
            "  (none declared: nothing blocks this and it was not bounced)"
        )?;
    }
    for input in &set.inputs {
        writeln!(
            out,
            "  {:<22} {:<9} {}  [{}]",
            input.id, input.state, input.title, input.relation
        )?;
        if input.deeds.is_empty() {
            // Said rather than left blank. An input that produced nothing is the
            // case where this view has nothing to hand over, and a silent gap
            // reads as though the walk missed it.
            writeln!(out, "    (no deeds cited)")?;
        }
        for deed in &input.deeds {
            writeln!(out, "    {deed}")?;
        }
        if let Some(excerpt) = &input.excerpt {
            // Indented under its input, so a working set carrying several
            // stays readable as a list rather than running together.
            for line in excerpt.lines() {
                writeln!(out, "      {line}")?;
            }
        }
        if let Some(note) = &input.last_note {
            // The last thing said about an input is what a reader falls back on
            // when it named no product.
            writeln!(
                out,
                "    note: {}",
                note.lines().next().unwrap_or_default().trim()
            )?;
        }
    }

    writeln!(out, "\nProduced")?;
    if set.produced.is_empty() {
        writeln!(out, "  (nothing cited yet)")?;
    }
    for deed in &set.produced {
        writeln!(out, "  {deed}")?;
    }

    writeln!(out, "\nBody")?;
    if set.body.is_empty() {
        writeln!(out, "  (no body)")?;
    } else {
        for line in set.body.lines() {
            writeln!(out, "  {line}")?;
        }
    }
    Ok(out)
}

/// Just the deed accessions [`recall`] found, inputs first, one per line.
///
/// The form a shell substitutes: `deedar get $(vissue recall <id> --deeds-only)`
/// opens the working set without a parser in between.
///
/// # Errors
///
/// Same as [`recall`].
pub fn recall_deeds(layout: &Layout, id: &str, depth: usize) -> Result<String> {
    let set = CatalogService::from_recs(&load_recs(layout)?).recall(id, depth, false)?;
    let mut out = String::new();
    // One line per deed even when two nodes cite the same one, which happens
    // whenever work continues on the product it was handed. The consumer is a
    // shell substitution, so a repeat would fetch or check the same deed twice.
    let mut seen: HashSet<&str> = HashSet::new();
    for deed in set
        .inputs
        .iter()
        .flat_map(|i| i.deeds.iter())
        .chain(set.produced.iter())
    {
        if seen.insert(deed.as_str()) {
            writeln!(out, "{deed}")?;
        }
    }
    Ok(out)
}

/// The DeGroot consensus over an issue's ballots, weighted by who the group
/// listens to.
///
/// `vote` counts; this weighs. Both are printed, because the useful thing about
/// the weighted answer is where it differs from the count, and a reader shown
/// only one of them cannot tell whether the trust configuration did anything.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read, `id` is not in it, or the
/// configuration names a weight the iteration cannot use.
pub fn consensus(layout: &Layout, id: &str) -> Result<String> {
    let ballots = crate::ops::ballots(layout, id)?;
    let outcome = crate::consensus::of_issue(layout, id)?;
    Ok(consensus_text(id, &ballots, &outcome))
}

fn consensus_text(
    id: &str,
    ballots: &[crate::ops::Ballot],
    outcome: &crate::consensus::Outcome,
) -> String {
    use crate::consensus::{Settling, TrustSource};

    if ballots.is_empty() {
        return format!("{id}: no votes\n");
    }
    let mut out = format!(
        "{id}: {} ballot{} over {} option{}, trust {}\n",
        ballots.len(),
        if ballots.len() == 1 { "" } else { "s" },
        outcome.choices.len(),
        if outcome.choices.len() == 1 { "" } else { "s" },
        match outcome.trust {
            TrustSource::Default => "default (equal weight)",
            TrustSource::Configured => "configured",
        }
    );

    let counts = crate::consensus::tally(ballots);
    let mut ranked: Vec<(&String, &Vec<String>)> = counts.iter().collect();
    ranked.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));
    let _ = writeln!(out, "  count");
    for (choice, who) in &ranked {
        let _ = writeln!(out, "    {:<24} {} ({})", choice, who.len(), who.join(", "));
    }

    match outcome.settling {
        Settling::Agreed => {
            let consensus = outcome.consensus.as_ref().expect("agreed carries a limit");
            let mut shares: Vec<(&str, f64)> = outcome
                .choices
                .iter()
                .map(String::as_str)
                .zip(consensus.iter().copied())
                .collect();
            shares.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
            let _ = writeln!(
                out,
                "  consensus after {} round(s){}",
                outcome.rounds,
                if outcome.budget_reached {
                    ", which is the whole budget: the shares are an estimate"
                } else {
                    ""
                }
            );
            for (choice, share) in &shares {
                let _ = writeln!(out, "    {choice:<24} {share:.3}");
            }
            let mut power: Vec<(&str, f64)> = outcome
                .agents
                .iter()
                .map(|a| (a.agent.as_str(), a.power.unwrap_or_default()))
                .collect();
            power.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
            let _ = writeln!(out, "  social power");
            for (agent, weight) in &power {
                let _ = writeln!(out, "    {agent:<24} {weight:.3}");
            }
            match outcome.leader() {
                // One agent agreeing with itself is not agreement, and the
                // weighted answer is exactly as unchecked as the count was.
                Some(_) if ballots.len() < 2 => {
                    let _ = writeln!(
                        out,
                        "  one ballot only: {}, which nobody has agreed with yet",
                        ranked[0].0
                    );
                }
                Some((choice, share)) => {
                    let _ = writeln!(out, "  holds: {choice} ({share:.3} of the group's weight)");
                    if ranked[0].0 != choice {
                        // The whole reason to weigh rather than count.
                        let _ = writeln!(
                            out,
                            "  the count leads with {} and the group's weight does not",
                            ranked[0].0
                        );
                    }
                }
                None => {
                    let _ = writeln!(
                        out,
                        "  no lead: the group's weight is split evenly across the options"
                    );
                }
            }
        }
        Settling::Split => {
            let _ = writeln!(
                out,
                "  no consensus: the trust graph holds {} group(s) that do not listen to each other",
                outcome.factions.len()
            );
            for faction in &outcome.factions {
                // What a group settled on is the actionable half of a split.
                // The members share a limit, so the first of them speaks for
                // the group.
                let held = faction
                    .first()
                    .and_then(|who| outcome.agents.iter().find(|a| a.agent == *who))
                    .and_then(|row| {
                        row.limit
                            .iter()
                            .enumerate()
                            .max_by(|a, b| a.1.total_cmp(b.1))
                            .map(|(at, share)| format!("{} {share:.3}", outcome.choices[at]))
                    })
                    .unwrap_or_default();
                let _ = writeln!(out, "    {:<32} {held}", faction.join(", "));
            }
        }
        Settling::Anchored => {
            // Under an anchor there is no single position to report, and saying
            // one would name a position none of them holds. What each agent
            // landed on, and how far apart they stayed, is the result.
            // The susceptibility is a diagonal, so it goes on the row when the
            // agents differ and on the header when they do not. Printing one
            // number over rows that used several would be the wrong number for
            // all but one of them.
            let uniform = outcome
                .agents
                .windows(2)
                .all(|pair| (pair[0].susceptibility - pair[1].susceptibility).abs() < f64::EPSILON);
            if uniform {
                let _ = writeln!(
                    out,
                    "  anchored after {} round(s), susceptibility {:.2}",
                    outcome.rounds,
                    outcome
                        .agents
                        .first()
                        .map_or(outcome.susceptibility, |a| a.susceptibility)
                );
            } else {
                let _ = writeln!(out, "  anchored after {} round(s)", outcome.rounds);
            }
            for row in &outcome.agents {
                let held = row
                    .limit
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(b.1))
                    .map(|(at, share)| format!("{} {share:.3}", outcome.choices[at]))
                    .unwrap_or_default();
                if uniform {
                    let _ = writeln!(out, "    {:<24} {held}", row.agent);
                } else {
                    let _ = writeln!(
                        out,
                        "    {:<24} {held:<16} susceptibility {:.2}",
                        row.agent, row.susceptibility
                    );
                }
            }
            let _ = writeln!(
                out,
                "  spread {:.3}: what the group keeps disagreeing about after listening",
                outcome.spread
            );
            // The unweighted mean across agents. Named as what it is: an
            // average of positions, not a position anybody argued for.
            let mut mean: Vec<(&str, f64)> = outcome
                .choices
                .iter()
                .enumerate()
                .map(|(at, choice)| {
                    let total: f64 = outcome.agents.iter().map(|a| a.limit[at]).sum();
                    (choice.as_str(), total / outcome.agents.len() as f64)
                })
                .collect();
            mean.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
            let _ = writeln!(out, "  mean of those positions");
            for (choice, share) in &mean {
                let _ = writeln!(out, "    {choice:<24} {share:.3}");
            }
        }
        Settling::Oscillating => {
            let _ = writeln!(
                out,
                "  no consensus: {} rounds did not settle, which is a trust graph with no \
                 weight on its own opinions",
                outcome.rounds
            );
        }
    }
    out
}

/// Case-insensitive substring scan over id, title, properties, and body. Linear
/// in the corpus, which is the right cost until the issue count climbs.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
pub fn search(layout: &Layout, query: &str, limit: usize) -> Result<String> {
    let recs = load_recs(layout)?;
    let hits = CatalogService::from_recs(&recs).search(query, limit)?;
    let mut out = String::new();
    for h in hits {
        let _ = writeln!(
            out,
            "{:<22} {:<9} [#{}]  {}  ({})",
            h.id, h.state, h.priority, h.title, h.project
        );
    }
    Ok(out)
}

/// Issues whose `:PARENT:` points at `parent_id`.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
pub fn children(layout: &Layout, parent_id: &str) -> Result<String> {
    let mut rows: Vec<(String, IssueHeading)> = load_all(layout)?
        .into_iter()
        .filter(|(_, h)| h.parent() == Some(parent_id))
        .collect();
    rows.sort_by(|a, b| {
        a.1.priority
            .cmp(&b.1.priority)
            .then_with(|| a.1.state.cmp(&b.1.state))
            .then_with(|| a.1.id.cmp(&b.1.id))
    });
    let mut out = String::new();
    for (project, h) in rows {
        let _ = writeln!(
            out,
            "{:<22} {:<9} [#{}]  {}  ({})",
            h.id, h.state, h.priority, h.title, project
        );
    }
    Ok(out)
}

/// Open issues whose `:CREATED:` is at least `days` old. An issue without a
/// parseable date is never stale, because its age is unknown.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
pub fn stale(layout: &Layout, days: i64, project_filter: Option<&str>) -> Result<String> {
    let today = Local::now().date_naive();
    let cutoff = today - chrono::Duration::days(days);
    let mut rows: Vec<(String, IssueHeading, NaiveDate)> = Vec::new();
    for (project, h) in load_all(layout)? {
        if !project_selected(&project, project_filter) {
            continue;
        }
        if !READY_STATES.contains(&h.state.as_str()) {
            continue;
        }
        let Some(created) = h.properties.get("CREATED") else {
            continue;
        };
        let Some(parsed) = parse_org_date(created) else {
            continue;
        };
        if parsed <= cutoff {
            rows.push((project, h, parsed));
        }
    }
    rows.sort_by_key(|r| r.2);
    let mut out = String::new();
    for (project, h, created) in rows {
        let age = (today - created).num_days();
        let _ = writeln!(
            out,
            "{:<22} {:<9} [#{}]  {} ({}d, {})",
            h.id, h.state, h.priority, h.title, age, project
        );
    }
    Ok(out)
}

/// Every live claim, oldest first: the who-holds-what view. A claim is live
/// while its issue is STARTED or BLOCKED (release happens on TODO, DONE, or
/// CANCELLED), so this is the working set, not history.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read, or JSON serialization fails
/// when `json` is set.
pub fn claims(
    layout: &Layout,
    holder_filter: Option<&str>,
    project_filter: Option<&str>,
    json: bool,
) -> Result<String> {
    let recs = load_recs(layout)?;
    let rows = CatalogService::from_recs(&recs).claims(holder_filter, project_filter)?;

    if json {
        return Ok(format!("{}\n", serde_json::to_value(&rows)?));
    }

    let mut out = String::new();
    for row in &rows {
        let age_txt = if row.age_days < 0 {
            "?d".to_string()
        } else {
            format!("{}d", row.age_days)
        };
        let _ = writeln!(
            out,
            "{:<22} {:<9} [#{}]  {:>4}  {}  {} ({})",
            row.id,
            row.state,
            row.priority,
            age_txt,
            row.holder.as_deref().unwrap_or("?"),
            row.title,
            row.project
        );
    }
    if rows.is_empty() {
        out.push_str("no live claims\n");
    }
    Ok(out)
}

/// Dated open work in the next `days` days, plus anything already overdue.
/// One line per (issue, date kind): deadlines first within a day, soonest day
/// first, overdue on top with a negative day count.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
pub fn agenda(layout: &Layout, days: i64, project_filter: Option<&str>) -> Result<String> {
    let today = Local::now().date_naive();
    let horizon = today + chrono::Duration::days(days);
    // kind sorts D before S so a same-day deadline outranks a scheduled start.
    let mut rows: Vec<(NaiveDate, char, String, IssueHeading)> = Vec::new();
    for (project, h) in load_all(layout)? {
        if !project_selected(&project, project_filter) {
            continue;
        }
        if !READY_STATES.contains(&h.state.as_str()) && h.state != "BLOCKED" {
            continue;
        }
        for (kind, value) in [('D', h.deadline()), ('S', h.scheduled())] {
            let Some(parsed) = value.and_then(parse_org_date) else {
                continue;
            };
            if parsed <= horizon {
                rows.push((parsed, kind, project.clone(), h.clone()));
            }
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.3.id.cmp(&b.3.id)));

    let mut out = String::new();
    for (date, kind, project, h) in rows {
        let delta = (date - today).num_days();
        let when = match delta {
            d if d < 0 => format!("{}d overdue", -d),
            0 => "today".to_string(),
            d => format!("in {d}d"),
        };
        let label = if kind == 'D' { "deadline" } else { "scheduled" };
        let _ = writeln!(
            out,
            "{date}  {label:<9} {when:<11} {:<22} {:<9} [#{}]  {}  ({})",
            h.id, h.state, h.priority, h.title, project
        );
    }
    if out.is_empty() {
        out.push_str("nothing dated in range\n");
    }
    Ok(out)
}

pub(crate) fn parse_org_date(s: &str) -> Option<NaiveDate> {
    let inner = s
        .trim_start_matches(['<', '['])
        .trim_end_matches(['>', ']']);
    let token = inner.split_whitespace().next()?;
    NaiveDate::parse_from_str(token, "%Y-%m-%d").ok()
}

/// The matching issue count and nothing else, for shell pipelines.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
pub fn count(
    layout: &Layout,
    project_filter: Option<&str>,
    state_filter: Option<&str>,
    ready_only: bool,
) -> Result<String> {
    let all = load_all(layout)?;
    let active_blockers: HashSet<String> = if ready_only {
        all.iter()
            .filter(|(_, h)| h.state != "DONE" && h.state != "CANCELLED")
            .map(|(_, h)| h.id.clone())
            .collect()
    } else {
        HashSet::new()
    };
    let n = all
        .iter()
        .filter(|(project, h)| {
            if !project_selected(project, project_filter) {
                return false;
            }
            if let Some(s) = state_filter
                && h.state != s
            {
                return false;
            }
            if ready_only {
                if !READY_STATES.contains(&h.state.as_str()) {
                    return false;
                }
                if blocker_ids(h).iter().any(|b| active_blockers.contains(*b)) {
                    return false;
                }
            }
            true
        })
        .count();
    Ok(format!("{n}\n"))
}

/// One JSON object per line: every property, the logbook, the body, and the
/// file line range. Round-trippable, and the seam other tools consume.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
pub fn export(layout: &Layout, project_filter: Option<&str>) -> Result<String> {
    let mut out = String::new();
    for rec in load_recs(layout)? {
        if !project_selected(&rec.project, project_filter) {
            continue;
        }
        let _ = writeln!(
            out,
            "{}",
            export_row(&rec.project, rec.heading, &rec.tag_settings)
        );
    }
    Ok(out)
}

/// The same lines as [`export`], grouped by project, from one read.
///
/// `export` filters a whole-corpus read down to one project, so digesting
/// every project separately re-read the corpus once per project: quadratic
/// in the project count, and six seconds on a tracker with a hundred of
/// them. The rows are built by the same function, so a project's text here
/// is byte for byte what `export(layout, Some(project))` returns, and the
/// digests taken from it do not move.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
pub fn export_by_project(layout: &Layout) -> Result<BTreeMap<String, String>> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for rec in load_recs(layout)? {
        let row = export_row(&rec.project, rec.heading, &rec.tag_settings);
        let _ = writeln!(out.entry(rec.project).or_default(), "{row}");
    }
    Ok(out)
}

fn export_row(
    project: &str,
    h: IssueHeading,
    settings: &crate::org::TagSettings,
) -> serde_json::Value {
    let logbook: Vec<serde_json::Value> = h
        .logbook
        .iter()
        .map(|e| {
            let mut row = serde_json::json!({
                "timestamp": e.timestamp,
                "from": e.from_state,
                "to": e.to_state,
                "note": e.note,
            });
            if let Some(raw) = &e.raw {
                row["raw"] = serde_json::Value::String(raw.clone());
            }
            row
        })
        .collect();
    serde_json::json!({
        "id": h.id,
        "project": project,
        "title": h.title,
        "state": h.state,
        "priority": h.priority.to_string(),
        "properties": h.properties,
        // Typed beside the drawer rather than only inside it, so a consumer of
        // the export reads the field the socket already hands over typed
        // instead of splitting a drawer string on whichever separator the
        // author happened to use. `properties` keeps `:DEEDS:` as well: a
        // reader that wants the drawer verbatim should still get it.
        "deeds": h.deeds(),
        "org_tags": h.org_tags,
        "tags": h.tags(),
        "all_tags": settings.all_tags(&h.tags()),
        "logbook": logbook,
        "body": h.body,
        "line_start": h.line_start,
        "line_end": h.line_end,
    })
}

/// Children and blockers below `root_id`, as indented text or Graphviz DOT.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read, `root_id` is not in it, or
/// `format` is not `ascii`, `text`, or `dot`.
pub fn tree(layout: &Layout, root_id: &str, format: &str) -> Result<String> {
    let all = load_all(layout)?;
    let graph = GraphIndex::new(&all);
    let Some(root_heading) = graph.by_id.get(root_id) else {
        return Err(Error::IssueNotFound {
            id: root_id.to_string(),
        });
    };
    let mut out = String::new();
    let root = root_heading.id.as_str();
    match format {
        "ascii" | "text" => tree_ascii(&graph, root, 0, &mut HashSet::new(), &mut out),
        "dot" => tree_dot(&graph, root, &mut out),
        _ => return Err(anyhow!("unknown format {format:?}; allowed: ascii, dot").into()),
    }
    Ok(out)
}

fn tree_ascii<'a>(
    graph: &GraphIndex<'a>,
    id: &'a str,
    depth: usize,
    seen: &mut HashSet<&'a str>,
    out: &mut String,
) {
    if !seen.insert(id) {
        let _ = writeln!(out, "{}{id} (cycle, stopping)", "  ".repeat(depth));
        return;
    }
    let Some(h) = graph.by_id.get(id) else {
        let _ = writeln!(out, "{}{id} (missing)", "  ".repeat(depth));
        return;
    };
    let _ = writeln!(
        out,
        "{}{id} {:<9} [#{}]  {}",
        "  ".repeat(depth),
        h.state,
        h.priority,
        h.title
    );
    if let Some(blockers) = graph.blockers.get(id) {
        for blocker in blockers {
            let _ = writeln!(out, "{}* blocked-by {blocker}", "  ".repeat(depth + 1));
        }
    }
    if let Some(kids) = graph.children.get(id) {
        for k in kids {
            tree_ascii(graph, k, depth + 1, seen, out);
        }
    }
}

/// Escape text for a Graphviz quoted string. Backslash goes first, or the
/// escape introduced for a quote is itself re-escaped; a raw newline would end
/// the statement early. Titles and ids are whatever someone committed to the
/// tracker, so neither is trusted here.
pub(crate) fn dot_quoted(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "")
}

fn tree_dot<'a>(graph: &GraphIndex<'a>, root_id: &str, out: &mut String) {
    let _ = writeln!(out, "digraph vissue_tree {{");
    let _ = writeln!(out, "  rankdir=LR;");
    let _ = writeln!(
        out,
        "  node [shape=box, fontname=\"Jost\", style=filled, fillcolor=\"#E0F2F1\"];"
    );
    let mut visited: HashSet<&str> = HashSet::new();
    let mut stack = vec![graph.by_id.get(root_id).unwrap().id.as_str()];
    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        if let Some(h) = graph.by_id.get(id) {
            let _ = writeln!(
                out,
                "  \"{}\" [label=\"{}\\n{} [#{}]\"];",
                dot_quoted(&h.id),
                dot_quoted(&h.title),
                dot_quoted(&h.state),
                dot_quoted(&h.priority.to_string())
            );
            if let Some(kids) = graph.children.get(id) {
                for k in kids {
                    let _ = writeln!(
                        out,
                        "  \"{}\" -> \"{}\" [color=\"#00897B\"];",
                        dot_quoted(&h.id),
                        dot_quoted(k)
                    );
                    stack.push(k);
                }
            }
            if let Some(blockers) = graph.blockers.get(id) {
                for b in blockers {
                    let _ = writeln!(
                        out,
                        "  \"{}\" -> \"{}\" [style=dashed, color=\"#FF7043\", label=\"blocks\"];",
                        dot_quoted(b),
                        dot_quoted(&h.id)
                    );
                    stack.push(b);
                }
            }
        }
    }
    let _ = writeln!(out, "}}");
}

/// Cycles in the blocker graph, one per line, or a line saying there are none.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
pub fn cycles(layout: &Layout) -> Result<String> {
    let all = load_all(layout)?;
    let graph = GraphIndex::new(&all);

    // Colored depth-first search over BLOCKED_BY edges. Grey marks the
    // current stack, black a finished node, so a shared blocker reached
    // from two branches (a diamond) is never mistaken for a cycle.
    const WHITE: u8 = 0;
    const GREY: u8 = 1;
    const BLACK: u8 = 2;
    let mut color: HashMap<&str, u8> = HashMap::new();
    let mut found: Vec<Vec<String>> = Vec::new();

    fn dfs<'a>(
        id: &'a str,
        graph: &GraphIndex<'a>,
        color: &mut HashMap<&'a str, u8>,
        path: &mut Vec<&'a str>,
        found: &mut Vec<Vec<String>>,
    ) {
        color.insert(id, GREY);
        path.push(id);
        if let Some(blockers) = graph.blockers.get(id) {
            for b in blockers {
                if !graph.by_id.contains_key(b) {
                    continue; // a broken edge cannot close a loop; `check` reports it
                }
                match color.get(b).copied().unwrap_or(WHITE) {
                    GREY => {
                        let start = path.iter().position(|&x| x == *b).unwrap();
                        let mut cycle: Vec<String> =
                            path[start..].iter().map(|s| s.to_string()).collect();
                        // Rotate so the smallest id leads: one canonical form
                        // per cycle no matter where the walk entered it.
                        let min = cycle
                            .iter()
                            .enumerate()
                            .min_by(|a, b| a.1.cmp(b.1))
                            .map(|(i, _)| i)
                            .unwrap();
                        cycle.rotate_left(min);
                        cycle.push(cycle[0].clone());
                        if !found.contains(&cycle) {
                            found.push(cycle);
                        }
                    }
                    WHITE => dfs(b, graph, color, path, found),
                    _ => {}
                }
            }
        }
        path.pop();
        color.insert(id, BLACK);
    }

    for (_, start) in &all {
        if color.get(start.id.as_str()).copied().unwrap_or(WHITE) == WHITE {
            let mut path = Vec::new();
            dfs(start.id.as_str(), &graph, &mut color, &mut path, &mut found);
        }
    }

    let mut out = String::new();
    if found.is_empty() {
        let _ = writeln!(out, "no cycles");
    } else {
        for cycle in found {
            let _ = writeln!(out, "{}", cycle.join(" -> "));
        }
    }
    Ok(out)
}

/// Transitive blocker ancestors, limited to a bounded number of hops.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read, the blocker graph cannot be
/// built, or `id` is not in it.
pub fn ancestors(layout: &Layout, id: &str, depth: usize) -> Result<String> {
    let graph = DependencyGraph::from_issues(&load_all(layout)?)?;
    let mut out = String::new();
    for (distance, ancestor) in graph.ancestors(id, depth)? {
        writeln!(out, "{distance} {ancestor}")?;
    }
    Ok(out)
}

/// Transitive issues waiting on this issue, limited to a bounded number of hops.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read, the blocker graph cannot be
/// built, or `id` is not in it.
pub fn impact(layout: &Layout, id: &str, depth: usize) -> Result<String> {
    let graph = DependencyGraph::from_issues(&load_all(layout)?)?;
    let mut out = String::new();
    for (distance, descendant) in graph.descendants(id, depth)? {
        writeln!(out, "{distance} {descendant}")?;
    }
    Ok(out)
}

/// The whole blocker and parent graph as Graphviz DOT. Node fill encodes state.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
/// The lines a DOT document opens with, up to and including the graph
/// attributes. Exposed for the same reason as [`ROADMAP_HEADER`]: a caller
/// drawing several projects as one graph writes them once. Concatenating whole
/// documents gives `dot` a file of many graphs, and it renders the first.
pub const GRAPH_HEADER: &str = concat!(
    "digraph vissue_graph {\n",
    "  rankdir=LR;\n",
    "  node [shape=box, fontname=\"Jost\", style=filled];\n",
    "  edge [fontname=\"Jost\"];\n"
);

/// The line that closes a DOT document.
pub const GRAPH_FOOTER: &str = "}\n";

/// A DOT graph of the corpus: one node per issue, blocker and parent edges.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
pub fn graph(layout: &Layout, project_filter: Option<&str>) -> Result<String> {
    Ok(format!(
        "{GRAPH_HEADER}{}{GRAPH_FOOTER}",
        graph_body(layout, project_filter)?
    ))
}

/// The nodes and edges of the graph, without the enclosing `digraph` block.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
pub fn graph_body(layout: &Layout, project_filter: Option<&str>) -> Result<String> {
    let all = load_all(layout)?;
    let graph = GraphIndex::new(&all);
    let mut out = String::new();
    for (project, h) in &all {
        if !project_selected(project, project_filter) {
            continue;
        }
        let fill = match h.state.as_str() {
            "DONE" => "#A5D6A7",
            "CANCELLED" => "#CFD8DC",
            "BLOCKED" => "#FFCC80",
            "STARTED" => "#80CBC4",
            _ => "#E0F2F1",
        };
        let _ = writeln!(
            out,
            "  \"{}\" [label=\"{}\\n{} [#{}]\", fillcolor=\"{}\"];",
            dot_quoted(&h.id),
            dot_quoted(&h.title),
            dot_quoted(&h.state),
            dot_quoted(&h.priority.to_string()),
            fill
        );
    }
    for (project, h) in &all {
        if !project_selected(project, project_filter) {
            continue;
        }
        if let Some(blockers) = graph.blockers.get(h.id.as_str()) {
            for b in blockers {
                writeln!(
                    out,
                    "  \"{}\" -> \"{}\" [color=\"#FF7043\"];",
                    dot_quoted(b),
                    dot_quoted(&h.id)
                )?;
            }
        }
        if let Some(parent) = h.parent() {
            writeln!(
                out,
                "  \"{}\" -> \"{}\" [color=\"#00897B\", style=dashed];",
                dot_quoted(parent),
                dot_quoted(&h.id)
            )?;
        }
    }
    Ok(out)
}

/// A markdown roadmap grouped by project and state. Closed items collapse into
/// one section so the document stays about live work.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
/// The document furniture a roadmap opens with. Exposed because a caller that
/// assembles one roadmap out of several projects writes it once rather than once
/// per project: concatenating whole roadmaps puts a title above every project
/// section, so a corpus of six carries six titles in one document.
pub const ROADMAP_HEADER: &str = concat!(
    "# Roadmap\n\n",
    "Generated from `vissue roadmap`. Source of truth lives in the per-project issues.org files.\n\n"
);

/// A markdown roadmap of active and closed work, with its title.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
pub fn roadmap(layout: &Layout, project_filter: Option<&str>) -> Result<String> {
    Ok(format!(
        "{ROADMAP_HEADER}{}",
        roadmap_body(layout, project_filter)?
    ))
}

/// The roadmap's project sections, without the document title.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
pub fn roadmap_body(layout: &Layout, project_filter: Option<&str>) -> Result<String> {
    let all = load_all(layout)?;
    let mut by_project: BTreeMap<String, Vec<&IssueHeading>> = BTreeMap::new();
    for (project, h) in &all {
        if !project_selected(project, project_filter) {
            continue;
        }
        by_project.entry(project.clone()).or_default().push(h);
    }
    let mut out = String::new();
    for (project, mut headings) in by_project {
        headings.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| a.state.cmp(&b.state))
                .then_with(|| a.id.cmp(&b.id))
        });
        let buckets = ["STARTED", "TODO", "BLOCKED"];
        let active: Vec<&&IssueHeading> = headings
            .iter()
            .filter(|h| buckets.contains(&h.state.as_str()))
            .collect();
        let closed: Vec<&&IssueHeading> = headings
            .iter()
            .filter(|h| h.state == "DONE" || h.state == "CANCELLED")
            .collect();
        if active.is_empty() && closed.is_empty() {
            continue;
        }
        writeln!(out, "## {project}")?;
        writeln!(out)?;
        for state in buckets {
            let in_state: Vec<&&IssueHeading> = active
                .iter()
                .copied()
                .filter(|h| h.state == state)
                .collect();
            if in_state.is_empty() {
                continue;
            }
            writeln!(out, "### {state}")?;
            writeln!(out)?;
            for h in in_state {
                let deadline = h
                    .deadline()
                    .map(|d| format!(" :: deadline {d}"))
                    .unwrap_or_default();
                let blockers = blocker_ids(h);
                let blocked_by = if blockers.is_empty() {
                    String::new()
                } else {
                    format!(" :: blocked by {}", blockers.join(", "))
                };
                writeln!(
                    out,
                    "- **{}** [#{}] {}{}{}",
                    h.id, h.priority, h.title, deadline, blocked_by
                )?;
            }
            writeln!(out)?;
        }
        if !closed.is_empty() {
            writeln!(out, "### Closed ({} items)", closed.len())?;
            writeln!(out)?;
            for h in closed.iter().take(10) {
                writeln!(
                    out,
                    "- {} [#{}] {} ({})",
                    h.id, h.priority, h.title, h.state
                )?;
            }
            if closed.len() > 10 {
                writeln!(out, "- ... and {} more", closed.len() - 10)?;
            }
            writeln!(out)?;
        }
    }
    Ok(out)
}

/// Does this body say *this issue* was rejected, as opposed to using the word.
///
/// `contains("rejected")` cannot tell the two apart, and the difference is the
/// whole finding. Every bug report about input validation says it: "silently
/// corrupted rather than rejected", "ignored rather than enforced or rejected".
/// A design note says it too: "a hand-written parser is rejected as strictly
/// dominated". Three issues in one corpus were flagged for exactly those, all
/// of them worked and closed properly, and a check that cries wolf about closed
/// issues is a check nobody re-reads.
///
/// So this looks for the shapes a rejection is actually written in: the tool's
/// own phrasing, a redirect, or a heading that says so.
fn looks_like_reject_prose(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    const CLOSING: &[&str] = &[
        "vissue reject",
        "superseded by",
        "rejected in favour",
        "rejected in favor",
        "rejected as a duplicate",
        "closed as a duplicate",
        "closed as duplicate",
        "not doing this",
        "rejected this",
        "rejected: ",
    ];
    if CLOSING.iter().any(|phrase| lower.contains(phrase)) {
        return true;
    }
    // A heading that names the outcome, which is where a hand-written rejection
    // goes when it is not one of the phrases above.
    //
    // "superseded" and not "supersede": the participle says this issue was
    // replaced, and the third person says it replaced others. A corpus here has
    // an issue whose "** Supersedes" section rolls up seven others it did not
    // close, and reading that as its own rejection is the false positive this
    // function exists to stop. Do not shorten the stem.
    lower.lines().any(|line| {
        line.starts_with('*')
            && (line.contains("rejected")
                || line.contains("superseded")
                || line.contains("reject:"))
    })
}

/// Does the prose around this link claim the relation the properties name.
///
/// A body mentions other issues for every reason there is: a parent lists its
/// children, an umbrella rolls up what it does not close, a note says "see
/// also". Warning that any of those lacks a `DISCOVERED_FROM` asks for an edge
/// nobody can honestly supply, and the answer is a wrong edge or a warning that
/// gets ignored. One corpus had twenty-two of these and not one was a discovery.
///
/// So the warning is for a body that says discovery or a pivot and has no edge
/// to match, which is the case the properties exist for.
fn claims_discovery_or_pivot(body: &str, linked: &str) -> bool {
    const CLAIMS: &[&str] = &[
        "discovered from",
        "discovered while",
        "discovered during",
        "found while",
        "filed from",
        "split from",
        "pivoted to",
        "pivots to",
        "pivoted from",
        "replaced by",
        "moved to",
    ];
    let needle = format!("id:{linked}");
    let lower = body.to_ascii_lowercase();
    let lower_needle = needle.to_ascii_lowercase();
    // The claim has to be near the link rather than anywhere in the body: a long
    // issue can say "discovered while auditing" in one section and link three
    // unrelated ids in another.
    let window = 240;
    let mut from = 0;
    while let Some(at) = lower[from..].find(&lower_needle) {
        let hit = from + at;
        let start = hit.saturating_sub(window);
        let end = (hit + lower_needle.len() + window).min(lower.len());
        let near = &lower[floor_char_boundary(&lower, start)..ceil_char_boundary(&lower, end)];
        if CLAIMS.iter().any(|phrase| near.contains(phrase)) {
            return true;
        }
        from = hit + lower_needle.len();
    }
    false
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Is the relation between these two already held as an edge, in either
/// direction.
///
/// Discovery and a pivot are not the only relations people write. A parent
/// mentioning a child, or an issue naming what blocks it, is a stated relation
/// the tracker already holds, and warning that it lacks a DISCOVERED_FROM asks
/// for an edge nobody can honestly supply: the answer is either a wrong edge or
/// a warning that gets ignored.
fn edge_connects(all: &[(String, IssueHeading)], a: &str, b: &str) -> bool {
    all.iter().any(|(_, h)| {
        let far = if h.id == a {
            b
        } else if h.id == b {
            a
        } else {
            return false;
        };
        [
            crate::props::DISCOVERED_FROM,
            crate::props::PIVOTED_TO,
            crate::props::PARENT,
            crate::props::BLOCKED_BY,
            crate::props::EDNA_BLOCKER,
        ]
        .iter()
        .any(|key| {
            crate::props::get(&h.properties, key)
                .is_some_and(|value| value.split(&[',', ' '][..]).any(|part| part.trim() == far))
        })
    })
}

/// Outcome of [`check`]: the findings, and how many were errors.
#[derive(Debug, Clone)]
pub struct CheckReport {
    /// Rendered findings, ending in a summary line.
    pub text: String,
    /// Count of `[err]` findings.
    pub errors: usize,
    /// Count of `[warn]` findings.
    pub warnings: usize,
}

/// Findings as they accumulate, each carrying its own severity.
///
/// The counts are the point: `check` exits non-zero on an error, and a caller reads
/// the two numbers without reading the prose. Keeping them beside the text is what
/// stops a finding being written without being counted, which is a silent way for the
/// exit code to disagree with the report.
#[derive(Default)]
struct Findings {
    text: String,
    errors: usize,
    warnings: usize,
}

impl Findings {
    /// A finding a reader has to fix. Writing to a `String` cannot fail.
    fn err(&mut self, what: std::fmt::Arguments) {
        let _ = writeln!(self.text, "[err]  {what}");
        self.errors += 1;
    }

    /// A finding a reader may leave, which does not change the exit code.
    fn warn(&mut self, what: std::fmt::Arguments) {
        let _ = writeln!(self.text, "[warn] {what}");
        self.warnings += 1;
    }
}

/// Validate the corpus: every parent and blocker id resolves, dates parse, open
/// issues carry a creation date, and ids are unique across projects.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
pub fn check(layout: &Layout) -> Result<CheckReport> {
    let all = load_all(layout)?;

    // A parent is usually another issue, and those ids are already in hand.
    // Only the ones that are not send us looking through the rest of the
    // tree, which on a tracker sharing a root with a notes vault is most of
    // the bytes on disk.
    let issue_ids: HashSet<&str> = all.iter().map(|(_, h)| h.id.as_str()).collect();
    let unresolved: HashSet<String> = all
        .iter()
        .filter_map(|(_, h)| h.parent())
        .filter(|p| !issue_ids.contains(p))
        .map(str::to_string)
        .collect();
    let elsewhere = find_org_ids(layout, &unresolved)?;
    let resolves = |id: &str| issue_ids.contains(id) || elsewhere.contains(id);

    let mut f = Findings::default();

    let mut by_id: HashMap<String, (String, &IssueHeading)> = HashMap::new();
    for (project, h) in &all {
        if let Some(prev) = by_id.insert(h.id.clone(), (project.clone(), h)) {
            // An error, not a note: an id that names two issues makes every
            // blocker and parent edge pointing at it ambiguous.
            f.err(format_args!(
                "duplicate id: {} appears in {} and {}",
                h.id, prev.0, project
            ));
        }
    }

    for project in list_projects(layout)? {
        check_project(&project, layout, &mut f)?;
    }

    for (project, h) in &all {
        check_issue(project, h, &resolves, &by_id, &mut f);
    }

    let known: HashSet<&str> = all.iter().map(|(_, h)| h.id.as_str()).collect();
    for (project, h) in &all {
        check_provenance_links(&all, project, h, &known, &mut f);
    }

    // A :PARENT: loop passes every edge check, because each id resolves, yet
    // it makes the hierarchy unwalkable: `tree` stops on it and prints
    // "(cycle, stopping)". Naming it here is what keeps a corpus that holds
    // one from reading as clean.
    let mut settled: HashSet<&str> = HashSet::new();
    for (_, h) in &all {
        check_parent_cycle(h, &by_id, &mut settled, &mut f);
    }

    if f.errors == 0
        && let Err(err) = DependencyGraph::from_issues(&all)
    {
        f.err(format_args!("blocker graph: {err}"));
    }

    let _ = writeln!(f.text);
    let projects = list_projects(layout)?.len();
    let _ = writeln!(
        f.text,
        "checked {} issue(s) across {projects} project(s): {} error(s), {} warning(s)",
        all.len(),
        f.errors,
        f.warnings
    );
    Ok(CheckReport {
        text: f.text,
        errors: f.errors,
        warnings: f.warnings,
    })
}

/// Validate one project's file: its preamble, and the headings it holds.
///
/// The one cohesive thing in `check` that is about a file rather than about the
/// corpus. Everything here reads one project's own preamble and its own headings and
/// needs none of the others.
///
/// # Errors
///
/// Returns an error if the project's file cannot be read or parsed.
fn check_project(project: &str, layout: &Layout, f: &mut Findings) -> Result<()> {
    let path = layout.project_issues_path(project);
    let doc = IssueDoc::parse_file(project, &path)?;
    check_preamble(project, &doc, &path, f);
    // The loader skips a heading a calendar sync owns, so the parsed headings cannot
    // hold one and counting them there counted nothing. The heading is still in the
    // file, and one the tracker will not touch is the surprise worth reporting, so the
    // file is what gets counted.
    let gcal_ids = crate::store::org_ids(&std::fs::read_to_string(&path)?)
        .filter(|id| crate::org::is_gcal_event_id(id))
        .count();
    if gcal_ids > 0 {
        f.err(format_args!(
            "{project}: {gcal_ids} heading(s) use an org-gcal event id as :ID:"
        ));
    }
    check_headings(project, &doc, f);
    Ok(())
}

/// What Org needs from the file's preamble to render the tracker as intended.
///
/// Each of these is a keyword whose absence Org does not complain about and a reader
/// notices later: an agenda that labels every row `issues`, a publish that exports
/// the tracker, a priority cookie outside the range the file declares.
fn check_preamble(project: &str, doc: &IssueDoc, path: &std::path::Path, f: &mut Findings) {
    match crate::org::protocol_from_preamble(&doc.preamble) {
        None => {
            f.warn(format_args!(
                "{project}: preamble has no #+VISSUE: protocol stamp"
            ));
        }
        Some(n) if n < crate::org::PROTOCOL_VERSION => {
            f.warn(format_args!(
                "{project}: #+VISSUE: {n} is behind protocol {}",
                crate::org::PROTOCOL_VERSION
            ));
        }
        Some(n) if n > crate::org::PROTOCOL_VERSION => {
            f.err(format_args!(
                "{project}: #+VISSUE: {n} is newer than this vissue (protocol {})",
                crate::org::PROTOCOL_VERSION
            ));
        }
        Some(_) => {}
    }
    if !crate::org::preamble_has_keyword(&doc.preamble, "CATEGORY") {
        f.warn(format_args!(
            "{project}: preamble has no #+CATEGORY: (org-agenda labels every row \"issues\")"
        ));
    }
    if !crate::org::preamble_has_keyword(&doc.preamble, "FILETAGS") {
        f.warn(format_args!("{project}: preamble has no #+FILETAGS:"));
    } else if !doc
        .tag_settings
        .filetags
        .iter()
        .any(|t| t.eq_ignore_ascii_case("noexport"))
    {
        f.warn(format_args!(
            "{project}: #+FILETAGS: has no noexport; a vault publish will export this tracker"
        ));
    }
    if !crate::org::preamble_has_keyword(&doc.preamble, "TAGS") {
        f.warn(format_args!(
            "{project}: preamble has no #+TAGS:; Emacs fast tag selection has no type group"
        ));
    }
    if !crate::org::preamble_has_keyword(
        &crate::org::merge_setupfile_settings(&doc.preamble, path.parent()),
        "PRIORITIES",
    ) {
        f.warn(format_args!(
            "{project}: preamble has no #+PRIORITIES:; cookies default to C and the range is A..C"
        ));
    }
}

/// What the tracker needs from each heading in the file.
///
/// Counted rather than named one by one, because a file with forty headings that all
/// put `:PRIORITY:` in the drawer wants one line saying so, not forty.
fn check_headings(project: &str, doc: &IssueDoc, f: &mut Findings) {
    let spec = doc.priority_spec();
    let mut type_not_tagged = 0usize;
    let mut exclusive_clash = 0usize;
    let mut priority_out_of_range = 0usize;
    let mut ordered_skip = 0usize;
    let mut done_with_open_children = 0usize;
    let mut priority_in_drawer = 0usize;
    let mut blockedby_typo = 0usize;
    let mut blocker_as_ids = 0usize;
    let mut computed_specials = 0usize;
    let mut bad_effort = 0usize;
    for h in &doc.headings {
        if let Some(kind) = crate::props::get(&h.properties, crate::props::TYPE) {
            let kind = kind.trim();
            if !kind.is_empty()
                && kind.chars().all(crate::model::is_org_tag_char)
                && !h.org_tags.iter().any(|t| t == kind)
            {
                type_not_tagged += 1;
            }
        }
        for group in &doc.tag_settings.exclusive {
            let hits = group
                .iter()
                .filter(|name| h.org_tags.iter().any(|t| t == *name))
                .count();
            if hits > 1 {
                exclusive_clash += 1;
                break;
            }
        }
        if !spec.contains(h.priority) {
            priority_out_of_range += 1;
        }
        if h.properties.contains_key("PRIORITY") {
            priority_in_drawer += 1;
        }
        if h.properties.contains_key("BLOCKEDBY") {
            blockedby_typo += 1;
        }
        if let Some(raw) = h.properties.get("BLOCKER")
            && !crate::org::is_edna_blocker(raw)
        {
            blocker_as_ids += 1;
        }
        if crate::org::COMPUTED_SPECIALS
            .iter()
            .any(|k| *k != "PRIORITY" && h.properties.contains_key(*k))
        {
            computed_specials += 1;
        }
        if let Some(effort) = h.effort()
            && !crate::org::is_org_effort(effort)
        {
            bad_effort += 1;
        }
        if let Some(pid) = h.parent()
            && let Some(parent) = doc.headings.iter().find(|p| p.id == pid)
            && crate::org::org_property_is_set(&parent.properties, "ORDERED")
            && !crate::org::org_property_is_set(&h.properties, "NOBLOCKING")
        {
            let earlier_open = doc.headings.iter().any(|sib| {
                sib.parent() == Some(pid)
                    && sib.line_start < h.line_start
                    && sib.state != "DONE"
                    && sib.state != "CANCELLED"
            });
            if earlier_open && (h.state == "STARTED" || h.state == "DONE") {
                ordered_skip += 1;
            }
        }
        if h.state == "DONE"
            && !crate::org::org_property_is_set(&h.properties, "NOBLOCKING")
            && doc.headings.iter().any(|c| {
                c.parent() == Some(h.id.as_str()) && c.state != "DONE" && c.state != "CANCELLED"
            })
        {
            done_with_open_children += 1;
        }
    }
    if type_not_tagged > 0 {
        f.warn(format_args!("{project}: {type_not_tagged} heading(s) have :TYPE: that is a legal Org tag but is not on the heading"));
    }
    if exclusive_clash > 0 {
        f.warn(format_args!("{project}: {exclusive_clash} heading(s) carry more than one tag from a #+TAGS: exclusive group"));
    }
    if priority_in_drawer > 0 {
        f.warn(format_args!("{project}: {priority_in_drawer} heading(s) put :PRIORITY: in the drawer; Org reads the [#A] cookie"));
    }
    if blockedby_typo > 0 {
        f.warn(format_args!(
            "{project}: {blockedby_typo} heading(s) use :BLOCKEDBY: instead of :BLOCKED_BY:"
        ));
    }
    if blocker_as_ids > 0 {
        f.warn(format_args!("{project}: {blocker_as_ids} heading(s) use :BLOCKER: as a bare id list; a rewrite folds them into :BLOCKED_BY:"));
    }
    if computed_specials > 0 {
        f.warn(format_args!("{project}: {computed_specials} heading(s) set a computed Org special (TODO/ITEM/TAGS/...) in the drawer; Org ignores it"));
    }
    if bad_effort > 0 {
        f.warn(format_args!(
            "{project}: {bad_effort} heading(s) have an Effort value Org will not parse"
        ));
    }
    if priority_out_of_range > 0 {
        f.warn(format_args!(
            "{project}: {priority_out_of_range} heading(s) have a [#prio] outside #+PRIORITIES:"
        ));
    }
    if ordered_skip > 0 {
        f.warn(format_args!("{project}: {ordered_skip} heading(s) started or closed before an earlier ORDERED sibling"));
    }
    if done_with_open_children > 0 {
        f.warn(format_args!("{project}: {done_with_open_children} DONE heading(s) still have open children (Org ORDERED / todo-dependencies)"));
    }
}

/// Validate one issue on its own: its edges resolve, its dates parse, and its state
/// agrees with what the drawer and the body say.
fn check_issue<'a>(
    project: &str,
    h: &'a IssueHeading,
    resolves: &impl Fn(&str) -> bool,
    by_id: &HashMap<String, (String, &'a IssueHeading)>,
    f: &mut Findings,
) {
    if let Some(parent) = h.parent()
        && !resolves(parent)
    {
        f.err(format_args!(
            "{} (in {}) :PARENT: {} -> not found",
            h.id, project, parent
        ));
    }
    for blk in blocker_ids(h) {
        if !by_id.contains_key(blk) {
            f.err(format_args!(
                "{} (in {}) :BLOCKED_BY: {} -> not found",
                h.id, project, blk
            ));
        }
    }
    // A citation nothing can be asked for fails wherever it is finally opened,
    // which is a different process on a different day. `deed` refuses one; a
    // hand-edited drawer is how one gets in anyway.
    for cited in h.deeds() {
        if !crate::ops::is_deed_accession(&cited) {
            f.warn(format_args!(
                "{} (in {}) :DEEDS: {} -> not a deed accession",
                h.id, project, cited
            ));
        }
    }
    if let Some(d) = h.deadline()
        && parse_org_date(d).is_none()
    {
        f.err(format_args!(
            "{} (in {}) :DEADLINE: {} -> unparseable",
            h.id, project, d
        ));
    }
    if let Some(s) = h.scheduled()
        && parse_org_date(s).is_none()
    {
        f.err(format_args!(
            "{} (in {}) :SCHEDULED: {} -> unparseable",
            h.id, project, s
        ));
    }
    if matches!(h.state.as_str(), "TODO" | "STARTED") && !h.properties.contains_key("CREATED") {
        f.warn(format_args!(
            "{} (in {}) state={} but :CREATED: is missing",
            h.id, project, h.state
        ));
    }
    if h.state == "DONE" && looks_like_reject_prose(&h.body) {
        f.warn(format_args!(
            "{} (in {}) is DONE but the body reads as a reject",
            h.id, project
        ));
    }
    if crate::props::get(&h.properties, crate::props::SIBLING_TERMINAL).is_some() {
        f.warn(format_args!(
            "{} (in {}) holds {} and sibling {}",
            h.id,
            project,
            h.state,
            crate::props::get(&h.properties, crate::props::SIBLING_TERMINAL).unwrap_or("?")
        ));
    }
}

/// Report a body claiming one issue came out of another with no edge either way.
fn check_provenance_links<'a>(
    all: &[(String, IssueHeading)],
    project: &str,
    h: &'a IssueHeading,
    known: &HashSet<&'a str>,
    f: &mut Findings,
) {
    for linked in crate::related::org_link_targets(&h.body, known) {
        if edge_connects(all, &h.id, &linked) {
            continue;
        }
        if !claims_discovery_or_pivot(&h.body, &linked) {
            continue;
        }
        f.warn(format_args!(
            "{} (in {}) mentions [[id:{}]] as discovered or pivoted with no edge either way",
            h.id, project, linked
        ));
    }
}

/// Walk `:PARENT:` from one heading and report a loop.
///
/// `settled` carries across headings, so the walk stays linear over the corpus: an id
/// already reached from somewhere else cannot start a loop that was not already
/// reported.
fn check_parent_cycle<'a>(
    start: &'a IssueHeading,
    by_id: &HashMap<String, (String, &'a IssueHeading)>,
    settled: &mut HashSet<&'a str>,
    f: &mut Findings,
) {
    if settled.contains(start.id.as_str()) {
        return;
    }
    let mut path: Vec<&str> = Vec::new();
    let mut on_path: HashSet<&str> = HashSet::new();
    let mut cursor = start.id.as_str();
    loop {
        if settled.contains(cursor) {
            break;
        }
        if !on_path.insert(cursor) {
            let start = path.iter().position(|id| *id == cursor).unwrap_or(0);
            let mut loop_ids: Vec<&str> = path[start..].to_vec();
            loop_ids.push(cursor);
            f.err(format_args!("parent cycle: {}", loop_ids.join(" -> ")));
            break;
        }
        path.push(cursor);
        match by_id.get(cursor).and_then(|(_, owner)| owner.parent()) {
            Some(parent) if by_id.contains_key(parent) => cursor = parent,
            _ => break,
        }
    }
    settled.extend(path);
}

/// Every issue referring to `target_id` through a blocker edge, a parent link,
/// a discovered-from or pivoted-to property, or a body mention. The relation
/// is named on the row.
///
/// # Errors
///
/// Returns an error if the corpus cannot be read.
pub fn backlinks(layout: &Layout, target_id: &str) -> Result<String> {
    let all = load_all(layout)?;
    let mut out = String::new();

    // A deed accession is a different namespace from an issue id, and asking
    // what points at a product is the question you have when the product turns
    // out to be wrong. The corpus decides which namespace this is: a known id
    // is an issue, whatever it looks like, so a project actually named `deed`
    // keeps working. Only a token nobody minted is read as an accession.
    let known = all.iter().any(|(_, h)| h.id == target_id);
    if !known && crate::ops::is_deed_accession(target_id) {
        for (project, h) in &all {
            let relation = if h.deeds().iter().any(|cited| cited == target_id) {
                "cites"
            } else if h.body.contains(target_id) {
                "body mention"
            } else {
                continue;
            };
            let _ = writeln!(out, "{:<22} ({relation}) ({project})", h.id);
        }
        return Ok(out);
    }

    for (project, h) in &all {
        if h.id == target_id {
            continue;
        }
        let mut hit = false;
        if blocker_ids(h).contains(&target_id) {
            let _ = writeln!(out, "{:<22} (blocked-by) ({})", h.id, project);
            hit = true;
        }
        if h.parent() == Some(target_id) {
            let _ = writeln!(out, "{:<22} (parent) ({})", h.id, project);
            hit = true;
        }
        if crate::props::get(&h.properties, crate::props::DISCOVERED_FROM) == Some(target_id) {
            let _ = writeln!(out, "{:<22} (discovered-from) ({})", h.id, project);
            hit = true;
        }
        if crate::props::get(&h.properties, crate::props::PIVOTED_TO) == Some(target_id) {
            let _ = writeln!(out, "{:<22} (pivoted-to) ({})", h.id, project);
            hit = true;
        }
        if !hit && h.body.contains(target_id) {
            let _ = writeln!(out, "{:<22} (body mention) ({})", h.id, project);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_labels_escape_untrusted_issue_text() {
        assert_eq!(dot_quoted(r#"a "quoted" title"#), r#"a \"quoted\" title"#);
        // A trailing backslash would otherwise escape the closing quote and
        // let the rest of the title become DOT syntax.
        assert_eq!(dot_quoted(r"ends with\"), r"ends with\\");
        assert_eq!(dot_quoted("two\nlines"), "two\\nlines");
    }
}
