// ABOUTME: Execution-flow analysis: entry points, tracing and criticality.
// ABOUTME: Ports flows.py of code-review-graph on top of AnalysisGraph.

mod entry;

pub use entry::detect_entry_points;

use crate::analysis::{AnalysisGraph, SECURITY_KEYWORDS, round_half_even, sanitize_name};
use anyhow::Result;
use rusqlite::Transaction;
use std::collections::{HashMap, HashSet, VecDeque};

pub const FLOW_MAX_DEPTH: u32 = 15;

#[derive(Debug, Clone)]
pub struct Flow {
    pub name: String,
    pub entry_point: String,
    pub entry_point_id: i64,
    pub path: Vec<i64>,
    pub depth: u32,
    pub node_count: usize,
    pub file_count: usize,
    pub criticality: f64,
}

/// Traces one flow per entry point by BFS over CALLS edges (flows.py `_trace_single_flow`).
/// Flows with fewer than two nodes are dropped; the result is ordered by criticality, descending,
/// keeping entry order for ties.
pub fn trace_flows(g: &AnalysisGraph, max_depth: u32) -> Vec<Flow> {
    let mut calls_out: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut has_tested_by: HashSet<&str> = HashSet::new();
    for e in &g.edges {
        match e.kind.as_str() {
            "CALLS" => calls_out
                .entry(e.source.as_str())
                .or_default()
                .push(e.target.as_str()),
            "TESTED_BY" => {
                has_tested_by.insert(e.source.as_str());
            }
            _ => {}
        }
    }
    let mut flows = Vec::new();
    for ep in entry::detect_entry_points(g, false) {
        let entry = &g.nodes[ep];
        let mut visited: HashSet<&str> = HashSet::from([entry.qualified.as_str()]);
        let mut queue: VecDeque<(&str, u32)> = VecDeque::from([(entry.qualified.as_str(), 0)]);
        let mut path = vec![entry.id];
        let mut path_idx = vec![ep];
        let mut depth = 0;
        while let Some((qn, d)) = queue.pop_front() {
            depth = depth.max(d);
            if d >= max_depth {
                continue;
            }
            for &target in calls_out.get(qn).map(Vec::as_slice).unwrap_or(&[]) {
                if visited.contains(target) {
                    continue;
                }
                let Some(ti) = g.index_of(target) else {
                    continue;
                };
                visited.insert(target);
                queue.push_back((target, d + 1));
                path.push(g.nodes[ti].id);
                path_idx.push(ti);
            }
        }
        if path.len() < 2 {
            continue;
        }
        let files: HashSet<&str> = path_idx
            .iter()
            .map(|&i| g.nodes[i].file_path.as_str())
            .collect();
        let criticality = criticality(g, &path_idx, &calls_out, &has_tested_by, depth, files.len());
        flows.push(Flow {
            name: sanitize_name(&entry.name),
            entry_point: entry.qualified.clone(),
            entry_point_id: entry.id,
            node_count: path.len(),
            file_count: files.len(),
            path,
            depth,
            criticality,
        });
    }
    flows.sort_by(|a, b| b.criticality.total_cmp(&a.criticality));
    flows
}

fn criticality(
    g: &AnalysisGraph,
    path: &[usize],
    calls_out: &HashMap<&str, Vec<&str>>,
    has_tested_by: &HashSet<&str>,
    depth: u32,
    files: usize,
) -> f64 {
    let len = path.len() as f64;
    let file_spread = if files > 1 {
        ((files - 1) as f64 / 4.0).min(1.0)
    } else {
        0.0
    };
    let external: usize = path
        .iter()
        .flat_map(|&i| {
            calls_out
                .get(g.nodes[i].qualified.as_str())
                .into_iter()
                .flatten()
        })
        .filter(|t| g.index_of(t).is_none())
        .count();
    let external_score = (external as f64 / 5.0).min(1.0);
    let security = path
        .iter()
        .filter(|&&i| {
            let n = &g.nodes[i];
            let name = n.name.to_lowercase();
            let qn = n.qualified.to_lowercase();
            SECURITY_KEYWORDS
                .iter()
                .any(|k| name.contains(k) || qn.contains(k))
        })
        .count();
    let security_score = (security as f64 / len).min(1.0);
    let tested = path
        .iter()
        .filter(|&&i| has_tested_by.contains(g.nodes[i].qualified.as_str()))
        .count();
    let test_gap = 1.0 - tested as f64 / len;
    let depth_score = (depth as f64 / 10.0).min(1.0);
    let crit = 0.30 * file_spread
        + 0.20 * external_score
        + 0.25 * security_score
        + 0.15 * test_gap
        + 0.10 * depth_score;
    round_half_even(crit.clamp(0.0, 1.0), 4)
}

/// Replaces every stored flow (flows.py `store_flows`). Returns the number written.
pub fn store_flows(tx: &Transaction, flows: &[Flow]) -> Result<usize> {
    tx.execute("DELETE FROM flow_memberships", [])?;
    tx.execute("DELETE FROM flows", [])?;
    let mut insert = tx.prepare(
        "INSERT INTO flows (name, entry_point_id, depth, node_count, file_count, criticality, \
         path_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    let mut member = tx.prepare(
        "INSERT OR IGNORE INTO flow_memberships (flow_id, node_id, position) VALUES (?1, ?2, ?3)",
    )?;
    for f in flows {
        // Python's json.dumps puts a space after each comma.
        let path_json = format!(
            "[{}]",
            f.path
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
        insert.execute(rusqlite::params![
            f.name,
            f.entry_point_id,
            f.depth,
            f.node_count as i64,
            f.file_count as i64,
            f.criticality,
            path_json
        ])?;
        let flow_id = tx.last_insert_rowid();
        for (pos, node_id) in f.path.iter().enumerate() {
            member.execute(rusqlite::params![flow_id, node_id, pos as i64])?;
        }
    }
    tx.execute("DELETE FROM metadata WHERE key = 'csharp_flows_dirty'", [])?;
    Ok(flows.len())
}
