// ABOUTME: Functions at or above a line-count threshold, read from SQLite line spans.
// ABOUTME: Ranks by length, unlike find_complexity_hotspots, which ranks by complexity.

use super::GraphQuery;
use super::types::{LargeFunction, NodeLocation};
use anyhow::Result;
use rusqlite::params;

const DEFAULT_MIN_LINES: i64 = 50;
const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;

// Storage keeps methods as kind 'Function' (schema v15 has File, Class, Function, Test).
const LARGE_SQL: &str = "SELECT n.qualified_name, n.name, n.language, n.file_path, \
     n.line_start, n.line_end, c.uuid \
     FROM nodes n \
     LEFT JOIN cg_nodes c ON c.qualified_name = n.qualified_name \
     WHERE n.kind = 'Function' AND n.line_start IS NOT NULL AND n.line_end IS NOT NULL \
       AND n.line_end - n.line_start + 1 >= ?1 \
     ORDER BY n.line_end - n.line_start DESC, n.qualified_name \
     LIMIT ?2";

impl GraphQuery {
    /// Functions whose span covers at least `min_lines` lines, longest first.
    /// `min_lines` <= 0 means the default (50); `limit` 0 means 20, capped at 100.
    pub async fn find_large_functions(
        &self,
        min_lines: i64,
        limit: usize,
    ) -> Result<Vec<LargeFunction>> {
        let min_lines = if min_lines > 0 {
            min_lines
        } else {
            DEFAULT_MIN_LINES
        };
        let limit = match limit {
            0 => DEFAULT_LIMIT,
            n => n.min(MAX_LIMIT),
        } as i64;
        self.store()
            .with_conn(move |conn| {
                let mut stmt = conn.prepare(LARGE_SQL)?;
                let rows = stmt.query_map(params![min_lines, limit], |row| {
                    let qualified: String = row.get(0)?;
                    let uuid: Option<String> = row.get(6)?;
                    let start: i32 = row.get(4)?;
                    let end: i32 = row.get(5)?;
                    Ok(LargeFunction {
                        id: uuid.unwrap_or(qualified),
                        name: row.get(1)?,
                        language: row.get(2)?,
                        location: NodeLocation {
                            file_path: row.get(3)?,
                            start_line: Some(start),
                            end_line: Some(end),
                        },
                        line_count: end - start + 1,
                    })
                })?;
                Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .await
    }
}
