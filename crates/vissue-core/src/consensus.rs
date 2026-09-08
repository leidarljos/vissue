//! DeGroot consensus over the ballots on one issue.
//!
//! A tally counts. Counting is the right answer when every voter is worth the
//! same, and agents are not: a maintainer, a reviewer that has been wrong twice,
//! and a fresh worker all cast one ballot each, and a plurality reports them as
//! three equal opinions. `vote` already refuses to call a plurality agreement,
//! but it has nothing to say about *whose* agreement it is.
//!
//! DeGroot's model (1974) is the standard answer and is one line: each agent holds
//! an opinion, listens to the agents it trusts, and replaces its opinion with the
//! weighted average of theirs. Written as a matrix, `x(t+1) = W x(t)` with `W`
//! row-stochastic. Where that iteration settles is the group's position, and it
//! is not the mean unless the trust is symmetric.
//!
//! Three things come out of it that a count cannot give:
//!
//! - the limit itself, which weights each ballot by how much the group actually
//!   listens to the agent that cast it;
//! - social power, the left Perron vector `π` of `W`, which says how much each
//!   agent moved the result: the limit is `πᵀ x(0)`;
//! - the failure to reach one. `W` converges to agreement only when the trust
//!   graph has a single closed group every agent can reach (Berger, 1981). Two teams
//!   that cite only each other never converge, and that is a fact about the team
//!   worth reporting rather than a number worth averaging.
//!
//! The opinion here is a distribution over the choices already on the ballots, so
//! nothing new has to be cast: an agent that voted `ship` starts at one on
//! `ship`, and the limit is how much of the group's weight ends up on each
//! option. With no trust configured every agent listens to every other equally,
//! `W` is doubly stochastic, `π` is uniform, and the consensus is the tally as a
//! fraction. Configuration only ever moves weight away from that.
//!
//! M. H. DeGroot, "Reaching a Consensus", J. Am. Stat. Assoc. 69(345), 1974.
//!
//! R. A. Berger, "A necessary and sufficient condition for reaching a consensus
//! using DeGroot's method", J. Am. Stat. Assoc. 76(374), 1981.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use petgraph::algo::kosaraju_scc;
use petgraph::graph::{DiGraph, NodeIndex};

use crate::config::ConsensusSection;
use crate::ops::Ballot;

/// Where the influence matrix came from, for a reader wondering why a result
/// looks the way it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustSource {
    /// No row in the configuration named any agent that voted.
    Default,
    /// At least one voting agent had a configured row.
    Configured,
}

/// Whether the iteration settled, and on what.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Settling {
    /// Every agent holds the same opinion: a consensus.
    Agreed,
    /// The iteration reached a fixed point on which agents still differ, which
    /// means the trust graph holds more than one closed group.
    Split,
    /// No fixed point inside the iteration budget: a periodic trust graph, which
    /// is what a pair that listen only to each other and not at all to
    /// themselves produce.
    Oscillating,
    /// The agents settled while still holding different opinions, because each
    /// stayed partly anchored to the ballot it cast.
    ///
    /// Not a failure and not the same thing as a split. Under Friedkin and
    /// Johnsen the persistent disagreement *is* the result: reporting one
    /// number for the group would be reporting a position none of them holds.
    Anchored,
}

/// One agent's row of the result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentLimit {
    /// Identity that cast the ballot.
    pub agent: String,
    /// The choice it voted for.
    pub voted: String,
    /// Where that agent's opinion ended up, aligned with [`Outcome::choices`].
    /// The opinion it started with, when the trust graph never settles.
    pub limit: Vec<f64>,
    /// How much of the consensus this agent's ballot accounts for, when the
    /// group agreed. `None` when it did not, because a split group has no single
    /// weighting to report.
    pub power: Option<f64>,
}

/// The result of running DeGroot over one issue's ballots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    /// Distinct choices, sorted, indexing every `limit` vector.
    pub choices: Vec<String>,
    /// One row per voting agent, sorted by identity.
    pub agents: Vec<AgentLimit>,
    /// Whether the iteration settled, and on what.
    pub settling: Settling,
    /// The shared limit when the group agreed, aligned with `choices`.
    pub consensus: Option<Vec<f64>>,
    /// Agents grouped by the opinion they settled on, when they did not agree.
    pub factions: Vec<Vec<String>>,
    /// Rounds the iteration took.
    pub rounds: usize,
    /// Whether the iteration stopped because it ran out of rounds rather than
    /// because it settled. The shares are then an estimate, not the limit.
    pub budget_reached: bool,
    /// Whether any voting agent had a configured trust row.
    pub trust: TrustSource,
    /// How far agents were allowed to move off their own ballot. One is
    /// DeGroot; below one is Friedkin and Johnsen.
    pub susceptibility: f64,
    /// The largest gap left between any two agents on any one choice.
    ///
    /// Zero within tolerance when they agreed. Under an anchor it is the
    /// disagreement the group keeps, which is the quantity worth reading.
    pub spread: f64,
}

impl Outcome {
    /// The winning choice and its share, when the group agreed and one choice
    /// leads.
    ///
    /// `None` on a split, on an oscillation, and on an exact tie inside a
    /// consensus, because reporting the first of two equal options as the
    /// group's position is how a coin toss gets recorded as agreement.
    #[must_use]
    pub fn leader(&self) -> Option<(&str, f64)> {
        let consensus = self.consensus.as_ref()?;
        let mut ranked: Vec<(usize, f64)> = consensus.iter().copied().enumerate().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        let (top, share) = *ranked.first()?;
        if ranked.len() > 1 && (ranked[1].1 - share).abs() < TIE_EPS {
            return None;
        }
        Some((self.choices[top].as_str(), share))
    }
}

impl Outcome {
    /// Whether a gate over this issue should pass.
    ///
    /// True only when the group agreed and one choice leads. A plurality, a
    /// tie, a split and an oscillation are all cases where acting on the number
    /// would be acting on agreement that is not there, which is what the verb
    /// exists to make visible.
    #[must_use]
    pub fn settled(&self) -> bool {
        self.settling == Settling::Agreed && self.leader().is_some()
    }
}

/// Two shares this close are a tie rather than a lead.
///
/// Coarser than the settling tolerance on purpose: the question is whether a
/// reader would call the result a win, and a lead in the twelfth decimal is an
/// artefact of the arithmetic rather than a position the group holds.
const TIE_EPS: f64 = 1e-6;

/// Settle `ballots` under `cfg`.
///
/// DeGroot when `cfg.susceptibility` is one, which is the default: every agent
/// gives up its own starting position and the group converges on a single
/// number. Below one it is Friedkin and Johnsen's generalisation, `x(t+1) = λ W
/// x(t) + (1 - λ) x(0)`, where each agent stays partly anchored to the ballot it
/// cast and what settles is a profile of persistent disagreement.
///
/// Returns an empty outcome when nobody has voted; a single ballot settles on
/// itself in one round, which the caller reports as the one opinion it is rather
/// than as agreement.
#[must_use]
pub fn settle(ballots: &[Ballot], cfg: &ConsensusSection) -> Outcome {
    let agents: Vec<&Ballot> = {
        let mut sorted: Vec<&Ballot> = ballots.iter().collect();
        sorted.sort_by(|a, b| a.agent.cmp(&b.agent));
        sorted
    };
    let choices: Vec<String> = agents
        .iter()
        .map(|b| b.choice.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let n = agents.len();
    let m = choices.len();
    if n == 0 {
        return Outcome {
            choices,
            agents: Vec::new(),
            settling: Settling::Agreed,
            consensus: None,
            factions: Vec::new(),
            rounds: 0,
            budget_reached: false,
            trust: TrustSource::Default,
            susceptibility: cfg.susceptibility,
            spread: 0.0,
        };
    }

    let names: Vec<&str> = agents.iter().map(|b| b.agent.as_str()).collect();
    let (weights, trust) = influence(&names, cfg);

    // One-hot: an agent that voted `ship` puts all of its opinion on `ship`. The
    // choice set was collected from these same ballots, so the position is
    // always there; written as a match rather than an unwrap so the function
    // has no panicking path at all.
    let mut opinion = vec![vec![0.0f64; m]; n];
    for (i, ballot) in agents.iter().enumerate() {
        if let Some(at) = choices.iter().position(|c| *c == ballot.choice) {
            opinion[i][at] = 1.0;
        }
    }

    // An anchor makes the iteration a contraction whatever the trust graph
    // looks like, so it always settles, and it settles on agents that still
    // differ. That is the model working rather than failing, so the structural
    // question below is only asked of the unanchored case.
    //
    // Without an anchor the trust graph alone decides whether there is a
    // consensus to reach, and the iteration only works out what it is. Deciding
    // it from the iteration instead means asking whether a number stopped
    // moving, and a chain that mixes slowly stops moving long before its agents
    // agree, which reads as a split that is not there.
    let anchored = cfg.susceptibility < 1.0;
    let settling = if anchored {
        Settling::Anchored
    } else {
        match structure(&weights) {
            Structure::Convergent => Settling::Agreed,
            Structure::Split => Settling::Split,
            Structure::Periodic => Settling::Oscillating,
        }
    };
    let start = opinion.clone();
    let rounds = iterate(&mut opinion, &weights, &start, cfg, settling);
    let consensus = (settling == Settling::Agreed).then(|| opinion[0].clone());
    // Social power is the left Perron vector of the trust matrix, which weighs
    // the ballots into the one position the group reached. Under an anchor
    // there is no one position, so there is nothing for it to weigh.
    let power = (settling == Settling::Agreed).then(|| social_power(&weights, cfg));
    let factions = match settling {
        Settling::Split => group_by_limit(&names, &opinion, cfg.tolerance),
        // Nothing converged on, so there is no position to group agents by.
        Settling::Agreed | Settling::Oscillating | Settling::Anchored => Vec::new(),
    };
    let spread = spread(&opinion, m);

    Outcome {
        choices,
        agents: agents
            .iter()
            .enumerate()
            .map(|(i, ballot)| AgentLimit {
                agent: ballot.agent.clone(),
                voted: ballot.choice.clone(),
                limit: opinion[i].clone(),
                power: power.as_ref().map(|p| p[i]),
            })
            .collect(),
        settling,
        consensus,
        factions,
        rounds,
        budget_reached: settling != Settling::Oscillating && rounds >= cfg.max_iterations,
        trust,
        susceptibility: cfg.susceptibility,
        spread,
    }
}

/// The consensus on one issue of a tracker, under that tracker's trust rows.
///
/// # Errors
///
/// Returns an error if `id` is not in the corpus, the corpus cannot be read, or
/// the configuration names a weight the iteration cannot use.
pub fn of_issue(layout: &crate::config::Layout, id: &str) -> crate::error::Result<Outcome> {
    let ballots = crate::ops::ballots(layout, id)?;
    let cfg = crate::config::VissueConfig::load(layout)?.consensus;
    Ok(settle(&ballots, &cfg))
}

/// What each child of `plan` settled on.
///
/// Every child is read on its own, and the rows are left as rows. See the
/// design note: no weighting over children can be picked without a judgement
/// the tracker has no basis for, a split child has no position to fold in, and
/// an unvoted child is absent rather than neutral.
///
/// # Errors
///
/// Returns an error if `plan` is not in the corpus, the corpus cannot be read,
/// or the configuration names a weight the iteration cannot use.
pub fn of_plan(
    layout: &crate::config::Layout,
    plan: &str,
) -> crate::error::Result<crate::views::PlanConsensus> {
    use crate::views::{ChildConsensus, PlanConsensus};

    let recs = crate::catalog::load_recs(layout)?;
    let service = crate::catalog::CatalogService::from_recs(&recs);
    let parent = service.detail(plan)?;
    let cfg = crate::config::VissueConfig::load(layout)?.consensus;

    let mut children = Vec::new();
    for hit in service.children(plan)? {
        let ballots = crate::ops::ballots(layout, &hit.id)?;
        let outcome = (!ballots.is_empty()).then(|| settle(&ballots, &cfg));
        children.push(ChildConsensus {
            id: hit.id,
            state: hit.state,
            title: hit.title,
            ballots: ballots.len(),
            settling: outcome.as_ref().map(|o| o.settling),
            holds: outcome.as_ref().and_then(|o| {
                o.leader()
                    .map(|(choice, share)| (choice.to_string(), share))
            }),
        });
    }

    Ok(PlanConsensus {
        plan: parent.id,
        title: parent.title,
        children,
    })
}

/// Build the row-stochastic influence matrix over `names`.
///
/// A configured row names the agents this one listens to, in whatever units the
/// author found natural; only the ratios matter, because the row is normalised.
/// Weight on an agent that did not vote is dropped: it has no opinion to average,
/// and keeping it would quietly scale everyone else down.
///
/// The agent's weight on itself is `self_weight` unless its own row names it, in
/// which case that value is used as written and the whole row is normalised
/// together. An agent with no row, or whose row named nobody who voted, listens
/// to itself with `self_weight` and splits the rest equally: that is the prior
/// that makes the no-configuration case reduce to the tally.
fn influence(names: &[&str], cfg: &ConsensusSection) -> (Vec<Vec<f64>>, TrustSource) {
    let n = names.len();
    let mut weights = vec![vec![0.0f64; n]; n];
    let mut source = TrustSource::Default;
    for (i, name) in names.iter().enumerate() {
        let row = &mut weights[i];
        let configured = cfg.trust.get(*name).map(|spec| {
            let mut named = 0usize;
            for (j, other) in names.iter().enumerate() {
                if let Some(w) = spec.get(*other)
                    && *w > 0.0
                {
                    row[j] = *w;
                    named += 1;
                }
            }
            (named > 0, spec.contains_key(*name))
        });
        match configured {
            Some((true, names_itself)) => {
                source = TrustSource::Configured;
                if names_itself {
                    normalise(row, 1.0);
                } else {
                    normalise(row, 1.0 - cfg.self_weight);
                    row[i] += cfg.self_weight;
                }
            }
            _ => {
                if n == 1 {
                    row[i] = 1.0;
                } else {
                    let share = (1.0 - cfg.self_weight) / ((n - 1) as f64);
                    for weight in row.iter_mut() {
                        *weight = share;
                    }
                    row[i] = cfg.self_weight;
                }
            }
        }
    }
    (weights, source)
}

/// Scale `row` so it sums to `total`. A row that sums to nothing is left alone;
/// the caller only calls this on a row with a positive entry.
fn normalise(row: &mut [f64], total: f64) {
    let sum: f64 = row.iter().sum();
    if sum <= 0.0 {
        return;
    }
    for weight in row.iter_mut() {
        *weight *= total / sum;
    }
}

/// What the trust graph alone determines about the outcome.
///
/// DeGroot's iteration converges to agreement exactly when the graph holds one
/// closed group that every agent can reach, and that group is aperiodic (Berger,
/// 1981). All three are properties of which weights are positive, not of their
/// sizes, so they are decided here once rather than inferred from a number that
/// stopped moving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Structure {
    /// One closed group, aperiodic: every agent converges on it.
    Convergent,
    /// More than one closed group: agents inside different ones never agree.
    Split,
    /// One closed group with a period: opinions cycle instead of settling.
    Periodic,
}

/// Classify the trust graph `i -> j` for every positive weight agent `i` puts on
/// agent `j`.
fn structure(weights: &[Vec<f64>]) -> Structure {
    let n = weights.len();
    let mut graph = DiGraph::<usize, ()>::with_capacity(n, n);
    let nodes: Vec<NodeIndex> = (0..n).map(|i| graph.add_node(i)).collect();
    for (i, row) in weights.iter().enumerate() {
        for (j, weight) in row.iter().enumerate() {
            if *weight > 0.0 {
                graph.add_edge(nodes[i], nodes[j], ());
            }
        }
    }
    let components = kosaraju_scc(&graph);
    let mut component_of = vec![0usize; n];
    for (at, component) in components.iter().enumerate() {
        for node in component {
            component_of[graph[*node]] = at;
        }
    }
    let closed: Vec<usize> = components
        .iter()
        .enumerate()
        .filter(|(at, component)| {
            !component.iter().any(|node| {
                graph
                    .neighbors(*node)
                    .any(|to| component_of[graph[to]] != *at)
            })
        })
        .map(|(at, _)| at)
        .collect();
    // A row-stochastic matrix always has at least one closed group, so the only
    // interesting count is more than one.
    let [only] = closed[..] else {
        return Structure::Split;
    };
    if period(&graph, &components[only], &component_of, only) == 1 {
        Structure::Convergent
    } else {
        Structure::Periodic
    }
}

/// The period of one strongly connected component: the greatest common divisor
/// of its cycle lengths.
///
/// Read off a breadth-first layering rather than by enumerating cycles: for
/// every edge inside the component, `level(u) + 1 - level(v)` is the length of a
/// cycle through the tree, and the gcd of those is the period. A component
/// holding a self-loop has period one, which is why a positive `self_weight`
/// makes the default case converge.
fn period(
    graph: &DiGraph<usize, ()>,
    component: &[NodeIndex],
    component_of: &[usize],
    at: usize,
) -> usize {
    let Some(&root) = component.first() else {
        return 1;
    };
    let mut level: HashMap<NodeIndex, i64> = HashMap::from([(root, 0)]);
    let mut queue = VecDeque::from([root]);
    while let Some(node) = queue.pop_front() {
        let depth = level[&node];
        for to in graph.neighbors(node) {
            if component_of[graph[to]] != at || level.contains_key(&to) {
                continue;
            }
            level.insert(to, depth + 1);
            queue.push_back(to);
        }
    }
    let mut divisor = 0i64;
    for node in component {
        let Some(&depth) = level.get(node) else {
            continue;
        };
        for to in graph.neighbors(*node) {
            if component_of[graph[to]] != at {
                continue;
            }
            if let Some(&other) = level.get(&to) {
                divisor = gcd(divisor, depth + 1 - other);
            }
        }
    }
    let period = divisor.unsigned_abs() as usize;
    period.max(1)
}

fn gcd(a: i64, b: i64) -> i64 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// Iterate the opinions and return the rounds it took.
///
/// The step is `x(t+1) = λ W x(t) + (1 - λ) x(0)`. At `λ = 1` the anchor term
/// vanishes and this is DeGroot; below one each agent keeps pulling back toward
/// the ballot it cast, which is what makes the step a contraction and the
/// settling certain.
///
/// What counts as done depends on what the run can do. An unanchored convergent
/// graph is run until the agents agree, because that is the quantity being
/// reported. A split graph never will, and an anchored run never should, so
/// both are run to a fixed point instead. A periodic unanchored graph has no
/// limit at all, so it is not run: the opinions each agent holds are the ones it
/// started with.
fn iterate(
    opinion: &mut Vec<Vec<f64>>,
    weights: &[Vec<f64>],
    start: &[Vec<f64>],
    cfg: &ConsensusSection,
    settling: Settling,
) -> usize {
    if settling == Settling::Oscillating {
        return 0;
    }
    let n = opinion.len();
    let m = opinion.first().map_or(0, Vec::len);
    let pull = cfg.susceptibility;
    let anchor = 1.0 - pull;
    for round in 0..cfg.max_iterations {
        if settling == Settling::Agreed && spread(opinion, m) < cfg.tolerance {
            return round;
        }
        let mut next = vec![vec![0.0f64; m]; n];
        let mut step = 0.0f64;
        for i in 0..n {
            for c in 0..m {
                let mut acc = 0.0;
                for (j, row) in opinion.iter().enumerate() {
                    acc += weights[i][j] * row[c];
                }
                let value = pull * acc + anchor * start[i][c];
                next[i][c] = value;
                step = step.max((value - opinion[i][c]).abs());
            }
        }
        *opinion = next;
        if matches!(settling, Settling::Split | Settling::Anchored) && step < cfg.tolerance {
            return round + 1;
        }
    }
    cfg.max_iterations
}

/// The largest disagreement between any two agents on any one choice.
fn spread(opinion: &[Vec<f64>], m: usize) -> f64 {
    let mut worst = 0.0f64;
    for c in 0..m {
        let mut low = f64::INFINITY;
        let mut high = f64::NEG_INFINITY;
        for row in opinion {
            low = low.min(row[c]);
            high = high.max(row[c]);
        }
        worst = worst.max(high - low);
    }
    worst
}

/// The left Perron vector of `weights`: how much each agent's starting opinion
/// accounts for in the limit.
///
/// Power iteration from uniform, which is what the model itself does with the
/// rows transposed. Reported only when the group agreed, because a matrix with
/// more than one closed group has more than one such vector and none of them is
/// the answer.
fn social_power(weights: &[Vec<f64>], cfg: &ConsensusSection) -> Vec<f64> {
    let n = weights.len();
    let mut power = vec![1.0 / (n as f64); n];
    for _ in 0..cfg.max_iterations {
        let mut next = vec![0.0f64; n];
        for j in 0..n {
            for (i, row) in weights.iter().enumerate() {
                next[j] += power[i] * row[j];
            }
        }
        let step = next
            .iter()
            .zip(&power)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f64, f64::max);
        power = next;
        if step < cfg.tolerance {
            break;
        }
    }
    let sum: f64 = power.iter().sum();
    if sum > 0.0 {
        for weight in &mut power {
            *weight /= sum;
        }
    }
    power
}

/// Agents that settled on the same opinion, as sorted groups.
fn group_by_limit(names: &[&str], opinion: &[Vec<f64>], tolerance: f64) -> Vec<Vec<String>> {
    let mut groups: Vec<(Vec<f64>, Vec<String>)> = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let row = &opinion[i];
        match groups.iter_mut().find(|(seen, _)| {
            seen.iter()
                .zip(row)
                .all(|(a, b)| (a - b).abs() < tolerance.max(TIE_EPS))
        }) {
            Some((_, members)) => members.push((*name).to_string()),
            None => groups.push((row.clone(), vec![(*name).to_string()])),
        }
    }
    groups.into_iter().map(|(_, members)| members).collect()
}

/// The plain count, for the line that shows what the weighting changed.
#[must_use]
pub fn tally(ballots: &[Ballot]) -> BTreeMap<String, Vec<String>> {
    let mut counts: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for ballot in ballots {
        counts
            .entry(ballot.choice.clone())
            .or_default()
            .push(ballot.agent.clone());
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ballot(agent: &str, choice: &str) -> Ballot {
        Ballot {
            agent: agent.to_string(),
            choice: choice.to_string(),
            stamp: "[2026-09-07 Mon]".to_string(),
        }
    }

    fn share(outcome: &Outcome, choice: &str) -> f64 {
        let at = outcome
            .choices
            .iter()
            .position(|c| c == choice)
            .expect("choice");
        outcome.consensus.as_ref().expect("consensus")[at]
    }

    /// With nothing configured every agent listens to every other equally, so the
    /// matrix is doubly stochastic and the limit is the mean: the tally as a
    /// fraction. This is the property that makes the verb safe to run on a
    /// tracker nobody has configured.
    #[test]
    fn without_configuration_the_consensus_is_the_tally_as_a_fraction() {
        let cfg = ConsensusSection::default();
        let ballots = [
            ballot("alice", "ship"),
            ballot("bob", "ship"),
            ballot("carol", "hold"),
        ];
        let outcome = settle(&ballots, &cfg);
        assert_eq!(outcome.settling, Settling::Agreed);
        assert!((share(&outcome, "ship") - 2.0 / 3.0).abs() < 1e-6);
        assert!((share(&outcome, "hold") - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(outcome.leader().map(|(c, _)| c), Some("ship"));
        for row in &outcome.agents {
            assert!((row.power.expect("power") - 1.0 / 3.0).abs() < 1e-6);
        }
    }

    /// The case the verb exists for: a plurality that the group's own weighting
    /// reverses. Two agents vote ship, one votes hold, both of the two listen to
    /// the third, and the third mostly listens to itself.
    #[test]
    fn trust_can_move_the_group_off_the_plurality() {
        let mut cfg = ConsensusSection::default();
        cfg.trust.insert(
            "alice".to_string(),
            BTreeMap::from([("carol".to_string(), 1.0)]),
        );
        cfg.trust.insert(
            "bob".to_string(),
            BTreeMap::from([("carol".to_string(), 1.0)]),
        );
        cfg.trust.insert(
            "carol".to_string(),
            BTreeMap::from([("carol".to_string(), 4.0), ("alice".to_string(), 1.0)]),
        );
        let ballots = [
            ballot("alice", "ship"),
            ballot("bob", "ship"),
            ballot("carol", "hold"),
        ];
        let outcome = settle(&ballots, &cfg);
        assert_eq!(outcome.settling, Settling::Agreed);
        assert_eq!(
            outcome.leader().map(|(c, _)| c),
            Some("hold"),
            "the plurality is ship; the group listens to carol: {outcome:?}"
        );
        let power = |who: &str| {
            outcome
                .agents
                .iter()
                .find(|a| a.agent == who)
                .expect("agent")
                .power
                .expect("power")
        };
        assert!(power("carol") > power("alice"), "{outcome:?}");
        // Nobody listens to bob, and an opinion nobody listens to moves the
        // group by nothing at all. A count cannot express that.
        assert!(power("bob") < 1e-6, "{outcome:?}");
    }

    /// The limit is `πᵀ x(0)`: social power is the weighting the consensus
    /// applies to the ballots, not a separate statistic that happens to be
    /// printed beside it.
    #[test]
    fn the_consensus_is_the_ballots_weighted_by_social_power() {
        let mut cfg = ConsensusSection::default();
        cfg.trust.insert(
            "alice".to_string(),
            BTreeMap::from([("bob".to_string(), 3.0), ("carol".to_string(), 1.0)]),
        );
        cfg.trust.insert(
            "bob".to_string(),
            BTreeMap::from([("carol".to_string(), 1.0)]),
        );
        let ballots = [
            ballot("alice", "ship"),
            ballot("bob", "hold"),
            ballot("carol", "hold"),
        ];
        let outcome = settle(&ballots, &cfg);
        assert_eq!(outcome.settling, Settling::Agreed);
        for (at, choice) in outcome.choices.iter().enumerate() {
            let weighted: f64 = outcome
                .agents
                .iter()
                .map(|a| {
                    let vote = f64::from(u8::from(a.voted == *choice));
                    a.power.expect("power") * vote
                })
                .sum();
            assert!(
                (weighted - outcome.consensus.as_ref().expect("consensus")[at]).abs() < 1e-6,
                "{choice}: {weighted} vs {outcome:?}"
            );
        }
    }

    /// Two groups that cite only each other never converge, and DeGroot says so
    /// rather than averaging across them. Reporting that is the point: it is a
    /// fact about the reviewers, not a number to round.
    #[test]
    fn two_closed_groups_do_not_reach_a_consensus() {
        let mut cfg = ConsensusSection::default();
        for (who, whom) in [
            ("alice", "bob"),
            ("bob", "alice"),
            ("carol", "dave"),
            ("dave", "carol"),
        ] {
            cfg.trust
                .insert(who.to_string(), BTreeMap::from([(whom.to_string(), 1.0)]));
        }
        let ballots = [
            ballot("alice", "ship"),
            ballot("bob", "ship"),
            ballot("carol", "hold"),
            ballot("dave", "hold"),
        ];
        let outcome = settle(&ballots, &cfg);
        assert_eq!(outcome.settling, Settling::Split, "{outcome:?}");
        assert!(outcome.consensus.is_none());
        assert!(outcome.leader().is_none());
        assert_eq!(outcome.factions.len(), 2, "{:?}", outcome.factions);
        assert_eq!(
            outcome.factions[0],
            vec!["alice".to_string(), "bob".to_string()]
        );
    }

    /// A pair that listen only to each other and not at all to themselves swap
    /// opinions forever. Periodicity is a different failure from a split, and
    /// calling it one would suggest the two sides had settled.
    #[test]
    fn a_periodic_trust_graph_is_reported_as_oscillating() {
        let mut cfg = ConsensusSection {
            self_weight: 0.0,
            max_iterations: 50,
            ..ConsensusSection::default()
        };
        cfg.trust.insert(
            "alice".to_string(),
            BTreeMap::from([("bob".to_string(), 1.0)]),
        );
        cfg.trust.insert(
            "bob".to_string(),
            BTreeMap::from([("alice".to_string(), 1.0)]),
        );
        let ballots = [ballot("alice", "ship"), ballot("bob", "hold")];
        let outcome = settle(&ballots, &cfg);
        assert_eq!(outcome.settling, Settling::Oscillating, "{outcome:?}");
        assert!(outcome.consensus.is_none());
    }

    /// Weight on an agent that never voted has no opinion behind it. Keeping it
    /// in the row would scale down everyone who did vote, so a trusted absentee
    /// would quietly pull the result toward the truster's own position.
    #[test]
    fn weight_on_an_agent_that_did_not_vote_is_dropped() {
        let ballots = [ballot("alice", "ship"), ballot("bob", "hold")];
        let mut with_absentee = ConsensusSection::default();
        with_absentee.trust.insert(
            "alice".to_string(),
            BTreeMap::from([("absent".to_string(), 9.0), ("bob".to_string(), 1.0)]),
        );
        let mut without = ConsensusSection::default();
        without.trust.insert(
            "alice".to_string(),
            BTreeMap::from([("bob".to_string(), 1.0)]),
        );

        assert_eq!(
            settle(&ballots, &with_absentee).agents,
            settle(&ballots, &without).agents,
            "nine parts trust in an agent that did not vote changed the answer"
        );
    }

    /// A group that mixes slowly still agrees. Deciding that from the size of
    /// the last step called it a split, because an agent that weights itself
    /// heavily stops moving long before it has finished moving, and the whole
    /// point of the structural test is that the answer does not depend on how
    /// far along the arithmetic happens to be.
    #[test]
    fn a_slowly_mixing_group_still_reaches_a_consensus() {
        let cfg = ConsensusSection {
            self_weight: 0.99,
            max_iterations: 40,
            ..ConsensusSection::default()
        };
        let ballots = [
            ballot("alice", "ship"),
            ballot("bob", "ship"),
            ballot("carol", "hold"),
        ];
        let outcome = settle(&ballots, &cfg);
        assert_eq!(outcome.settling, Settling::Agreed, "{outcome:?}");
        assert!(
            outcome.budget_reached,
            "40 rounds cannot settle this one, and the report has to say so"
        );
    }

    /// An anchor is what keeps a minority position from being averaged away.
    /// Under DeGroot the whole group lands on one number; under an anchor the
    /// agent that voted the other way is still visibly holding it.
    #[test]
    fn an_anchor_leaves_the_minority_still_holding_its_position() {
        let ballots = [
            ballot("alice", "ship"),
            ballot("bob", "ship"),
            ballot("carol", "hold"),
        ];
        let unanchored = settle(&ballots, &ConsensusSection::default());
        assert_eq!(unanchored.settling, Settling::Agreed);
        assert!(unanchored.spread < 1e-6, "{unanchored:?}");

        let anchored = settle(
            &ballots,
            &ConsensusSection {
                susceptibility: 0.6,
                ..ConsensusSection::default()
            },
        );
        assert_eq!(anchored.settling, Settling::Anchored, "{anchored:?}");
        assert!(anchored.consensus.is_none(), "no one position to report");
        assert!(
            anchored.spread > 0.1,
            "the disagreement is the result: {anchored:?}"
        );
        let hold = anchored
            .choices
            .iter()
            .position(|c| c == "hold")
            .expect("hold");
        let carol = anchored
            .agents
            .iter()
            .find(|a| a.agent == "carol")
            .expect("carol");
        let alice = anchored
            .agents
            .iter()
            .find(|a| a.agent == "alice")
            .expect("alice");
        assert!(
            carol.limit[hold] > alice.limit[hold],
            "carol voted hold and stays nearer it: {anchored:?}"
        );
    }

    /// The anchored limit is the fixed point of `x = λ W x + (1 - λ) x(0)`.
    /// Asserting the equation rather than a number is what makes this a test of
    /// the model rather than of the arithmetic that happened to run.
    #[test]
    fn the_anchored_limit_solves_the_friedkin_johnsen_equation() {
        let mut cfg = ConsensusSection {
            susceptibility: 0.7,
            ..ConsensusSection::default()
        };
        cfg.trust.insert(
            "alice".to_string(),
            BTreeMap::from([("carol".to_string(), 2.0), ("bob".to_string(), 1.0)]),
        );
        let ballots = [
            ballot("alice", "ship"),
            ballot("bob", "ship"),
            ballot("carol", "hold"),
        ];
        let outcome = settle(&ballots, &cfg);
        assert_eq!(outcome.settling, Settling::Anchored);

        let names: Vec<&str> = outcome.agents.iter().map(|a| a.agent.as_str()).collect();
        let (weights, _) = influence(&names, &cfg);
        for (i, row) in outcome.agents.iter().enumerate() {
            for (c, choice) in outcome.choices.iter().enumerate() {
                let neighbours: f64 = outcome
                    .agents
                    .iter()
                    .enumerate()
                    .map(|(j, other)| weights[i][j] * other.limit[c])
                    .sum();
                let own = f64::from(u8::from(row.voted == *choice));
                let want = cfg.susceptibility * neighbours + (1.0 - cfg.susceptibility) * own;
                assert!(
                    (want - row.limit[c]).abs() < 1e-6,
                    "{} on {choice}: {want} vs {}",
                    row.agent,
                    row.limit[c]
                );
            }
        }
    }

    /// Any anchor at all makes the step a contraction, so the pair that swap
    /// opinions forever under DeGroot settle instead. The periodic case is a
    /// property of the unanchored model, not of the group.
    #[test]
    fn an_anchor_removes_the_periodic_case() {
        let mut cfg = ConsensusSection {
            self_weight: 0.0,
            susceptibility: 0.9,
            max_iterations: 500,
            ..ConsensusSection::default()
        };
        cfg.trust.insert(
            "alice".to_string(),
            BTreeMap::from([("bob".to_string(), 1.0)]),
        );
        cfg.trust.insert(
            "bob".to_string(),
            BTreeMap::from([("alice".to_string(), 1.0)]),
        );
        let ballots = [ballot("alice", "ship"), ballot("bob", "hold")];

        let unanchored = settle(
            &ballots,
            &ConsensusSection {
                susceptibility: 1.0,
                ..cfg.clone()
            },
        );
        assert_eq!(unanchored.settling, Settling::Oscillating);

        let anchored = settle(&ballots, &cfg);
        assert_eq!(anchored.settling, Settling::Anchored, "{anchored:?}");
        assert!(
            !anchored.budget_reached,
            "a contraction settles well inside the budget: {anchored:?}"
        );
    }

    /// Full susceptibility is exactly DeGroot, which is what makes the knob
    /// safe to add: a tracker that never sets it sees the model it had.
    #[test]
    fn full_susceptibility_is_the_unanchored_model() {
        let ballots = [
            ballot("alice", "ship"),
            ballot("bob", "hold"),
            ballot("carol", "ship"),
        ];
        let default = settle(&ballots, &ConsensusSection::default());
        let explicit = settle(
            &ballots,
            &ConsensusSection {
                susceptibility: 1.0,
                ..ConsensusSection::default()
            },
        );
        assert_eq!(default.settling, explicit.settling);
        assert_eq!(default.agents, explicit.agents);
        assert_eq!(default.consensus, explicit.consensus);
    }

    /// An exact tie is not a lead. Reporting the first of two equal options as
    /// the group's position is how a coin toss becomes a decision.
    #[test]
    fn an_exact_tie_has_no_leader() {
        let cfg = ConsensusSection::default();
        let ballots = [ballot("alice", "ship"), ballot("bob", "hold")];
        let outcome = settle(&ballots, &cfg);
        assert_eq!(outcome.settling, Settling::Agreed);
        assert!(outcome.leader().is_none(), "{outcome:?}");
    }

    /// One ballot settles on itself immediately. The verb still has to say that
    /// nobody has agreed with it, which the caller does from the agent count.
    #[test]
    fn a_single_ballot_settles_on_itself() {
        let cfg = ConsensusSection::default();
        let outcome = settle(&[ballot("alice", "ship")], &cfg);
        assert_eq!(outcome.settling, Settling::Agreed);
        assert_eq!(outcome.agents.len(), 1);
        assert!((share(&outcome, "ship") - 1.0).abs() < 1e-9);
        assert!((outcome.agents[0].power.unwrap() - 1.0).abs() < 1e-9);
    }

    /// No ballots is not a consensus of zero agents that agree; it is nothing to
    /// report, and the caller says so.
    #[test]
    fn no_ballots_leaves_no_consensus_to_report() {
        let outcome = settle(&[], &ConsensusSection::default());
        assert!(outcome.agents.is_empty());
        assert!(outcome.consensus.is_none());
        assert!(outcome.leader().is_none());
    }

    /// Every row of the influence matrix sums to one, whatever units the trust
    /// was written in. This is what makes the iteration an averaging rather than
    /// a growth, and a row that did not would send an opinion off to infinity.
    #[test]
    fn every_influence_row_is_stochastic() {
        let mut cfg = ConsensusSection::default();
        cfg.trust.insert(
            "alice".to_string(),
            BTreeMap::from([("bob".to_string(), 7.5), ("carol".to_string(), 0.25)]),
        );
        cfg.trust.insert(
            "bob".to_string(),
            BTreeMap::from([("bob".to_string(), 4.0), ("alice".to_string(), 1.0)]),
        );
        let (weights, source) = influence(&["alice", "bob", "carol"], &cfg);
        assert_eq!(source, TrustSource::Configured);
        for row in &weights {
            let sum: f64 = row.iter().sum();
            assert!((sum - 1.0).abs() < 1e-12, "{row:?} sums to {sum}");
        }
        // bob named itself, so its own weight is what the row said rather than
        // the default: 4 of 5.
        assert!((weights[1][1] - 0.8).abs() < 1e-12, "{:?}", weights[1]);
        // alice did not, so it keeps self_weight and splits the rest by ratio.
        assert!((weights[0][0] - cfg.self_weight).abs() < 1e-12);
    }
}
