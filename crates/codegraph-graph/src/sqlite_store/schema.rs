// ABOUTME: Creates or validates the code-review-graph v15 schema plus Rust-only cg_* tables.
// ABOUTME: Never migrates: other versions are left for code-review-graph to upgrade.

use super::SCHEMA_VERSION;
use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension};

const CRG_V15: &str = include_str!("schema_v15.sql");
const CG_TABLES: &str = include_str!("schema_cg.sql");

pub(super) fn configure(conn: &Connection) -> Result<()> {
    // In-memory databases report "memory"; only file databases can switch to WAL.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    Ok(())
}

pub(super) fn ensure(conn: &mut Connection) -> Result<()> {
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
        match version {
            Some(v) if v == SCHEMA_VERSION.to_string() => {}
            Some(v) => bail!(
                "graph.db schema version {v} is not supported (expected {SCHEMA_VERSION}); \
                 run code-review-graph update"
            ),
            None => bail!("graph.db has no schema_version; run code-review-graph update"),
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
