// ABOUTME: Graph insights: hubs, bridges (betweenness), knowledge gaps, import cycles, surprises.
// ABOUTME: Ports analysis.py (code-review-graph); one submodule per analysis, re-exported here.

mod bridges;
mod cycles;
mod gaps;
mod hubs;
mod questions;
mod surprising;

pub use bridges::{BridgeRow, EXACT_BETWEENNESS_LIMIT, SAMPLE_SOURCES, betweenness, bridge_nodes};
pub use cycles::{CycleHop, ImportCycle, MAX_CYCLE_FILES, MAX_CYCLE_HOPS, import_cycles};
pub use gaps::{GapNode, KnowledgeGaps, SingleFileCommunity, ThinCommunity, knowledge_gaps};
pub use hubs::{HubRow, hub_nodes};
pub use questions::{Question, suggested_questions};
pub use surprising::{SurprisingRow, surprising_connections};

use super::AnalysisGraph;
use std::collections::HashMap;

/// Edge ends per qualified name, both directions and repeats counted.
fn undirected_degree(g: &AnalysisGraph) -> HashMap<&str, usize> {
    let mut degree: HashMap<&str, usize> = HashMap::new();
    for e in &g.edges {
        *degree.entry(e.source.as_str()).or_default() += 1;
        *degree.entry(e.target.as_str()).or_default() += 1;
    }
    degree
}
