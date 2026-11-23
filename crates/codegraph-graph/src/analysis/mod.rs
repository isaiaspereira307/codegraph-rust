// ABOUTME: Graph analysis over the CRG SQLite tables: flows, communities, insights and risk.
// ABOUTME: AnalysisGraph is a plain snapshot of nodes/edges (ordered by id) for deterministic work.

use anyhow::Result;
use rusqlite::Connection;
use std::collections::HashMap;

/// Security-sensitive name fragments (constants.py SECURITY_KEYWORDS in code-review-graph).
pub const SECURITY_KEYWORDS: &[&str] = &[
    "auth",
    "login",
    "password",
    "token",
    "session",
    "crypt",
    "secret",
    "credential",
    "permission",
    "sql",
    "query",
    "execute",
    "connect",
    "socket",
    "request",
    "http",
    "sanitize",
    "validate",
    "encrypt",
    "decrypt",
    "hash",
    "sign",
    "verify",
    "admin",
    "privilege",
];

#[derive(Debug, Clone)]
pub struct AnalysisNode {
    pub id: i64,
    pub qualified: String,
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub language: Option<String>,
    pub line_start: Option<i64>,
    pub line_end: Option<i64>,
    pub is_test: bool,
    pub decorators: Vec<String>,
    pub community_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct AnalysisEdge {
    pub kind: String,
    pub source: String,
    pub target: String,
    pub file_path: String,
    pub line: i64,
}

#[derive(Debug, Clone, Default)]
pub struct AnalysisGraph {
    pub nodes: Vec<AnalysisNode>,
    pub edges: Vec<AnalysisEdge>,
    by_qualified: HashMap<String, usize>,
    by_node_id: HashMap<i64, usize>,
}

impl AnalysisGraph {
    /// Loads every node and edge, ordered by `id` so downstream results are deterministic.
    pub fn load(conn: &Connection) -> Result<Self> {
        let mut graph = AnalysisGraph::default();
        let mut stmt = conn.prepare(
            "SELECT id, qualified_name, name, kind, file_path, language, line_start, line_end, \
             is_test, extra, community_id FROM nodes ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            let extra: Option<String> = r.get(9)?;
            Ok(AnalysisNode {
                id: r.get(0)?,
                qualified: r.get(1)?,
                name: r.get(2)?,
                kind: r.get(3)?,
                file_path: r.get(4)?,
                language: r.get(5)?,
                line_start: r.get(6)?,
                line_end: r.get(7)?,
                is_test: r.get::<_, Option<i64>>(8)?.unwrap_or(0) != 0,
                decorators: decorators_from_extra(extra.as_deref()),
                community_id: r.get(10)?,
            })
        })?;
        for row in rows {
            let node = row?;
            let idx = graph.nodes.len();
            graph.by_qualified.insert(node.qualified.clone(), idx);
            graph.by_node_id.insert(node.id, idx);
            graph.nodes.push(node);
        }
        let mut stmt = conn.prepare(
            "SELECT kind, source_qualified, target_qualified, file_path, line FROM edges \
             ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(AnalysisEdge {
                kind: r.get(0)?,
                source: r.get(1)?,
                target: r.get(2)?,
                file_path: r.get(3)?,
                line: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
            })
        })?;
        for row in rows {
            graph.edges.push(row?);
        }
        Ok(graph)
    }

    pub fn index_of(&self, qualified: &str) -> Option<usize> {
        self.by_qualified.get(qualified).copied()
    }

    pub fn by_id(&self, id: i64) -> Option<usize> {
        self.by_node_id.get(&id).copied()
    }
}

fn decorators_from_extra(extra: Option<&str>) -> Vec<String> {
    let Some(value) = extra.and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()) else {
        return Vec::new();
    };
    match value.get("decorators") {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// Rounds like Python's `round(x, places)`: decided on the float's exact decimal expansion,
/// ties to the even digit.
pub fn round_half_even(x: f64, places: i32) -> f64 {
    let p = places.max(0) as usize;
    let exact = format!("{:.*}", p + 20, x);
    let tie = exact
        .split_once('.')
        .map(|(_, f)| f[p..].starts_with('5') && f[p + 1..].chars().all(|c| c == '0'))
        .unwrap_or(false);
    let scale = 10f64.powi(places);
    let scaled = x * scale;
    let floor = scaled.floor();
    let rounded = if tie {
        if floor % 2.0 == 0.0 {
            floor
        } else {
            floor + 1.0
        }
    } else {
        scaled.round()
    };
    rounded / scale
}

/// Removes control characters and caps the name at 256 chars (`_sanitize_name` in the CRG).
pub fn sanitize_name(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(*c as u32, 0..=8 | 0x0b..=0x1f))
        .take(256)
        .collect()
}
