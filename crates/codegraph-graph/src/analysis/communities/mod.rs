// ABOUTME: Community detection over the call/import graph: Leiden, directory fallback and split.
// ABOUTME: Deterministic port of communities.py (code-review-graph); naming and storage follow.

pub mod leiden;
pub mod naming;

use super::{AnalysisEdge, AnalysisGraph, AnalysisNode, round_half_even};
use anyhow::Result;
use leiden::{WeightedGraph, leiden};
use rusqlite::{Transaction, params, params_from_iter};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct Community {
    pub name: String,
    pub level: i64,
    pub cohesion: f64,
    pub size: usize,
    pub dominant_language: String,
    pub description: String,
    /// Qualified names, in the order the detector produced them.
    pub members: Vec<String>,
    /// `"leiden"` or `"file"`.
    pub detector: &'static str,
}

fn edge_weight(kind: &str) -> f64 {
    match kind {
        "CALLS" => 1.0,
        "IMPORTS_FROM" => 0.5,
        "INHERITS" => 0.8,
        "IMPLEMENTS" => 0.7,
        "CONTAINS" => 0.3,
        "TESTED_BY" => 0.4,
        "DEPENDS_ON" => 0.6,
        _ => 0.5,
    }
}

fn is_test_node(n: &AnalysisNode) -> bool {
    n.kind == "Test" || n.is_test
}

/// Indices (into `g.nodes`) of the non-File nodes in canonical order: file, start line, outer
/// span first, qualified name. Row ids are not used: the indexer writes rows in an order derived
/// from absolute paths, and an update appends rows. Every path shares the repository root, so
/// the order is the same wherever the repository lives. On a fresh build it is the CRG's own
/// insertion order (files by path, nodes in source order), which its Leiden vertex ids follow.
pub fn canonical_vertices(g: &AnalysisGraph) -> Vec<usize> {
    let mut vertices: Vec<usize> = (0..g.nodes.len())
        .filter(|&i| g.nodes[i].kind != "File")
        .collect();
    fn key(n: &AnalysisNode) -> (&str, i64, std::cmp::Reverse<i64>, &str) {
        (
            n.file_path.as_str(),
            n.line_start.unwrap_or(0),
            std::cmp::Reverse(n.line_end.unwrap_or(0)),
            n.qualified.as_str(),
        )
    }
    vertices.sort_by(|&a, &b| key(&g.nodes[a]).cmp(&key(&g.nodes[b])));
    vertices
}

/// Edges in canonical order (file, line, kind, source, target), independent of row ids like
/// `canonical_vertices`; the CRG adds them file by file in source order.
fn canonical_edges(g: &AnalysisGraph) -> Vec<&AnalysisEdge> {
    let mut edges: Vec<&AnalysisEdge> = g.edges.iter().collect();
    edges.sort_by(|a, b| {
        (&a.file_path, a.line, &a.kind, &a.source, &a.target).cmp(&(
            &b.file_path,
            b.line,
            &b.kind,
            &b.source,
            &b.target,
        ))
    });
    edges
}

/// Indices (into `g.nodes`) of the non-File nodes in `canonical_vertices` order, and the
/// weighted graph over them. The first edge for a vertex pair in `canonical_edges` order fixes
/// its weight; edges to unknown targets are ignored.
pub fn community_graph(g: &AnalysisGraph) -> (Vec<usize>, WeightedGraph) {
    let vertices = canonical_vertices(g);
    let index: HashMap<&str, usize> = vertices
        .iter()
        .enumerate()
        .map(|(v, &i)| (g.nodes[i].qualified.as_str(), v))
        .collect();
    let mut wg = WeightedGraph::new(vertices.len());
    for e in canonical_edges(g) {
        if let (Some(&a), Some(&b)) = (index.get(e.source.as_str()), index.get(e.target.as_str())) {
            wg.add_edge(a, b, edge_weight(&e.kind));
        }
    }
    (vertices, wg)
}

/// Internal / (internal + external) per member set, counting every edge (repeats included).
/// An edge with both ends inside one set is internal; otherwise it is external for each end that
/// belongs to a set. Unrounded.
pub fn compute_cohesion(members: &[Vec<String>], edges: &[AnalysisEdge]) -> Vec<f64> {
    let mut owner: HashMap<&str, usize> = HashMap::new();
    for (idx, set) in members.iter().enumerate() {
        for qn in set {
            owner.insert(qn.as_str(), idx);
        }
    }
    let mut internal = vec![0usize; members.len()];
    let mut external = vec![0usize; members.len()];
    for e in edges {
        let s = owner.get(e.source.as_str()).copied();
        let t = owner.get(e.target.as_str()).copied();
        match (s, t) {
            (None, None) => {}
            (Some(a), Some(b)) if a == b => internal[a] += 1,
            _ => {
                if let Some(a) = s {
                    external[a] += 1;
                }
                if let Some(b) = t {
                    external[b] += 1;
                }
            }
        }
    }
    (0..members.len())
        .map(|i| {
            let total = internal[i] + external[i];
            if total > 0 {
                internal[i] as f64 / total as f64
            } else {
                0.0
            }
        })
        .collect()
}

/// Moves each test to the cluster holding most of its distinct tested subjects (`TESTED_BY`
/// edges, direction ignored). Ties keep the current cluster, else the lowest cluster id.
fn reassign_test_nodes(
    clusters: Vec<Vec<usize>>,
    nodes: &[&AnalysisNode],
    edges: &[AnalysisEdge],
    resolution_nodes: &[&AnalysisNode],
) -> Vec<Vec<usize>> {
    let qn_to_idx: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.qualified.as_str(), i))
        .collect();
    let mut vertex_cluster: HashMap<usize, usize> = HashMap::new();
    for (c, cluster) in clusters.iter().enumerate() {
        for &v in cluster {
            vertex_cluster.insert(v, c);
        }
    }
    let mut names: HashMap<&str, Option<&str>> = HashMap::new();
    for n in resolution_nodes {
        let value = if names.contains_key(n.name.as_str()) {
            None
        } else {
            Some(n.qualified.as_str())
        };
        names.insert(n.name.as_str(), value);
    }
    let resolve = |endpoint: &str| -> Option<usize> {
        qn_to_idx.get(endpoint).copied().or_else(|| {
            names
                .get(endpoint)
                .copied()
                .flatten()
                .and_then(|qn| qn_to_idx.get(qn).copied())
        })
    };
    let mut subjects: BTreeMap<usize, HashSet<usize>> = BTreeMap::new();
    for e in edges.iter().filter(|e| e.kind == "TESTED_BY") {
        let (Some(s), Some(t)) = (resolve(&e.source), resolve(&e.target)) else {
            continue;
        };
        let (s_test, t_test) = (is_test_node(nodes[s]), is_test_node(nodes[t]));
        if s_test == t_test {
            continue;
        }
        let (test, subject) = if s_test { (s, t) } else { (t, s) };
        subjects.entry(test).or_default().insert(subject);
    }
    let mut target_by_test: HashMap<usize, usize> = HashMap::new();
    for (test, subs) in &subjects {
        let Some(&current) = vertex_cluster.get(test) else {
            continue;
        };
        let mut votes: BTreeMap<usize, usize> = BTreeMap::new();
        for s in subs {
            if let Some(&c) = vertex_cluster.get(s) {
                *votes.entry(c).or_default() += 1;
            }
        }
        let Some(&highest) = votes.values().max() else {
            continue;
        };
        let tied: Vec<usize> = votes
            .iter()
            .filter(|&(_, &v)| v == highest)
            .map(|(&c, _)| c)
            .collect();
        let target = if tied.contains(&current) {
            current
        } else {
            tied[0]
        };
        target_by_test.insert(*test, target);
    }
    let mut out = vec![Vec::new(); clusters.len()];
    for (c, cluster) in clusters.iter().enumerate() {
        for &v in cluster {
            out[*target_by_test.get(&v).unwrap_or(&c)].push(v);
        }
    }
    out
}

fn dominant_language(members: &[&AnalysisNode]) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for lang in members
        .iter()
        .filter_map(|m| m.language.as_deref())
        .filter(|l| !l.is_empty())
    {
        match counts.iter_mut().find(|(l, _)| *l == lang) {
            Some(entry) => entry.1 += 1,
            None => counts.push((lang, 1)),
        }
    }
    let mut best: Option<(&str, usize)> = None;
    for &(l, c) in &counts {
        if best.is_none_or(|(_, bc)| c > bc) {
            best = Some((l, c));
        }
    }
    best.map(|(l, _)| l.to_string()).unwrap_or_default()
}

fn group_by_cluster(part: &[usize]) -> Vec<Vec<usize>> {
    let count = part.iter().copied().max().map_or(0, |m| m + 1);
    let mut clusters = vec![Vec::new(); count];
    for (v, &c) in part.iter().enumerate() {
        clusters[c].push(v);
    }
    clusters
}

/// Detects communities (Leiden, or directory grouping when the graph has no usable edge), moves
/// tests next to their subjects, drops those under `min_size`, and splits oversized ones.
pub fn detect_communities(g: &AnalysisGraph, min_size: usize) -> Vec<Community> {
    let (vertices, wg) = community_graph(g);
    if vertices.is_empty() {
        return Vec::new();
    }
    let nodes: Vec<&AnalysisNode> = vertices.iter().map(|&i| &g.nodes[i]).collect();
    let has_edges = (0..wg.len()).any(|v| !wg.neighbors(v).is_empty());
    let communities = if has_edges {
        let n = wg.len();
        let resolution = (1.0 / (n.max(10) as f64).log10()).max(0.05);
        let part = leiden(&wg, resolution, 2);
        let clusters = reassign_test_nodes(group_by_cluster(&part), &nodes, &g.edges, &nodes);
        let kept: Vec<Vec<usize>> = clusters
            .into_iter()
            .filter(|c| c.len() >= min_size)
            .collect();
        let sets: Vec<Vec<String>> = kept
            .iter()
            .map(|c| c.iter().map(|&v| nodes[v].qualified.clone()).collect())
            .collect();
        let cohesions = compute_cohesion(&sets, &g.edges);
        kept.iter()
            .zip(sets)
            .zip(cohesions)
            .map(|((cluster, members), cohesion)| {
                let nodes_in: Vec<&AnalysisNode> = cluster.iter().map(|&v| nodes[v]).collect();
                Community {
                    name: String::new(),
                    level: 0,
                    cohesion: round_half_even(cohesion, 4),
                    size: members.len(),
                    dominant_language: dominant_language(&nodes_in),
                    description: format!("Community of {} nodes", members.len()),
                    members,
                    detector: "leiden",
                }
            })
            .collect()
    } else {
        detect_by_directory(&nodes, &g.edges, min_size)
    };
    // Parents are named before the split (sub-communities fall back to "{parent}-{n}"); names
    // are made unique last, like code-review-graph.
    let mut communities = communities;
    naming::fill_names(g, &mut communities);
    let mut communities = split_oversized(communities, g);
    naming::dedupe_names(g, &mut communities);
    communities
}

fn dir_parts(path: &str) -> (Vec<String>, String) {
    let normalized = path.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').collect();
    let last = parts.last().copied().unwrap_or("root");
    let dirs = parts[..parts.len().saturating_sub(1)]
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| p.to_string())
        .collect();
    let stem = last.rsplit_once('.').map_or(last, |(s, _)| s).to_string();
    (dirs, stem)
}

/// Groups nodes by directory below the longest common prefix, deepening the grouping until at
/// least 10 groups have `min_size` members (communities.py `_detect_file_based`).
fn detect_by_directory(
    nodes: &[&AnalysisNode],
    edges: &[AnalysisEdge],
    min_size: usize,
) -> Vec<Community> {
    let parsed: Vec<(Vec<String>, String)> =
        nodes.iter().map(|n| dir_parts(&n.file_path)).collect();
    let mut prefix_len = 0;
    if let Some(shortest) = parsed.iter().map(|(d, _)| d.len()).min() {
        for i in 0..shortest {
            if parsed.iter().all(|(d, _)| d[i] == parsed[0].0[i]) {
                prefix_len = i + 1;
            } else {
                break;
            }
        }
    }
    let group_at = |depth: usize| -> Vec<(String, Vec<usize>)> {
        let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for (i, (dirs, stem)) in parsed.iter().enumerate() {
            let remainder = &dirs[prefix_len.min(dirs.len())..];
            let key = if remainder.is_empty() {
                stem.clone()
            } else {
                remainder[..depth.min(remainder.len())].join("/")
            };
            let slot = *index.entry(key.clone()).or_insert_with(|| {
                groups.push((key, Vec::new()));
                groups.len() - 1
            });
            groups[slot].1.push(i);
        }
        groups
    };
    let max_depth = parsed
        .iter()
        .map(|(d, _)| d.len().saturating_sub(prefix_len))
        .max()
        .unwrap_or(0);
    let mut best = group_at(1);
    for depth in 1..=max_depth {
        let groups = group_at(depth);
        let qualifying = groups.iter().filter(|(_, m)| m.len() >= min_size).count();
        best = groups;
        if qualifying >= 10 {
            break;
        }
    }
    let kept: Vec<(String, Vec<usize>)> = best
        .into_iter()
        .filter(|(_, m)| m.len() >= min_size)
        .collect();
    let sets: Vec<Vec<String>> = kept
        .iter()
        .map(|(_, m)| m.iter().map(|&i| nodes[i].qualified.clone()).collect())
        .collect();
    let cohesions = compute_cohesion(&sets, edges);
    kept.iter()
        .zip(sets)
        .zip(cohesions)
        .map(|(((dir, idx), members), cohesion)| {
            let nodes_in: Vec<&AnalysisNode> = idx.iter().map(|&i| nodes[i]).collect();
            Community {
                name: String::new(),
                level: 0,
                cohesion: round_half_even(cohesion, 4),
                size: members.len(),
                dominant_language: dominant_language(&nodes_in),
                description: format!("Directory-based community: {dir}"),
                members,
                detector: "file",
            }
        })
        .collect()
}

/// Splits communities above `max(25% of all members, 10)` with a second Leiden pass at
/// resolution 0.5 on their own subgraph (one pass, not recursive). A community that does not
/// split, has too few nodes or has no internal edge stays as it is. Sub-communities have level
/// `parent + 1`, the parent's dominant language and an unrounded cohesion. Each sub-community is
/// named from its members, falling back to `{parent}-{n}` (n counts sub-communities created).
pub fn split_oversized(communities: Vec<Community>, g: &AnalysisGraph) -> Vec<Community> {
    let total: usize = communities.iter().map(|c| c.size).sum();
    if total == 0 {
        return communities;
    }
    let threshold = ((total as f64 * 0.25) as usize).max(10);
    let vertices: Vec<&AnalysisNode> = canonical_vertices(g)
        .into_iter()
        .map(|i| &g.nodes[i])
        .collect();
    let mut out = Vec::new();
    let mut next_id = 1;
    for comm in communities {
        let members: HashSet<&str> = comm.members.iter().map(String::as_str).collect();
        if members.len() <= threshold {
            out.push(comm);
            continue;
        }
        let member_nodes: Vec<&AnalysisNode> = vertices
            .iter()
            .copied()
            .filter(|n| members.contains(n.qualified.as_str()))
            .collect();
        if member_nodes.len() < 10 {
            out.push(comm);
            continue;
        }
        let local: HashMap<&str, usize> = member_nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.qualified.as_str(), i))
            .collect();
        let mut pair_weight: BTreeMap<(usize, usize), f64> = BTreeMap::new();
        for e in &g.edges {
            if let (Some(&a), Some(&b)) =
                (local.get(e.source.as_str()), local.get(e.target.as_str()))
                && a != b
            {
                let w = pair_weight.entry((a.min(b), a.max(b))).or_insert(0.0);
                *w = w.max(edge_weight(&e.kind));
            }
        }
        if pair_weight.is_empty() {
            out.push(comm);
            continue;
        }
        let mut wg = WeightedGraph::new(member_nodes.len());
        for (&(a, b), &w) in &pair_weight {
            wg.add_edge(a, b, w);
        }
        let part = leiden(&wg, 0.5, 2);
        let clusters =
            reassign_test_nodes(group_by_cluster(&part), &member_nodes, &g.edges, &vertices);
        let subs: Vec<Vec<usize>> = clusters.into_iter().filter(|c| !c.is_empty()).collect();
        if subs.len() <= 1 {
            out.push(comm);
            continue;
        }
        let sets: Vec<Vec<String>> = subs
            .iter()
            .map(|c| {
                c.iter()
                    .map(|&v| member_nodes[v].qualified.clone())
                    .collect()
            })
            .collect();
        let cohesions = compute_cohesion(&sets, &g.edges);
        for (members, cohesion) in sets.into_iter().zip(cohesions) {
            let sub_nodes: Vec<&AnalysisNode> = members
                .iter()
                .filter_map(|qn| g.index_of(qn).map(|i| &g.nodes[i]))
                .collect();
            let mut name = naming::generate_name(&sub_nodes);
            if matches!(name.as_str(), "" | "empty" | "cluster") {
                name = format!("{}-{next_id}", comm.name);
            }
            next_id += 1;
            out.push(Community {
                name,
                level: comm.level + 1,
                cohesion,
                size: members.len(),
                dominant_language: comm.dominant_language.clone(),
                description: format!("Split from {}", comm.name),
                members,
                detector: "leiden",
            });
        }
    }
    out
}

const SQL_BATCH: usize = 450;

/// Replaces the stored communities and every node's `community_id`, and records which
/// detectors produced them in `metadata.community_detector` (communities.py `store_communities`).
pub fn store_communities(tx: &Transaction, comms: &[Community]) -> Result<usize> {
    tx.execute("DELETE FROM communities", [])?;
    // Restart AUTOINCREMENT so an unchanged graph keeps the same community ids on every build.
    tx.execute("DELETE FROM sqlite_sequence WHERE name = 'communities'", [])?;
    tx.execute("UPDATE nodes SET community_id = NULL", [])?;
    for comm in comms {
        tx.execute(
            "INSERT INTO communities (name, level, cohesion, size, dominant_language, description) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                comm.name,
                comm.level,
                comm.cohesion,
                comm.size as i64,
                comm.dominant_language,
                comm.description
            ],
        )?;
        let id = tx.last_insert_rowid();
        for batch in comm.members.chunks(SQL_BATCH) {
            let marks = vec!["?"; batch.len()].join(",");
            let sql =
                format!("UPDATE nodes SET community_id = ? WHERE qualified_name IN ({marks})");
            let args = std::iter::once(rusqlite::types::Value::Integer(id)).chain(
                batch
                    .iter()
                    .map(|qn| rusqlite::types::Value::Text(qn.clone())),
            );
            tx.execute(&sql, params_from_iter(args))?;
        }
    }
    if comms.is_empty() {
        tx.execute("DELETE FROM metadata WHERE key = 'community_detector'", [])?;
    } else {
        let detectors: std::collections::BTreeSet<&str> =
            comms.iter().map(|c| c.detector).collect();
        tx.execute(
            "INSERT INTO metadata (key, value) VALUES ('community_detector', ?1) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [detectors.into_iter().collect::<Vec<_>>().join("+")],
        )?;
    }
    Ok(comms.len())
}
