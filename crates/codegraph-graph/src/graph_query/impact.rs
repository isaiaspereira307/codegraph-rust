// ABOUTME: Impact radius of a change: transitive dependents of changed nodes plus the tests covering them.
// ABOUTME: Multi-source BFS over reverse dependency edges, bounded by depth and a node budget.

use super::types::NodeReference;
use super::{GraphQuery, is_dependency};
use anyhow::Result;
use petgraph::Direction::{Incoming, Outgoing};
use petgraph::graph::NodeIndex;
use std::collections::{BTreeSet, HashSet};

const DEFAULT_DEPTH: i32 = 3;
const MAX_DEPTH: i32 = 10;
const DEFAULT_MAX_NODES: usize = 200;
const MAX_NODES: usize = 1000;

/// A node reached by the impact walk, with the depth it was first reached at.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImpactedNode {
    #[serde(flatten)]
    pub node: NodeReference,
    pub depth: i32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImpactRadius {
    /// Changed nodes: the given node_ids plus every node in the given files that exists.
    pub changed: Vec<NodeReference>,
    /// Transitive dependents of the changed nodes, nearest first.
    pub impacted: Vec<ImpactedNode>,
    /// Distinct file paths of changed and impacted nodes, sorted.
    pub affected_files: Vec<String>,
    /// Tests that cover a changed or impacted node (TESTED_BY), sorted by name.
    pub tests: Vec<NodeReference>,
    /// True when `max_nodes` stopped the walk before it ran out of dependents.
    pub truncated: bool,
}

impl GraphQuery {
    /// Dependents of `node_ids` and of every node whose file path is one of `files` (exact or
    /// as a path suffix). Unknown ids are ignored. `depth` defaults to 3 (cap 10); `max_nodes`
    /// defaults to 200 (cap 1000).
    pub async fn get_impact_radius(
        &self,
        node_ids: &[String],
        files: &[String],
        depth: i32,
        max_nodes: usize,
    ) -> Result<ImpactRadius> {
        let snap = self.snapshot().await?;
        let depth = if (1..=MAX_DEPTH).contains(&depth) {
            depth
        } else {
            DEFAULT_DEPTH
        };
        let budget = match max_nodes {
            0 => DEFAULT_MAX_NODES,
            n => n.min(MAX_NODES),
        };

        let mut seeds: BTreeSet<(&str, &str, NodeIndex)> = BTreeSet::new();
        let mut seed_set: HashSet<NodeIndex> = HashSet::new();
        for id in node_ids {
            if let Some(ix) = snap.resolve(id) {
                if seed_set.insert(ix) {
                    seeds.insert((snap.order_key(ix).0, snap.order_key(ix).1, ix));
                }
            }
        }
        for ix in snap.graph.node_indices() {
            let path = snap.node(ix).file_path.as_str();
            let hit = files
                .iter()
                .any(|f| path == f || path.ends_with(&format!("/{f}")));
            if hit && seed_set.insert(ix) {
                seeds.insert((snap.order_key(ix).0, snap.order_key(ix).1, ix));
            }
        }
        let changed_ix: Vec<NodeIndex> = seeds.iter().map(|(_, _, ix)| *ix).collect();

        let mut visited: HashSet<NodeIndex> = seed_set.clone();
        let mut frontier = changed_ix.clone();
        let mut impacted: Vec<(NodeIndex, i32)> = Vec::new();
        let mut truncated = false;
        'walk: for level in 1..=depth {
            let mut found = Vec::new();
            for &n in &frontier {
                for m in snap.neighbours(n, Incoming, is_dependency) {
                    if visited.insert(m) {
                        found.push(m);
                    }
                }
            }
            if found.is_empty() {
                break;
            }
            found.sort_by(|a, b| snap.order_key(*a).cmp(&snap.order_key(*b)));
            for &m in &found {
                if impacted.len() == budget {
                    truncated = true;
                    break 'walk;
                }
                impacted.push((m, level));
            }
            frontier = found;
        }

        let mut files_out: BTreeSet<String> = BTreeSet::new();
        for &ix in changed_ix.iter().chain(impacted.iter().map(|(ix, _)| ix)) {
            files_out.insert(snap.node(ix).file_path.clone());
        }

        let mut tests: BTreeSet<(&str, &str, NodeIndex)> = BTreeSet::new();
        for &ix in changed_ix.iter().chain(impacted.iter().map(|(ix, _)| ix)) {
            for t in snap.neighbours(ix, Outgoing, |k| k == "TESTED_BY") {
                tests.insert((snap.order_key(t).0, snap.order_key(t).1, t));
            }
        }

        Ok(ImpactRadius {
            changed: changed_ix.iter().map(|&ix| snap.reference(ix)).collect(),
            impacted: impacted
                .into_iter()
                .map(|(ix, d)| ImpactedNode {
                    node: snap.reference(ix),
                    depth: d,
                })
                .collect(),
            affected_files: files_out.into_iter().collect(),
            tests: tests
                .into_iter()
                .map(|(_, _, ix)| snap.reference(ix))
                .collect(),
            truncated,
        })
    }
}
