// ABOUTME: Import cycles: strongly connected groups of files that import each other.
// ABOUTME: Ports find_import_cycles from code-review-graph analysis.py (load-time vs deferred).

use super::super::{AnalysisGraph, sanitize_name};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

/// Files and hops of one import cycle spelled out; the rest is told as a count.
pub const MAX_CYCLE_FILES: usize = 10;
pub const MAX_CYCLE_HOPS: usize = 10;

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
/// count; deferred imports count for the group but not for `load_time`. Files that hold only
/// `extra.virtual` nodes are no files.
pub fn import_cycles(g: &AnalysisGraph, repo_root: Option<&str>) -> Vec<ImportCycle> {
    // CRG `get_all_files`: node files except those of `extra.virtual` nodes, plus edge files.
    let real = || g.nodes.iter().filter(|n| !n.is_virtual);
    let mut files: HashSet<&str> = real().map(|n| n.file_path.as_str()).collect();
    files.extend(g.edges.iter().map(|e| e.file_path.as_str()));
    files.extend(
        real()
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
