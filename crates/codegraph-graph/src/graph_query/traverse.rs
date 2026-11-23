// ABOUTME: Breadth-first traversal from one node over dependency edges, in either direction.
// ABOUTME: Bounded by depth and by a node budget; reports when the budget cut the walk short.

use super::types::NodeReference;
use super::{GraphQuery, is_dependency};
use crate::sqlite_store::mapping::edge_kind_from_input;
use anyhow::Result;
use petgraph::Direction::{self, Incoming, Outgoing};
use petgraph::graph::NodeIndex;
use std::collections::HashSet;

const DEFAULT_DEPTH: i32 = 3;
const MAX_DEPTH: i32 = 10;
const DEFAULT_MAX_NODES: usize = 100;
const MAX_NODES: usize = 500;

/// Which way the walk follows edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraverseDirection {
    Out,
    In,
    Both,
}

impl TraverseDirection {
    pub fn parse(input: &str) -> Option<Self> {
        Some(match input {
            "out" => Self::Out,
            "in" => Self::In,
            "both" => Self::Both,
            _ => return None,
        })
    }

    fn directions(self) -> &'static [Direction] {
        match self {
            Self::Out => &[Outgoing],
            Self::In => &[Incoming],
            Self::Both => &[Outgoing, Incoming],
        }
    }
}

/// One node reached by the walk, with the depth it was first reached at.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TraversedNode {
    #[serde(flatten)]
    pub node: NodeReference,
    pub depth: i32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TraversalResult {
    pub start: NodeReference,
    /// Reached nodes, nearest first; the start node is not included.
    pub nodes: Vec<TraversedNode>,
    /// True when `max_nodes` stopped the walk before it ran out of edges.
    pub truncated: bool,
}

impl GraphQuery {
    /// Walks from `node_id` along `edge_type` (any dependency edge when `None`). `max_depth`
    /// defaults to 3 and is capped at 10; `max_nodes` defaults to 100 and is capped at 500.
    /// Returns `None` when the node is unknown.
    pub async fn traverse_graph(
        &self,
        node_id: &str,
        direction: TraverseDirection,
        edge_type: Option<&str>,
        max_depth: i32,
        max_nodes: usize,
    ) -> Result<Option<TraversalResult>> {
        let snap = self.snapshot().await?;
        let Some(start) = snap.resolve(node_id) else {
            return Ok(None);
        };
        let kind = edge_type.map(edge_kind_from_input);
        let depth = if (1..=MAX_DEPTH).contains(&max_depth) {
            max_depth
        } else {
            DEFAULT_DEPTH
        };
        let budget = match max_nodes {
            0 => DEFAULT_MAX_NODES,
            n => n.min(MAX_NODES),
        };

        let keep = |k: &str| match &kind {
            Some(wanted) => k == wanted,
            None => is_dependency(k),
        };
        let mut visited = HashSet::from([start]);
        let mut frontier = vec![start];
        let mut reached: Vec<(NodeIndex, i32)> = Vec::new();
        let mut truncated = false;
        'walk: for level in 1..=depth {
            let mut found = Vec::new();
            for &n in &frontier {
                for &dir in direction.directions() {
                    for m in snap.neighbours(n, dir, keep) {
                        if visited.insert(m) {
                            found.push(m);
                        }
                    }
                }
            }
            if found.is_empty() {
                break;
            }
            // Sorted per level, so the budget cuts the same nodes on every run.
            found.sort_by(|a, b| snap.order_key(*a).cmp(&snap.order_key(*b)));
            for &m in &found {
                if reached.len() == budget {
                    truncated = true;
                    break 'walk;
                }
                reached.push((m, level));
            }
            frontier = found;
        }

        Ok(Some(TraversalResult {
            start: snap.reference(start),
            nodes: reached
                .into_iter()
                .map(|(ix, d)| TraversedNode {
                    node: snap.reference(ix),
                    depth: d,
                })
                .collect(),
            truncated,
        }))
    }
}
