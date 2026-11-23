// ABOUTME: Per-node review inputs: direct dependents and covering tests (TESTED_BY).
// ABOUTME: Feeds the risk order of get_review_context.

use super::{GraphQuery, is_dependency};
use anyhow::Result;
use petgraph::Direction::{Incoming, Outgoing};

/// Direct dependents and test coverage of one node.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NodeCoverage {
    pub id: String,
    /// Distinct nodes with a dependency edge into this node.
    pub dependents: usize,
    /// Distinct tests with a TESTED_BY edge from this node.
    pub tests: usize,
}

impl GraphQuery {
    /// Coverage for each known id, in the order given; unknown ids are skipped.
    pub async fn node_coverage(&self, node_ids: &[String]) -> Result<Vec<NodeCoverage>> {
        let snap = self.snapshot().await?;
        Ok(node_ids
            .iter()
            .filter_map(|id| {
                let ix = snap.resolve(id)?;
                Some(NodeCoverage {
                    id: snap.node(ix).id.clone(),
                    dependents: snap.neighbours(ix, Incoming, is_dependency).len(),
                    tests: snap.neighbours(ix, Outgoing, |k| k == "TESTED_BY").len(),
                })
            })
            .collect())
    }
}
