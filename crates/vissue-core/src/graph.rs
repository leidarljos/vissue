//! In-memory dependency graph for validation and bounded multi-hop queries.

use anyhow::anyhow;

use crate::error::Result;
use daggy::{Dag, NodeIndex};
use petgraph::Direction;
use petgraph::algo::{has_path_connecting, toposort};
use std::collections::{HashMap, HashSet, VecDeque};

use crate::error::Error;
use crate::model::IssueHeading;

/// A blocker edge points from the prerequisite to the issue waiting on it.
#[derive(Debug)]
pub struct DependencyGraph {
    dag: Dag<String, ()>,
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
            for blocker in issue.blocked_by() {
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
                    .add_edge(from, to, ())
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
