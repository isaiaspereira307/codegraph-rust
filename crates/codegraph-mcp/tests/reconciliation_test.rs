// ABOUTME: Exercises complete-project reconciliation without network services or inference.
// ABOUTME: Checks cached callers, deletion, empty sources, docs invalidation and no-change runs.
use anyhow::Result;
use codegraph_core::config_manager::{CodeGraphConfig, IndexingTier};
use codegraph_graph::sqlite_store::SqliteGraphStore;
use codegraph_mcp::indexer::{IndexerConfig, ProjectIndexer};
use indicatif::{MultiProgress, ProgressDrawTarget};
use serde_json::Value;

/// Each row's columns joined with '|', in query order.
fn rows(store: &SqliteGraphStore, sql: &str) -> Result<Vec<String>> {
    store.with_conn_blocking(|conn| {
        let mut stmt = conn.prepare(sql)?;
        let columns = stmt.column_count();
        let rows = stmt.query_map([], |row| {
            (0..columns)
                .map(|i| {
                    row.get::<_, Option<String>>(i)
                        .map(Option::unwrap_or_default)
                })
                .collect::<rusqlite::Result<Vec<_>>>()
                .map(|values| values.join("|"))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
}

fn file_metadata(store: &SqliteGraphStore) -> Result<Vec<Value>> {
    rows(
        store,
        "SELECT record FROM cg_file_metadata ORDER BY file_path",
    )?
    .iter()
    .map(|row| Ok(serde_json::from_str(row)?))
    .collect()
}

/// Node (name, file) and edge (source, target, kind, metadata) rows, independent of row ids.
fn graph(store: &SqliteGraphStore) -> Result<(Vec<String>, Vec<String>)> {
    Ok((
        rows(
            store,
            "SELECT name, file_path FROM nodes ORDER BY file_path, name",
        )?,
        rows(
            store,
            "SELECT source_qualified, target_qualified, kind, extra FROM edges \
             ORDER BY source_qualified, target_qualified, kind, extra",
        )?,
    ))
}

fn has_node(graph: &(Vec<String>, Vec<String>), name: &str) -> bool {
    graph
        .0
        .iter()
        .any(|row| row.split('|').next() == Some(name))
}

#[tokio::test]
async fn incremental_catalog_matches_full_graph_and_removes_stale_definitions() -> Result<()> {
    if !test_env::run(
        concat!(
            module_path!(),
            "::incremental_catalog_matches_full_graph_and_removes_stale_definitions"
        ),
        &[
            ("CODEGRAPH_NO_PROGRESS", Some("1")),
            ("CODEGRAPH_ANALYZERS", Some("0")),
            ("CODEGRAPH_ANALYZERS_REQUIRE_TOOLS", Some("0")),
        ],
    ) {
        return Ok(());
    }
    let dir = tempfile::tempdir()?;
    let caller = dir.path().join("caller.rs");
    let target = dir.path().join("target.rs");
    std::fs::write(&caller, "fn caller() { target(); }\n")?;
    std::fs::write(&target, "fn target() {}\n")?;
    let config = IndexerConfig {
        project_root: dir.path().into(),
        indexing_tier: IndexingTier::Fast,
        ..Default::default()
    };
    let indexer = ProjectIndexer::new(
        config,
        &CodeGraphConfig::default(),
        MultiProgress::with_draw_target(ProgressDrawTarget::hidden()),
    )
    .await?;
    let cold = indexer.index_project(dir.path()).await?;
    assert!(cold.complete);
    assert_eq!(cold.files, 2);
    let storage = indexer.graph_store();
    assert!(storage.root().join(".code-review-graph/graph.db").is_file());
    let before_metadata = file_metadata(&storage)?;
    let cold_edges: Vec<Value> = rows(&storage, "SELECT extra FROM edges WHERE kind = 'CALLS'")?
        .iter()
        .map(|row| serde_json::from_str(row))
        .collect::<Result<_, _>>()?;
    assert!(!cold_edges.is_empty());
    assert!(
        cold_edges
            .iter()
            .all(|edge| edge["resolution_method"] == "exact")
    );
    let warm = indexer.index_project(dir.path()).await?;
    assert_eq!(warm.nodes, cold.nodes);
    assert_eq!(warm.edges, cold.edges);
    assert_eq!(warm.skipped, 2);
    std::fs::write(&target, "fn replacement() {}\n")?;
    let edited = indexer.reconcile_project(dir.path(), false).await?;
    assert_eq!(edited.cached_files, 1);
    assert!(edited.writer_rows_acked < cold.writer_rows_acked);
    let after_metadata = file_metadata(&storage)?;
    let caller_metadata = |rows: &[Value]| {
        rows.iter()
            .find(|row| row["file_path"] == caller.to_string_lossy().as_ref())
            .map(|row| (row["file_path"].clone(), row["last_indexed_at"].clone()))
            .unwrap()
    };
    assert_eq!(
        caller_metadata(&before_metadata),
        caller_metadata(&after_metadata)
    );
    let incremental = graph(&storage)?;
    assert!(!has_node(&incremental, "target"));
    assert!(has_node(&incremental, "replacement"));
    indexer.reconcile_project(dir.path(), true).await?;
    assert_eq!(incremental, graph(&storage)?);
    std::fs::write(dir.path().join("README.md"), "new documentation")?;
    let docs = indexer.index_project(dir.path()).await?;
    assert_eq!(docs.cached_files, 2);
    assert_eq!(docs.skipped, 2);
    std::fs::remove_file(&target)?;
    indexer.delete_file_data(&target).await?;
    assert!(!has_node(&graph(&storage)?, "replacement"));
    std::fs::write(&caller, "")?;
    indexer.index_single_file(&caller).await?;
    assert!(!has_node(&graph(&storage)?, "caller"));
    Ok(())
}

mod test_env {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/support/env.rs"
    ));
}
