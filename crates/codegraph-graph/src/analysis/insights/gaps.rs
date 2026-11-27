// ABOUTME: Knowledge gaps: isolated nodes, thin and single-file communities, untested hotspots.
// ABOUTME: Ports find_knowledge_gaps from code-review-graph analysis.py (import cycles included).

use super::super::{AnalysisGraph, AnalysisNode, sanitize_name};
use super::cycles::{ImportCycle, import_cycles};
use super::undirected_degree;
use crate::graph_query::CommunityRow;
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, HashSet};

const MAX_ISOLATED: usize = 50;
const MAX_UNTESTED_HOTSPOTS: usize = 20;

#[derive(Debug, Clone, Serialize)]
pub struct GapNode {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub file: String,
    pub degree: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThinCommunity {
    pub community_id: i64,
    pub name: String,
    pub size: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SingleFileCommunity {
    pub community_id: i64,
    pub name: String,
    pub size: usize,
    pub file: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct KnowledgeGaps {
    pub isolated_nodes: Vec<GapNode>,
    pub thin_communities: Vec<ThinCommunity>,
    pub untested_hotspots: Vec<GapNode>,
    pub single_file_communities: Vec<SingleFileCommunity>,
    pub import_cycles: Vec<ImportCycle>,
}

fn gap_node(n: &AnalysisNode, degree: usize) -> GapNode {
    GapNode {
        name: sanitize_name(&n.name),
        qualified_name: n.qualified.clone(),
        kind: n.kind.clone(),
        file: n.file_path.clone(),
        degree,
    }
}

/// Structural weaknesses (`find_knowledge_gaps` in the CRG). `communities` may come in any order;
/// they are reported in id order. `repo_root` is stripped from import-cycle paths.
pub fn knowledge_gaps(
    g: &AnalysisGraph,
    communities: &[CommunityRow],
    repo_root: Option<&str>,
) -> KnowledgeGaps {
    let degree = undirected_degree(g);
    let tested: HashSet<&str> = g
        .edges
        .iter()
        .filter(|e| e.kind == "TESTED_BY")
        .map(|e| e.source.as_str())
        .collect();
    // Canonical order, as the CRG's rowid order on a fresh build: the isolated cap and the
    // stable hotspot sort keep it.
    let nodes: Vec<&AnalysisNode> = g
        .canonical_nodes()
        .into_iter()
        .map(|i| &g.nodes[i])
        .filter(|n| n.kind != "File")
        .collect();
    let deg = |n: &AnalysisNode| degree.get(n.qualified.as_str()).copied().unwrap_or(0);

    let isolated_nodes: Vec<GapNode> = nodes
        .iter()
        .filter(|n| deg(n) <= 1)
        .take(MAX_ISOLATED)
        .map(|n| gap_node(n, deg(n)))
        .collect();

    let mut comm_sizes: HashMap<i64, usize> = HashMap::new();
    let mut comm_files: HashMap<i64, BTreeSet<&str>> = HashMap::new();
    for n in &nodes {
        if let Some(cid) = n.community_id {
            *comm_sizes.entry(cid).or_default() += 1;
            comm_files
                .entry(cid)
                .or_default()
                .insert(n.file_path.as_str());
        }
    }
    let mut ordered: Vec<&CommunityRow> = communities.iter().collect();
    ordered.sort_by_key(|c| c.id);
    let size_of = |cid: i64| comm_sizes.get(&cid).copied().unwrap_or(0);
    let thin_communities = ordered
        .iter()
        .filter(|c| size_of(c.id) < 3)
        .map(|c| ThinCommunity {
            community_id: c.id,
            name: c.name.clone(),
            size: size_of(c.id),
        })
        .collect();

    let mut untested_hotspots: Vec<GapNode> = nodes
        .iter()
        .filter(|n| deg(n) >= 5 && !tested.contains(n.qualified.as_str()) && !n.is_test)
        .map(|n| gap_node(n, deg(n)))
        .collect();
    untested_hotspots.sort_by_key(|n| std::cmp::Reverse(n.degree));
    untested_hotspots.truncate(MAX_UNTESTED_HOTSPOTS);

    let single_file_communities = ordered
        .iter()
        .filter_map(|c| {
            let files = comm_files.get(&c.id)?;
            let size = size_of(c.id);
            (files.len() == 1 && size >= 3).then(|| SingleFileCommunity {
                community_id: c.id,
                name: c.name.clone(),
                size,
                file: files
                    .iter()
                    .next()
                    .map(|f| f.to_string())
                    .unwrap_or_default(),
            })
        })
        .collect();

    KnowledgeGaps {
        isolated_nodes,
        thin_communities,
        untested_hotspots,
        single_file_communities,
        import_cycles: import_cycles(g, repo_root),
    }
}
