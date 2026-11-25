// ABOUTME: query_graph patterns: callers, callees, imports, importers, tests and inheritors of one node.
// ABOUTME: Walks the petgraph snapshot and counts text targets that are not graph nodes.

use super::GraphQuery;
use super::types::NodeReference;
use anyhow::Result;
use petgraph::Direction::{self, Incoming, Outgoing};
use rusqlite::params;

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;

const UNRESOLVED_OUT: &str = "SELECT COUNT(*) FROM edges WHERE kind = ?1 AND source_qualified = ?2 \
     AND target_qualified NOT IN (SELECT qualified_name FROM nodes)";
const UNRESOLVED_IN: &str = "SELECT COUNT(*) FROM edges WHERE kind = ?1 AND target_qualified = ?2 \
     AND source_qualified NOT IN (SELECT qualified_name FROM nodes)";

/// Relationship asked by `query_graph`; each maps to one edge kind and direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryPattern {
    CallersOf,
    CalleesOf,
    ImportsOf,
    ImportersOf,
    TestsFor,
    /// Subclasses and implementers: incoming `INHERITS` and `IMPLEMENTS` (handled apart from
    /// `edge()`, which names a single kind).
    InheritorsOf,
}

impl QueryPattern {
    /// Parses the snake_case name used by the tool schema.
    pub fn parse(input: &str) -> Option<Self> {
        Some(match input {
            "callers_of" => Self::CallersOf,
            "callees_of" => Self::CalleesOf,
            "imports_of" => Self::ImportsOf,
            "importers_of" => Self::ImportersOf,
            "tests_for" => Self::TestsFor,
            "inheritors_of" => Self::InheritorsOf,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::CallersOf => "callers_of",
            Self::CalleesOf => "callees_of",
            Self::ImportsOf => "imports_of",
            Self::ImportersOf => "importers_of",
            Self::TestsFor => "tests_for",
            Self::InheritorsOf => "inheritors_of",
        }
    }

    fn edge(self) -> (&'static str, Direction) {
        match self {
            Self::CallersOf => ("CALLS", Incoming),
            Self::CalleesOf => ("CALLS", Outgoing),
            Self::ImportsOf => ("IMPORTS_FROM", Outgoing),
            Self::ImportersOf => ("IMPORTS_FROM", Incoming),
            Self::TestsFor => ("TESTED_BY", Outgoing),
            Self::InheritorsOf => ("INHERITS", Incoming),
        }
    }
}

/// Answer to one `query_graph` call.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryGraphResult {
    pub pattern: String,
    pub target: NodeReference,
    /// Distinct matches, sorted by name; at most `limit`.
    pub results: Vec<NodeReference>,
    /// Matches before the limit was applied.
    pub total: usize,
    /// Edges of this pattern whose other end is text that names no node (ignored above).
    pub unresolved: usize,
}

impl GraphQuery {
    /// Runs `pattern` from the node `node_id` (uuid or qualified name). `None` when the node is
    /// unknown. `limit` 0 means 20, capped at 100.
    pub async fn query_graph(
        &self,
        pattern: QueryPattern,
        node_id: &str,
        limit: usize,
    ) -> Result<Option<QueryGraphResult>> {
        let snap = self.snapshot().await?;
        let Some(ix) = snap.resolve(node_id) else {
            return Ok(None);
        };
        let (kind, dir) = pattern.edge();
        let inheritors = pattern == QueryPattern::InheritorsOf;
        let neighbours =
            snap.neighbours(ix, dir, |k| k == kind || (inheritors && k == "IMPLEMENTS"));
        let limit = match limit {
            0 => DEFAULT_LIMIT,
            n => n.min(MAX_LIMIT),
        };
        let qualified = snap.node(ix).qualified.clone();
        let unresolved: i64 = if inheritors {
            // Bare-name heritage edges that had this class among several candidates.
            snap.ambiguous_heritage.get(&ix).copied().unwrap_or(0) as i64
        } else {
            let sql = if dir == Outgoing {
                UNRESOLVED_OUT
            } else {
                UNRESOLVED_IN
            };
            let kind = kind.to_string();
            self.store()
                .with_conn(move |conn| {
                    Ok(conn.query_row(sql, params![kind, qualified], |row| row.get(0))?)
                })
                .await?
        };
        let total = neighbours.len();
        let results = neighbours
            .iter()
            .take(limit)
            .map(|&n| snap.reference(n))
            .collect();
        Ok(Some(QueryGraphResult {
            pattern: pattern.name().to_string(),
            target: snap.reference(ix),
            results,
            total,
            unresolved: unresolved as usize,
        }))
    }
}
