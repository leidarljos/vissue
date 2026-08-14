//! Explainable, derived connections between Org issue headings.

use anyhow::{bail, Result};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;

use crate::config::Layout;
use crate::model::IssueHeading;
use crate::store::load_all;

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

#[derive(Debug)]
struct Candidate {
    score: f64,
    evidence: Vec<String>,
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
        if matches!(key.as_str(), "TYPE") || key == crate::model::TAGS_PROPERTY {
            text.push(' ');
            text.push_str(value);
        } else if !matches!(
            key.as_str(),
            "ID" | "CREATED"
                | "BLOCKED_BY"
                | "PARENT"
                | "DEADLINE"
                | "SCHEDULED"
                | "CLAIMED_BY"
                | "CLAIMED_AT"
                | "DISCOVERED_FROM"
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

fn add_evidence(candidate: &mut Candidate, score: f64, evidence: &str) {
    candidate.score += score;
    if !candidate.evidence.iter().any(|item| item == evidence) {
        candidate.evidence.push(evidence.to_string());
    }
}

fn org_link_targets(body: &str, known_ids: &HashSet<&str>) -> Vec<String> {
    let mut targets = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("[[") {
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("]]") else {
            break;
        };
        let raw = &after_start[..end];
        let target = raw.split_once("][").map_or(raw, |(target, _)| target);
        let target = target.trim();
        let target = target.strip_prefix("id:").unwrap_or(target);
        let target = target.rsplit_once("::").map_or(target, |(_, fragment)| {
            fragment.strip_prefix('#').unwrap_or(fragment)
        });
        let target = target.strip_prefix('#').unwrap_or(target);
        if known_ids.contains(target) {
            targets.push(target.to_string());
        }
        rest = &after_start[end + 2..];
    }
    targets
}

fn org_link(id: &str) -> String {
    format!("id:{id}")
}

/// Rank local, derived connections for an issue. Explicit Org relations and
/// lexical overlap are separate evidence so callers can inspect the reason.
pub fn related(
    layout: &Layout,
    id: &str,
    depth: usize,
    limit: usize,
    format: &str,
) -> Result<String> {
    if !matches!(format, "text" | "org") {
        bail!("related format must be text or org, got {format:?}");
    }
    let all = load_all(layout)?;
    let target_idx = all
        .iter()
        .position(|(_, issue)| issue.id == id)
        .ok_or_else(|| anyhow::anyhow!("issue {id} not found"))?;
    let terms: Vec<IssueTerms> = all
        .iter()
        .map(|(project, issue)| issue_terms(project, issue))
        .collect();

    let mut document_frequency: HashMap<&str, usize> = HashMap::new();
    for item in &terms {
        for term in &item.terms {
            *document_frequency.entry(term.as_str()).or_default() += 1;
        }
    }

    let mut inverted: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, item) in terms.iter().enumerate() {
        for term in &item.terms {
            inverted.entry(term.as_str()).or_default().push(index);
        }
    }

    let target = &all[target_idx].1;
    let mut candidates: HashMap<usize, Candidate> = HashMap::new();
    let total = all.len() as f64;
    for term in &terms[target_idx].terms {
        let frequency = document_frequency[term.as_str()] as f64;
        let idf = ((total + 1.0) / (frequency + 1.0)).ln() + 1.0;
        for &index in inverted.get(term.as_str()).into_iter().flatten() {
            if index != target_idx {
                add_evidence(
                    candidates.entry(index).or_insert_with(|| Candidate {
                        score: 0.0,
                        evidence: Vec::new(),
                    }),
                    idf * idf,
                    &format!("term:{term}"),
                );
            }
        }
    }

    let mut neighbors: HashMap<usize, Vec<usize>> = HashMap::new();
    let ids: HashMap<&str, usize> = all
        .iter()
        .enumerate()
        .map(|(index, (_, issue))| (issue.id.as_str(), index))
        .collect();
    let known_ids: HashSet<&str> = ids.keys().copied().collect();
    for (index, (_, issue)) in all.iter().enumerate() {
        if let Some(parent) = issue.parent().and_then(|parent| ids.get(parent).copied()) {
            neighbors.entry(index).or_default().push(parent);
            neighbors.entry(parent).or_default().push(index);
        }
        for blocker in issue.blocked_by() {
            if let Some(blocker) = ids.get(blocker.as_str()).copied() {
                neighbors.entry(index).or_default().push(blocker);
                neighbors.entry(blocker).or_default().push(index);
            }
        }
        if let Some(origin) = issue
            .properties
            .get("DISCOVERED_FROM")
            .and_then(|origin| ids.get(origin.as_str()).copied())
        {
            neighbors.entry(index).or_default().push(origin);
            neighbors.entry(origin).or_default().push(index);
        }
        for linked_id in org_link_targets(&issue.body, &known_ids) {
            let Some(linked) = ids.get(linked_id.as_str()).copied() else {
                continue;
            };
            neighbors.entry(index).or_default().push(linked);
            neighbors.entry(linked).or_default().push(index);
        }
    }

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
                    let evidence = format!(
                        "org_distance:{distance_plus_one}",
                        distance_plus_one = distance + 1
                    );
                    add_evidence(
                        candidates.entry(neighbor).or_insert_with(|| Candidate {
                            score: 0.0,
                            evidence: Vec::new(),
                        }),
                        100.0 / (distance + 1) as f64,
                        &evidence,
                    );
                }
            }
        }
    }

    for (index, (_, issue)) in all.iter().enumerate() {
        if index == target_idx {
            continue;
        }
        let mut explicit = Vec::new();
        if target.blocked_by().iter().any(|item| item == &issue.id) {
            explicit.push("blocked_by");
        }
        if issue.blocked_by().iter().any(|item| item == id) {
            explicit.push("blocks");
        }
        if target.parent() == Some(issue.id.as_str()) {
            explicit.push("parent");
        }
        if issue.parent() == Some(id) {
            explicit.push("child");
        }
        if issue.properties.get("DISCOVERED_FROM").map(String::as_str) == Some(id) {
            explicit.push("discovered_from");
        }
        if target.properties.get("DISCOVERED_FROM").map(String::as_str) == Some(issue.id.as_str()) {
            explicit.push("source_of");
        }
        if org_link_targets(&target.body, &known_ids)
            .iter()
            .any(|linked_id| linked_id == &issue.id)
        {
            explicit.push("org_link");
        }
        if org_link_targets(&issue.body, &known_ids)
            .iter()
            .any(|linked_id| linked_id == id)
        {
            explicit.push("org_link");
        }
        if !explicit.is_empty() {
            for relation in explicit {
                add_evidence(
                    candidates.entry(index).or_insert_with(|| Candidate {
                        score: 0.0,
                        evidence: Vec::new(),
                    }),
                    1_000.0,
                    relation,
                );
            }
        }
        let shared_tags = terms[target_idx]
            .tags
            .intersection(&terms[index].tags)
            .count();
        if shared_tags > 0 {
            add_evidence(
                candidates.entry(index).or_insert_with(|| Candidate {
                    score: 0.0,
                    evidence: Vec::new(),
                }),
                25.0 * shared_tags as f64,
                "shared_tags",
            );
        }
        if terms[target_idx].project == terms[index].project {
            add_evidence(
                candidates.entry(index).or_insert_with(|| Candidate {
                    score: 0.0,
                    evidence: Vec::new(),
                }),
                2.0,
                "same_project",
            );
        }
    }

    let mut ranked: Vec<(usize, Candidate)> = candidates.into_iter().collect();
    ranked.retain(|(_, candidate)| !candidate.evidence.is_empty());
    ranked.sort_by(|(a, left), (b, right)| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| all[*a].1.id.cmp(&all[*b].1.id))
    });
    ranked.truncate(limit);

    let mut out = String::new();
    for (index, candidate) in ranked {
        let (_, issue) = &all[index];
        if format == "org" {
            writeln!(
                out,
                "- [[{}][{}]] :: {:.3} {}",
                org_link(&issue.id),
                issue.id,
                candidate.score,
                candidate.evidence.join(", ")
            )?;
        } else {
            writeln!(
                out,
                "{:.3} {} ({}) [{}]",
                candidate.score,
                issue.id,
                issue.title,
                candidate.evidence.join(", ")
            )?;
        }
    }
    Ok(out)
}
