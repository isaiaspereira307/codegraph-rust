// ABOUTME: Full-text node search over the nodes_fts FTS5 index (porter stemming, bm25 ranking).
// ABOUTME: Reads SQLite directly, so it never builds the petgraph snapshot.

use super::GraphQuery;
use super::types::{NodeLocation, NodeReference};
use anyhow::Result;
use rusqlite::params;

const DEFAULT_LIMIT: usize = 10;

// bm25 column weights follow nodes_fts order: name, qualified_name, file_path, signature,
// docstring, name_tokens. A name hit outranks a path-only hit.
const SEARCH_SQL: &str = "SELECT n.qualified_name, n.name, n.kind, n.file_path, \
     n.line_start, n.line_end, c.uuid \
     FROM nodes_fts f \
     JOIN nodes n ON n.id = f.rowid \
     LEFT JOIN cg_nodes c ON c.qualified_name = n.qualified_name \
     WHERE nodes_fts MATCH ?1 \
     ORDER BY bm25(nodes_fts, 10.0, 5.0, 1.0, 2.0, 2.0, 5.0), n.name, n.qualified_name \
     LIMIT ?2";

/// Turns free text into an FTS5 expression: each whitespace-separated term becomes a quoted
/// prefix term, and the terms are ANDed. Embedded quotes are doubled so input cannot inject
/// FTS operators. Returns `None` when there is no term to search for.
pub(crate) fn fts_match_expression(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

impl GraphQuery {
    /// Searches names, qualified names, paths, signatures, docstrings and camelCase tokens.
    /// Results reflect the index as of the last `finalize`. `limit` 0 means the default.
    pub async fn search_nodes(&self, query: &str, limit: usize) -> Result<Vec<NodeReference>> {
        let Some(expression) = fts_match_expression(query) else {
            return Ok(Vec::new());
        };
        let limit = if limit == 0 { DEFAULT_LIMIT } else { limit } as i64;
        self.store
            .with_conn(move |conn| {
                let mut stmt = conn.prepare(SEARCH_SQL)?;
                let rows = stmt.query_map(params![expression, limit], |row| {
                    let qualified: String = row.get(0)?;
                    let uuid: Option<String> = row.get(6)?;
                    Ok(NodeReference {
                        id: uuid.unwrap_or(qualified),
                        name: row.get(1)?,
                        kind: row.get(2)?,
                        location: Some(NodeLocation {
                            file_path: row.get(3)?,
                            start_line: row.get(4)?,
                            end_line: row.get(5)?,
                        }),
                    })
                })?;
                let out = rows.collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(out)
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::fts_match_expression;

    #[test]
    fn expression_quotes_terms_and_escapes_quotes() {
        assert_eq!(
            fts_match_expression("parse  file").as_deref(),
            Some("\"parse\"* \"file\"*")
        );
        assert_eq!(
            fts_match_expression("a\"b OR c").as_deref(),
            Some("\"a\"\"b\"* \"OR\"* \"c\"*")
        );
        assert_eq!(fts_match_expression("   "), None);
    }
}
