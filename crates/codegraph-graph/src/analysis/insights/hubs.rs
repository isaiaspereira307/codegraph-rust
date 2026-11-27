// ABOUTME: Hub nodes: the most connected non-File nodes by total edge degree.
// ABOUTME: Ports find_hub_nodes from code-review-graph analysis.py.

use super::super::{AnalysisGraph, sanitize_name};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct HubRow {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub file: String,
    pub in_degree: usize,
    pub out_degree: usize,
    pub total_degree: usize,
    pub community_id: Option<i64>,
}

/// Non-File nodes with at least one edge end, by total degree (in + out over every edge, repeats
/// included), highest first; ties by node id.
pub fn hub_nodes(g: &AnalysisGraph) -> Vec<HubRow> {
    let mut degree: HashMap<&str, (usize, usize)> = HashMap::new();
    for e in &g.edges {
        degree.entry(e.source.as_str()).or_default().1 += 1;
        degree.entry(e.target.as_str()).or_default().0 += 1;
    }
    let mut rows: Vec<HubRow> = g
        .nodes
        .iter()
        .filter(|n| n.kind != "File")
        .filter_map(|n| {
            let (ind, outd) = degree
                .get(n.qualified.as_str())
                .copied()
                .unwrap_or_default();
            (ind + outd > 0).then(|| HubRow {
                name: sanitize_name(&n.name),
                qualified_name: n.qualified.clone(),
                kind: n.kind.clone(),
                file: n.file_path.clone(),
                in_degree: ind,
                out_degree: outd,
                total_degree: ind + outd,
                community_id: n.community_id,
            })
        })
        .collect();
    // `g.nodes` is in id order and the sort is stable, so ties keep id order.
    rows.sort_by_key(|r| std::cmp::Reverse(r.total_degree));
    rows
}
