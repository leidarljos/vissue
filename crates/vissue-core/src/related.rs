//! Explainable, derived connections between Org issue headings.

use anyhow::anyhow;

use crate::error::Result;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;

use crate::config::Layout;
use crate::error::Error;
use crate::model::IssueHeading;
use crate::store::load_all;
use crate::views::{IssueRec, RelatedHit};

const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "in", "is", "it", "of", "on",
    "or", "the", "to", "with",
];

#[derive(Debug)]
struct IssueTerms {
    project: String,
    terms: HashSet<String>,
    tags: HashSet<String>,
}

#[derive(Debug, Default)]
struct Candidate {
    score: f64,
    evidence: Vec<String>,
    edge: f64,
    distance: f64,
    tags: f64,
    project: f64,
    terms: f64,
}

/// A Borda ballot is at most this long, so four guess rankers cannot
/// outvote one declared edge (`1000 e`).
const BORDA_CAP: usize = 249;

#[derive(Clone, Copy)]
enum Signal {
    Edge,
    Distance,
    Tags,
    Project,
    Terms,
}

fn tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.len() > 2)
        .filter(|token| token.chars().any(char::is_alphabetic))
        .map(str::to_lowercase)
        .filter(|token| !STOP_WORDS.contains(&token.as_str()))
}

fn issue_terms(project: &str, issue: &IssueHeading) -> IssueTerms {
    let mut text = String::new();
    text.push_str(&issue.title);
    text.push(' ');
    text.push_str(&issue.body);
    // A tag counts as a term wherever it was written, drawer or heading.
    for tag in &issue.org_tags {
        text.push(' ');
        text.push_str(tag);
    }
    for (key, value) in &issue.properties {
        if matches!(key.as_str(), "TYPE" | "VISSUE_TYPE") || key == crate::model::TAGS_PROPERTY {
            text.push(' ');
            text.push_str(value);
        } else if !matches!(
            key.as_str(),
            "ID" | "CREATED"
                | "VISSUE_BLOCKED_BY"
                | "BLOCKED_BY"
                | "VISSUE_PARENT"
                | "PARENT"
                | "DEADLINE"
                | "SCHEDULED"
                | "VISSUE_CLAIMED_BY"
                | "CLAIMED_BY"
                | "CLAIMED_AT"
                | "VISSUE_DISCOVERED_FROM"
                | "DISCOVERED_FROM"
                | "VISSUE_PIVOTED_TO"
                | "PIVOTED_TO"
                | "VISSUE_SIBLING_TERMINAL"
                | "SIBLING_TERMINAL"
        ) {
            text.push(' ');
            text.push_str(key);
            text.push(' ');
            text.push_str(value);
        }
    }
    IssueTerms {
        project: project.to_string(),
        terms: tokens(&text).collect(),
        tags: issue
            .tags()
            .into_iter()
            .map(|tag| tag.to_lowercase())
            .collect(),
    }
}

fn add_evidence(candidate: &mut Candidate, score: f64, evidence: &str, signal: Signal) {
    match signal {
        Signal::Edge => candidate.edge += score,
        Signal::Distance => candidate.distance += score,
        Signal::Tags => candidate.tags += score,
        Signal::Project => candidate.project += score,
        Signal::Terms => candidate.terms += score,
    }
    if !candidate.evidence.iter().any(|item| item == evidence) {
        candidate.evidence.push(evidence.to_string());
    }
}

/// Borda points: `k - position` (position 0 is first). `k` is the ballot
/// length, capped so guess rankers stay under the 1000-point edge floor.
fn borda_points(raw: &HashMap<usize, f64>) -> HashMap<usize, f64> {
    let mut rows: Vec<(usize, f64)> = raw
        .iter()
        .filter(|(_, score)| **score > 0.0)
        .map(|(&index, &score)| (index, score))
        .collect();
    rows.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    let k = rows.len().min(BORDA_CAP);
    let mut out = HashMap::new();
    let mut i = 0;
    while i < rows.len() {
        let score = rows[i].1;
        let mut j = i + 1;
        while j < rows.len() && (rows[j].1 - score).abs() < f64::EPSILON {
            j += 1;
        }
        let points = k.saturating_sub(i) as f64;
        if points > 0.0 {
            for &(index, _) in &rows[i..j] {
                out.insert(index, points);
            }
        }
        i = j;
    }
    out
}

fn apply_borda(candidates: &mut HashMap<usize, Candidate>) {
    let mut distance = HashMap::new();
    let mut tags = HashMap::new();
    let mut project = HashMap::new();
    let mut terms = HashMap::new();
    for (&index, candidate) in candidates.iter() {
        if candidate.distance > 0.0 {
            distance.insert(index, candidate.distance);
        }
        if candidate.tags > 0.0 {
            tags.insert(index, candidate.tags);
        }
        if candidate.project > 0.0 {
            project.insert(index, candidate.project);
        }
        if candidate.terms > 0.0 {
            terms.insert(index, candidate.terms);
        }
    }
    let distance = borda_points(&distance);
    let tags = borda_points(&tags);
    let project = borda_points(&project);
    let terms = borda_points(&terms);
    for (index, candidate) in candidates.iter_mut() {
        candidate.score = 1_000.0 * candidate.edge
            + distance.get(index).copied().unwrap_or(0.0)
            + tags.get(index).copied().unwrap_or(0.0)
            + project.get(index).copied().unwrap_or(0.0)
            + terms.get(index).copied().unwrap_or(0.0);
    }
}

pub(crate) fn org_link_targets(body: &str, known_ids: &HashSet<&str>) -> Vec<String> {
    crate::org::org_link_targets(body, known_ids)
}

fn org_link(id: &str) -> String {
    format!("id:{id}")
}

/// Rank local, derived connections for an issue. Explicit Org relations and
/// lexical overlap are separate evidence so callers can inspect the reason.
///
/// # Errors
///
/// Returns an error if `format` is not `text` or `org`, the corpus cannot be
/// read, or `id` is not in the corpus.
pub fn related(
    layout: &Layout,
    id: &str,
    depth: usize,
    limit: usize,
    format: &str,
) -> Result<String> {
    if !matches!(format, "text" | "org") {
        return Err(anyhow!("related format must be text or org, got {format:?}").into());
    }
    let loaded = load_all(layout)?;
    let recs: Vec<IssueRec> = loaded
        .into_iter()
        .map(|(project, heading)| IssueRec {
            project,
            heading,
            path: std::path::PathBuf::new(),
            tag_settings: crate::org::TagSettings::default(),
        })
        .collect();
    let hits = related_hits_from(&recs, id, depth, limit)?;
    let mut out = String::new();
    for hit in hits {
        if format == "org" {
            writeln!(
                out,
                "- [[{}][{}]] :: {:.3} {}",
                org_link(&hit.id),
                hit.id,
                hit.score,
                hit.evidence.join(", ")
            )?;
        } else {
            writeln!(
                out,
                "{:.3} {} ({}) [{}]",
                hit.score,
                hit.id,
                hit.title,
                hit.evidence.join(", ")
            )?;
        }
    }
    Ok(out)
}

/// Add score and a reason to one candidate, creating it if this is its first.
///
/// Six scorers reach for this, and each spelled the same `entry().or_insert_with()`
/// with a fresh zeroed `Candidate` inline.
fn bump(
    candidates: &mut HashMap<usize, Candidate>,
    index: usize,
    score: f64,
    evidence: &str,
    signal: Signal,
) {
    add_evidence(
        candidates.entry(index).or_default(),
        score,
        evidence,
        signal,
    );
}

/// Score every issue sharing a word with the target, by inverse document frequency.
///
/// A word in one issue of a hundred says more about what an issue is about than a word
/// in ninety of them, and squaring the weight is what keeps a rare shared word from
/// being drowned by several common ones.
fn score_shared_terms(
    terms: &[IssueTerms],
    target_idx: usize,
    candidates: &mut HashMap<usize, Candidate>,
) {
    let mut document_frequency: HashMap<&str, usize> = HashMap::new();
    let mut inverted: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, item) in terms.iter().enumerate() {
        for term in &item.terms {
            *document_frequency.entry(term.as_str()).or_default() += 1;
            inverted.entry(term.as_str()).or_default().push(index);
        }
    }

    let total = terms.len() as f64;
    for term in &terms[target_idx].terms {
        let frequency = document_frequency[term.as_str()] as f64;
        let idf = ((total + 1.0) / (frequency + 1.0)).ln() + 1.0;
        for &index in inverted.get(term.as_str()).into_iter().flatten() {
            if index != target_idx {
                bump(
                    candidates,
                    index,
                    idf * idf,
                    &format!("term:{term}"),
                    Signal::Terms,
                );
            }
        }
    }
}

/// Every declared edge in the corpus, both ways round.
///
/// Undirected on purpose: relatedness does not care which end of a blocker edge an
/// issue sits at, and walking outward from the target has to cross an edge whichever
/// way it was written.
fn neighbour_graph(
    all: &[(&str, &IssueHeading)],
    ids: &HashMap<&str, usize>,
    known_ids: &HashSet<&str>,
) -> HashMap<usize, Vec<usize>> {
    let mut neighbors: HashMap<usize, Vec<usize>> = HashMap::new();
    let join = |a: usize, b: usize, neighbors: &mut HashMap<usize, Vec<usize>>| {
        neighbors.entry(a).or_default().push(b);
        neighbors.entry(b).or_default().push(a);
    };
    for (index, (_, issue)) in all.iter().enumerate() {
        if let Some(parent) = issue.parent().and_then(|parent| ids.get(parent).copied()) {
            join(index, parent, &mut neighbors);
        }
        for blocker in issue.blocked_by() {
            if let Some(blocker) = ids.get(blocker.as_str()).copied() {
                join(index, blocker, &mut neighbors);
            }
        }
        for key in [crate::props::DISCOVERED_FROM, crate::props::PIVOTED_TO] {
            if let Some(origin) = crate::props::get(&issue.properties, key)
                .and_then(|origin| ids.get(origin).copied())
            {
                join(index, origin, &mut neighbors);
            }
        }
        for linked_id in org_link_targets(&issue.body, known_ids) {
            if let Some(linked) = ids.get(linked_id.as_str()).copied() {
                join(index, linked, &mut neighbors);
            }
        }
    }
    neighbors
}

/// Score by how far the target is from each issue along declared edges, out to
/// `depth`, nearer being worth more.
fn score_graph_distance(
    neighbors: &HashMap<usize, Vec<usize>>,
    target_idx: usize,
    depth: usize,
    candidates: &mut HashMap<usize, Candidate>,
) {
    let mut queue = VecDeque::from([(target_idx, 0usize)]);
    let mut seen = HashSet::from([target_idx]);
    while let Some((index, distance)) = queue.pop_front() {
        if distance == depth {
            continue;
        }
        for &neighbor in neighbors.get(&index).into_iter().flatten() {
            if seen.insert(neighbor) {
                queue.push_back((neighbor, distance + 1));
                if neighbor != target_idx {
                    let reached = distance + 1;
                    bump(
                        candidates,
                        neighbor,
                        100.0 / reached as f64,
                        &format!("org_distance:{reached}"),
                        Signal::Distance,
                    );
                }
            }
        }
    }
}

/// The names for every declared relation between the target and one other issue.
///
/// Named from the target's end, and the two ends of one edge get different names:
/// `blocks` and `blocked_by` are the same edge, and which of them a reader is told
/// decides whether they are looking at what waits or at what is waited on.
fn declared_relations(
    target: &IssueHeading,
    target_id: &str,
    issue: &IssueHeading,
    known_ids: &HashSet<&str>,
) -> Vec<&'static str> {
    let mut named = Vec::new();
    if target.blocked_by().iter().any(|item| item == &issue.id) {
        named.push("blocked_by");
    }
    if issue.blocked_by().iter().any(|item| item == target_id) {
        named.push("blocks");
    }
    if target.parent() == Some(issue.id.as_str()) {
        named.push("parent");
    }
    if issue.parent() == Some(target_id) {
        named.push("child");
    }
    if crate::props::get(&issue.properties, crate::props::DISCOVERED_FROM) == Some(target_id) {
        named.push("discovered_from");
    }
    if crate::props::get(&target.properties, crate::props::DISCOVERED_FROM)
        == Some(issue.id.as_str())
    {
        named.push("source_of");
    }
    if crate::props::get(&target.properties, crate::props::PIVOTED_TO) == Some(issue.id.as_str()) {
        named.push("pivoted_to");
    }
    if crate::props::get(&issue.properties, crate::props::PIVOTED_TO) == Some(target_id) {
        named.push("successor_of");
    }
    if org_link_targets(&target.body, known_ids)
        .iter()
        .any(|linked_id| linked_id == &issue.id)
        || org_link_targets(&issue.body, known_ids)
            .iter()
            .any(|linked_id| linked_id == target_id)
    {
        named.push("org_link");
    }
    named
}

/// Score what the target and one other issue share directly: a declared relation, a
/// tag, a project.
///
/// Each signal ranks on its own. Borda merges the guess rankers; a declared
/// relation still adds 1000 so no amount of coincidence outvotes a fact.
fn score_direct_relations(
    all: &[(&str, &IssueHeading)],
    terms: &[IssueTerms],
    target_idx: usize,
    known_ids: &HashSet<&str>,
    candidates: &mut HashMap<usize, Candidate>,
) {
    let target = all[target_idx].1;
    let target_id = target.id.as_str();
    // Read off the drawer once rather than per candidate.
    let target_deeds = target.deeds();
    for (index, (_, issue)) in all.iter().enumerate() {
        if index == target_idx {
            continue;
        }
        for relation in declared_relations(target, target_id, issue, known_ids) {
            bump(candidates, index, 1.0, relation, Signal::Edge);
        }
        // Two issues citing one deed worked on the same product. That is a
        // declared fact rather than a resemblance, so it scores with the edges
        // and not with the tags: the deed is the thing they have in common, and
        // it is named.
        let theirs = issue.deeds();
        for cited in &target_deeds {
            if theirs.contains(cited) {
                bump(
                    candidates,
                    index,
                    1.0,
                    &format!("deed:{cited}"),
                    Signal::Edge,
                );
            }
        }
        let shared_tags = terms[target_idx]
            .tags
            .intersection(&terms[index].tags)
            .count();
        if shared_tags > 0 {
            bump(
                candidates,
                index,
                25.0 * shared_tags as f64,
                "shared_tags",
                Signal::Tags,
            );
        }
        if terms[target_idx].project == terms[index].project {
            bump(candidates, index, 2.0, "same_project", Signal::Project);
        }
    }
}

/// Order the candidates and keep the first `limit`.
///
/// Score first, then id, so the order is total: a caller reading the top of the list
/// twice reads the same thing, where a sort on score alone leaves ties to the hash
/// order they arrived in.
fn rank(
    candidates: HashMap<usize, Candidate>,
    all: &[(&str, &IssueHeading)],
    limit: usize,
) -> Vec<RelatedHit> {
    let mut ranked: Vec<(usize, Candidate)> = candidates.into_iter().collect();
    ranked.retain(|(_, candidate)| !candidate.evidence.is_empty());
    ranked.sort_by(|(a, left), (b, right)| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| all[*a].1.id.cmp(&all[*b].1.id))
    });
    ranked.truncate(limit);

    ranked
        .into_iter()
        .map(|(index, candidate)| {
            let (project, issue) = all[index];
            RelatedHit {
                id: issue.id.clone(),
                project: project.to_string(),
                state: issue.state.clone(),
                title: issue.title.clone(),
                score: candidate.score,
                evidence: candidate.evidence,
            }
        })
        .collect()
}

/// Structured related hits, without going through the text formatter.
///
/// Four scorers over one candidate set: shared words, distance along declared edges,
/// direct relations, and what the pair have in common. Each adds its own evidence, so
/// a hit carries the reasons it is here and not only a number.
///
/// # Errors
///
/// Returns an error if `id` is not in `recs`.
pub fn related_hits_from(
    recs: &[IssueRec],
    id: &str,
    depth: usize,
    limit: usize,
) -> std::result::Result<Vec<RelatedHit>, Error> {
    let all: Vec<(&str, &IssueHeading)> = recs
        .iter()
        .map(|r| (r.project.as_str(), &r.heading))
        .collect();
    let target_idx = all
        .iter()
        .position(|(_, issue)| issue.id == id)
        .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
    let terms: Vec<IssueTerms> = all
        .iter()
        .map(|(project, issue)| issue_terms(project, issue))
        .collect();
    let ids: HashMap<&str, usize> = all
        .iter()
        .enumerate()
        .map(|(index, (_, issue))| (issue.id.as_str(), index))
        .collect();
    let known_ids: HashSet<&str> = ids.keys().copied().collect();

    let mut candidates: HashMap<usize, Candidate> = HashMap::new();
    score_shared_terms(&terms, target_idx, &mut candidates);
    let neighbors = neighbour_graph(&all, &ids, &known_ids);
    score_graph_distance(&neighbors, target_idx, depth, &mut candidates);
    score_direct_relations(&all, &terms, target_idx, &known_ids, &mut candidates);
    apply_borda(&mut candidates);

    Ok(rank(candidates, &all, limit))
}

#[cfg(test)]
mod tests {
    use super::borda_points;
    use std::collections::HashMap;

    #[test]
    fn borda_gives_k_minus_position() {
        let raw = HashMap::from([(0, 9.0), (1, 3.0), (2, 6.0)]);
        let points = borda_points(&raw);
        assert_eq!(points.get(&0).copied(), Some(3.0), "{points:?}");
        assert_eq!(points.get(&2).copied(), Some(2.0), "{points:?}");
        assert_eq!(points.get(&1).copied(), Some(1.0), "{points:?}");
    }

    #[test]
    fn borda_gives_tied_raw_scores_the_same_points() {
        let raw = HashMap::from([(0, 2.0), (1, 2.0)]);
        let points = borda_points(&raw);
        assert_eq!(points.get(&0), points.get(&1), "{points:?}");
        assert_eq!(points.get(&0).copied(), Some(2.0), "{points:?}");
    }
}
