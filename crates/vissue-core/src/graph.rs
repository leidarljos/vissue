//! In-memory dependency graph for validation and bounded multi-hop queries.

use anyhow::anyhow;

use crate::error::Result;
use daggy::{Dag, NodeIndex};
use petgraph::Direction;
use petgraph::algo::{has_path_connecting, toposort};
use petgraph::visit::{EdgeRef, NodeIndexable};
use std::collections::{HashMap, HashSet, VecDeque};

use crate::error::Error;
use crate::model::IssueHeading;

/// A blocker edge points from the prerequisite to the issue waiting on it.
///
/// The edge value is that dependency's importance. A token with no marker
/// weighs [`crate::org::DEFAULT_EDGE_WEIGHT`].
#[derive(Debug)]
pub struct DependencyGraph {
    dag: Dag<String, f64>,
    ids: HashMap<String, NodeIndex>,
}

impl DependencyGraph {
    /// Build a graph and reject malformed duplicate IDs or cyclic dependencies.
    ///
    /// # Errors
    ///
    /// Returns an error if two headings share an id, an issue blocks itself, or
    /// the blocker edges form a cycle.
    pub fn from_issues(issues: &[(String, IssueHeading)]) -> Result<Self> {
        Self::from_headings(issues.iter().map(|(_, heading)| heading))
    }

    /// Same graph as [`Self::from_issues`], without copying headings.
    ///
    /// # Errors
    ///
    /// Returns an error if two headings share an id, an issue blocks itself, or
    /// the blocker edges form a cycle.
    pub fn from_headings<'a, I>(issues: I) -> Result<Self>
    where
        I: IntoIterator<Item = &'a IssueHeading>,
    {
        let headings: Vec<&IssueHeading> = issues.into_iter().collect();
        let mut graph = Self {
            dag: Dag::new(),
            ids: HashMap::with_capacity(headings.len()),
        };
        let mut seen_edges = HashSet::new();
        for issue in &headings {
            if graph.ids.contains_key(&issue.id) {
                return Err(anyhow!("duplicate issue id {}", issue.id).into());
            }
            let node = graph.dag.add_node(issue.id.clone());
            graph.ids.insert(issue.id.clone(), node);
        }
        for issue in &headings {
            for (blocker, weight) in issue.blocked_by_edges() {
                let Some(&from) = graph.ids.get(&blocker) else {
                    continue;
                };
                let to = graph.ids[&issue.id];
                if from == to {
                    return Err(anyhow!("issue {} blocks itself", issue.id).into());
                }
                if !seen_edges.insert((from, to)) {
                    continue;
                }
                graph
                    .dag
                    .add_edge(from, to, weight)
                    .map_err(|_| anyhow!("blocker cycle involving {} and {}", blocker, issue.id))?;
            }
        }
        Ok(graph)
    }

    /// Validate a prospective blocker insertion without mutating the graph.
    ///
    /// Unknown ids are accepted: a missing endpoint is not a cycle.
    ///
    /// # Errors
    ///
    /// Returns an error if `blocker` is `issue`, or if adding the edge would
    /// close a loop.
    pub fn accepts_edge(&self, blocker: &str, issue: &str) -> std::result::Result<(), Error> {
        let Some(&from) = self.ids.get(blocker) else {
            return Ok(());
        };
        let Some(&to) = self.ids.get(issue) else {
            return Ok(());
        };
        if from == to || has_path_connecting(self.dag.graph(), to, from, None) {
            return Err(Error::BlockerCycle {
                blocker: blocker.to_string(),
                issue: issue.to_string(),
            });
        }
        Ok(())
    }

    /// Return a deterministic prerequisite-first order.
    ///
    /// # Errors
    ///
    /// Returns an error if the graph contains a cycle.
    pub fn topological_ids(&self) -> Result<Vec<String>> {
        let ids = toposort(self.dag.graph(), None)
            .map_err(|cycle| anyhow!("dependency cycle at {}", self.dag.graph()[cycle.node_id()]))?
            .into_iter()
            .map(|node| self.dag.graph()[node].clone())
            .collect::<Vec<_>>();
        Ok(ids)
    }

    /// Importance of the edge from `blocker` to `issue`, when that edge exists.
    pub fn edge_weight(&self, blocker: &str, issue: &str) -> Option<f64> {
        let (&from, &to) = (self.ids.get(blocker)?, self.ids.get(issue)?);
        self.dag
            .graph()
            .edges_connecting(from, to)
            .next()
            .map(|edge| *edge.weight())
    }

    /// How much weighted work `issue` unblocks.
    ///
    /// An edge `b → i` of weight `w(b, i)` means `i` lists `b` in
    /// `:BLOCKED_BY:`. `reach(i)` counts `i` and every issue reachable from it:
    ///
    /// ```text
    /// reach(i) = 1 + |{ j : a blocker path i → j exists }|
    /// unblock(b) = Σ_{i : b → i} w(b, i) · reach(i)
    /// ```
    ///
    /// A missing id scores 0. An unweighted edge weighs 1, so it contributes
    /// the size of the work behind it. A shared descendant is counted once
    /// inside `reach` and again for each direct edge that leads to it, because
    /// each of those edges is a separate way the work is held.
    pub fn unblock_score(&self, issue: &str) -> f64 {
        let Some(&node) = self.ids.get(issue) else {
            return 0.0;
        };
        score_node(self.dag.graph(), node, &self.reaches())
    }

    /// [`Self::unblock_score`] for every issue that unblocks something.
    ///
    /// One reach pass for the whole graph. A zero is omitted.
    pub fn unblock_scores(&self) -> HashMap<String, f64> {
        let reach = self.reaches();
        let graph = self.dag.graph();
        let mut scores = HashMap::new();
        for (id, &node) in &self.ids {
            let score = score_node(graph, node, &reach);
            if score > 0.0 {
                scores.insert(id.clone(), score);
            }
        }
        scores
    }

    /// `reach(i)` for every node, computed sinks-first in chunked bitsets so
    /// large DAGs stay O(N) in memory.
    fn reaches(&self) -> Vec<usize> {
        let graph = self.dag.graph();
        let bound = graph.node_bound();
        let mut reach = vec![1usize; bound];
        let order = toposort(graph, None).unwrap_or_default();
        if order.is_empty() {
            return reach;
        }

        let mut targets: Vec<NodeIndex> = Vec::new();
        let mut target_slot = vec![usize::MAX; bound];
        for &node in &order {
            if graph
                .neighbors_directed(node, Direction::Incoming)
                .next()
                .is_some()
            {
                target_slot[node.index()] = targets.len();
                targets.push(node);
            }
        }
        if targets.is_empty() {
            return reach;
        }

        const WORDS: usize = 8;
        const CHUNK_BITS: usize = WORDS * 64;
        let mut bits = vec![[0u64; WORDS]; bound];

        for chunk_start in (0..targets.len()).step_by(CHUNK_BITS) {
            let chunk_end = (chunk_start + CHUNK_BITS).min(targets.len());
            if chunk_start > 0 {
                bits.fill([0u64; WORDS]);
            }
            for &node in order.iter().rev() {
                let mut acc = [0u64; WORDS];
                for child in graph.neighbors_directed(node, Direction::Outgoing) {
                    let child_idx = child.index();
                    let slot = target_slot[child_idx];
                    if slot >= chunk_start && slot < chunk_end {
                        let local = slot - chunk_start;
                        acc[local / 64] |= 1u64 << (local % 64);
                    }
                    let child_bits = bits[child_idx];
                    for (dst, src) in acc.iter_mut().zip(child_bits) {
                        *dst |= src;
                    }
                }
                bits[node.index()] = acc;
                let count: usize = acc.iter().map(|w| w.count_ones() as usize).sum();
                reach[node.index()] += count;
            }
        }
        reach
    }

    /// Return nodes that transitively block issue, bounded by hop depth.
    ///
    /// # Errors
    ///
    /// Returns an error if `issue` is not in the graph.
    pub fn ancestors(
        &self,
        issue: &str,
        depth: usize,
    ) -> std::result::Result<Vec<(usize, String)>, Error> {
        self.walk(issue, depth, Direction::Incoming)
    }

    /// Return nodes transitively waiting on issue, bounded by hop depth.
    ///
    /// # Errors
    ///
    /// Returns an error if `issue` is not in the graph.
    pub fn descendants(
        &self,
        issue: &str,
        depth: usize,
    ) -> std::result::Result<Vec<(usize, String)>, Error> {
        self.walk(issue, depth, Direction::Outgoing)
    }

    fn walk(
        &self,
        issue: &str,
        depth: usize,
        direction: Direction,
    ) -> std::result::Result<Vec<(usize, String)>, Error> {
        let Some(&root) = self.ids.get(issue) else {
            return Err(Error::IssueNotFound {
                id: issue.to_string(),
            });
        };
        let mut seen = HashSet::from([root]);
        let mut queue = VecDeque::from([(root, 0usize)]);
        let mut result = Vec::new();
        while let Some((node, distance)) = queue.pop_front() {
            if distance == depth {
                continue;
            }
            for neighbor in self.dag.graph().neighbors_directed(node, direction) {
                if !seen.insert(neighbor) {
                    continue;
                }
                let next_distance = distance + 1;
                result.push((next_distance, self.dag.graph()[neighbor].clone()));
                queue.push_back((neighbor, next_distance));
            }
        }
        result.sort();
        Ok(result)
    }
}

fn score_node(
    graph: &petgraph::Graph<String, f64, petgraph::Directed, u32>,
    node: NodeIndex,
    reach: &[usize],
) -> f64 {
    let mut score = 0.0;
    for edge in graph.edges_directed(node, Direction::Outgoing) {
        let child = edge.target();
        score += *edge.weight() * reach.get(child.index()).copied().unwrap_or(1) as f64;
    }
    score
}

#[cfg(test)]
mod tests {
    use super::DependencyGraph;
    use crate::model::IssueHeading;
    use std::collections::BTreeMap;

    fn heading(id: &str, blocked: &str) -> IssueHeading {
        let mut properties = BTreeMap::new();
        properties.insert("ID".into(), id.into());
        if !blocked.is_empty() {
            properties.insert("BLOCKED_BY".into(), blocked.into());
        }
        IssueHeading {
            id: id.into(),
            title: id.into(),
            state: "TODO".into(),
            priority: 'B',
            properties,
            org_tags: Vec::new(),
            statistics: None,
            property_order: vec!["ID".into(), "BLOCKED_BY".into()],
            extra_drawers: Vec::new(),
            body: String::new(),
            logbook: Vec::new(),
            line_start: 1,
            line_end: 4,
        }
    }

    #[test]
    fn an_unweighted_edge_weighs_one_and_a_marker_is_stored() {
        let issues = [heading("a", ""), heading("b", "a*2.5"), heading("c", "a")];
        let graph = DependencyGraph::from_headings(issues.iter()).unwrap();
        assert_eq!(graph.edge_weight("a", "b"), Some(2.5));
        assert_eq!(graph.edge_weight("a", "c"), Some(1.0));
        assert_eq!(graph.edge_weight("b", "c"), None);
    }

    #[test]
    fn unblock_score_is_weight_times_downstream_reach() {
        // a -2→ b -1→ d
        // a -1→ c -1→ d
        // reach(d) = 1, reach(b) = 2, reach(c) = 2
        // unblock(a) = 2*2 + 1*2 = 6
        // unblock(b) = 1*1 = 1
        let issues = [
            heading("a", ""),
            heading("b", "a*2"),
            heading("c", "a"),
            heading("d", "b c"),
        ];
        let graph = DependencyGraph::from_headings(issues.iter()).unwrap();
        assert!((graph.unblock_score("a") - 6.0).abs() < 1e-9);
        assert!((graph.unblock_score("b") - 1.0).abs() < 1e-9);
        assert!((graph.unblock_score("c") - 1.0).abs() < 1e-9);
        assert!((graph.unblock_score("d") - 0.0).abs() < 1e-9);
        assert!((graph.unblock_score("missing") - 0.0).abs() < 1e-9);
    }

    #[test]
    fn unblock_score_scales_past_one_bitset_chunk() {
        // A 1200-node chain plus a second diamond branch spans three 512-bit
        // chunks and verifies exact descendant deduplication across chunks.
        let n = 1200usize;
        let mut issues = Vec::with_capacity(n);
        issues.push(heading("n-0000", ""));
        for i in 1..n {
            let blocked = if i == n - 1 {
                format!("n-{:04} n-{:04}", i - 1, i - 2)
            } else {
                format!("n-{:04}", i - 1)
            };
            issues.push(heading(&format!("n-{i:04}"), &blocked));
        }
        let graph = DependencyGraph::from_headings(issues.iter()).unwrap();
        let scores = graph.unblock_scores();
        assert!((scores["n-0000"] - (n - 1) as f64).abs() < 1e-9);
        assert!((scores["n-0512"] - (n - 1 - 512) as f64).abs() < 1e-9);
        assert!((scores[&format!("n-{:04}", n - 3)] - 3.0).abs() < 1e-9);
    }
}
