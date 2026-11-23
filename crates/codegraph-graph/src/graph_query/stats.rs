// ABOUTME: Graph-wide counts (nodes and edges by kind, files, languages, last build time).
// ABOUTME: Reads SQLite with GROUP BY, so it never builds the petgraph snapshot.

use super::GraphQuery;
use super::types::GraphStats;
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use std::collections::BTreeMap;

const NODES_BY_KIND: &str = "SELECT kind, COUNT(*) FROM nodes GROUP BY kind";
const EDGES_BY_KIND: &str = "SELECT kind, COUNT(*) FROM edges GROUP BY kind";
const FILES: &str = "SELECT COUNT(DISTINCT file_path) FROM nodes";
const LANGUAGES: &str = "SELECT DISTINCT language FROM nodes \
     WHERE language IS NOT NULL AND language <> '' ORDER BY language";
const LAST_UPDATED: &str = "SELECT value FROM metadata WHERE key = 'last_updated'";

fn count_by_kind(conn: &Connection, sql: &str) -> rusqlite::Result<BTreeMap<String, usize>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
    })?;
    rows.collect()
}

impl GraphQuery {
    /// Counts of nodes and edges per kind, distinct files and languages, and the last build time.
    /// Reflects the database as committed; an empty database yields zeros and no languages.
    pub async fn graph_stats(&self) -> Result<GraphStats> {
        self.store()
            .with_conn(|conn| {
                let nodes_by_kind = count_by_kind(conn, NODES_BY_KIND)?;
                let edges_by_kind = count_by_kind(conn, EDGES_BY_KIND)?;
                let files: i64 = conn.query_row(FILES, [], |row| row.get(0))?;
                let mut stmt = conn.prepare(LANGUAGES)?;
                let languages = stmt
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let last_updated = conn
                    .query_row(LAST_UPDATED, [], |row| row.get::<_, String>(0))
                    .optional()?;
                Ok(GraphStats {
                    total_nodes: nodes_by_kind.values().sum(),
                    total_edges: edges_by_kind.values().sum(),
                    nodes_by_kind,
                    edges_by_kind,
                    files: files as usize,
                    languages,
                    last_updated,
                })
            })
            .await
    }
}
