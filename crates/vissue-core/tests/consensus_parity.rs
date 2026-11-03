//! `vissue consensus` against the consensus crate on the same ballots.
//!
//! ljos prints `ljos-consensus settle` and falls back to `vissue consensus`
//! when the crate is not on PATH, passing both the same ballots, `--trust`
//! rows and `--susceptibility-of` anchors. `data/consensus_parity.json` holds
//! what the pinned crate printed for each case, and `data/consensus_parity.sh`
//! regenerates it. Each case is cast into a tracker and settled the way
//! `vissue consensus ID --trust ROWS --susceptibility-of ANCHORS` settles it.

#![allow(missing_docs)]

use serde::Deserialize;
use vissue_core::config::{DEFAULT_PREFIX, Layout};
use vissue_core::consensus::{self, Outcome, Settling};
use vissue_core::ops::{self, CreateOpts};

/// Neither settle stops further than `q / (1 - q) * 1e-9` from the fixed
/// point, `q` the largest susceptibility under one: the crate bounds its
/// residual by Banach's estimate, and vissue stops on a step that small. The
/// stiffest case here holds 0.8, so 4e-9 apiece. 1e-6 is over a hundred
/// times their sum, and still a thousand times finer than the three decimals
/// both reports print.
const TOLERANCE: f64 = 1e-6;

#[derive(Deserialize)]
struct Golden {
    generator: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    ballots: Vec<CastBallot>,
    trust: serde_json::Value,
    anchors: serde_json::Value,
    #[serde(rename = "crate")]
    expected: Settled,
}

#[derive(Deserialize)]
struct CastBallot {
    agent: String,
    choice: String,
}

/// The fields of `ljos-consensus settle` this compares.
#[derive(Deserialize)]
struct Settled {
    options: Vec<String>,
    shares: Vec<f64>,
    converged: bool,
    tie: bool,
    polarization: f64,
    agents: Vec<String>,
    influence: Vec<f64>,
}

/// Every case, settled by vissue from a tracker holding its ballots.
fn settle_every_case() -> Vec<(Case, Outcome)> {
    let golden: Golden =
        serde_json::from_str(include_str!("data/consensus_parity.json")).expect("golden data");
    assert_eq!(golden.generator, "ljos-consensus 0.9.0");
    let dir = tempfile::tempdir().expect("tempdir");
    let layout = Layout::new(dir.path().to_path_buf(), DEFAULT_PREFIX);
    golden
        .cases
        .into_iter()
        .enumerate()
        .map(|(at, case)| {
            assert!(
                case.expected.converged,
                "{}: the crate did not settle it, so it is no reference",
                case.name
            );
            let id = format!("parity-{at}");
            let opts = CreateOpts {
                id: Some(&id),
                quiet: true,
                ..CreateOpts::default()
            };
            ops::create(&layout, "parity", &case.name, opts).expect("create");
            for ballot in &case.ballots {
                ops::vote(&layout, &id, Some(&ballot.choice), &ballot.agent).expect("vote");
            }
            let rows = consensus::trust_rows(&case.trust.to_string()).expect("trust");
            let anchors = consensus::anchor_rows(&case.anchors.to_string()).expect("anchors");
            let outcome =
                consensus::of_issue_anchored(&layout, &id, &rows, &anchors).expect("settle");
            (case, outcome)
        })
        .collect()
}

/// The group's position when it agreed, else the mean of where each voter
/// landed: the line the anchored report prints, and the crate's shares.
fn shares(outcome: &Outcome) -> Vec<f64> {
    if let Some(consensus) = &outcome.consensus {
        return consensus.clone();
    }
    let n = outcome.agents.len() as f64;
    (0..outcome.choices.len())
        .map(|c| outcome.agents.iter().map(|a| a.limit[c]).sum::<f64>() / n)
        .collect()
}

fn near(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < TOLERANCE)
}

/// Each case that fails `check`, with what it said, so one run names them all.
fn mismatches(
    settled: &[(Case, Outcome)],
    check: impl Fn(&Case, &Outcome) -> Option<String>,
) -> Vec<String> {
    settled
        .iter()
        .filter_map(|(case, outcome)| {
            check(case, outcome).map(|why| format!("{}: {why}", case.name))
        })
        .collect()
}

/// The same choices in the same order, each with the share the crate gives
/// it. This is the number either settle prints for the issue.
#[test]
fn every_case_settles_to_the_crate_shares() {
    let wrong = mismatches(&settle_every_case(), |case, outcome| {
        let ours = shares(outcome);
        (outcome.choices != case.expected.options || !near(&ours, &case.expected.shares)).then(
            || {
                format!(
                    "vissue {:?} {ours:?}, crate {:?} {:?}",
                    outcome.choices, case.expected.options, case.expected.shares
                )
            },
        )
    });
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Where each voter landed, through the crate's polarization: the squared
/// distance of every voter's position from the mean. Equal shares can hide
/// voters that sit apart differently; this cannot.
#[test]
fn every_voter_lands_where_the_crate_puts_it() {
    let wrong = mismatches(&settle_every_case(), |case, outcome| {
        let mean = shares(outcome);
        let polarization: f64 = outcome
            .agents
            .iter()
            .flat_map(|a| a.limit.iter().zip(&mean).map(|(x, m)| (x - m) * (x - m)))
            .sum();
        let names: Vec<&str> = outcome.agents.iter().map(|a| a.agent.as_str()).collect();
        (names != case.expected.agents
            || (polarization - case.expected.polarization).abs() >= TOLERANCE)
            .then(|| {
                format!(
                    "vissue {names:?} {polarization}, crate {:?} {}",
                    case.expected.agents, case.expected.polarization
                )
            })
    });
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// When the group agreed, each voter's social power is the crate's
/// influence, and the leader is named exactly when the crate sees no tie.
#[test]
fn where_the_group_agreed_power_and_the_leader_match_the_crate() {
    let agreed: Vec<(Case, Outcome)> = settle_every_case()
        .into_iter()
        .filter(|(_, outcome)| outcome.settling == Settling::Agreed)
        .collect();
    assert!(
        !agreed.is_empty(),
        "no case agreed, so nothing was compared"
    );
    let wrong = mismatches(&agreed, |case, outcome| {
        let power: Option<Vec<f64>> = outcome.agents.iter().map(|a| a.power).collect();
        let power = power.unwrap_or_default();
        let leader = outcome.leader().map(|(choice, _)| choice.to_string());
        (!near(&power, &case.expected.influence) || leader.is_none() != case.expected.tie).then(
            || {
                format!(
                    "vissue power {power:?} leader {leader:?}, crate influence {:?} tie {}",
                    case.expected.influence, case.expected.tie
                )
            },
        )
    });
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
