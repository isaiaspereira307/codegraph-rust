// ABOUTME: Exercises complete-project reconciliation without network services or inference.
// ABOUTME: Checks cached callers, deletion, empty sources, docs invalidation and no-change runs.
use anyhow::Result;
use codegraph_core::config_manager::{CodeGraphConfig, IndexingTier};
use codegraph_graph::analysis::PostprocessLevel;
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
    // The crate root is the caller, so `crate::target` anchors at `src/`. Rust paths resolve
    // only inside a Cargo package, like code-review-graph.
    std::fs::create_dir(dir.path().join("src"))?;
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n",
    )?;
    let caller = dir.path().join("src/lib.rs");
    let target = dir.path().join("src/target.rs");
    // `compute()` crosses files through its `use`; `missing()` names nothing and stays text.
    std::fs::write(
        &caller,
        "mod target;\nuse crate::target::compute;\n\
         fn caller() { compute(); helper(); missing(); }\nfn helper() {}\n",
    )?;
    std::fs::write(&target, "fn compute() {}\n")?;
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
    let calls = |storage: &SqliteGraphStore| -> Result<Vec<(String, Value)>> {
        let prefix = format!("{}/", dir.path().to_string_lossy());
        let mut calls = rows(
            storage,
            "SELECT target_qualified, extra FROM edges WHERE kind = 'CALLS'",
        )?
        .iter()
        .map(|row| {
            let (target, extra) = row.split_once('|').unwrap();
            Ok((target.replace(&prefix, ""), serde_json::from_str(extra)?))
        })
        .collect::<Result<Vec<_>>>()?;
        calls.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(calls)
    };
    let cold_calls = calls(&storage)?;
    let targets: Vec<_> = cold_calls.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        targets,
        ["missing", "src/lib.rs::helper", "src/target.rs::compute"]
    );
    assert_eq!(cold_calls[0].1["unresolved_target"], "missing");
    assert_eq!(cold_calls[1].1["resolution_method"], "exact");
    assert_eq!(cold_calls[2].1["resolution_method"], "exact");
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
    assert!(!has_node(&incremental, "compute"));
    // The edit unbinds the cross-file call: it is now kept as its text.
    let targets: Vec<_> = calls(&storage)?.into_iter().map(|(t, _)| t).collect();
    assert_eq!(targets, ["compute", "missing", "src/lib.rs::helper"]);
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

#[tokio::test]
async fn test_detection_reads_paths_relative_to_the_project_root() -> Result<()> {
    if !test_env::run(
        concat!(
            module_path!(),
            "::test_detection_reads_paths_relative_to_the_project_root"
        ),
        &[
            ("CODEGRAPH_NO_PROGRESS", Some("1")),
            ("CODEGRAPH_ANALYZERS", Some("0")),
            ("CODEGRAPH_ANALYZERS_REQUIRE_TOOLS", Some("0")),
        ],
    ) {
        return Ok(());
    }
    // The checkout itself sits in a directory named `test`, which must not count.
    let outer = tempfile::tempdir()?;
    let root = outer.path().join("test/repo");
    std::fs::create_dir_all(root.join("src"))?;
    std::fs::create_dir_all(root.join("tests"))?;
    std::fs::write(
        root.join("src/app.ts"),
        "import os from 'os';\nfunction it() {}\n",
    )?;
    std::fs::write(root.join("tests/spec.ts"), "function it() {}\n")?;
    let config = IndexerConfig {
        project_root: root.clone(),
        indexing_tier: IndexingTier::Fast,
        ..Default::default()
    };
    let indexer = ProjectIndexer::new(
        config,
        &CodeGraphConfig::default(),
        MultiProgress::with_draw_target(ProgressDrawTarget::hidden()),
    )
    .await?;
    let check = |storage: &SqliteGraphStore| -> Result<()> {
        let functions = rows(
            storage,
            "SELECT substr(file_path, length(file_path) - 10), kind, CAST(is_test AS TEXT) FROM nodes \
             WHERE name = 'it' ORDER BY file_path",
        )?;
        assert_eq!(functions, ["/src/app.ts|Function|0", "sts/spec.ts|Test|1"]);
        assert!(rows(storage, "SELECT name FROM nodes WHERE kind = 'Import'")?.is_empty());
        Ok(())
    };
    // Full index from the absolute root, then a watch-style single-file run.
    indexer.index_project(&root).await?;
    check(&indexer.graph_store())?;
    indexer
        .index_single_file(&root.join("tests/spec.ts"))
        .await?;
    check(&indexer.graph_store())?;
    Ok(())
}

/// Indexes `files` (fast tier, analyzers off) and returns the indexer, or None when the
/// environment guard skips the test.
async fn index_files(
    name: &str,
    files: &[(&str, &str)],
) -> Result<Option<(tempfile::TempDir, ProjectIndexer)>> {
    if !test_env::run(
        name,
        &[
            ("CODEGRAPH_NO_PROGRESS", Some("1")),
            ("CODEGRAPH_ANALYZERS", Some("0")),
            ("CODEGRAPH_ANALYZERS_REQUIRE_TOOLS", Some("0")),
        ],
    ) {
        return Ok(None);
    }
    let dir = tempfile::tempdir()?;
    for (path, source) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().expect("file in a directory"))?;
        std::fs::write(path, source)?;
    }
    let config = IndexerConfig {
        project_root: dir.path().to_path_buf(),
        indexing_tier: IndexingTier::Fast,
        ..Default::default()
    };
    let indexer = ProjectIndexer::new(
        config,
        &CodeGraphConfig::default(),
        MultiProgress::with_draw_target(ProgressDrawTarget::hidden()),
    )
    .await?;
    indexer.index_project(dir.path()).await?;
    Ok(Some((dir, indexer)))
}

/// Sorted `kind|source|target` rows of the given kinds, with the project root prefix removed.
fn edges_of(store: &SqliteGraphStore, root: &std::path::Path, kinds: &str) -> Result<Vec<String>> {
    let prefix = format!("{}/", root.to_string_lossy());
    let mut edges: Vec<_> = rows(
        store,
        &format!(
            "SELECT kind, source_qualified, target_qualified FROM edges WHERE kind IN ({kinds})"
        ),
    )?
    .into_iter()
    .map(|row| row.replace(&prefix, ""))
    .collect();
    edges.sort();
    Ok(edges)
}

#[tokio::test]
async fn unresolved_call_is_kept_as_text() -> Result<()> {
    let Some((dir, indexer)) = index_files(
        concat!(module_path!(), "::unresolved_call_is_kept_as_text"),
        &[("a.py", "import os\n\ndef f():\n    print(\"x\")\n")],
    )
    .await?
    else {
        return Ok(());
    };
    assert_eq!(
        edges_of(
            &indexer.graph_store(),
            dir.path(),
            "'CALLS', 'IMPORTS_FROM'"
        )?,
        ["CALLS|a.py::f|print", "IMPORTS_FROM|a.py|os"]
    );
    Ok(())
}

/// Java calls on a typed receiver bind to the defining file even without an import: same-package
/// types and fully qualified types use the CRG's file lookup, not just `IMPORTS_FROM` evidence.
#[tokio::test]
async fn java_typed_receivers_bind_without_imports() -> Result<()> {
    let Some((dir, indexer)) = index_files(
        concat!(module_path!(), "::java_typed_receivers_bind_without_imports"),
        &[
            (
                "com/x/Cart.java",
                "package com.x;\npublic class Cart { public long total() { return 1; } }\n",
            ),
            (
                "com/x/OrderService.java",
                "package com.x;\npublic class OrderService { public long run(Cart cart) { return cart.total(); } }\n",
            ),
            (
                "com/x/Price.java",
                "package com.x;\npublic class Price { public long amount() { return 2; } }\n",
            ),
            (
                "com/y/Checkout.java",
                "package com.y;\npublic class Checkout { public long price(com.x.Price price) { return price.amount(); } }\n",
            ),
        ],
    )
    .await?
    else {
        return Ok(());
    };
    let calls = edges_of(&indexer.graph_store(), dir.path(), "'CALLS'")?;
    assert!(
        calls.contains(
            &"CALLS|com/x/OrderService.java::OrderService.run|com/x/Cart.java::Cart.total"
                .to_string()
        ),
        "{calls:?}"
    );
    assert!(
        calls.contains(
            &"CALLS|com/y/Checkout.java::Checkout.price|com/x/Price.java::Price.amount".to_string()
        ),
        "{calls:?}"
    );
    Ok(())
}

/// Rust `use` paths bind to File nodes through Cargo layouts: `crate::` inside the importer's
/// crate, a path dependency's name to that crate's modules; `std` stays text.
#[tokio::test]
async fn rust_imports_bind_to_files_across_crates() -> Result<()> {
    let Some((dir, indexer)) = index_files(
        concat!(module_path!(), "::rust_imports_bind_to_files_across_crates"),
        &[
            (
                "app/Cargo.toml",
                "[package]\nname = \"app\"\n\n[dependencies]\n\
                 util-lib = { path = \"../util-lib\" }\n",
            ),
            (
                "app/src/main.rs",
                "mod config;\nuse crate::config::load;\nuse crate::config::Tr;\n\
                 use util_lib::text::shout;\nuse std::fmt;\n\nstruct Disk;\nimpl Tr for Disk {}\n\
                 impl fmt::Display for Disk {}\n\nfn main() { load(); shout(); }\n\
                 fn run() { config::load(); util_lib::text::shout(); Vec::new(); }\n",
            ),
            ("app/src/config.rs", "pub fn load() {}\npub trait Tr {}\n"),
            ("util-lib/Cargo.toml", "[package]\nname = \"util-lib\"\n"),
            ("util-lib/src/lib.rs", "pub mod text;\n"),
            ("util-lib/src/text.rs", "pub fn shout() {}\n"),
        ],
    )
    .await?
    else {
        return Ok(());
    };
    let imports = edges_of(&indexer.graph_store(), dir.path(), "'IMPORTS_FROM'")?;
    for expected in [
        "IMPORTS_FROM|app/src/main.rs|app/src/config.rs",
        "IMPORTS_FROM|app/src/main.rs|util-lib/src/text.rs",
        "IMPORTS_FROM|app/src/main.rs|std::fmt",
    ] {
        assert!(
            imports.iter().any(|row| row == expected),
            "missing {expected} in {imports:?}"
        );
    }
    // Scoped calls and `impl Trait for T` bind across files; `std` paths stay text.
    let store = indexer.graph_store();
    let bound = edges_of(&store, dir.path(), "'CALLS', 'IMPLEMENTS'")?;
    for expected in [
        "CALLS|app/src/main.rs::run|app/src/config.rs::load",
        "CALLS|app/src/main.rs::run|util-lib/src/text.rs::shout",
        "CALLS|app/src/main.rs::run|Vec::new",
        "IMPLEMENTS|app/src/main.rs::Disk|app/src/config.rs::Tr",
        "IMPLEMENTS|app/src/main.rs::Disk|fmt::Display",
    ] {
        assert!(
            bound.iter().any(|row| row == expected),
            "missing {expected} in {bound:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn unresolved_target_never_binds_to_other_file() -> Result<()> {
    let Some((dir, indexer)) = index_files(
        concat!(
            module_path!(),
            "::unresolved_target_never_binds_to_other_file"
        ),
        &[
            (
                "a.py",
                "def f():\n    helper()\n    local()\n\ndef local():\n    pass\n",
            ),
            ("b.py", "def helper():\n    return 1\n"),
        ],
    )
    .await?
    else {
        return Ok(());
    };
    assert_eq!(
        edges_of(&indexer.graph_store(), dir.path(), "'CALLS'")?,
        ["CALLS|a.py::f|a.py::local", "CALLS|a.py::f|helper"]
    );
    Ok(())
}

#[tokio::test]
async fn tested_by_mirrors_calls_from_tests() -> Result<()> {
    let Some((dir, indexer)) = index_files(
        concat!(module_path!(), "::tested_by_mirrors_calls_from_tests"),
        &[(
            "test_a.py",
            "def f():\n    pass\n\ndef test_x():\n    f()\n    print(1)\n",
        )],
    )
    .await?
    else {
        return Ok(());
    };
    let expected = [
        "TESTED_BY|print|test_a.py::test_x",
        "TESTED_BY|test_a.py::f|test_a.py::test_x",
    ];
    let store = indexer.graph_store();
    assert_eq!(edges_of(&store, dir.path(), "'TESTED_BY'")?, expected);
    // A watch-style single-file run after an edit replaces rather than duplicates them.
    std::fs::write(
        dir.path().join("test_a.py"),
        "def f():\n    pass\n\ndef test_x():\n    f()\n    len(1)\n",
    )?;
    indexer
        .index_single_file(&dir.path().join("test_a.py"))
        .await?;
    assert_eq!(
        edges_of(&store, dir.path(), "'TESTED_BY'")?,
        [
            "TESTED_BY|len|test_a.py::test_x",
            "TESTED_BY|test_a.py::f|test_a.py::test_x",
        ]
    );
    Ok(())
}

#[tokio::test]
async fn imported_module_lets_a_bare_call_cross_files() -> Result<()> {
    let Some((dir, indexer)) = index_files(
        concat!(
            module_path!(),
            "::imported_module_lets_a_bare_call_cross_files"
        ),
        &[
            (
                "a.py",
                "from b import helper\n\ndef f():\n    helper()\n    other()\n",
            ),
            ("b.py", "def helper():\n    return 1\n"),
            ("c.py", "def other():\n    return 2\n"),
        ],
    )
    .await?
    else {
        return Ok(());
    };
    assert_eq!(
        edges_of(
            &indexer.graph_store(),
            dir.path(),
            "'CALLS', 'IMPORTS_FROM'"
        )?,
        [
            "CALLS|a.py::f|b.py::helper",
            "CALLS|a.py::f|other",
            "IMPORTS_FROM|a.py|b.py",
        ]
    );
    Ok(())
}

#[tokio::test]
async fn python_submodule_imports_and_import_bound_calls_follow_crg() -> Result<()> {
    let Some((dir, indexer)) = index_files(
        concat!(
            module_path!(),
            "::python_submodule_imports_and_import_bound_calls_follow_crg"
        ),
        &[
            (
                "a.py",
                "from pkg import b, missing\nfrom pkg.b import Thing, helper as hh\n\
                 import pkg.b as bm\nimport json as js\nfrom ns import c\n\n\
                 def f(x):\n    b.helper()\n    bm.helper()\n    hh()\n    Thing.go()\n    \
                 js.loads('1')\n    x.go()\n    c.run()\n",
            ),
            ("pkg/__init__.py", ""),
            (
                "pkg/b.py",
                "def helper():\n    return 1\n\nclass Thing:\n    def go(self):\n        pass\n",
            ),
            ("pkg/typing.py", "def cast():\n    pass\n"),
            ("pkg/m.py", "import typing\n\ndef go():\n    pass\n"),
            ("ns/c.py", "def run():\n    pass\n"),
        ],
    )
    .await?
    else {
        return Ok(());
    };
    assert_eq!(
        edges_of(
            &indexer.graph_store(),
            dir.path(),
            "'CALLS', 'IMPORTS_FROM'"
        )?,
        [
            "CALLS|a.py::f|go",
            "CALLS|a.py::f|loads",
            "CALLS|a.py::f|ns/c.py::run",
            "CALLS|a.py::f|pkg/b.py::Thing.go",
            "CALLS|a.py::f|pkg/b.py::helper",
            "CALLS|a.py::f|pkg/b.py::helper",
            "CALLS|a.py::f|pkg/b.py::helper",
            "IMPORTS_FROM|a.py|json",
            "IMPORTS_FROM|a.py|ns/c.py",
            "IMPORTS_FROM|a.py|pkg/__init__.py",
            "IMPORTS_FROM|a.py|pkg/b.py",
            "IMPORTS_FROM|a.py|pkg/b.py",
            "IMPORTS_FROM|a.py|pkg/b.py",
            "IMPORTS_FROM|pkg/m.py|typing",
        ]
    );
    Ok(())
}

#[tokio::test]
async fn package_imports_never_bind_to_a_same_named_repo_file() -> Result<()> {
    let Some((dir, indexer)) = index_files(
        concat!(
            module_path!(),
            "::package_imports_never_bind_to_a_same_named_repo_file"
        ),
        &[
            (
                "main.go",
                "package main\n\nimport \"errors\"\n\nfunc run() { New() }\n",
            ),
            ("errors.go", "package main\n\nfunc New() {}\n"),
            (
                "app.ts",
                "import React from 'react';\nfunction f() { render(); }\n",
            ),
            ("react.ts", "export function render() {}\n"),
        ],
    )
    .await?
    else {
        return Ok(());
    };
    let imports = edges_of(&indexer.graph_store(), dir.path(), "'IMPORTS_FROM'")?;
    assert_eq!(
        imports,
        ["IMPORTS_FROM|app.ts|react", "IMPORTS_FROM|main.go|errors"]
    );
    let calls = edges_of(&indexer.graph_store(), dir.path(), "'CALLS'")?;
    assert_eq!(calls, ["CALLS|app.ts::f|render", "CALLS|main.go::run|New"]);
    Ok(())
}

#[tokio::test]
async fn fast_ml_pattern_labels_are_not_stored_as_edges() -> Result<()> {
    let Some((dir, indexer)) = index_files(
        concat!(module_path!(), "::fast_ml_pattern_labels_are_not_stored_as_edges"),
        &[
            (
                "A.java",
                "class A extends B implements Runnable {\n    @Override\n    public void run() {}\n}\n",
            ),
            ("lib.rs", "use std::fmt;\ntrait T { fn f(&self); }\nstruct S;\nimpl T for S { fn f(&self) {} }\n"),
        ],
    )
    .await?
    else {
        return Ok(());
    };
    let labels = rows(
        &indexer.graph_store(),
        "SELECT kind || '|' || target_qualified FROM edges \
         WHERE target_qualified LIKE 'java:%' OR target_qualified LIKE 'rust:%'",
    )?;
    assert!(
        labels.is_empty(),
        "pattern labels stored as edges: {labels:?}"
    );
    let _ = dir;
    Ok(())
}

#[tokio::test]
async fn bare_import_never_binds_to_a_same_file_symbol() -> Result<()> {
    let Some((dir, indexer)) = index_files(
        concat!(
            module_path!(),
            "::bare_import_never_binds_to_a_same_file_symbol"
        ),
        &[
            ("a.py", "import json\n\ndef json():\n    pass\n"),
            (
                "lib.rs",
                "use anyhow::{Result, anyhow};\nfn f() -> Result<()> { Err(anyhow!(\"x\")) }\nfn anyhow() {}\n",
            ),
        ],
    )
    .await?
    else {
        return Ok(());
    };
    let imports = edges_of(&indexer.graph_store(), dir.path(), "'IMPORTS_FROM'")?;
    assert!(
        imports
            .iter()
            .all(|row| !row.contains(".py::") && !row.contains(".rs::")),
        "an import bound to a symbol: {imports:?}"
    );
    assert!(
        imports.contains(&"IMPORTS_FROM|a.py|json".to_string()),
        "{imports:?}"
    );
    Ok(())
}

#[tokio::test]
async fn test_function_keeps_its_function_node_id() -> Result<()> {
    let Some((dir, indexer)) = index_files(
        concat!(module_path!(), "::test_function_keeps_its_function_node_id"),
        &[("test_a.py", "def test_x():\n    pass\n")],
    )
    .await?
    else {
        return Ok(());
    };
    let found = rows(
        &indexer.graph_store(),
        "SELECT c.uuid, n.file_path, n.kind, CAST(n.line_start AS TEXT) FROM nodes n \
         JOIN cg_nodes c ON c.qualified_name = n.qualified_name WHERE n.name = 'test_x'",
    )?;
    assert_eq!(found.len(), 1, "{found:?}");
    let columns: Vec<_> = found[0].split('|').collect();
    assert_eq!(columns[2], "Test");
    let expected = codegraph_core::generate_node_id(
        &dir.path().display().to_string(),
        &codegraph_core::canonical_file_key(dir.path(), columns[1]),
        "test_x",
        "Function",
        columns[3].parse()?,
    );
    assert_eq!(columns[0], expected.to_string());
    // A test function still counts as a function in the index summary.
    let stats = indexer.index_project(dir.path()).await?;
    assert_eq!(stats.functions, 1);
    Ok(())
}

#[tokio::test]
async fn marker_from_another_graph_shape_triggers_a_rebuild() -> Result<()> {
    let Some((dir, indexer)) = index_files(
        concat!(
            module_path!(),
            "::marker_from_another_graph_shape_triggers_a_rebuild"
        ),
        &[("a.py", "def f():\n    pass\n")],
    )
    .await?
    else {
        return Ok(());
    };
    let store = indexer.graph_store();
    let files = "SELECT name FROM nodes WHERE kind = 'File'";
    assert_eq!(rows(&store, files)?.len(), 1);
    // What an older build leaves: a marker without this build's graph shape, and a graph
    // without File nodes, for unchanged sources.
    store.with_conn_blocking(|conn| {
        conn.execute("DELETE FROM nodes WHERE kind = 'File'", [])?;
        conn.execute(
            "UPDATE cg_project_metadata SET record = \
             json_remove(record, '$.metadata.graph_shape')",
            [],
        )?;
        Ok(())
    })?;
    indexer.index_project(dir.path()).await?;
    assert_eq!(
        rows(&store, files)?.len(),
        1,
        "unchanged sources kept the old shape"
    );
    Ok(())
}

mod test_env {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/support/env.rs"
    ));
}

#[tokio::test]
async fn inherits_targets_stay_text() -> Result<()> {
    let Some((dir, indexer)) = index_files(
        concat!(module_path!(), "::inherits_targets_stay_text"),
        &[(
            "base.ts",
            "export class Base {}\nexport class Leaf extends Base {}\n",
        )],
    )
    .await?
    else {
        return Ok(());
    };
    let inherits = edges_of(&indexer.graph_store(), dir.path(), "'INHERITS'")?;
    assert_eq!(inherits, vec!["INHERITS|base.ts::Leaf|Base".to_string()]);
    Ok(())
}

/// Every file under `dir`, as (path relative to `dir`, contents), sorted by path.
fn probe_files(dir: &std::path::Path) -> Result<Vec<(String, String)>> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path.strip_prefix(dir)?.to_string_lossy().into_owned();
                files.push((relative, std::fs::read_to_string(&path)?));
            }
        }
    }
    files.sort();
    Ok(files)
}

/// The inheritance parity probe, indexed end to end: text INHERITS/IMPLEMENTS targets bind to
/// the base in the same language family across files; external and ambiguous bases do not.
#[tokio::test]
async fn inherits_text_targets_bind_across_files_and_languages() -> Result<()> {
    use codegraph_graph::graph_query::{GraphQuery, QueryPattern};
    let probe = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/parity/probes/inheritance");
    let files = probe_files(&probe)?;
    let borrowed: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, source)| (path.as_str(), source.as_str()))
        .collect();
    let Some((dir, indexer)) = index_files(
        concat!(
            module_path!(),
            "::inherits_text_targets_bind_across_files_and_languages"
        ),
        &borrowed,
    )
    .await?
    else {
        return Ok(());
    };
    let prefix = format!("{}/", dir.path().to_string_lossy());
    let qn = |relative: &str| format!("{prefix}{relative}");
    let store = indexer.graph_store();
    let graph = GraphQuery::new(store.clone());
    // `file::name` of each inheritor of `base`, and the ambiguous count reported for it.
    let inheritors = async |base: &str| -> Result<(Vec<String>, usize)> {
        let result = graph
            .query_graph(QueryPattern::InheritorsOf, &qn(base), 100)
            .await?
            .unwrap_or_else(|| panic!("base {base} is not a node"));
        let mut names: Vec<String> = result
            .results
            .iter()
            .map(|n| {
                let file = n.location.as_ref().map_or("", |l| l.file_path.as_str());
                format!("{}::{}", file.replace(&prefix, ""), n.name)
            })
            .collect();
        names.sort();
        Ok((names, result.unresolved))
    };
    for (base, expected) in [
        (
            "py/shapes.py::Shape",
            ["py/circle.py::Circle", "py/square.py::Square"],
        ),
        (
            "ts/shape.ts::Shape",
            ["ts/circle.ts::Circle", "ts/square.ts::Square"],
        ),
        (
            "java/Shape.java::Shape",
            ["java/Circle.java::Circle", "java/Square.java::Square"],
        ),
        (
            "go/shape.go::Shape",
            ["go/circle.go::Circle", "go/square.go::Square"],
        ),
    ] {
        let (names, _) = inheritors(base).await?;
        assert_eq!(names, expected, "inheritors of {base}");
        let impact = graph.get_impact_radius(&[qn(base)], &[], 1, 0).await?;
        let impacted: Vec<String> = impact.impacted.iter().map(|n| n.node.id.clone()).collect();
        for subclass in expected {
            assert!(
                impact.impacted.iter().any(|n| n.depth == 1
                    && n.node.location.as_ref().is_some_and(|l| {
                        format!("{}::{}", l.file_path.replace(&prefix, ""), n.node.name) == subclass
                    })),
                "impact of {base} misses {subclass}: {impacted:?}"
            );
        }
    }
    // The duplicated base: one ambiguous edge per language, no inheritor for it.
    for shared in [
        "py/shared_a.py::Shared",
        "py/shared_b.py::Shared",
        "ts/shared_a.ts::Shared",
        "java/a/Shared.java::Shared",
        "go/a/shared.go::Shared",
    ] {
        let (names, unresolved) = inheritors(shared).await?;
        assert!(
            names.is_empty(),
            "{shared} must have no inheritors: {names:?}"
        );
        assert_eq!(unresolved, 1, "ambiguous edges reported for {shared}");
    }
    // No subclass of an external or ambiguous base is an inheritor of any in-repo type.
    let types = rows(
        &store,
        "SELECT qualified_name FROM nodes \
         WHERE kind IN ('Class', 'Interface', 'Type', 'Struct', 'Enum', 'Trait')",
    )?;
    for base in &types {
        let (names, _) = inheritors(&base.replace(&prefix, "")).await?;
        for name in &names {
            assert!(
                ["CircleCase", "Widget", "AppError", "Guarded", "Leaf"]
                    .iter()
                    .all(|outsider| !name.ends_with(&format!("::{outsider}"))),
                "{name} bound to in-repo base {base}"
            );
        }
    }
    Ok(())
}

const FLOW_SOURCE: (&str, &str) = (
    "app.py",
    "def main():\n    helper()\n\ndef helper():\n    leaf()\n\ndef leaf():\n    pass\n",
);

fn flow_counts(store: &SqliteGraphStore) -> Result<Vec<String>> {
    rows(
        store,
        "SELECT CAST((SELECT count(*) FROM flows) AS TEXT), \
         CAST((SELECT count(*) FROM flow_memberships) AS TEXT)",
    )
}

#[tokio::test]
async fn finalize_twice_keeps_flow_counts() -> Result<()> {
    let Some((dir, indexer)) = index_files(
        concat!(module_path!(), "::finalize_twice_keeps_flow_counts"),
        &[FLOW_SOURCE],
    )
    .await?
    else {
        return Ok(());
    };
    let store = indexer.graph_store();
    let first = flow_counts(&store)?;
    assert_ne!(first, ["0|0"], "main -> helper -> leaf must yield a flow");
    // Finalize again, twice, directly: an unchanged-source index run would be a no-op.
    for _ in 0..2 {
        refinalize(&store).await?;
        assert_eq!(flow_counts(&store)?, first);
    }
    let _ = dir;
    Ok(())
}

fn postprocess_level(store: &SqliteGraphStore) -> Result<Vec<String>> {
    rows(
        store,
        "SELECT value FROM metadata WHERE key = 'postprocess_level'",
    )
}

/// Runs `finalize_with(full)` and checks the post-process really ran: a sentinel written
/// into `postprocess_level` beforehand must come back as `full`.
async fn refinalize(store: &SqliteGraphStore) -> Result<()> {
    store.with_conn_blocking(|conn| {
        conn.execute(
            "UPDATE metadata SET value = 'sentinel' WHERE key = 'postprocess_level'",
            [],
        )?;
        Ok(())
    })?;
    assert_eq!(postprocess_level(store)?, ["sentinel"]);
    store
        .finalize_with("incremental", PostprocessLevel::Full)
        .await?;
    assert_eq!(
        postprocess_level(store)?,
        ["full"],
        "post-process did not run"
    );
    Ok(())
}

async fn indexer_with(
    dir: &std::path::Path,
    postprocess: PostprocessLevel,
) -> Result<ProjectIndexer> {
    let config = IndexerConfig {
        project_root: dir.to_path_buf(),
        indexing_tier: IndexingTier::Fast,
        postprocess: Some(postprocess),
        ..Default::default()
    };
    ProjectIndexer::new(
        config,
        &CodeGraphConfig::default(),
        MultiProgress::with_draw_target(ProgressDrawTarget::hidden()),
    )
    .await
}

/// `index --skip-postprocess` then a plain `index` over unchanged sources: the build is
/// skipped but the missing `full` post-process runs; a further run does nothing.
#[tokio::test]
async fn noop_index_runs_missing_full_postprocess() -> Result<()> {
    if !test_env::run(
        concat!(module_path!(), "::noop_index_runs_missing_full_postprocess"),
        &[
            ("CODEGRAPH_NO_PROGRESS", Some("1")),
            ("CODEGRAPH_ANALYZERS", Some("0")),
            ("CODEGRAPH_ANALYZERS_REQUIRE_TOOLS", Some("0")),
        ],
    ) {
        return Ok(());
    }
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join(FLOW_SOURCE.0), FLOW_SOURCE.1)?;
    let skipped = indexer_with(dir.path(), PostprocessLevel::None).await?;
    skipped.index_project(dir.path()).await?;
    let store = skipped.graph_store();
    assert_eq!(flow_counts(&store)?, ["0|0"]);
    assert_eq!(postprocess_level(&store)?, ["none"]);
    drop(skipped);

    let full = indexer_with(dir.path(), PostprocessLevel::Full).await?;
    let stats = full.index_project(dir.path()).await?;
    assert_eq!(stats.skipped, 1, "unchanged sources must not be rebuilt");
    assert!(stats.phase_ms.contains_key("postprocess"));
    assert_ne!(
        flow_counts(&store)?,
        ["0|0"],
        "flows computed on the no-op run"
    );
    assert_eq!(postprocess_level(&store)?, ["full"]);

    let stats = full.index_project(dir.path()).await?;
    assert!(
        !stats.phase_ms.contains_key("postprocess"),
        "a complete full post-process is not repeated"
    );
    Ok(())
}

/// A failing analysis step warns instead of failing the build (as code-review-graph does).
#[tokio::test]
async fn postprocess_failure_is_not_fatal() -> Result<()> {
    let Some((dir, indexer)) = index_files(
        concat!(module_path!(), "::postprocess_failure_is_not_fatal"),
        &[FLOW_SOURCE],
    )
    .await?
    else {
        return Ok(());
    };
    let store = indexer.graph_store();
    store.with_conn_blocking(|conn| {
        conn.execute_batch("DROP TABLE flow_memberships; DROP TABLE flows;")?;
        Ok(())
    })?;
    store
        .finalize_with("incremental", PostprocessLevel::Full)
        .await?;
    assert_eq!(postprocess_level(&store)?, ["failed"]);
    let warning = rows(
        &store,
        "SELECT value FROM metadata WHERE key = 'postprocess_warning'",
    )?;
    assert_eq!(warning.len(), 1);
    assert!(warning[0].contains("no such table"), "{warning:?}");
    assert_eq!(
        rows(
            &store,
            "SELECT value FROM metadata WHERE key = 'build_state'"
        )?,
        ["complete"]
    );
    let _ = dir;
    Ok(())
}

#[tokio::test]
async fn skip_postprocess_leaves_flows_empty() -> Result<()> {
    if !test_env::run(
        concat!(module_path!(), "::skip_postprocess_leaves_flows_empty"),
        &[
            ("CODEGRAPH_NO_PROGRESS", Some("1")),
            ("CODEGRAPH_ANALYZERS", Some("0")),
            ("CODEGRAPH_ANALYZERS_REQUIRE_TOOLS", Some("0")),
        ],
    ) {
        return Ok(());
    }
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join(FLOW_SOURCE.0), FLOW_SOURCE.1)?;
    let config = IndexerConfig {
        project_root: dir.path().to_path_buf(),
        indexing_tier: IndexingTier::Fast,
        postprocess: Some(codegraph_graph::analysis::PostprocessLevel::None),
        ..Default::default()
    };
    let indexer = ProjectIndexer::new(
        config,
        &CodeGraphConfig::default(),
        MultiProgress::with_draw_target(ProgressDrawTarget::hidden()),
    )
    .await?;
    indexer.index_project(dir.path()).await?;
    assert_eq!(flow_counts(&indexer.graph_store())?, ["0|0"]);
    Ok(())
}

/// Editing only a `Cargo.toml` changes the input fingerprint, so sources are re-resolved even
/// though their content hashes are unchanged (Task 5 of the P1 parity plan).
#[tokio::test]
async fn manifest_only_edit_re_resolves_rust_sources() -> Result<()> {
    if !test_env::run(
        concat!(
            module_path!(),
            "::manifest_only_edit_re_resolves_rust_sources"
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
    let manifest = dir.path().join("Cargo.toml");
    std::fs::write(&manifest, "[package]\nname = \"app\"\n")?;
    let caller = dir.path().join("src/lib.rs");
    std::fs::create_dir(dir.path().join("src"))?;
    std::fs::write(&caller, "fn caller() { other::thing::remote_only(); }\n")?;
    std::fs::create_dir_all(dir.path().join("other/src"))?;
    std::fs::write(
        dir.path().join("other/Cargo.toml"),
        "[package]\nname = \"other\"\n",
    )?;
    std::fs::write(dir.path().join("other/src/lib.rs"), "pub mod thing;\n")?;
    std::fs::write(
        dir.path().join("other/src/thing.rs"),
        "pub fn remote_only() {}\n",
    )?;
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
    let store = indexer.graph_store();
    let prefix = format!("{}/", dir.path().to_string_lossy());
    let targets = || -> Result<Vec<String>> {
        Ok(rows(
            &store,
            "SELECT target_qualified FROM edges WHERE kind = 'CALLS'",
        )?
        .into_iter()
        .map(|row| row.replace(&prefix, ""))
        .collect())
    };
    assert_eq!(targets()?, ["other::thing::remote_only"]);
    // No source file changed: only the manifest now declares the path dependency.
    std::fs::write(
        &manifest,
        "[package]\nname = \"app\"\n\n[dependencies]\nother = { path = \"other\" }\n",
    )?;
    let edited = indexer.reconcile_project(dir.path(), false).await?;
    assert_eq!(edited.cached_files, cold.files, "sources were not reused");
    assert_eq!(targets()?, ["other/src/thing.rs::remote_only"]);
    Ok(())
}

#[tokio::test]
async fn community_ids_stable_across_finalize() -> Result<()> {
    let module = |prefix: &str| {
        format!(
            "def {p}_one():\n    {p}_two()\n    {p}_three()\n\ndef {p}_two():\n    {p}_three()\n\n\
             def {p}_three():\n    pass\n",
            p = prefix
        )
    };
    let (a, b) = (module("alpha"), module("beta"));
    let Some((dir, indexer)) = index_files(
        concat!(module_path!(), "::community_ids_stable_across_finalize"),
        &[("alpha.py", a.as_str()), ("beta.py", b.as_str())],
    )
    .await?
    else {
        return Ok(());
    };
    let sql = "SELECT qualified_name, CAST(community_id AS TEXT) FROM nodes \
               WHERE kind IN ('Function', 'Class', 'Test') ORDER BY qualified_name";
    let first = rows(&indexer.graph_store(), sql)?;
    assert!(!first.is_empty());
    assert!(
        first.iter().all(|row| !row.ends_with('|')),
        "every function belongs to a community: {first:?}"
    );
    let store = indexer.graph_store();
    let communities = "SELECT CAST(count(*) AS TEXT) FROM communities";
    let community_count = rows(&store, communities)?;
    // Re-run the post-process twice; the assignment must not move.
    for _ in 0..2 {
        refinalize(&store).await?;
        assert_eq!(rows(&store, sql)?, first);
        assert_eq!(rows(&store, communities)?, community_count);
    }
    let _ = dir;
    Ok(())
}
