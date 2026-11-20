// ABOUTME: Repeatable offline indexing speed and graph-equivalence benchmark.
// ABOUTME: Uses temporary corpora and reports feature/analyzer policies with every sample.
use anyhow::{Result, ensure};
use codegraph_core::config_manager::{CodeGraphConfig, IndexingTier};
use codegraph_mcp::{IndexStats, IndexerConfig, ProjectIndexer};
use indicatif::{MultiProgress, ProgressDrawTarget};
use serde_json::{Value, json};
use std::{path::Path, time::Instant};

async fn graph(indexer: &ProjectIndexer) -> Result<Value> {
    let rows = |sql: &str| {
        indexer.graph_store().with_conn_blocking(|conn| {
            let mut stmt = conn.prepare(sql)?;
            let names: Vec<String> = stmt.column_names().iter().map(|n| n.to_string()).collect();
            let rows = stmt.query_map([], |row| {
                let mut object = serde_json::Map::new();
                for (i, name) in names.iter().enumerate() {
                    let value = match row.get::<_, rusqlite::types::Value>(i)? {
                        rusqlite::types::Value::Null => Value::Null,
                        rusqlite::types::Value::Integer(v) => json!(v),
                        rusqlite::types::Value::Real(v) => json!(v),
                        rusqlite::types::Value::Text(v) => json!(v),
                        rusqlite::types::Value::Blob(v) => json!(v),
                    };
                    object.insert(name.clone(), value);
                }
                Ok(Value::Object(object))
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    };
    // Row ids and timestamps vary between equivalent builds; compare the graph content.
    Ok(json!({
        "nodes": rows("SELECT c.uuid, n.name, n.kind, c.rust_kind, n.file_path, n.language, \
                       c.attributes FROM cg_nodes c JOIN nodes n USING (qualified_name) \
                       ORDER BY c.uuid")?,
        "edges": rows("SELECT c.uuid, e.source_qualified, e.target_qualified, e.kind, e.extra \
                       FROM cg_edges c JOIN edges e ON e.id = c.edge_id ORDER BY c.uuid")?,
    }))
}
async fn sample(
    indexer: &ProjectIndexer,
    path: &Path,
    name: &str,
    repeat: usize,
    output: &mut Vec<Value>,
) -> Result<IndexStats> {
    let start = Instant::now();
    let stats = indexer.reconcile_project(path, false).await?;
    ensure!(
        stats.complete && stats.errors == 0,
        "Incomplete benchmark run"
    );
    output.push(json!({"scenario":name,"repeat":repeat,"wall_us":start.elapsed().as_micros(),"stats":stats,"graph_fingerprint":codegraph_core::artifact_cache::fingerprint(&graph(indexer).await?)?}));
    Ok(stats)
}
#[tokio::main]
async fn main() -> Result<()> {
    let repeats = std::env::var("CODEGRAPH_BENCH_REPEATS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(3)
        .max(1);
    let files = std::env::var("CODEGRAPH_BENCH_FILES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(100)
        .max(2);
    let mut samples = Vec::new();
    for repeat in 0..repeats {
        let directory = tempfile::tempdir()?;
        let root = std::fs::canonicalize(directory.path())?;
        std::fs::write(root.join("target.rs"), "pub fn target() {}\n")?;
        for index in 0..files - 1 {
            std::fs::write(
                root.join(format!("file_{index}.rs")),
                format!("pub fn caller_{index}() {{ target(); }}\n"),
            )?;
        }
        let startup = Instant::now();
        let make = |tier| IndexerConfig {
            project_root: root.clone(),
            indexing_tier: tier,
            ..Default::default()
        };
        let mut indexer = ProjectIndexer::new(
            make(IndexingTier::Fast),
            &CodeGraphConfig::default(),
            MultiProgress::with_draw_target(ProgressDrawTarget::hidden()),
        )
        .await?;
        let startup_us = startup.elapsed().as_micros();
        sample(&indexer, &root, "cold", repeat, &mut samples).await?;
        let warm = sample(&indexer, &root, "no_change", repeat, &mut samples).await?;
        ensure!(
            warm.cached_files == files && warm.writer_rows_acked == 0 && warm.inference_texts == 0,
            "No-change run performed unexpected work"
        );
        let start = Instant::now();
        let stats = indexer.reconcile_project(&root, true).await?;
        ensure!(
            stats.cached_files == files,
            "Warm forced run missed AST cache"
        );
        samples.push(json!({"scenario":"warm_full","repeat":repeat,"wall_us":start.elapsed().as_micros(),"stats":stats}));
        std::fs::write(
            root.join("file_0.rs"),
            "pub fn caller_0() { target(); target(); }\n",
        )?;
        let edit = sample(&indexer, &root, "single_file", repeat, &mut samples).await?;
        ensure!(
            edit.cached_files == files - 1,
            "Single-file edit reparsed unrelated files"
        );
        std::fs::rename(root.join("target.rs"), root.join("renamed.rs"))?;
        sample(&indexer, &root, "cross_file_rename", repeat, &mut samples).await?;
        let incremental = graph(&indexer).await?;
        indexer.reconcile_project(&root, true).await?;
        ensure!(
            graph(&indexer).await? == incremental,
            "Incremental rename differs from full indexing"
        );
        std::fs::remove_file(root.join("renamed.rs"))?;
        sample(&indexer, &root, "delete", repeat, &mut samples).await?;
        ensure!(
            !graph(&indexer).await?["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|node| node["name"] == "target"),
            "Deleted definition survived"
        );
        std::fs::write(root.join("README.md"), "`caller_0` is public\n")?;
        sample(&indexer, &root, "docs_change", repeat, &mut samples).await?;
        std::fs::write(root.join("package.json"), "{\"name\":\"fixture\"}")?;
        sample(&indexer, &root, "manifest_change", repeat, &mut samples).await?;
        for (name, tier) in [
            ("balanced_upgrade", IndexingTier::Balanced),
            ("full_upgrade", IndexingTier::Full),
        ] {
            indexer.set_indexing_tier(tier);
            sample(&indexer, &root, name, repeat, &mut samples).await?;
        }
        samples.push(json!({"scenario":"startup","repeat":repeat,"wall_us":startup_us}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"format":1,"profile":if cfg!(debug_assertions){"debug"}else{"release"},"files":files,"repeats":repeats,"features":{"embeddings":false,"ai_enhanced":false},"analyzers_env":std::env::var("CODEGRAPH_ANALYZERS").unwrap_or_default(),"samples":samples})
        )?
    );
    Ok(())
}
