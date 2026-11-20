// ABOUTME: Writes CodeNode/CodeEdge batches and indexer metadata into the CRG tables.
// ABOUTME: Each call is one transaction on the blocking pool.

use super::SqliteGraphStore;
use super::mapping::{self, absolute_path};
use crate::edge::CodeEdge;
use crate::records::{FileMetadataRecord, ProjectMetadataRecord};
use anyhow::Result;
use codegraph_core::CodeNode;
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Prefix of an edge endpoint whose node has not been written yet.
pub(crate) const PENDING_PREFIX: &str = "cg:";
/// Prefix of `cg_edges.to_uuid` for an edge whose target is stored as text, not a node.
pub(crate) const TEXT_TARGET_PREFIX: &str = "text:";
const DOCSTRING_CHARS: usize = 400;

pub(crate) fn now_epoch() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64 / 1000.0
}

impl SqliteGraphStore {
    pub async fn upsert_nodes_batch(&self, nodes: &[CodeNode]) -> Result<()> {
        let nodes = nodes.to_vec();
        let root = self.root.clone();
        self.with_conn(move |conn| {
            let tx = conn.transaction()?;
            let now = now_epoch();
            for node in &nodes {
                write_node(&tx, &root, node, now)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn upsert_edges_batch(&self, edges: &[CodeEdge]) -> Result<()> {
        let edges = edges.to_vec();
        self.with_conn(move |conn| {
            let tx = conn.transaction()?;
            let now = now_epoch();
            for edge in &edges {
                write_edge(&tx, edge, now)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn upsert_file_metadata_batch(&self, rows: &[FileMetadataRecord]) -> Result<()> {
        let rows = rows.to_vec();
        self.with_conn(move |conn| {
            let tx = conn.transaction()?;
            for row in &rows {
                tx.execute(
                    "INSERT INTO cg_file_metadata (project_id, file_path, record) \
                     VALUES (?1, ?2, ?3) \
                     ON CONFLICT(project_id, file_path) DO UPDATE SET record = excluded.record",
                    params![row.project_id, row.file_path, serde_json::to_string(row)?],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn upsert_project_metadata(&self, row: ProjectMetadataRecord) -> Result<()> {
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO cg_project_metadata (project_id, record) VALUES (?1, ?2) \
                 ON CONFLICT(project_id) DO UPDATE SET record = excluded.record",
                params![row.project_id, serde_json::to_string(&row)?],
            )?;
            Ok(())
        })
        .await
    }

    /// Removes the files' nodes, every edge touching them, and their file metadata.
    pub async fn delete_data_for_files(&self, project_id: &str, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let project_id = project_id.to_string();
        let paths = paths.to_vec();
        let root = self.root.clone();
        self.with_conn(move |conn| {
            let tx = conn.transaction()?;
            delete_files(&tx, &root, &paths)?;
            for path in &paths {
                tx.execute(
                    "DELETE FROM cg_file_metadata WHERE project_id = ?1 AND file_path = ?2",
                    params![project_id, path],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }

    /// Replaces `cg:<uuid>` endpoints whose node now exists; returns the edges fixed.
    pub async fn resolve_pending_edges(&self) -> Result<usize> {
        self.with_conn(|conn| {
            let tx = conn.transaction()?;
            let fixed = resolve_pending(&tx)?;
            tx.commit()?;
            Ok(fixed)
        })
        .await
    }
}

fn write_node(tx: &Transaction, root: &Path, node: &CodeNode, now: f64) -> Result<()> {
    let uuid = node.id.to_string();
    let base = mapping::qualified_name(root, node);
    let existing: Option<String> = tx
        .query_row(
            "SELECT qualified_name FROM cg_nodes WHERE uuid = ?1",
            [&uuid],
            |r| r.get(0),
        )
        .optional()?;
    let qualified = match existing {
        Some(qn) if qn == base || qn.starts_with(&format!("{base}@L")) => qn,
        previous => {
            if let Some(old) = previous {
                tx.execute(
                    "DELETE FROM nodes WHERE qualified_name = ?1 AND NOT EXISTS \
                     (SELECT 1 FROM cg_nodes WHERE qualified_name = ?1 AND uuid != ?2)",
                    params![old, uuid],
                )?;
            }
            free_qualified_name(tx, &base, &uuid, node.location.line)?
        }
    };

    let attrs = &node.metadata.attributes;
    let kind = node
        .node_type
        .as_ref()
        .map(mapping::crg_node_kind)
        .unwrap_or_else(|| "Function".to_string());
    let file_path = absolute_path(root, &node.location.file_path);
    // CRG names a File node by its absolute path.
    let name = if kind == "File" {
        file_path.clone()
    } else {
        node.name.to_string()
    };
    let parent = attrs.get("parent_name").or_else(|| attrs.get("parent"));
    let symbol = qualified
        .split_once("::")
        .map(|(_, s)| s.to_string())
        .unwrap_or_else(|| qualified.clone());
    let docstring = attrs
        .get("doc")
        .or_else(|| attrs.get("docstring"))
        .map(|d| d.trim().chars().take(DOCSTRING_CHARS).collect::<String>());
    let tokens = mapping::name_tokens(&kind, &node.name, parent.map(String::as_str), &file_path);

    tx.execute(
        "INSERT INTO nodes (kind, name, qualified_name, file_path, line_start, line_end, \
         language, parent_name, params, return_type, modifiers, is_test, file_hash, extra, \
         symbol, docstring, name_tokens, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, '{}', ?14, ?15, ?16, ?17) \
         ON CONFLICT(qualified_name) DO UPDATE SET kind = excluded.kind, name = excluded.name, \
         file_path = excluded.file_path, line_start = excluded.line_start, \
         line_end = excluded.line_end, language = excluded.language, \
         parent_name = excluded.parent_name, params = excluded.params, \
         return_type = excluded.return_type, modifiers = excluded.modifiers, \
         is_test = excluded.is_test, file_hash = excluded.file_hash, symbol = excluded.symbol, \
         docstring = excluded.docstring, name_tokens = excluded.name_tokens, \
         updated_at = excluded.updated_at",
        params![
            kind,
            name,
            qualified,
            file_path,
            node.location.line,
            node.location.end_line.unwrap_or(node.location.line),
            node.language.as_ref().map(mapping::language_name),
            parent,
            attrs.get("params"),
            attrs.get("return_type"),
            attrs.get("modifiers"),
            attrs.get("is_test").is_some_and(|v| v == "true") as i64,
            attrs.get("file_hash"),
            symbol,
            docstring,
            tokens,
            now,
        ],
    )?;
    tx.execute(
        "INSERT INTO cg_nodes (uuid, qualified_name, file_path, rust_kind, complexity, content, \
         attributes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(uuid) DO UPDATE SET qualified_name = excluded.qualified_name, \
         file_path = excluded.file_path, rust_kind = excluded.rust_kind, \
         complexity = excluded.complexity, content = excluded.content, \
         attributes = excluded.attributes",
        params![
            uuid,
            qualified,
            file_path,
            node.node_type.as_ref().map(mapping::rust_node_kind),
            node.complexity,
            node.content.as_ref().map(|c| c.to_string()),
            serde_json::to_string(&attrs.iter().collect::<BTreeMap<_, _>>())?,
        ],
    )?;
    Ok(())
}

/// `base`, or `base@L<line>` (then `#2`, `#3`, …) when another node already owns `base`.
fn free_qualified_name(tx: &Transaction, base: &str, uuid: &str, line: u32) -> Result<String> {
    let taken = |candidate: &str| -> Result<bool> {
        Ok(tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM cg_nodes WHERE qualified_name = ?1 AND uuid != ?2)",
            params![candidate, uuid],
            |r| r.get(0),
        )?)
    };
    if !taken(base)? {
        return Ok(base.to_string());
    }
    let with_line = format!("{base}@L{line}");
    let mut candidate = with_line.clone();
    let mut n = 2;
    while taken(&candidate)? {
        candidate = format!("{with_line}#{n}");
        n += 1;
    }
    Ok(candidate)
}

fn endpoint(tx: &Transaction, uuid: &str) -> Result<Option<(String, String)>> {
    Ok(tx
        .query_row(
            "SELECT qualified_name, file_path FROM cg_nodes WHERE uuid = ?1",
            [uuid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

fn write_edge(tx: &Transaction, edge: &CodeEdge, now: f64) -> Result<()> {
    let uuid = edge.id.to_string();
    let from = edge.from.to_string();
    let source = endpoint(tx, &from)?;
    let kind = mapping::crg_edge_kind(&edge.edge_type);
    let file_path = source.as_ref().map(|s| s.1.clone()).unwrap_or_default();
    let source_q = source
        .map(|s| s.0)
        .unwrap_or_else(|| format!("{PENDING_PREFIX}{from}"));
    let (to, target_q, resolution) = match edge.unresolved_target() {
        // CRG keeps the call/import text and marks call-like targets 'unresolved'.
        Some(text) => (
            format!("{TEXT_TARGET_PREFIX}{}", edge.to),
            text.to_string(),
            matches!(kind.as_str(), "CALLS" | "REFERENCES").then_some("unresolved"),
        ),
        None => {
            let to = edge.to.to_string();
            let target = endpoint(tx, &to)?;
            let resolution = target_resolution(&kind, target.is_some());
            let target_q = target
                .map(|t| t.0)
                .unwrap_or_else(|| format!("{PENDING_PREFIX}{to}"));
            (to, target_q, resolution)
        }
    };
    let line: i64 = edge
        .metadata
        .get("line")
        .and_then(|l| l.parse().ok())
        .unwrap_or(0);
    let confidence: f64 = edge
        .metadata
        .get("confidence")
        .and_then(|c| c.parse().ok())
        .unwrap_or(1.0);
    let extra = serde_json::to_string(&edge.metadata.iter().collect::<BTreeMap<_, _>>())?;

    let existing: Option<i64> = tx
        .query_row(
            "SELECT edge_id FROM cg_edges WHERE uuid = ?1",
            [&uuid],
            |r| r.get(0),
        )
        .optional()?;
    let updated = match existing {
        Some(id) => tx.execute(
            "UPDATE edges SET kind = ?1, source_qualified = ?2, target_qualified = ?3, \
             file_path = ?4, line = ?5, extra = ?6, confidence = ?7, target_resolution = ?8, \
             updated_at = ?9 WHERE id = ?10",
            params![
                kind, source_q, target_q, file_path, line, extra, confidence, resolution, now, id
            ],
        )?,
        None => 0,
    };
    let edge_id = match existing {
        Some(id) if updated == 1 => id,
        _ => {
            tx.execute(
                "INSERT INTO edges (kind, source_qualified, target_qualified, file_path, line, \
                 extra, confidence, confidence_tier, target_resolution, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'EXTRACTED', ?8, ?9)",
                params![
                    kind, source_q, target_q, file_path, line, extra, confidence, resolution, now
                ],
            )?;
            tx.last_insert_rowid()
        }
    };
    tx.execute(
        "INSERT INTO cg_edges (uuid, edge_id, from_uuid, to_uuid, file_path) \
         VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(uuid) DO UPDATE SET edge_id = excluded.edge_id, \
         from_uuid = excluded.from_uuid, to_uuid = excluded.to_uuid, \
         file_path = excluded.file_path",
        params![uuid, edge_id, from, to, file_path],
    )?;
    Ok(())
}

/// CRG marks call/reference targets bound to an indexed node as `direct`; other kinds get NULL.
fn target_resolution(kind: &str, resolved: bool) -> Option<&'static str> {
    (resolved && matches!(kind, "CALLS" | "REFERENCES")).then_some("direct")
}

pub(crate) fn resolve_pending(tx: &Transaction) -> Result<usize> {
    let fixable: i64 = tx.query_row(
        "SELECT count(*) FROM edges WHERE \
         (source_qualified LIKE 'cg:%' AND EXISTS (SELECT 1 FROM cg_nodes n \
          WHERE n.uuid = substr(edges.source_qualified, 4))) OR \
         (target_qualified LIKE 'cg:%' AND EXISTS (SELECT 1 FROM cg_nodes n \
          WHERE n.uuid = substr(edges.target_qualified, 4)))",
        [],
        |r| r.get(0),
    )?;
    if fixable == 0 {
        return Ok(0);
    }
    tx.execute(
        "UPDATE edges SET source_qualified = n.qualified_name, file_path = n.file_path \
         FROM cg_nodes n WHERE edges.source_qualified LIKE 'cg:%' \
         AND n.uuid = substr(edges.source_qualified, 4)",
        [],
    )?;
    tx.execute(
        "UPDATE edges SET target_qualified = n.qualified_name, \
         target_resolution = CASE WHEN edges.kind IN ('CALLS', 'REFERENCES') \
         THEN 'direct' ELSE NULL END \
         FROM cg_nodes n WHERE edges.target_qualified LIKE 'cg:%' \
         AND n.uuid = substr(edges.target_qualified, 4)",
        [],
    )?;
    tx.execute(
        "UPDATE cg_edges SET file_path = n.file_path FROM cg_nodes n \
         WHERE cg_edges.file_path = '' AND n.uuid = cg_edges.from_uuid",
        [],
    )?;
    Ok(fixable as usize)
}

fn delete_files(tx: &Transaction, root: &PathBuf, paths: &[String]) -> Result<()> {
    tx.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS cg_del_files (path TEXT PRIMARY KEY); \
         CREATE TEMP TABLE IF NOT EXISTS cg_del_nodes (uuid TEXT PRIMARY KEY); \
         CREATE TEMP TABLE IF NOT EXISTS cg_del_qn (qn TEXT PRIMARY KEY); \
         DELETE FROM cg_del_files; DELETE FROM cg_del_nodes; DELETE FROM cg_del_qn;",
    )?;
    for path in paths {
        tx.execute(
            "INSERT OR IGNORE INTO cg_del_files (path) VALUES (?1)",
            [absolute_path(root, path)],
        )?;
    }
    tx.execute_batch(
        "INSERT OR IGNORE INTO cg_del_nodes SELECT uuid FROM cg_nodes \
           WHERE file_path IN (SELECT path FROM cg_del_files); \
         INSERT OR IGNORE INTO cg_del_qn SELECT qualified_name FROM nodes \
           WHERE file_path IN (SELECT path FROM cg_del_files); \
         DELETE FROM edges WHERE id IN (SELECT edge_id FROM cg_edges \
             WHERE from_uuid IN (SELECT uuid FROM cg_del_nodes) \
                OR to_uuid IN (SELECT uuid FROM cg_del_nodes)) \
           OR source_qualified IN (SELECT qn FROM cg_del_qn) \
           OR target_qualified IN (SELECT qn FROM cg_del_qn) \
           OR file_path IN (SELECT path FROM cg_del_files); \
         DELETE FROM cg_edges WHERE from_uuid IN (SELECT uuid FROM cg_del_nodes) \
           OR to_uuid IN (SELECT uuid FROM cg_del_nodes) \
           OR file_path IN (SELECT path FROM cg_del_files) \
           OR edge_id NOT IN (SELECT id FROM edges); \
         DELETE FROM nodes WHERE file_path IN (SELECT path FROM cg_del_files); \
         DELETE FROM cg_nodes WHERE file_path IN (SELECT path FROM cg_del_files); \
         DELETE FROM cg_del_files; DELETE FROM cg_del_nodes; DELETE FROM cg_del_qn;",
    )?;
    Ok(())
}
