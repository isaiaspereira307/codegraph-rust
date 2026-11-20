// ABOUTME: Verifies directory traversal, analyzer failures and embedding batch overrides.
// ABOUTME: Exercises the shipped CLI in isolated embedded stores with offline mock providers.

use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Stdio},
};

fn offline_index_command(root: &Path, config: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_codegraph"));
    command
        .current_dir(root)
        .env("CODEGRAPH_CONFIG_PATH", config)
        .env(
            "CODEGRAPH_SURREALDB_URL",
            format!("surrealkv://{}", root.join(".codegraph/db").display()),
        )
        .env("CODEGRAPH_SURREALDB_USERNAME", "")
        .env("CODEGRAPH_SURREALDB_PASSWORD", "")
        .env("CODEGRAPH_EMBEDDING_POLICY", "off")
        .env("CODEGRAPH_SEMANTIC_RESOLUTION", "off")
        .env("CODEGRAPH_VECTOR_INDEX_MODE", "off")
        .env("CODEGRAPH_PROJECT_ID", "cli-traversal")
        .env("CODEGRAPH_USE_GRAPH_SCHEMA", "false")
        .env("CODEGRAPH_SCHEMA", "v2")
        .env("CODEGRAPH_DEBUG", "0")
        .env("CODEGRAPH_NO_PROGRESS", "1")
        .stdin(Stdio::null());
    command
}

fn index_files(traversal: Option<&str>, root_source: bool) -> usize {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let config = root.join("providers.toml");
    codegraph_core::config_manager::ConfigManager::create_default_config(&config).unwrap();
    for (name, contents) in [
        ("src/main.rs", "fn main() {}\n"),
        ("crates/example/src/lib.rs", "pub fn nested() {}\n"),
        ("target/generated.rs", "pub fn ignored_build_output() {}\n"),
        ("src/other.py", "def ignored_language(): pass\n"),
    ] {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
    if root_source {
        std::fs::write(root.join("root.rs"), "pub fn at_root() {}\n").unwrap();
    }
    let stats = root.join("stats.json");
    let mut command = offline_index_command(root, &config);
    command.args([
        "index",
        "--languages",
        "Rust",
        "--verbose",
        "--index-tier",
        "balanced",
        "--stats-json",
        "stats.json",
    ]);
    if let Some(traversal) = traversal {
        command.arg(traversal);
    }
    let output = command
        .arg(".")
        .env("CODEGRAPH_ANALYZERS", "0")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stats: Value = serde_json::from_slice(&std::fs::read(stats).unwrap()).unwrap();
    assert_eq!(stats["complete"], true);
    assert_eq!(stats["inference_texts"], 0);
    assert!(stats["nodes"].as_u64().unwrap() > 0);
    stats["files"].as_u64().unwrap().try_into().unwrap()
}

#[test]
fn balanced_index_finds_nested_rust_without_a_recursive_flag() {
    // Reproduce the reported command in a project containing no root-level Rust files.
    assert_eq!(index_files(None, false), 2);
    for flag in ["-r", "--recursive"] {
        assert_eq!(index_files(Some(flag), false), 2);
    }
}

#[test]
fn root_only_index_requires_explicit_no_recursive_flag() {
    assert_eq!(index_files(None, true), 3);
    assert_eq!(index_files(Some("--no-recursive"), true), 1);
}

#[test]
fn rerank_dotenv_and_toml_settings_resolve_without_breaking_offline_indexing() {
    for (toml, dotenv, provider, model) in [
        (
            "",
            "JINA_ENABLE_RERANKING=true\n",
            "jina",
            "jina-reranker-v3",
        ),
        (
            "[rerank]\nprovider = 'jina'\n",
            "",
            "jina",
            "jina-reranker-v3",
        ),
        (
            "[rerank]\nprovider = 'jina'\n[rerank.jina]\nmodel = 'custom-reranker'\n",
            "",
            "jina",
            "custom-reranker",
        ),
        (
            "[rerank]\nprovider = 'ollama'\n",
            "CODEGRAPH_OLLAMA_RERANK_MODEL=env-reranker\n",
            "ollama",
            "env-reranker",
        ),
        (
            "",
            "JINA_ENABLE_RERANKING=true\nCODEGRAPH_RERANK_PROVIDER=ollama\nCODEGRAPH_OLLAMA_RERANK_MODEL=env-reranker\n",
            "ollama",
            "env-reranker",
        ),
        (
            "[rerank]\nprovider = 'jina'\n",
            "CODEGRAPH_ENABLE_RERANKING=false\n",
            "none",
            "",
        ),
    ] {
        let project = tempfile::tempdir().unwrap();
        let root = project.path();
        let config = root.join("codegraph.toml");
        std::fs::write(&config, toml).unwrap();
        std::fs::write(root.join(".env"), dotenv).unwrap();
        std::fs::write(root.join("lib.rs"), "pub fn fixture() {}\n").unwrap();
        let command = || {
            let mut command = offline_index_command(root, &config);
            for variable in [
                "CODEGRAPH_RERANK_PROVIDER",
                "CODEGRAPH_ENABLE_RERANKING",
                "JINA_ENABLE_RERANKING",
                "JINA_RERANKING_MODEL",
                "JINA_API_KEY",
                "JINA_API_BASE",
                "JINA_RERANKING_TOP_N",
                "CODEGRAPH_RERANKING_CANDIDATES",
                "CODEGRAPH_OLLAMA_RERANK_MODEL",
                "OLLAMA_RERANK_MODEL",
                "CODEGRAPH_OLLAMA_URL",
                "OLLAMA_URL",
            ] {
                command.env_remove(variable);
            }
            command
        };
        let output = command()
            .args(["config", "show", "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let resolved: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(resolved["rerank"]["provider"], provider);
        if provider != "none" {
            assert_eq!(resolved["rerank"][format!("{provider}_model")], model);
        }
        // Indexing needs no reranker API call or key: reranking runs at query time.
        let output = command()
            .args([
                "index",
                "--languages",
                "Rust",
                "--index-tier",
                "fast",
                "--stats-json",
                "stats.json",
                ".",
            ])
            .env("CODEGRAPH_ANALYZERS", "0")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stats: Value =
            serde_json::from_slice(&std::fs::read(root.join("stats.json")).unwrap()).unwrap();
        assert_eq!(stats["complete"], true);
        assert_eq!(stats["files"], 1);
        assert_eq!(stats["inference_texts"], 0);
    }
}

#[cfg(unix)]
#[test]
fn broken_rustup_shim_is_reported_before_parsing_even_for_another_project() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let invocation_root = directory.path();
    let root = invocation_root.join("project");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("project.marker"), "").unwrap();
    let config = root.join("providers.toml");
    codegraph_core::config_manager::ConfigManager::create_default_config(&config).unwrap();
    std::fs::create_dir(invocation_root.join("bin")).unwrap();
    let shim = invocation_root.join("bin/rust-analyzer");
    std::fs::write(&shim, "#!/bin/sh\n[ -f project.marker ] || { printf 'wrong project directory' >&2; exit 2; }\nprintf \"Unknown binary 'rust-analyzer' in official toolchain 'stable'\" >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = offline_index_command(&root, &config)
        .current_dir(invocation_root)
        .args([
            "index",
            "--languages",
            "Rust",
            "--verbose",
            "--index-tier",
            "balanced",
            "--stats-json",
            "stats.json",
            "project",
        ])
        .env("PATH", "bin")
        .env("CODEGRAPH_ANALYZERS", "1")
        .env("CODEGRAPH_ANALYZERS_REQUIRE_TOOLS", "1")
        .env_remove("CODEGRAPH_SCIP_INDEX")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let logs = format!("{}\n{}", String::from_utf8_lossy(&output.stdout), stderr);
    assert!(stderr.contains("Unknown binary 'rust-analyzer'"), "{logs}");
    assert!(
        stderr.contains("rustup component add rust-analyzer"),
        "{logs}"
    );
    assert!(!logs.contains("wrong project directory"), "{logs}");
    assert!(!logs.contains("UNIFIED AST EXTRACTION COMPLETE"), "{logs}");
    assert!(
        !logs.contains("Language-server analysis starting"),
        "{logs}"
    );
    assert!(!invocation_root.join("stats.json").exists());
}
