// ABOUTME: Graph insights: hubs, bridges (betweenness), knowledge gaps, import cycles, surprises.
// ABOUTME: Ports analysis.py (code-review-graph); Brandes is exact up to 5000 vertices, then sampled.

use super::{AnalysisGraph, round_half_even, sanitize_name};
use crate::graph_query::CommunityRow;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

/// Above this many vertices betweenness uses a fixed sample of sources.
pub const EXACT_BETWEENNESS_LIMIT: usize = 5000;
/// Sources used when sampling.
pub const SAMPLE_SOURCES: usize = 500;

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

#[derive(Debug, Clone, Serialize)]
pub struct BridgeRow {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub file: String,
    pub betweenness: f64,
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
    rows.sort_by(|a, b| b.total_degree.cmp(&a.total_degree));
    rows
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

/// Files and hops of one import cycle spelled out; the rest is told as a count.
pub const MAX_CYCLE_FILES: usize = 10;
pub const MAX_CYCLE_HOPS: usize = 10;
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

#[derive(Debug, Clone, Serialize)]
pub struct CycleHop {
    pub file: String,
    pub line: i64,
    pub imports: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportCycle {
    /// One loop written out, `a -> b -> a`.
    pub name: String,
    /// Files in the strongly connected group.
    pub size: usize,
    /// The loop can fail as the files load; false means every loop has a deferred import.
    pub load_time: bool,
    pub files: Vec<String>,
    pub loop_length: usize,
    pub cycle: Vec<CycleHop>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct KnowledgeGaps {
    pub isolated_nodes: Vec<GapNode>,
    pub thin_communities: Vec<ThinCommunity>,
    pub untested_hotspots: Vec<GapNode>,
    pub single_file_communities: Vec<SingleFileCommunity>,
    pub import_cycles: Vec<ImportCycle>,
}

fn undirected_degree(g: &AnalysisGraph) -> HashMap<&str, usize> {
    let mut degree: HashMap<&str, usize> = HashMap::new();
    for e in &g.edges {
        *degree.entry(e.source.as_str()).or_default() += 1;
        *degree.entry(e.target.as_str()).or_default() += 1;
    }
    degree
}

fn gap_node(n: &super::AnalysisNode, degree: usize) -> GapNode {
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
    let nodes: Vec<&super::AnalysisNode> = g.nodes.iter().filter(|n| n.kind != "File").collect();
    let deg = |n: &super::AnalysisNode| degree.get(n.qualified.as_str()).copied().unwrap_or(0);

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

/// The fewest files that lead from `start` back to itself (BFS, neighbours in name order).
fn shortest_loop(adjacency: &BTreeMap<&str, BTreeSet<&str>>, start: &str) -> Vec<String> {
    let mut came_from: HashMap<&str, &str> = HashMap::new();
    let mut queue = VecDeque::from([start]);
    while let Some(current) = queue.pop_front() {
        for &next in adjacency.get(current).into_iter().flatten() {
            if next == start {
                let mut path = vec![current];
                while *path.last().expect("non-empty") != start {
                    path.push(came_from[path.last().expect("non-empty")]);
                }
                return path.into_iter().rev().map(str::to_string).collect();
            }
            if !came_from.contains_key(next) {
                came_from.insert(next, current);
                queue.push_back(next);
            }
        }
    }
    vec![start.to_string()]
}

/// Strongly connected groups with more than one file (petgraph Tarjan).
fn import_groups<'a>(
    pairs: impl Iterator<Item = &'a (&'a str, &'a str)>,
) -> Vec<BTreeSet<&'a str>> {
    let mut graph = petgraph::graph::DiGraph::<&str, ()>::new();
    let mut index: BTreeMap<&str, petgraph::graph::NodeIndex> = BTreeMap::new();
    for &(source, target) in pairs {
        let s = *index
            .entry(source)
            .or_insert_with(|| graph.add_node(source));
        let t = *index
            .entry(target)
            .or_insert_with(|| graph.add_node(target));
        graph.add_edge(s, t, ());
    }
    petgraph::algo::tarjan_scc(&graph)
        .into_iter()
        .filter(|c| c.len() > 1)
        .map(|c| c.into_iter().map(|i| graph[i]).collect())
        .collect()
}

fn adjacency<'a>(
    pairs: &BTreeMap<(&'a str, &'a str), i64>,
) -> BTreeMap<&'a str, BTreeSet<&'a str>> {
    let mut result: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for &(source, target) in pairs.keys() {
        result.entry(source).or_default().insert(target);
    }
    result
}

/// Groups of files that import each other (`find_import_cycles` in the CRG), worst first:
/// load-time loops, then bigger groups, then by name. Type-only and parent-package imports never
/// count; deferred imports count for the group but not for `load_time`.
pub fn import_cycles(g: &AnalysisGraph, repo_root: Option<&str>) -> Vec<ImportCycle> {
    let mut files: HashSet<&str> = g.nodes.iter().map(|n| n.file_path.as_str()).collect();
    files.extend(g.edges.iter().map(|e| e.file_path.as_str()));
    files.extend(
        g.nodes
            .iter()
            .filter(|n| n.kind == "File")
            .map(|n| n.qualified.as_str()),
    );
    let mut all: BTreeMap<(&str, &str), i64> = BTreeMap::new();
    let mut load: BTreeMap<(&str, &str), i64> = BTreeMap::new();
    for e in &g.edges {
        let (source, target) = (e.source.as_str(), e.target.as_str());
        if e.kind != "IMPORTS_FROM"
            || source == target
            || !files.contains(source)
            || !files.contains(target)
        {
            continue;
        }
        let context = e.import_context.as_deref();
        if matches!(context, Some("type_only" | "parent_package")) {
            continue;
        }
        let line = all.entry((source, target)).or_insert(e.line);
        *line = (*line).min(e.line);
        if context != Some("deferred") {
            let line = load.entry((source, target)).or_insert(e.line);
            *line = (*line).min(e.line);
        }
    }
    let all_keys: Vec<(&str, &str)> = all.keys().copied().collect();
    let groups = import_groups(all_keys.iter());
    if groups.is_empty() {
        return Vec::new();
    }
    let load_keys: Vec<(&str, &str)> = load.keys().copied().collect();
    let mut load_groups = import_groups(load_keys.iter());
    load_groups.sort_by(|a, b| b.len().cmp(&a.len()).then(a.first().cmp(&b.first())));

    let prefix = repo_root.map(|r| format!("{}/", r.replace('\\', "/").trim_end_matches('/')));
    let short = |path: &str| -> String {
        let p = prefix
            .as_deref()
            .and_then(|p| path.strip_prefix(p))
            .unwrap_or(path);
        sanitize_name(p)
    };
    let all_adjacency = adjacency(&all);
    let load_adjacency = adjacency(&load);

    let mut cycles: Vec<ImportCycle> = groups
        .iter()
        .map(|group| {
            let load_group = load_groups.iter().find(|lg| lg.is_subset(group));
            // Groups come from at least two-file components, so `first` always exists.
            let path = match load_group {
                Some(lg) => shortest_loop(&load_adjacency, lg.first().copied().unwrap_or_default()),
                None => shortest_loop(&all_adjacency, group.first().copied().unwrap_or_default()),
            };
            let hops: Vec<(&str, &str)> = (0..path.len())
                .map(|i| (path[i].as_str(), path[(i + 1) % path.len()].as_str()))
                .collect();
            let written: Vec<String> = path.iter().chain(path.first()).map(|p| short(p)).collect();
            ImportCycle {
                name: sanitize_name(&written.join(" -> ")),
                size: group.len(),
                load_time: load_group.is_some(),
                files: group
                    .iter()
                    .take(MAX_CYCLE_FILES)
                    .map(|p| short(p))
                    .collect(),
                loop_length: path.len(),
                cycle: hops
                    .iter()
                    .take(MAX_CYCLE_HOPS)
                    .map(|&(source, target)| CycleHop {
                        file: short(source),
                        line: load
                            .get(&(source, target))
                            .or_else(|| all.get(&(source, target)))
                            .copied()
                            .unwrap_or(0),
                        imports: short(target),
                    })
                    .collect(),
            }
        })
        .collect();
    cycles.sort_by(|a, b| {
        (!a.load_time, std::cmp::Reverse(a.size), &a.name).cmp(&(
            !b.load_time,
            std::cmp::Reverse(b.size),
            &b.name,
        ))
    });
    cycles
}

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
/// (`find_surprising_connections` in the CRG), highest score first; ties keep edge order.
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
    for e in &g.edges {
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

/// One generated review question.
#[derive(Debug, Clone, Serialize)]
pub struct Question {
    pub category: &'static str,
    pub question: String,
    pub target: String,
    pub priority: &'static str,
}

/// Review questions from the other analyses (`generate_suggested_questions` in the CRG): bridges,
/// untested hubs, cross-community surprises, thin communities, untested hotspots, in that order.
pub fn suggested_questions(g: &AnalysisGraph, communities: &[CommunityRow]) -> Vec<Question> {
    let mut out = Vec::new();
    for b in bridge_nodes(g).into_iter().take(3) {
        out.push(Question {
            category: "bridge_node",
            question: format!(
                "'{}' is a critical connector between multiple code regions. Is it adequately \
                 tested and documented?",
                b.name
            ),
            target: b.qualified_name,
            priority: "high",
        });
    }
    let tested: HashSet<&str> = g
        .edges
        .iter()
        .filter(|e| e.kind == "TESTED_BY")
        .map(|e| e.source.as_str())
        .collect();
    for h in hub_nodes(g).into_iter().take(3) {
        if !tested.contains(h.qualified_name.as_str()) {
            out.push(Question {
                category: "hub_risk",
                question: format!(
                    "Hub node '{}' has {} connections but no direct test coverage. Should it be \
                     tested?",
                    h.name, h.total_degree
                ),
                target: h.qualified_name,
                priority: "high",
            });
        }
    }
    for s in surprising_connections(g).into_iter().take(3) {
        if s.reasons.contains(&"cross-community") {
            let cid = |c: Option<i64>| c.map_or("None".to_string(), |c| c.to_string());
            out.push(Question {
                category: "surprising_connection",
                question: format!(
                    "'{}' (community {}) calls '{}' (community {}). Is this coupling intentional?",
                    s.source,
                    cid(s.source_community),
                    s.target,
                    cid(s.target_community)
                ),
                target: s.source_qualified,
                priority: "medium",
            });
        }
    }
    let gaps = knowledge_gaps(g, communities, None);
    for c in gaps.thin_communities.into_iter().take(2) {
        out.push(Question {
            category: "thin_community",
            question: format!(
                "Community '{}' has only {} member(s). Should it be merged with a neighbor?",
                c.name, c.size
            ),
            target: format!("community:{}", c.community_id),
            priority: "low",
        });
    }
    for h in gaps.untested_hotspots.into_iter().take(2) {
        out.push(Question {
            category: "untested_hotspot",
            question: format!(
                "'{}' has {} connections but no test coverage. Is this a risk?",
                h.name, h.degree
            ),
            target: h.qualified_name,
            priority: "medium",
        });
    }
    out
}
