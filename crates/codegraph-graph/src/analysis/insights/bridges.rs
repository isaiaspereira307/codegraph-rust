// ABOUTME: Bridge nodes: Brandes betweenness over the directed edge graph.
// ABOUTME: Exact up to EXACT_BETWEENNESS_LIMIT vertices, then a fixed name-ordered source sample.

use super::super::{AnalysisGraph, round_half_even, sanitize_name};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, VecDeque};

/// Above this many vertices betweenness uses a fixed sample of sources.
pub const EXACT_BETWEENNESS_LIMIT: usize = 5000;
/// Sources used when sampling.
pub const SAMPLE_SOURCES: usize = 500;

#[derive(Debug, Clone, Serialize)]
pub struct BridgeRow {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub file: String,
    pub betweenness: f64,
    pub community_id: Option<i64>,
}

/// Brandes betweenness on a directed, unweighted graph, normalised by `1 / ((n-1)(n-2))`. With a
/// subset of `sources` the sums are rescaled by `n / sources.len()`.
pub fn betweenness(n: usize, adj: &[Vec<usize>], sources: Option<&[usize]>) -> Vec<f64> {
    let mut score = vec![0.0f64; n];
    if n == 0 {
        return score;
    }
    let all: Vec<usize>;
    let sources = match sources {
        Some(s) => s,
        None => {
            all = (0..n).collect();
            &all
        }
    };
    let mut dist = vec![-1i64; n];
    let mut sigma = vec![0.0f64; n];
    let mut delta = vec![0.0f64; n];
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &s in sources {
        let mut order = Vec::new();
        let mut queue = VecDeque::from([s]);
        dist[s] = 0;
        sigma[s] = 1.0;
        while let Some(v) = queue.pop_front() {
            order.push(v);
            for &w in &adj[v] {
                if dist[w] < 0 {
                    dist[w] = dist[v] + 1;
                    queue.push_back(w);
                }
                if dist[w] == dist[v] + 1 {
                    sigma[w] += sigma[v];
                    preds[w].push(v);
                }
            }
        }
        while let Some(w) = order.pop() {
            for &v in &preds[w] {
                delta[v] += sigma[v] / sigma[w] * (1.0 + delta[w]);
            }
            if w != s {
                score[w] += delta[w];
            }
            dist[w] = -1;
            sigma[w] = 0.0;
            delta[w] = 0.0;
            preds[w].clear();
        }
    }
    if n > 2 {
        let mut scale = 1.0 / ((n - 1) as f64 * (n - 2) as f64);
        if sources.len() < n {
            scale *= n as f64 / sources.len() as f64;
        }
        for s in &mut score {
            *s *= scale;
        }
    }
    score
}

/// Non-File nodes with positive betweenness over the directed graph of every edge (parallel edges
/// collapsed; text endpoints are vertices too), highest first; ties by node id.
pub fn bridge_nodes(g: &AnalysisGraph) -> Vec<BridgeRow> {
    let mut index: HashMap<&str, usize> = HashMap::new();
    let mut names: Vec<&str> = Vec::new();
    for e in &g.edges {
        for qn in [e.source.as_str(), e.target.as_str()] {
            if !index.contains_key(qn) {
                index.insert(qn, names.len());
                names.push(qn);
            }
        }
    }
    let n = names.len();
    if n == 0 {
        return Vec::new();
    }
    let mut adj: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    for e in &g.edges {
        let (a, b) = (index[e.source.as_str()], index[e.target.as_str()]);
        if a != b {
            adj[a].insert(b);
        }
    }
    let adj: Vec<Vec<usize>> = adj.into_iter().map(|s| s.into_iter().collect()).collect();
    let scores = if n > EXACT_BETWEENNESS_LIMIT {
        let k = SAMPLE_SOURCES.min(n);
        let mut by_name: Vec<usize> = (0..n).collect();
        by_name.sort_by(|&a, &b| names[a].cmp(names[b]));
        let step = n / k;
        let sources: Vec<usize> = (0..k).map(|i| by_name[i * step]).collect();
        betweenness(n, &adj, Some(&sources))
    } else {
        betweenness(n, &adj, None)
    };
    let mut rows: Vec<BridgeRow> = g
        .nodes
        .iter()
        .filter(|node| node.kind != "File")
        .filter_map(|node| {
            let score = scores[*index.get(node.qualified.as_str())?];
            (score > 0.0).then(|| BridgeRow {
                name: sanitize_name(&node.name),
                qualified_name: node.qualified.clone(),
                kind: node.kind.clone(),
                file: node.file_path.clone(),
                betweenness: round_half_even(score, 6),
                community_id: node.community_id,
            })
        })
        .collect();
    rows.sort_by(|a, b| b.betweenness.total_cmp(&a.betweenness));
    rows
}
