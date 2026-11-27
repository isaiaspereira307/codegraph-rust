// ABOUTME: Surprising connections: edges scored for unexpected cross-boundary coupling.
// ABOUTME: Ports find_surprising_connections from code-review-graph analysis.py.

use super::super::{AnalysisGraph, round_half_even, sanitize_name};
use super::undirected_degree;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SurprisingRow {
    pub source: String,
    pub source_qualified: String,
    pub target: String,
    pub target_qualified: String,
    pub edge_kind: String,
    pub surprise_score: f64,
    pub reasons: Vec<&'static str>,
    pub source_community: Option<i64>,
    pub target_community: Option<i64>,
}

/// Text after the last `.` of the whole path, or empty (the CRG's language guess).
fn extension(path: &str) -> &str {
    path.rsplit_once('.').map_or("", |(_, ext)| ext)
}

/// Edges between two non-File nodes scored for unexpected coupling
/// (`find_surprising_connections` in the CRG), highest score first; ties keep canonical edge
/// order ([`AnalysisGraph::canonical_edges`]).
pub fn surprising_connections(g: &AnalysisGraph) -> Vec<SurprisingRow> {
    let degree = undirected_degree(g);
    let mut degrees: Vec<usize> = degree.values().copied().filter(|&d| d > 0).collect();
    if degrees.is_empty() {
        return Vec::new();
    }
    degrees.sort_unstable();
    let median = degrees[degrees.len() / 2];
    let hub_threshold = (median * 3).max(10);
    let deg = |qn: &str| degree.get(qn).copied().unwrap_or(0);

    let mut rows = Vec::new();
    for e in g.canonical_edges() {
        let (Some(si), Some(ti)) = (g.index_of(&e.source), g.index_of(&e.target)) else {
            continue;
        };
        let (src, tgt) = (&g.nodes[si], &g.nodes[ti]);
        if src.kind == "File" || tgt.kind == "File" {
            continue;
        }
        let mut score = 0.0f64;
        let mut reasons = Vec::new();
        if let (Some(a), Some(b)) = (src.community_id, tgt.community_id)
            && a != b
        {
            score += 0.3;
            reasons.push("cross-community");
        }
        let (src_lang, tgt_lang) = (extension(&src.file_path), extension(&tgt.file_path));
        if !src_lang.is_empty() && !tgt_lang.is_empty() && src_lang != tgt_lang {
            score += 0.2;
            reasons.push("cross-language");
        }
        let (src_deg, tgt_deg) = (deg(&e.source), deg(&e.target));
        if (src_deg <= 2 && tgt_deg >= hub_threshold) || (tgt_deg <= 2 && src_deg >= hub_threshold)
        {
            score += 0.2;
            reasons.push("peripheral-to-hub");
        }
        if src.is_test != tgt.is_test && e.kind == "CALLS" {
            score += 0.15;
            reasons.push("cross-test-boundary");
        }
        if e.kind == "CALLS" && src.kind == "Type" {
            score += 0.15;
            reasons.push("unusual-edge-kind");
        }
        if score > 0.0 {
            rows.push(SurprisingRow {
                source: sanitize_name(&src.name),
                source_qualified: e.source.clone(),
                target: sanitize_name(&tgt.name),
                target_qualified: e.target.clone(),
                edge_kind: e.kind.clone(),
                surprise_score: round_half_even(score, 2),
                reasons,
                source_community: src.community_id,
                target_community: tgt.community_id,
            });
        }
    }
    rows.sort_by(|a, b| b.surprise_score.total_cmp(&a.surprise_score));
    rows
}
