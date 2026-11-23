// ABOUTME: Creates or validates the code-review-graph v15 schema plus Rust-only cg_* tables.
// ABOUTME: Never migrates: other versions are left for code-review-graph to upgrade.

use super::SCHEMA_VERSION;
use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;

const CRG_V15: &str = include_str!("schema_v15.sql");
const CG_TABLES: &str = include_str!("schema_cg.sql");

pub(super) fn configure(conn: &Connection) -> Result<()> {
    // In-memory databases report "memory"; only file databases can switch to WAL.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    Ok(())
}

/// Explains a schema mismatch: what wrote the database and how to get a readable one.
fn version_error(found: Option<&str>, path: &Path) -> String {
    let path = path.display();
    let reindex = format!("delete {path} and run `codegraph index`");
    match found.and_then(|v| v.parse::<u32>().ok()) {
        Some(v) if v < SCHEMA_VERSION => format!(
            "graph.db schema version {v} is not supported (expected {SCHEMA_VERSION}): {path} \
             was written by an older code-review-graph (2.3.x writes 9). Upgrade \
             code-review-graph to 2.4.0+ and run `code-review-graph update`, or {reindex}. \
             If code-review-graph hooks keep rewriting it, disable them for this repository \
             (see docs/architecture/storage-sqlite.md)"
        ),
        Some(v) => format!(
            "graph.db schema version {v} is not supported (expected {SCHEMA_VERSION}): {path} \
             was written by a newer code-review-graph. Use code-review-graph for this \
             repository, or {reindex}"
        ),
        None => match found {
            Some(v) => format!("graph.db schema version {v:?} is not a number: {reindex}"),
            None => format!("graph.db has no schema_version: {reindex}"),
        },
    }
}

pub(super) fn ensure(conn: &mut Connection, path: &Path) -> Result<()> {
    let tx = conn.transaction()?;
    let has_metadata: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'metadata')",
        [],
        |row| row.get(0),
    )?;
    if has_metadata {
        let version: Option<String> = tx
            .query_row(
                "SELECT value FROM metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if version.as_deref() != Some(SCHEMA_VERSION.to_string().as_str()) {
            bail!(version_error(version.as_deref(), path));
        }
    } else {
        tx.execute_batch(CRG_V15)?;
        tx.execute(
            "INSERT INTO metadata (key, value) VALUES ('schema_version', ?1), \
             ('fts_state_synced', '2')",
            [SCHEMA_VERSION.to_string()],
        )?;
    }
    tx.execute_batch(CG_TABLES)?;
    tx.commit()?;
    Ok(())
}
