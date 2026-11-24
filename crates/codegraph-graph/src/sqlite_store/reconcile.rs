// ABOUTME: Catalog reconciliation, readiness markers, counts and the end-of-build finalise step.
// ABOUTME: finalize() leaves the database in the state code-review-graph expects after a build.

use super::SqliteGraphStore;
use super::write::{now_epoch, resolve_pending};
use crate::analysis::{PostprocessLevel, run_postprocess};
use crate::records::{FileMetadataRecord, ProjectMetadataRecord};
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphCounts {
    pub nodes: u64,
    pub edges: u64,
    pub files: u64,
}

impl SqliteGraphStore {
    /// Makes the graph equal to the complete prepared catalog: nodes, edges and file metadata
    /// outside it are removed, as are edges whose endpoints were removed. Rows written by
    /// another tool (no `cg_*` identity) are outside every catalog and are removed too, except
    /// TESTED_BY: it is derived, has no identity, and `finalize` rebuilds it. Trade-off: if a
    /// run fails between this call and `finalize` (e.g. inputs changed during indexing), the
    /// old TESTED_BY rows may name removed nodes until the next successful `finalize`.
    pub async fn reconcile_catalog(
        &self,
        project: &str,
        nodes: Vec<String>,
        edges: Vec<String>,
        files: Vec<String>,
    ) -> Result<()> {
        let project = project.to_string();
        self.with_conn(move |conn| {
            let tx = conn.transaction()?;
            tx.execute_batch(
                "CREATE TEMP TABLE IF NOT EXISTS cg_keep_nodes (uuid TEXT PRIMARY KEY); \
                 CREATE TEMP TABLE IF NOT EXISTS cg_keep_edges (uuid TEXT PRIMARY KEY); \
                 CREATE TEMP TABLE IF NOT EXISTS cg_keep_files (path TEXT PRIMARY KEY); \
                 DELETE FROM cg_keep_nodes; DELETE FROM cg_keep_edges; DELETE FROM cg_keep_files;",
            )?;
            fill(&tx, "cg_keep_nodes", &nodes)?;
            fill(&tx, "cg_keep_edges", &edges)?;
            fill(&tx, "cg_keep_files", &files)?;
            // 'text:' is write::TEXT_TARGET_PREFIX: a text target has no node to lose.
            tx.execute_batch(
                "DELETE FROM cg_nodes WHERE uuid NOT IN (SELECT uuid FROM cg_keep_nodes); \
                 DELETE FROM nodes WHERE qualified_name NOT IN \
                   (SELECT qualified_name FROM cg_nodes); \
                 DELETE FROM cg_edges WHERE uuid NOT IN (SELECT uuid FROM cg_keep_edges) \
                   OR from_uuid NOT IN (SELECT uuid FROM cg_nodes) \
                   OR (to_uuid NOT IN (SELECT uuid FROM cg_nodes) \
                       AND to_uuid NOT LIKE 'text:%'); \
                 DELETE FROM edges WHERE id NOT IN (SELECT edge_id FROM cg_edges) \
                   AND kind != 'TESTED_BY'; \
                 DELETE FROM cg_keep_nodes; DELETE FROM cg_keep_edges;",
            )?;
            tx.execute(
                "DELETE FROM cg_file_metadata WHERE project_id = ?1 \
                 AND file_path NOT IN (SELECT path FROM cg_keep_files)",
                [&project],
            )?;
            tx.execute_batch("DELETE FROM cg_keep_files;")?;
            tx.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn get_file_metadata_for_project(
        &self,
        project: &str,
    ) -> Result<Vec<FileMetadataRecord>> {
        let project = project.to_string();
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT record FROM cg_file_metadata WHERE project_id = ?1 ORDER BY file_path",
            )?;
            let rows = stmt.query_map([project], |r| r.get::<_, String>(0))?;
            rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
        })
        .await
    }

    pub async fn get_file_metadata(&self, file_path: &str) -> Result<Option<FileMetadataRecord>> {
        let file_path = file_path.to_string();
        self.with_conn(move |conn| {
            let record: Option<String> = conn
                .query_row(
                    "SELECT record FROM cg_file_metadata WHERE file_path = ?1 LIMIT 1",
                    [file_path],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(record.map(|r| serde_json::from_str(&r)).transpose()?)
        })
        .await
    }

    /// The "index ready" marker, written last by a successful build.
    pub async fn project_marker(&self, project: &str) -> Result<Option<ProjectMetadataRecord>> {
        let project = project.to_string();
        self.with_conn(move |conn| {
            let record: Option<String> = conn
                .query_row(
                    "SELECT record FROM cg_project_metadata WHERE project_id = ?1",
                    [project],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(record.map(|r| serde_json::from_str(&r)).transpose()?)
        })
        .await
    }

    pub async fn delete_project_marker(&self, project: &str) -> Result<()> {
        let project = project.to_string();
        self.with_conn(move |conn| {
            conn.execute(
                "DELETE FROM cg_project_metadata WHERE project_id = ?1",
                [project],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn counts(&self) -> Result<GraphCounts> {
        self.with_conn(|conn| {
            Ok(conn.query_row(
                "SELECT (SELECT count(*) FROM nodes), (SELECT count(*) FROM edges), \
                 (SELECT count(DISTINCT file_path) FROM nodes)",
                [],
                |r| {
                    Ok(GraphCounts {
                        nodes: r.get(0)?,
                        edges: r.get(1)?,
                        files: r.get(2)?,
                    })
                },
            )?)
        })
        .await
    }

    /// Resolves pending edges, rebuilds the FTS index and its state mirror, and records the
    /// build in `metadata` the way code-review-graph does after a complete build.
    pub async fn finalize(&self, build_type: &str) -> Result<()> {
        self.finalize_with(build_type, PostprocessLevel::default())
            .await
    }

    /// Like [`finalize`](Self::finalize), choosing how much derived analysis (flows, ...) to
    /// compute after the build transaction commits.
    pub async fn finalize_with(&self, build_type: &str, level: PostprocessLevel) -> Result<()> {
        let build_type = build_type.to_string();
        let root = self.root.to_string_lossy().into_owned();
        self.with_conn(move |conn| {
            let tx = conn.transaction()?;
            resolve_pending(&tx)?;
            derive_tested_by(&tx)?;
            tx.execute_batch(
                "INSERT INTO nodes_fts(nodes_fts) VALUES ('rebuild'); \
                 DELETE FROM nodes_fts_state; \
                 INSERT INTO nodes_fts_state (node_id, name, qualified_name, file_path, \
                   signature, docstring, name_tokens) \
                 SELECT id, name, qualified_name, file_path, signature, docstring, name_tokens \
                 FROM nodes;",
            )?;
            // Python's datetime.now().isoformat(timespec="seconds"): local time, no offset.
            let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
            for (key, value) in [
                ("last_updated", now.as_str()),
                ("last_build_type", build_type.as_str()),
                ("repo_root", root.as_str()),
                ("fts_state_synced", "2"),
                ("build_state", "complete"),
            ] {
                set_metadata(&tx, key, value)?;
            }
            tx.commit()?;
            // Like code-review-graph, a failed analysis step only warns: the build above is
            // committed and the graph stays usable. `postprocess_level` records the level of
            // the last post-process (`failed` when it did not finish), so a later no-op
            // `full` run knows whether flows/communities still need computing.
            match run_postprocess(conn, level) {
                Ok(_) => {
                    set_metadata(conn, POSTPROCESS_LEVEL_KEY, level.as_str())?;
                    conn.execute(
                        "DELETE FROM metadata WHERE key = ?1",
                        [POSTPROCESS_WARNING_KEY],
                    )?;
                }
                Err(error) => {
                    tracing::warn!("post-build analysis ({}) failed: {error:#}", level.as_str());
                    set_metadata(conn, POSTPROCESS_LEVEL_KEY, "failed")?;
                    set_metadata(conn, POSTPROCESS_WARNING_KEY, &format!("{error:#}"))?;
                }
            }
            Ok(())
        })
        .await
    }

    /// Level of the last post-process `finalize_with` ran (`full`, `minimal`, `none` or
    /// `failed`); `None` for a database no Rust build has post-processed.
    pub async fn last_postprocess_level(&self) -> Result<Option<String>> {
        self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT value FROM metadata WHERE key = ?1",
                    [POSTPROCESS_LEVEL_KEY],
                    |r| r.get(0),
                )
                .optional()?)
        })
        .await
    }
}

/// `metadata` key holding the level of the last post-process (see `finalize_with`).
pub const POSTPROCESS_LEVEL_KEY: &str = "postprocess_level";
/// `metadata` key holding the error of the last failed post-process, removed on success.
pub const POSTPROCESS_WARNING_KEY: &str = "postprocess_warning";

/// Rebuilds TESTED_BY as code-review-graph does: one edge per CALLS edge whose caller is a
/// Test node, reversed (callee -> test), keeping the call's file and line. Deleting first
/// keeps repeated finalisation (full or incremental) idempotent.
fn derive_tested_by(tx: &Transaction) -> Result<()> {
    tx.execute("DELETE FROM edges WHERE kind = 'TESTED_BY'", [])?;
    tx.execute(
        "INSERT INTO edges (kind, source_qualified, target_qualified, file_path, line, \
         updated_at) \
         SELECT 'TESTED_BY', e.target_qualified, e.source_qualified, e.file_path, e.line, ?1 \
         FROM edges e JOIN nodes n ON n.qualified_name = e.source_qualified \
         WHERE e.kind = 'CALLS' AND n.kind = 'Test'",
        [now_epoch()],
    )?;
    Ok(())
}

fn fill(tx: &Transaction, table: &str, values: &[String]) -> Result<()> {
    let mut stmt = tx.prepare(&format!("INSERT OR IGNORE INTO {table} VALUES (?1)"))?;
    for value in values {
        stmt.execute([value])?;
    }
    Ok(())
}

fn set_metadata(tx: &Connection, key: &str, value: &str) -> Result<()> {
    tx.execute(
        "INSERT INTO metadata (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}
