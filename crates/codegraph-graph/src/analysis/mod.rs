// ABOUTME: Graph analysis over the CRG SQLite tables: flows, communities, insights and risk.
// ABOUTME: AnalysisGraph is a plain snapshot of nodes/edges (ordered by id) for deterministic work.

pub mod communities;
pub mod flows;
pub mod insights;
pub mod risk;

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
    /// `extra.import_context` of an import edge (`type_only`, `deferred`, `parent_package`).
    pub import_context: Option<String>,
    /// `extra` names candidate targets (`ambiguous_targets`, `unresolved_targets`) or a claimed
    /// endpoint, so the edge is not a fact about one symbol (`_edge_extra_is_unresolved`).
    pub unresolved: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AnalysisGraph {
    pub nodes: Vec<AnalysisNode>,
    pub edges: Vec<AnalysisEdge>,
    by_qualified: HashMap<String, usize>,
    by_node_id: HashMap<i64, usize>,
    by_source: HashMap<String, Vec<usize>>,
    by_target: HashMap<String, Vec<usize>>,
}

impl AnalysisGraph {
    /// Loads every node and edge, ordered by `id` so downstream results are deterministic.
    pub fn load(conn: &Connection) -> Result<Self> {
        let mut nodes = Vec::new();
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
            nodes.push(row?);
        }
        let mut edges = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT kind, source_qualified, target_qualified, file_path, line, extra FROM edges \
             ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            let extra = r
                .get::<_, Option<String>>(5)?
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
            Ok(AnalysisEdge {
                kind: r.get(0)?,
                source: r.get(1)?,
                target: r.get(2)?,
                file_path: r.get(3)?,
                line: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                import_context: extra
                    .as_ref()
                    .and_then(|v| v.get("import_context")?.as_str().map(str::to_string)),
                unresolved: extra.as_ref().is_some_and(extra_is_unresolved),
            })
        })?;
        for row in rows {
            edges.push(row?);
        }
        Ok(Self::from_parts(nodes, edges))
    }

    /// Builds a graph from rows already in id order (used by `load` and by tests).
    pub fn from_parts(nodes: Vec<AnalysisNode>, edges: Vec<AnalysisEdge>) -> Self {
        let by_qualified = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.qualified.clone(), i))
            .collect();
        let by_node_id = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
        let mut by_source: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_target: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, e) in edges.iter().enumerate() {
            by_source.entry(e.source.clone()).or_default().push(i);
            by_target.entry(e.target.clone()).or_default().push(i);
        }
        AnalysisGraph {
            nodes,
            edges,
            by_qualified,
            by_node_id,
            by_source,
            by_target,
        }
    }

    pub fn index_of(&self, qualified: &str) -> Option<usize> {
        self.by_qualified.get(qualified).copied()
    }

    pub fn by_id(&self, id: i64) -> Option<usize> {
        self.by_node_id.get(&id).copied()
    }

    /// Edges leaving `source`, in edge id order.
    pub fn edges_from<'a>(&'a self, source: &str) -> impl Iterator<Item = &'a AnalysisEdge> + 'a {
        self.by_source
            .get(source)
            .into_iter()
            .flatten()
            .map(|&i| &self.edges[i])
    }

    /// Edges entering `target`, in edge id order.
    pub fn edges_to<'a>(&'a self, target: &str) -> impl Iterator<Item = &'a AnalysisEdge> + 'a {
        self.by_target
            .get(target)
            .into_iter()
            .flatten()
            .map(|&i| &self.edges[i])
    }
}

fn extra_is_unresolved(extra: &serde_json::Value) -> bool {
    extra.as_object().is_some_and(|o| {
        o.contains_key("ambiguous_targets")
            || o.contains_key("unresolved_targets")
            || o.contains_key("claimed_endpoint")
    })
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
    if !x.is_finite() || places < 0 {
        return x;
    }
    let p = places as usize;
    // 1100 fractional digits cover the exact expansion of any f64.
    let exact = format!("{:.1100}", x.abs());
    let (int_part, frac) = exact.split_once('.').unwrap_or((&exact, ""));
    let (kept, rest) = frac.split_at(p.min(frac.len()));
    let mut digits: Vec<u8> = int_part
        .bytes()
        .chain(kept.bytes())
        .map(|b| b - b'0')
        .collect();
    let first = rest.as_bytes().first().copied().unwrap_or(b'0');
    let tail_zero = rest.len() <= 1 || rest[1..].bytes().all(|b| b == b'0');
    let last_odd = digits.last().is_some_and(|d| d % 2 == 1);
    let up = first > b'5' || (first == b'5' && (!tail_zero || last_odd));
    if up {
        let mut i = digits.len();
        loop {
            if i == 0 {
                digits.insert(0, 1);
                break;
            }
            i -= 1;
            if digits[i] == 9 {
                digits[i] = 0;
            } else {
                digits[i] += 1;
                break;
            }
        }
    }
    let int_len = digits.len() - p;
    let text: String = digits
        .iter()
        .enumerate()
        .flat_map(|(i, d)| {
            let c = (b'0' + d) as char;
            if i == int_len && p > 0 {
                vec!['.', c]
            } else {
                vec![c]
            }
        })
        .collect();
    let value: f64 = text.parse().unwrap_or(x.abs());
    if x.is_sign_negative() { -value } else { value }
}

/// Removes control characters and caps the name at 256 chars (`_sanitize_name` in the CRG).
pub fn sanitize_name(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(*c as u32, 0..=8 | 0x0b..=0x1f))
        .take(256)
        .collect()
}

/// How much derived analysis `finalize` computes after a build.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PostprocessLevel {
    #[default]
    Full,
    /// Same as `None` here. The CRG's `minimal` runs the resolvers, computes
    /// `nodes.signature` and refreshes the FTS index; the Rust `finalize` always resolves
    /// pending edges and rebuilds FTS (and TESTED_BY) whatever the level, and never
    /// computes `nodes.signature`, so nothing is left for `minimal` to add.
    Minimal,
    /// No derived analysis. Unlike the CRG's `none`, `finalize` still resolves edges and
    /// rebuilds FTS and TESTED_BY (harmless: they are cheap and keep the graph coherent).
    None,
}

impl std::str::FromStr for PostprocessLevel {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "full" => Ok(Self::Full),
            "minimal" => Ok(Self::Minimal),
            "none" => Ok(Self::None),
            other => anyhow::bail!(
                "invalid postprocess level '{other}' (expected full, minimal or none)"
            ),
        }
    }
}

impl PostprocessLevel {
    /// Lowercase name, as accepted by `CODEGRAPH_POSTPROCESS`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Minimal => "minimal",
            Self::None => "none",
        }
    }

    /// Reads `CODEGRAPH_POSTPROCESS` (default `full`); an invalid value is an error.
    pub fn from_env() -> Result<Self> {
        match std::env::var("CODEGRAPH_POSTPROCESS") {
            Ok(v) if !v.trim().is_empty() => v.parse(),
            _ => Ok(Self::default()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PostprocessStats {
    pub flows: usize,
    /// How the flow rows were written: inserted, updated, deleted or left unchanged.
    pub flow_writes: flows::FlowWriteStats,
    pub communities: usize,
}

/// Computes and stores the derived analysis for the current graph, in its own write transaction.
pub fn run_postprocess(conn: &mut Connection, level: PostprocessLevel) -> Result<PostprocessStats> {
    let mut stats = PostprocessStats::default();
    if level != PostprocessLevel::Full {
        return Ok(stats);
    }
    let started = std::time::Instant::now();
    let graph = AnalysisGraph::load(conn)?;
    let flows = flows::trace_flows(&graph, flows::FLOW_MAX_DEPTH);
    let flows_s = started.elapsed().as_secs_f64();
    let communities = communities::detect_communities(&graph, 2);
    let communities_s = started.elapsed().as_secs_f64() - flows_s;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let store_started = std::time::Instant::now();
    stats.flow_writes = flows::reconcile_flows(&tx, &flows)?;
    stats.flows = flows.len();
    let flows_store_s = store_started.elapsed().as_secs_f64();
    stats.communities = communities::store_communities(&tx, &communities)?;
    tx.commit()?;
    tracing::info!(
        flows_s,
        flows_store_s,
        communities_s,
        flows = stats.flows,
        flows_inserted = stats.flow_writes.inserted,
        flows_updated = stats.flow_writes.updated,
        flows_deleted = stats.flow_writes.deleted,
        flows_unchanged = stats.flow_writes.unchanged,
        communities = stats.communities,
        "analysis stored"
    );
    Ok(stats)
}
