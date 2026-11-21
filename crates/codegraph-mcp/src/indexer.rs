// ABOUTME: Drives the indexing pipeline for the Codegraph MCP CLI.
// ABOUTME: Coordinates parsing, analyzers and persistence into the SQLite graph store.
#![allow(dead_code, unused_variables, unused_imports)]

use crate::analyzers::{AnalyzerSettings, find_tool_on_path, required_tools_for_languages};
use crate::estimation::parse_files_with_unified_extraction as shared_unified_parse;
use anyhow::{Context, Result, anyhow};
use codegraph_core::{CodeNode, EdgeRelationship, EdgeType, NodeId, NodeType};
use codegraph_graph::sqlite_store::{SqliteGraphStore, WriteLock};
use codegraph_graph::{FileMetadataRecord, ProjectMetadataRecord, edge::CodeEdge};
use codegraph_parser::TreeSitterParser;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use num_cpus;
use rayon::prelude::*;
use regex::Regex;
use rustc_demangle::try_demangle;
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::HashSet;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use symbolic_demangle::demangle;
use syn::{Path as SynPath, PathArguments, parse_str as parse_syn_path};
use tokio::fs as tokio_fs;
use tokio::sync::{Mutex as TokioMutex, mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};
use url::Url;
use walkdir::WalkDir;

use std::sync::OnceLock;
use std::sync::{Arc, Mutex};

use std::collections::HashMap;

/// The graph shape and resolver this build writes. It feeds the project input fingerprint
/// and the ready marker, so a graph written by a build with another shape is rebuilt even
/// when no source changed. Bump it whenever node kinds, edge kinds or resolution change.
pub const GRAPH_SHAPE_VERSION: &str = "crg-shape-6";

#[derive(Debug, Clone, PartialEq)]
pub enum FileChangeType {
    Added,
    Modified,
    Deleted,
    Unchanged,
}

#[derive(Debug, Clone)]
pub struct FileChange {
    pub file_path: String,
    pub change_type: FileChangeType,
    pub current_hash: Option<String>,
    pub previous_hash: Option<String>,
}

static WATCH_TEST_NOTIFIER: OnceLock<Mutex<Option<tokio::sync::mpsc::UnboundedSender<PathBuf>>>> =
    OnceLock::new();

pub fn set_watch_test_notifier(sender: tokio::sync::mpsc::UnboundedSender<PathBuf>) {
    let mut guard = WATCH_TEST_NOTIFIER
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap();
    *guard = Some(sender);
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct IndexerConfig {
    pub languages: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub include_patterns: Vec<String>,
    pub recursive: bool,
    pub force_reindex: bool,
    pub watch: bool,
    pub workers: usize,
    /// Rows per durable storage write batch.
    pub batch_size: usize,
    pub indexing_tier: codegraph_core::config_manager::IndexingTier,
    /// Root directory of the project being indexed (where .codegraph/ will be created)
    /// Defaults to current directory if not specified
    pub project_root: PathBuf,
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            languages: vec![],
            exclude_patterns: vec![],
            include_patterns: vec![],
            recursive: true,
            force_reindex: false,
            watch: false,
            workers: 4,
            batch_size: 100,
            indexing_tier: codegraph_core::config_manager::IndexingTier::default(),
            project_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}

impl From<&IndexerConfig> for codegraph_parser::file_collect::FileCollectionConfig {
    fn from(config: &IndexerConfig) -> Self {
        codegraph_parser::file_collect::FileCollectionConfig {
            recursive: config.recursive,
            languages: config.languages.clone(),
            include_patterns: config.include_patterns.clone(),
            exclude_patterns: config.exclude_patterns.clone(),
        }
    }
}

pub(crate) fn filter_edges_for_tier(
    tier: codegraph_core::config_manager::IndexingTier,
    edges: &mut Vec<EdgeRelationship>,
) -> usize {
    let before = edges.len();
    match tier {
        codegraph_core::config_manager::IndexingTier::Full => {}
        codegraph_core::config_manager::IndexingTier::Balanced => {
            edges.retain(|edge| !matches!(edge.edge_type, EdgeType::References));
        }
        codegraph_core::config_manager::IndexingTier::Fast => {
            edges.retain(|edge| !matches!(edge.edge_type, EdgeType::Uses | EdgeType::References));
        }
    }
    before.saturating_sub(edges.len())
}

pub(crate) fn extraction_policy_for_tier(
    tier: codegraph_core::config_manager::IndexingTier,
) -> codegraph_parser::languages::ExtractionPolicy {
    use codegraph_core::config_manager::IndexingTier;
    codegraph_parser::languages::ExtractionPolicy {
        uses: tier != IndexingTier::Fast,
        references: tier == IndexingTier::Full,
    }
}

pub struct ProjectIndexer {
    config: IndexerConfig,
    global_config: codegraph_core::config_manager::CodeGraphConfig,
    progress: MultiProgress,
    parser: TreeSitterParser,
    store: SqliteGraphStore,
    writer: Option<GraphWriterHandle>,
    project_id: String,
    organization_id: Option<String>,
    repository_url: Option<String>,
    domain: Option<String>,
    project_root: PathBuf,
    reconcile_lock: TokioMutex<()>,
    lsp_pool: crate::analyzers::lsp::LspPool,
    policies: crate::policy::InferencePolicies,
    startup_ms: u64,
    lsp_support: TokioMutex<Option<String>>,
}

use crate::writer::GraphWriterHandle;

impl ProjectIndexer {
    async fn validate_analyzer_tools(
        languages: &[codegraph_core::Language],
        settings: AnalyzerSettings,
        path_env: &str,
        project_root: &Path,
    ) -> Result<()> {
        if !settings.lsp_enabled() {
            return Ok(());
        }
        if !settings.require_tools {
            return Ok(());
        }

        let required = required_tools_for_languages(languages);
        let mut problems: Vec<String> = Vec::new();

        for tool in required {
            if find_tool_on_path(tool.name, path_env).is_none() {
                problems.push(format!("Missing {} (for {:?})", tool.name, tool.language));
            } else if tool.name == "rust-analyzer" {
                let mut failures = Vec::new();
                for command in crate::analyzers::find_tool_candidates_on_path(tool.name, path_env) {
                    match crate::analyzers::lsp::probe_rust_analyzer(&command, project_root).await {
                        Ok(()) => {
                            failures.clear();
                            break;
                        }
                        Err(error) => failures.push(error.to_string()),
                    }
                }
                problems.extend(failures);
            }
        }

        if problems.is_empty() {
            return Ok(());
        }

        Err(anyhow!(
            "Required analyzer tools are unavailable: {}. Install the tools or choose a tier without LSP (e.g. --index-tier fast or CODEGRAPH_INDEX_TIER=fast).",
            problems.join("\n")
        ))
    }

    pub fn new(
        mut config: IndexerConfig,
        global_config: &codegraph_core::config_manager::CodeGraphConfig,
        multi_progress: MultiProgress,
    ) -> futures::future::BoxFuture<'_, Result<Self>> {
        Box::pin(async move {
            let startup = std::time::Instant::now();
            let project_id = std::env::var("CODEGRAPH_PROJECT_ID")
                .unwrap_or_else(|_| config.project_root.display().to_string());
            let policies = crate::policy::InferencePolicies::from_env()?;
            // Cap Rayon threads to a sensible default: leave one core free and obey user overrides
            let available = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4);
            let env_workers = std::env::var("CODEGRAPH_WORKERS")
                .ok()
                .and_then(|v| v.parse::<usize>().ok());
            let requested = std::env::var("RAYON_NUM_THREADS")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|threads| *threads > 0)
                .or(env_workers)
                .unwrap_or(config.workers);

            config.batch_size = config.batch_size.max(1);

            config.workers = requested.max(1).min(available.max(1));
            let parser = TreeSitterParser::new()
                .with_concurrency(config.workers)
                .with_project_root(&config.project_root)
                .with_extraction_policy(extraction_policy_for_tier(config.indexing_tier));
            let project_root = config.project_root.clone();
            let store = Self::open_store(&project_root).await?;
            let writer = GraphWriterHandle::new(store.clone());
            let project_id = std::env::var("CODEGRAPH_PROJECT_ID")
                .unwrap_or_else(|_| project_root.display().to_string());
            let organization_id = std::env::var("CODEGRAPH_ORGANIZATION_ID").ok();
            let repository_url = std::env::var("CODEGRAPH_REPOSITORY_URL").ok();
            let domain = std::env::var("CODEGRAPH_DOMAIN").ok();
            Ok(Self {
                config,
                global_config: global_config.clone(),
                progress: multi_progress,
                parser,
                store,
                writer: Some(writer),
                project_id,
                organization_id,
                repository_url,
                domain,
                project_root,
                reconcile_lock: TokioMutex::new(()),
                lsp_pool: Default::default(),
                policies,
                startup_ms: startup.elapsed().as_millis() as u64,
                lsp_support: TokioMutex::new(None),
            })
        })
    }

    pub fn set_indexing_tier(&mut self, tier: codegraph_core::config_manager::IndexingTier) {
        self.config.indexing_tier = tier;
        self.parser = TreeSitterParser::new()
            .with_concurrency(self.config.workers)
            .with_project_root(&self.project_root)
            .with_extraction_policy(extraction_policy_for_tier(tier));
    }
    pub async fn index_project(&self, path: impl AsRef<Path>) -> Result<IndexStats> {
        self.reconcile_project(path.as_ref(), self.config.force_reindex)
            .await
    }

    pub fn reconcile_project<'a>(
        &'a self,
        path: &'a Path,
        force: bool,
    ) -> futures::future::BoxFuture<'a, Result<IndexStats>> {
        Box::pin(async move {
            let _run = self.reconcile_lock.lock().await;
            // Shared with code-review-graph: one writer per database for the whole run.
            let _write_lock = self.acquire_write_lock().await?;
            let start = std::time::Instant::now();
            let cache_root = self.project_root.join(".codegraph/index-cache");
            let cache_budget = std::env::var("CODEGRAPH_INDEX_CACHE_BYTES")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(4 * 1024 * 1024 * 1024);
            if let Err(error) = tokio::task::spawn_blocking(move || {
                codegraph_core::artifact_cache::ArtifactCache::prune_tree(
                    &cache_root,
                    cache_budget,
                    &["pending-inference-v1"],
                )
            })
            .await?
            {
                warn!("Index artifact eviction failed: {error}");
            }
            let mut phase_ms = std::collections::BTreeMap::new();
            phase_ms.insert(
                "cache_eviction".to_owned(),
                start.elapsed().as_millis() as u64,
            );
            let writer_before = self
                .writer
                .as_ref()
                .map(|writer| writer.metrics())
                .unwrap_or_default();
            info!("Starting project indexing: {:?}", path);
            info!(
                "🗄️ Graph store: {}",
                codegraph_graph::sqlite_store::db_path(&self.project_root).display()
            );

            let capture_start = std::time::Instant::now();
            let file_config: codegraph_parser::file_collect::FileCollectionConfig =
                (&self.config).into();
            let all_files = codegraph_parser::file_collect::collect_source_files_with_config(
                path,
                &file_config,
            )?;
            let source_snapshots = codegraph_parser::SourceSnapshots::capture(
                &all_files,
                crate::estimation::source_memory_budget(),
                self.parser.concurrency(),
            )
            .await?;

            phase_ms.insert(
                "source_capture".to_owned(),
                capture_start.elapsed().as_millis() as u64,
            );
            let inputs_start = std::time::Instant::now();
            use codegraph_core::artifact_cache::{ArtifactCache, fingerprint};
            let state_cache = ArtifactCache::new(
                self.project_root.join(".codegraph/index-cache"),
                "project-catalog-v1",
            );
            let root = self.project_root.clone();
            let support = tokio::task::spawn_blocking(move || {
                crate::reconciliation::support_fingerprints(&root)
            })
            .await??;
            let policy: std::collections::BTreeMap<String, String> = [
                "CODEGRAPH_SEMANTIC_CANDIDATES",
                "CODEGRAPH_LOCAL_DTYPE",
                "CODEGRAPH_ONNX_MODEL_FILE",
                "CODEGRAPH_ONNX_EP",
                "CODEGRAPH_COREML_LOW_PRECISION",
                "CODEGRAPH_MODEL_REVISION",
                "CODEGRAPH_TOKENIZER_PATH",
                "CODEGRAPH_CHUNK_SPLITTER",
                "CODEGRAPH_CHUNK_MAX_TOKENS",
                "CODEGRAPH_MAX_CHUNK_TOKENS",
                "CODEGRAPH_MODEL_MAX_TOKENS",
                "CODEGRAPH_OLLAMA_NUM_CTX",
                "CODEGRAPH_TOKENIZER_REPO",
                "CODEGRAPH_TOKENIZER_REVISION",
                "CODEGRAPH_EMBEDDING_DOCUMENT_PREFIX",
                "CODEGRAPH_EMBEDDING_QUERY_PREFIX",
                "CODEGRAPH_CHUNK_OVERLAP_TOKENS",
                "CODEGRAPH_CHUNK_SMART_SPLIT",
                "CODEGRAPH_EMBEDDING_SKIP_CHUNKING",
                "CODEGRAPH_ANALYZERS",
                "CODEGRAPH_ANALYZERS_REQUIRE_TOOLS",
                "CODEGRAPH_PROJECT_ID",
                "CODEGRAPH_ORGANIZATION_ID",
                "CODEGRAPH_REPOSITORY_URL",
                "CODEGRAPH_DOMAIN",
                "RUSTFLAGS",
                "CARGO_BUILD_TARGET",
                "RUSTUP_TOOLCHAIN",
                "CODEGRAPH_VECTOR_INDEX_MODE",
            ]
            .into_iter()
            .map(|key| (key.to_owned(), std::env::var(key).unwrap_or_default()))
            .collect();
            let mut external_support =
                crate::analyzers::build_context::external_input_fingerprints(&self.project_root)?;
            let mutable_model_epoch = self.policies.model_epoch(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs(),
                std::env::var("CODEGRAPH_EMBEDDING_CACHE_TTL_SECONDS")
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(3600),
                std::env::var_os("CODEGRAPH_MODEL_REVISION").is_some(),
            );
            let tokenizer_fingerprint = std::env::var_os("CODEGRAPH_TOKENIZER_PATH")
                .map(PathBuf::from)
                .map(|path| crate::reconciliation::file_fingerprint(&path))
                .transpose()?;
            let scip_fingerprint =
                if let Some(path) = std::env::var_os("CODEGRAPH_SCIP_INDEX").map(PathBuf::from) {
                    Some(fingerprint(&(
                        crate::reconciliation::file_fingerprint(&path)?,
                        if path.with_extension("sources.json").exists() {
                            Some(crate::reconciliation::file_fingerprint(
                                &path.with_extension("sources.json"),
                            )?)
                        } else {
                            None
                        },
                        std::env::var("CODEGRAPH_SCIP_TRUST_SOURCE").unwrap_or_default(),
                    ))?)
                } else {
                    None
                };
            let make_fingerprint =
                |external_support: &std::collections::BTreeMap<String, String>| {
                    fingerprint(&(
                        "project-input-v7",
                        GRAPH_SHAPE_VERSION,
                        &self.project_id,
                        source_snapshots
                            .iter()
                            .map(|source| (&source.path, &source.content_hash))
                            .collect::<std::collections::BTreeMap<_, _>>(),
                        &support,
                        &external_support,
                        &tokenizer_fingerprint,
                        mutable_model_epoch,
                        &scip_fingerprint,
                        self.config.indexing_tier,
                        &policy,
                        self.policies.enabled_identity(),
                        false,
                        false,
                    ))
                };
            let mut input_fingerprint = make_fingerprint(&external_support)?;
            let marker_record = self.store.project_marker(&self.project_id).await?;
            let marker = marker_record
                .as_ref()
                .and_then(|record| record.metadata.get("input_fingerprint"))
                .and_then(|v| v.as_str());
            // A graph written with another shape is never reused, even record by record.
            let same_shape = marker_record
                .as_ref()
                .and_then(|record| record.metadata.get("graph_shape"))
                .and_then(|v| v.as_str())
                == Some(GRAPH_SHAPE_VERSION);
            let previous = state_cache
                .get::<crate::reconciliation::Catalog>(&self.project_id)
                .filter(|state| !force && same_shape && marker == Some(state.fingerprint.as_str()));
            if let Some(state) = &previous
                && state.fingerprint == input_fingerprint
                && self
                    .policies
                    .satisfied_by(&state.stats.embedding_status, &state.stats.semantic_status)
            {
                source_snapshots.validate_current().await?;
                let mut stats = state.stats.clone();
                stats.skipped = all_files.len();
                stats.cached_files = all_files.len();
                stats.index_ms = start.elapsed().as_millis() as u64;
                stats.startup_ms = self.startup_ms;
                stats.inference_texts = 0;
                stats.inference_tokens = 0;
                stats.embedding_cache_hits = 0;
                stats.source_read_operations = source_snapshots.read_operations;
                stats.source_read_bytes = source_snapshots.read_bytes;
                stats.writer_jobs_acked = 0;
                stats.writer_rows_acked = 0;
                stats.writer_payload_bytes_acked = 0;
                phase_ms.insert(
                    "input_validation".into(),
                    inputs_start.elapsed().as_millis() as u64,
                );
                stats.phase_ms = phase_ms;
                return Ok(stats);
            }
            phase_ms.insert(
                "input_validation".into(),
                inputs_start.elapsed().as_millis() as u64,
            );
            // Every run has the full symbol universe. Unchanged source uses cached ASTs.
            let files_to_index = all_files.clone();
            let mut next_catalog = crate::reconciliation::Catalog {
                fingerprint: input_fingerprint.clone(),
                ..Default::default()
            };

            let support_fingerprint = fingerprint(&(&support, &external_support))?;
            {
                let mut previous_support = self.lsp_support.lock().await;
                if previous_support.as_ref() != Some(&support_fingerprint) {
                    self.lsp_pool.clear().await;
                    *previous_support = Some(support_fingerprint);
                }
            }
            let analyzer_settings = AnalyzerSettings::from_env(self.config.indexing_tier);
            let path_env = std::env::var("PATH").unwrap_or_default();
            let mut analyzer_languages: HashSet<codegraph_core::Language> = HashSet::new();
            let needs_language_scan = analyzer_settings.lsp_enabled()
                || analyzer_settings.build_context
                || analyzer_settings.dataflow;
            if needs_language_scan {
                let registry = codegraph_parser::LanguageRegistry::new();
                for (p, _) in &files_to_index {
                    if let Some(lang) = registry.detect_language(&p.to_string_lossy()) {
                        analyzer_languages.insert(lang);
                    }
                }
            }
            let analyzer_languages: Vec<codegraph_core::Language> =
                analyzer_languages.into_iter().collect();
            let scip_path = std::env::var_os("CODEGRAPH_SCIP_INDEX").map(PathBuf::from);
            if scip_path.is_none() {
                Self::validate_analyzer_tools(
                    &analyzer_languages,
                    analyzer_settings,
                    &path_env,
                    &self.project_root,
                )
                .await?;
            }

            let lsp_mode_label = match analyzer_settings.lsp_mode {
                crate::analyzers::LspMode::Off => "off",
                crate::analyzers::LspMode::SymbolsOnly => "symbols",
                crate::analyzers::LspMode::SymbolsAndDefinitions => "symbols+definitions",
            };
            info!(
                "🧭 Indexing tier: {:?} | analyzers: build_context={} lsp={} enrichment={} module_linking={} dataflow={} docs_contracts={} architecture={}",
                self.config.indexing_tier,
                analyzer_settings.build_context,
                lsp_mode_label,
                analyzer_settings.enrichment,
                analyzer_settings.module_linking,
                analyzer_settings.dataflow,
                analyzer_settings.docs_contracts,
                analyzer_settings.architecture
            );

            let build_context_task = if analyzer_settings.build_context
                && analyzer_languages.contains(&codegraph_core::Language::Rust)
            {
                let root = self.project_root.clone();
                let project = self.project_id.clone();
                Some(tokio::task::spawn_blocking(move || {
                    crate::analyzers::build_context::analyze_cargo_workspace(&root, &project)
                }))
            } else {
                None
            };

            // STAGE 1: File Collection & Parsing
            let files = files_to_index;
            let total_files = files.len();

            // Single progress bar for unified AST + fast_ml extraction
            let ast_pb = self.create_progress_bar(
                total_files as u64,
                "🌳 Parsing & extracting (TreeSitter + FastML)",
            );

            // REVOLUTIONARY: Use unified extraction for nodes + edges in single pass (FASTEST approach)
            // Clone files for parsing (we need them again for metadata persistence)
            let ast_cache = codegraph_core::artifact_cache::ArtifactCache::new(
                self.project_root.join(".codegraph/index-cache"),
                &crate::estimation::ast_cache_namespace(),
            );
            let parse_start = std::time::Instant::now();
            let (mut nodes, mut edges, pstats) = crate::estimation::parse_snapshots_with_cache(
                &self.parser,
                files.clone(),
                total_files as u64,
                &source_snapshots,
                Some(&ast_cache),
                Some(path),
            )
            .await?;
            phase_ms.insert("ast_parse".into(), parse_start.elapsed().as_millis() as u64);
            let analyzers_start = std::time::Instant::now();
            let cache_budget = std::env::var("CODEGRAPH_AST_CACHE_BYTES")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(2 * 1024 * 1024 * 1024);
            let cache_for_prune = ast_cache.clone();
            if let Err(error) =
                tokio::task::spawn_blocking(move || cache_for_prune.prune(cache_budget)).await?
            {
                warn!("AST cache eviction failed: {error}");
            }
            if pstats.failed_files > 0 {
                return Err(anyhow!(
                    "{} source files failed parsing; file metadata will not be marked current",
                    pstats.failed_files
                ));
            }

            let build_context_out = match build_context_task {
                Some(task) => task.await??,
                None => Default::default(),
            };
            external_support =
                crate::analyzers::build_context::external_input_fingerprints(&self.project_root)?;
            input_fingerprint = make_fingerprint(&external_support)?;
            next_catalog.fingerprint = input_fingerprint.clone();
            let build_context_nodes = build_context_out.nodes.len();
            let build_context_edges = build_context_out.edges.len();
            if analyzer_settings.build_context
                && (!build_context_out.nodes.is_empty() || !build_context_out.edges.is_empty())
            {
                nodes.extend(build_context_out.nodes);
                edges.extend(build_context_out.edges);
            }

            let removed_edges = filter_edges_for_tier(self.config.indexing_tier, &mut edges);
            if removed_edges > 0 {
                info!(
                    "🧹 Tier edge filter removed {} edges (tier: {:?})",
                    removed_edges, self.config.indexing_tier
                );
            }

            self.finish_bar(
                ast_pb,
                format!(
                    "🌳 Parsed {} files | nodes: {} | edges: {}",
                    pstats.parsed_files,
                    nodes.len(),
                    edges.len()
                ),
            )?;

            // Generate deterministic IDs and build old_id -> new_id mapping to update edge references
            let mut id_mapping: std::collections::HashMap<NodeId, NodeId> =
                std::collections::HashMap::with_capacity(nodes.len());
            let mut identities: std::collections::HashMap<
                NodeId,
                std::collections::BTreeSet<String>,
            > = std::collections::HashMap::new();
            for node in &nodes {
                let id = codegraph_core::generate_node_id(
                    &self.project_id,
                    &node.location.file_path,
                    node.name.as_str(),
                    &codegraph_parser::languages::crg_shape::id_node_type(node),
                    node.location.line,
                );
                identities.entry(id).or_default().insert(fingerprint(&(
                    &node.location,
                    &node.span,
                    codegraph_parser::languages::crg_shape::id_qualified_name(node),
                ))?);
            }
            for node in nodes.iter_mut() {
                let old_id = node.id;
                // The pre-shape kind, so test functions keep their ids.
                node.id = codegraph_core::generate_node_id(
                    &self.project_id,
                    &node.location.file_path,
                    node.name.as_str(),
                    &codegraph_parser::languages::crg_shape::id_node_type(node),
                    node.location.line,
                );
                if identities
                    .get(&node.id)
                    .is_some_and(|identities| identities.len() > 1)
                {
                    let identity = format!(
                        "{}#{}:{}",
                        node.name,
                        node.location.column,
                        codegraph_parser::languages::crg_shape::id_qualified_name(node)
                            .map(String::as_str)
                            .unwrap_or("")
                    );
                    node.id = codegraph_core::generate_node_id(
                        &self.project_id,
                        &node.location.file_path,
                        &identity,
                        &codegraph_parser::languages::crg_shape::id_node_type(node),
                        node.location.line,
                    );
                }
                id_mapping.insert(old_id, node.id);
                self.annotate_node(node);
            }

            // Update edge.from references to use new deterministic IDs
            for edge in edges.iter_mut() {
                if let Some(new_id) = id_mapping.get(&edge.from) {
                    edge.from = *new_id;
                }
                if let Some(target) = edge
                    .metadata
                    .get("target_node_id")
                    .and_then(|id| id.parse::<NodeId>().ok())
                    .and_then(|id| id_mapping.get(&id))
                {
                    edge.metadata
                        .insert("target_node_id".into(), target.to_string());
                }
                if let Some(span) = &edge.span {
                    edge.metadata.insert(
                        "source_span".to_string(),
                        format!("{}:{}", span.start_byte, span.end_byte),
                    );
                }
            }

            nodes.sort_by_key(|node| node.id);
            nodes.dedup_by_key(|node| node.id);

            let mut lsp_enrichment_stats = crate::analyzers::lsp::LspEnrichmentStats::default();
            if analyzer_settings.lsp_enabled()
                && !analyzer_languages.is_empty()
                && scip_path.is_none()
            {
                let start = std::time::Instant::now();
                info!(
                    "🧠 Language-server analysis starting (mode: {}, languages: {:?})",
                    lsp_mode_label, analyzer_languages
                );

                let project_root = self.project_root.clone();
                let path_env = path_env.clone();

                let mut language_files: std::collections::HashMap<
                    codegraph_core::Language,
                    Vec<PathBuf>,
                > = std::collections::HashMap::new();
                let registry = codegraph_parser::LanguageRegistry::new();
                for (p, _) in &files {
                    if let Some(lang) = registry.detect_language(&p.to_string_lossy()) {
                        language_files.entry(lang).or_default().push(p.clone());
                    }
                }

                let mut grouped: std::collections::BTreeMap<
                    String,
                    (crate::analyzers::LspServerSpec, Vec<PathBuf>),
                > = std::collections::BTreeMap::new();
                for (language, files) in language_files {
                    if let Some(spec) = crate::analyzers::lsp_server_for_language(&language) {
                        grouped
                            .entry(spec.tool_name.to_owned())
                            .or_insert_with(|| (spec.clone(), Vec::new()))
                            .1
                            .extend(files);
                    }
                }
                let mut total_stats = crate::analyzers::lsp::LspEnrichmentStats::default();
                for (_, (spec, mut files)) in grouped {
                    files.sort();
                    files.dedup();
                    let candidates =
                        crate::analyzers::find_tool_candidates_on_path(spec.tool_name, &path_env);
                    if candidates.is_empty() {
                        return Err(anyhow!(
                            "Missing required analyzer tool: {}",
                            spec.tool_name
                        ));
                    }
                    let mut success = false;
                    let mut last_error = None;
                    for tool in candidates {
                        match crate::analyzers::lsp::enrich_async(
                            &self.lsp_pool,
                            Some(&source_snapshots),
                            &tool,
                            spec.args,
                            spec.language_id,
                            spec.name_joiner,
                            analyzer_settings.lsp_definitions_enabled(),
                            &project_root,
                            &files,
                            &mut nodes,
                            &mut edges,
                        )
                        .await
                        {
                            Ok(stats) => {
                                total_stats.nodes_enriched += stats.nodes_enriched;
                                total_stats.edges_resolved += stats.edges_resolved;
                                success = true;
                                break;
                            }
                            Err(error) => {
                                self.lsp_pool.clear().await;
                                last_error = Some(error);
                            }
                        }
                    }
                    if !success {
                        return Err(last_error
                            .unwrap_or_else(|| anyhow!("No LSP server candidate succeeded")));
                    }
                }

                lsp_enrichment_stats = total_stats;
                info!(
                    "🧠 Language-server analysis complete: {} nodes enriched + {} edges resolved in {:.1?}",
                    lsp_enrichment_stats.nodes_enriched,
                    lsp_enrichment_stats.edges_resolved,
                    start.elapsed()
                );
            }

            if let Some(path) = &scip_path
                && analyzer_settings.lsp_enabled()
            {
                lsp_enrichment_stats.edges_resolved = crate::analyzers::scip::import(
                    path,
                    &self.project_root,
                    &source_snapshots,
                    &mut nodes,
                    &mut edges,
                    analyzer_settings.lsp_definitions_enabled(),
                )?;
            }
            let mut enrichment_stats = crate::analyzers::enrichment::EnrichmentStats::default();
            if analyzer_settings.enrichment {
                let start = std::time::Instant::now();
                info!("🧾 Enrichment analysis starting (rustdoc + api surface)");
                let project_root = self.project_root.clone();

                let mut nodes_moved = Vec::new();
                std::mem::swap(&mut nodes_moved, &mut nodes);
                let mut edges_moved = Vec::new();
                std::mem::swap(&mut edges_moved, &mut edges);

                let sources = source_snapshots.clone();
                let (mut nodes_updated, mut edges_updated, enrich_stats) =
                tokio::task::spawn_blocking(move || -> Result<(Vec<CodeNode>, Vec<EdgeRelationship>, crate::analyzers::enrichment::EnrichmentStats)> {
                    let mut nodes = nodes_moved;
                    let mut edges = edges_moved;
                    let stats =
                        crate::analyzers::enrichment::apply_basic_enrichment_with_sources(&project_root, &mut nodes, &mut edges, Some(&sources))?;
                    Ok((nodes, edges, stats))
                })
                .await??;

                std::mem::swap(&mut nodes, &mut nodes_updated);
                std::mem::swap(&mut edges, &mut edges_updated);

                enrichment_stats = enrich_stats;
                info!(
                    "🧾 Enrichment analysis complete: docs={} api_marked={} exports={} reexports={} feature_enables={} lsp_uses={} in {:.1?}",
                    enrichment_stats.docs_attached,
                    enrichment_stats.api_marked,
                    enrichment_stats.export_edges_added,
                    enrichment_stats.reexport_edges_added,
                    enrichment_stats.feature_enables_edges_added,
                    enrichment_stats.uses_edges_derived,
                    start.elapsed()
                );
            }

            let mut module_linker_stats =
                crate::analyzers::module_linker::ModuleLinkerStats::default();
            if analyzer_settings.module_linking {
                let start = std::time::Instant::now();
                info!("🧭 Module linking starting (modules, imports, containment)");
                let project_root = self.project_root.clone();
                let project_id = self.project_id.clone();

                let mut nodes_moved = Vec::new();
                std::mem::swap(&mut nodes_moved, &mut nodes);
                let mut edges_moved = Vec::new();
                std::mem::swap(&mut edges_moved, &mut edges);

                let (mut nodes_updated, mut edges_updated, stats) =
                tokio::task::spawn_blocking(move || -> Result<(Vec<CodeNode>, Vec<EdgeRelationship>, crate::analyzers::module_linker::ModuleLinkerStats)> {
                    let mut nodes = nodes_moved;
                    let mut edges = edges_moved;
                    let stats = crate::analyzers::module_linker::link_modules(
                        &project_root,
                        &project_id,
                        &mut nodes,
                        &mut edges,
                    )?;
                    Ok((nodes, edges, stats))
                })
                .await??;

                std::mem::swap(&mut nodes, &mut nodes_updated);
                std::mem::swap(&mut edges, &mut edges_updated);
                for node in nodes.iter_mut() {
                    self.annotate_node(node);
                }

                module_linker_stats = stats;
                info!(
                    "🧭 Module linking complete: modules={} contains={} imports={} in {:.1?}",
                    module_linker_stats.module_nodes_added,
                    module_linker_stats.contains_edges_added,
                    module_linker_stats.module_import_edges_added,
                    start.elapsed()
                );
            }

            let mut dataflow_stats = crate::analyzers::dataflow::DataflowStats::default();
            if analyzer_settings.dataflow
                && analyzer_languages.contains(&codegraph_core::Language::Rust)
            {
                let start = std::time::Instant::now();
                info!("🌊 Dataflow enrichment starting (local def-use)");
                let project_root = self.project_root.clone();
                let project_id = self.project_id.clone();

                let mut nodes_moved = Vec::new();
                std::mem::swap(&mut nodes_moved, &mut nodes);
                let mut edges_moved = Vec::new();
                std::mem::swap(&mut edges_moved, &mut edges);

                let (mut nodes_updated, mut edges_updated, stats) =
                tokio::task::spawn_blocking(move || -> Result<(Vec<CodeNode>, Vec<EdgeRelationship>, crate::analyzers::dataflow::DataflowStats)> {
                    let mut nodes = nodes_moved;
                    let mut edges = edges_moved;
                    let stats = crate::analyzers::dataflow::enrich_rust_dataflow(
                        &project_root,
                        &project_id,
                        &mut nodes,
                        &mut edges,
                    )?;
                    Ok((nodes, edges, stats))
                })
                .await??;

                std::mem::swap(&mut nodes, &mut nodes_updated);
                std::mem::swap(&mut edges, &mut edges_updated);
                for node in nodes.iter_mut() {
                    self.annotate_node(node);
                }

                dataflow_stats = stats;
                info!(
                    "🌊 Dataflow enrichment complete: vars={} defines={} uses={} flows={} returns={} mutates={} in {:.1?}",
                    dataflow_stats.variable_nodes_added,
                    dataflow_stats.defines_edges_added,
                    dataflow_stats.uses_edges_added,
                    dataflow_stats.flows_to_edges_added,
                    dataflow_stats.returns_edges_added,
                    dataflow_stats.mutates_edges_added,
                    start.elapsed()
                );
            }

            let mut docs_contracts_stats =
                crate::analyzers::docs_contracts::DocsContractsStats::default();
            if analyzer_settings.docs_contracts {
                let start = std::time::Instant::now();
                info!("📚 Docs/contracts linking starting");
                let project_root = self.project_root.clone();
                let project_id = self.project_id.clone();

                let mut nodes_moved = Vec::new();
                std::mem::swap(&mut nodes_moved, &mut nodes);
                let mut edges_moved = Vec::new();
                std::mem::swap(&mut edges_moved, &mut edges);

                let (mut nodes_updated, mut edges_updated, stats) =
                tokio::task::spawn_blocking(move || -> Result<(Vec<CodeNode>, Vec<EdgeRelationship>, crate::analyzers::docs_contracts::DocsContractsStats)> {
                    let mut nodes = nodes_moved;
                    let mut edges = edges_moved;
                    let stats = crate::analyzers::docs_contracts::link_docs_and_contracts(
                        &project_root,
                        &project_id,
                        &mut nodes,
                        &mut edges,
                    )?;
                    Ok((nodes, edges, stats))
                })
                .await??;

                std::mem::swap(&mut nodes, &mut nodes_updated);
                std::mem::swap(&mut edges, &mut edges_updated);
                for node in nodes.iter_mut() {
                    self.annotate_node(node);
                }

                docs_contracts_stats = stats;
                info!(
                    "📚 Docs/contracts linking complete: docs={} documents={} specifies={} in {:.1?}",
                    docs_contracts_stats.document_nodes_added,
                    docs_contracts_stats.document_edges_added,
                    docs_contracts_stats.specification_edges_added,
                    start.elapsed()
                );
            }

            let mut architecture_stats =
                crate::analyzers::architecture::ArchitectureStats::default();
            if analyzer_settings.architecture {
                let start = std::time::Instant::now();
                info!("🏛️  Architecture analysis starting (cycles, boundaries)");

                architecture_stats = crate::analyzers::architecture::analyze_architecture(
                    &self.project_root,
                    &nodes,
                    &mut edges,
                )?;
                info!(
                    "🏛️  Architecture analysis complete: package_cycles={} boundary_violations={} in {:.1?}",
                    architecture_stats.package_cycles_detected,
                    architecture_stats.boundary_violations_added,
                    start.elapsed()
                );
            }

            phase_ms.insert(
                "analyzers".into(),
                analyzers_start.elapsed().as_millis() as u64,
            );
            let chunks_start = std::time::Instant::now();
            // Store counts for final summary (before consumption)
            let total_nodes_extracted = nodes.len();
            let total_edges_extracted = edges.len();

            {
                info!("⚠️ Embeddings feature disabled at compile time; chunking skipped");
                let _chunk_plan: Option<()> = None;
            }

            phase_ms.insert(
                "chunk_plan".into(),
                chunks_start.elapsed().as_millis() as u64,
            );
            let persist_start = std::time::Instant::now();
            let success_rate = if pstats.total_files > 0 {
                (pstats.parsed_files as f64 / pstats.total_files as f64) * 100.0
            } else {
                100.0
            };

            let parse_completion_msg = format!(
                "🌳 Unified fast_ml + AST extraction complete: {}/{} files (✅ {:.1}% success) | 📊 {} nodes + {} edges | ⚡ {:.0} lines/s",
                pstats.parsed_files,
                pstats.total_files,
                success_rate,
                total_nodes_extracted,
                total_edges_extracted,
                pstats.lines_per_second
            );

            // Enhanced parsing statistics
            info!("🌳 TreeSitter AST parsing results:");
            info!(
                "   📊 Semantic nodes extracted: {} (functions, structs, classes, etc.)",
                total_nodes_extracted
            );
            info!(
                "   🔗 Code relationships extracted: {} (calls, imports, dependencies)",
                total_edges_extracted
            );
            info!(
                "   📈 Extraction efficiency: {:.1} nodes/file | {:.1} edges/file",
                total_nodes_extracted as f64 / pstats.parsed_files.max(1) as f64,
                total_edges_extracted as f64 / pstats.parsed_files.max(1) as f64
            );
            info!(
                "   🎯 Sample nodes: {:?}",
                nodes.iter().take(3).map(|n| &n.name).collect::<Vec<_>>()
            );

            if nodes.is_empty() {
                warn!("No nodes generated from parsing! Check parser implementation.");
                warn!(
                    "Parsing stats: {} files, {} lines processed",
                    pstats.parsed_files, pstats.total_lines
                );
            }

            // STAGE 4: Persist nodes.
            // Parsing and analyzer validation must succeed before replacing the last usable
            // graph. A failed parse must never erase the previous index.
            self.flush_writer().await?;
            // Remove the ready marker before the first mutation; interrupted runs cannot skip.
            self.store.delete_project_marker(&self.project_id).await?;
            for node in &nodes {
                next_catalog.nodes.insert(
                    node.id.to_string(),
                    crate::reconciliation::node_digest(node)?,
                );
            }
            let store_nodes_pb = self.create_progress_bar(nodes.len() as u64, "📈 Storing nodes");
            let mut stats = IndexStats {
                files: pstats.parsed_files,
                lines: pstats.total_lines,
                skipped: pstats.cached_files,
                cached_files: pstats.cached_files,
                analyzers_enabled: analyzer_settings.any_enabled(),
                build_context_nodes,
                build_context_edges,
                lsp_nodes_enriched: lsp_enrichment_stats.nodes_enriched,
                lsp_edges_resolved: lsp_enrichment_stats.edges_resolved,
                docs_attached: enrichment_stats.docs_attached,
                export_edges_added: enrichment_stats.export_edges_added,
                reexport_edges_added: enrichment_stats.reexport_edges_added,
                feature_enables_edges_added: enrichment_stats.feature_enables_edges_added,
                uses_edges_derived: enrichment_stats.uses_edges_derived,
                module_nodes_added: module_linker_stats.module_nodes_added,
                module_contains_edges_added: module_linker_stats.contains_edges_added,
                module_import_edges_added: module_linker_stats.module_import_edges_added,
                dataflow_variable_nodes_added: dataflow_stats.variable_nodes_added,
                dataflow_defines_edges_added: dataflow_stats.defines_edges_added,
                dataflow_uses_edges_added: dataflow_stats.uses_edges_added,
                dataflow_flows_to_edges_added: dataflow_stats.flows_to_edges_added,
                dataflow_returns_edges_added: dataflow_stats.returns_edges_added,
                dataflow_mutates_edges_added: dataflow_stats.mutates_edges_added,
                doc_nodes_added: docs_contracts_stats.document_nodes_added,
                document_edges_added: docs_contracts_stats.document_edges_added,
                specification_edges_added: docs_contracts_stats.specification_edges_added,
                package_cycles_detected: architecture_stats.package_cycles_detected,
                boundary_violations_added: architecture_stats.boundary_violations_added,
                ..Default::default()
            };

            for node in nodes.iter() {
                match node.node_type {
                    // Test nodes are functions code-review-graph marks as tests.
                    Some(NodeType::Function) => stats.functions += 1,
                    Some(NodeType::Other(ref kind)) if kind == "Test" => stats.functions += 1,
                    Some(NodeType::Class) => stats.classes += 1,
                    Some(NodeType::Struct) => stats.structs += 1,
                    Some(NodeType::Trait) => stats.traits += 1,
                    _ => {}
                }
            }
            let storage_batch = self.config.batch_size.max(1);
            for chunk in nodes.chunks(storage_batch) {
                let dirty: Vec<_> = chunk
                    .iter()
                    .filter(|node| {
                        previous
                            .as_ref()
                            .and_then(|state| state.nodes.get(&node.id.to_string()))
                            != next_catalog.nodes.get(&node.id.to_string())
                    })
                    .cloned()
                    .collect();
                if !dirty.is_empty() {
                    self.persist_nodes_batch(&dirty).await?;
                }
                store_nodes_pb.inc(chunk.len() as u64);
            }
            self.flush_writer().await?;
            self.finish_bar(store_nodes_pb, "📈 Nodes stored")?;
            self.log_graph_counts("nodes", total_nodes_extracted).await;

            phase_ms.insert(
                "enqueue_nodes".into(),
                persist_start.elapsed().as_millis() as u64,
            );
            // Inference is always off (policy.rs): no embeddings are generated or stored.
            stats.embeddings = 0;

            // Resolve exact/contextual targets once before any semantic inference.
            let resolution_start = std::time::Instant::now();
            // Like code-review-graph after a build: calls on Spring-injected fields go to the
            // bean that answers them. Needs every file's INJECTS/INHERITS, so runs here.
            let spring_resolved = crate::spring::resolve_spring_di_calls(&nodes, &mut edges);
            if spring_resolved > 0 {
                info!("   🌱 Spring DI calls resolved: {spring_resolved}");
            }
            let mut catalog = crate::resolution::SymbolCatalog::new(&nodes);
            catalog.link_imports(&edges);
            let symbol_map = catalog.unique_aliases();
            let mut normalized = HashMap::new();
            let mut targets = Vec::with_capacity(edges.len());
            let mut resolution_provenance = Vec::with_capacity(edges.len());
            let mut exact_count = 0;
            let mut lexical_count = 0;
            let mut lexical_cache = HashMap::new();
            for edge in &edges {
                // Like code-review-graph, a parsed import binds only to the File its text
                // names; otherwise it stays text. It never binds to a symbol. Analyzer
                // imports (module linker `module::` targets) keep their exact resolution.
                if edge.edge_type == EdgeType::Imports && !edge.metadata.contains_key("analyzer") {
                    let target = catalog.import_target(edge);
                    if target.is_some() {
                        exact_count += 1;
                    }
                    targets.push(target);
                    resolution_provenance.push(("import", 1.0f64));
                    continue;
                }
                // Like code-review-graph, a parsed base class or injected type stays the text
                // the source wrote, even when the class it names is indexed.
                let injects = edge.edge_type
                    == EdgeType::Other(codegraph_parser::languages::java::INJECTS.into());
                if (edge.edge_type == EdgeType::Extends || injects)
                    && !edge.metadata.contains_key("analyzer")
                {
                    targets.push(None);
                    resolution_provenance.push(("inherits", 1.0f64));
                    continue;
                }
                let (target, provenance) =
                    Self::resolve_target(&catalog, edge, &mut normalized, &mut lexical_cache);
                if target.is_some() {
                    if provenance.0 == "lexical" {
                        lexical_count += 1;
                    } else {
                        exact_count += 1;
                    }
                }
                targets.push(target);
                resolution_provenance.push(provenance);
            }
            phase_ms.insert(
                "resolution_exact_lexical".into(),
                resolution_start.elapsed().as_millis() as u64,
            );
            let edge_persistence_start = std::time::Instant::now();
            let resolved_count = targets.iter().filter(|target| target.is_some()).count();
            let mut edge_records = Vec::with_capacity(edges.len());
            for ((edge, target), (method, score)) in
                edges.iter().zip(targets).zip(resolution_provenance)
            {
                let mut record = match target {
                    Some(target) => {
                        let mut record = CodeEdge::new(edge.from, target, edge.edge_type.clone());
                        record.metadata = edge.metadata.clone();
                        record
                            .metadata
                            .insert("resolution_method".into(), method.into());
                        record
                            .metadata
                            .insert("resolution_score".into(), format!("{score:.6}"));
                        record
                    }
                    // Like code-review-graph, keep an unresolved CRG-kind target as its text.
                    None if Self::keeps_unresolved_target(edge) => {
                        let text = Self::unbound_target(&catalog, edge);
                        let mut record =
                            CodeEdge::unresolved(edge.from, &text, edge.edge_type.clone());
                        record.metadata.extend(edge.metadata.clone());
                        if text != edge.to {
                            record
                                .metadata
                                .insert("claimed_endpoint".into(), edge.to.clone());
                        }
                        record
                    }
                    None => continue,
                };
                if let Some(span) = &edge.span {
                    record.metadata.insert(
                        "source_span".into(),
                        format!("{}:{}", span.start_byte, span.end_byte),
                    );
                }
                record.set_deterministic_id(&self.project_id);
                edge_records.push(record);
            }
            edge_records.sort_by_key(|edge| edge.id);
            edge_records.dedup_by_key(|edge| edge.id);
            let stored_edges = edge_records.len();
            next_catalog
                .edges
                .extend(edge_records.iter().map(|edge| edge.id.to_string()));
            edge_records.retain(|edge| {
                previous
                    .as_ref()
                    .is_none_or(|state| !state.edges.contains(&edge.id.to_string()))
            });
            self.enqueue_edges(edge_records).await?;
            self.flush_writer().await?;
            phase_ms.insert(
                "resolution_edge_preparation_and_writes".into(),
                edge_persistence_start.elapsed().as_millis() as u64,
            );
            let resolution_rate = if edges.is_empty() {
                100.0
            } else {
                100.0 * resolved_count as f64 / edges.len() as f64
            };
            info!(
                "Resolved {} of {} relationships (exact/contextual={}, lexical={}, semantic={}) in {:?}",
                resolved_count,
                edges.len(),
                exact_count,
                lexical_count,
                resolved_count.saturating_sub(exact_count + lexical_count),
                resolution_start.elapsed()
            );

            phase_ms.insert(
                "resolve_and_enqueue_edges".into(),
                resolution_start.elapsed().as_millis() as u64,
            );
            let finish_start = std::time::Instant::now();
            // ELIMINATED: No separate edge processing phase needed - edges extracted during parsing!
            self.log_graph_counts("edges", stored_edges).await;

            // Task 3.3: Update file metadata for incremental indexing
            info!("💾 Updating file metadata for change tracking");
            let file_paths_only: Vec<PathBuf> = files.iter().map(|(p, _)| p.clone()).collect();
            next_catalog.file_metadata_hashes = self
                .persist_file_metadata(
                    &file_paths_only,
                    &nodes,
                    &edges,
                    &source_snapshots,
                    previous.as_ref().map(|state| &state.file_metadata_hashes),
                )
                .await?;
            self.flush_writer().await?;
            self.verify_file_metadata_count(file_paths_only.len())
                .await?;

            // COMPREHENSIVE INDEXING COMPLETION SUMMARY
            let avg_nodes_per_file = if stats.files > 0 {
                total_nodes_extracted as f64 / stats.files as f64
            } else {
                0.0
            };
            let avg_edges_per_file = if stats.files > 0 {
                total_edges_extracted as f64 / stats.files as f64
            } else {
                0.0
            };

            let total_elapsed = start.elapsed().as_secs_f64();

            info!("🎉 INDEXING COMPLETE");
            info!(
                "📂 Files {} ({} skipped) | Lines {} | Time {:.1}s",
                pstats.parsed_files, stats.skipped, stats.lines, total_elapsed
            );
            info!(
                "🌳 Graph coverage: nodes {} | edges {} | nodes/file {:.1} | edges/file {:.1} | resolved {:.1}%",
                total_nodes_extracted,
                stored_edges,
                avg_nodes_per_file,
                avg_edges_per_file,
                resolution_rate
            );
            info!(
                "⚡ Throughput: {:.1} files/s | {:.1} nodes/s | {:.1} edges/s",
                pstats.parsed_files as f64 / total_elapsed.max(1e-3),
                total_nodes_extracted as f64 / total_elapsed.max(1e-3),
                stored_edges as f64 / total_elapsed.max(1e-3)
            );

            // Populate extended stats for CLI reporting
            stats.nodes = total_nodes_extracted;
            stats.edges = stored_edges;
            stats.resolved_edges = resolved_count;
            stats.unresolved_edges = total_edges_extracted.saturating_sub(resolved_count);
            stats.resolution_rate = resolution_rate;

            self.flush_writer().await?;
            self.store
                .reconcile_catalog(
                    &self.project_id,
                    next_catalog.nodes.keys().cloned().collect(),
                    next_catalog.edges.iter().cloned().collect(),
                    file_paths_only
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect(),
                )
                .await?;
            source_snapshots.validate_current().await?;
            let current_files = codegraph_parser::file_collect::collect_source_files_with_config(
                path,
                &file_config,
            )?;
            if current_files != all_files
                || crate::reconciliation::support_fingerprints(&self.project_root)? != support
                || crate::analyzers::build_context::external_input_fingerprints(&self.project_root)?
                    != external_support
            {
                return Err(anyhow!(
                    "Project inputs changed during indexing; retry reconciliation"
                ));
            }
            // No vector indexes exist without inference.
            stats.vector_index_ready = false;
            stats.index_ms = start.elapsed().as_millis() as u64;
            phase_ms.insert(
                "durable_reconcile_and_indexes".into(),
                finish_start.elapsed().as_millis() as u64,
            );
            stats.phase_ms = phase_ms;
            let writer_after = self
                .writer
                .as_ref()
                .map(|writer| writer.metrics())
                .unwrap_or_default();
            stats.writer_jobs_acked = writer_after.0.saturating_sub(writer_before.0);
            stats.writer_rows_acked = writer_after.1.saturating_sub(writer_before.1);
            stats.writer_payload_bytes_acked = writer_after.2.saturating_sub(writer_before.2);
            stats.source_read_operations = source_snapshots.read_operations;
            stats.source_read_bytes = source_snapshots.read_bytes;
            stats.source_spilled_bytes = source_snapshots.spilled_bytes;
            stats.startup_ms = self.startup_ms;
            stats.graph_complete = true;
            stats.embedding_status = self.policies.embeddings.status().into();
            stats.semantic_status = self.policies.semantic.status().into();
            stats.complete = !self.policies.pending();
            if self.policies.pending() {
                crate::policy::pending_cache(&self.project_root).put(
                    &self.project_id,
                    &crate::policy::DeferredJob {
                        input_fingerprint: input_fingerprint.clone(),
                        policies: self.policies,
                    },
                )?;
            }
            // Leave the database as code-review-graph expects after a build (pending edges
            // resolved, FTS rebuilt, build metadata), then write the ready marker last.
            self.store
                .finalize(if previous.is_some() {
                    "incremental"
                } else {
                    "full"
                })
                .await?;
            self.persist_project_metadata(&stats, stats.nodes, stats.edges, &input_fingerprint)
                .await?;
            self.flush_writer().await?;
            if !self.policies.pending() {
                crate::policy::pending_cache(&self.project_root).remove(&self.project_id)?;
            }
            next_catalog.stats = stats.clone();
            if let Err(error) = state_cache.put(&self.project_id, &next_catalog) {
                warn!("Project catalog cache unavailable; next run will reconcile fully: {error}");
            }
            Ok(stats)
        })
    }

    /// REVOLUTIONARY: Parse files with unified node+edge extraction for maximum speed
    async fn parse_files_with_unified_extraction(
        &self,
        files: Vec<(PathBuf, u64)>,
        total_files: u64,
    ) -> Result<(
        Vec<CodeNode>,
        Vec<codegraph_core::EdgeRelationship>,
        codegraph_parser::ParsingStatistics,
    )> {
        shared_unified_parse(&self.parser, files, total_files).await
    }

    /// Estimate available system memory for informative logging
    fn estimate_system_memory(&self) -> String {
        #[cfg(target_os = "macos")]
        {
            if let Ok(output) = std::process::Command::new("sysctl")
                .args(["-n", "hw.memsize"])
                .output()
                && let Ok(memsize_str) = String::from_utf8(output.stdout)
                && let Ok(memsize) = memsize_str.trim().parse::<u64>()
            {
                let gb = memsize / 1024 / 1024 / 1024;
                return format!("{}GB", gb);
            }
        }

        #[cfg(target_os = "linux")]
        {
            if let Ok(contents) = std::fs::read_to_string("/proc/meminfo") {
                if let Some(line) = contents.lines().find(|line| line.starts_with("MemTotal:")) {
                    if let Some(kb_str) = line.split_whitespace().nth(1) {
                        if let Ok(kb) = kb_str.parse::<u64>() {
                            let gb = kb / 1024 / 1024;
                            return format!("{}GB", gb);
                        }
                    }
                }
            }
        }

        "Unknown".to_string()
    }

    fn semantic_stop_symbol(symbol: &str) -> bool {
        matches!(
            symbol.to_lowercase().as_str(),
            "into"
                | "unwrap"
                | "ok"
                | "err"
                | "some"
                | "none"
                | "new"
                | "from"
                | "default"
                | "clone"
                | "drop"
                | "len"
                | "push"
                | "pop"
                | "to_string"
                | "println"
                | "debug"
                | "info"
                | "warn"
                | "error"
                | "fmt"
                | "str"
        )
    }

    async fn index_file(
        path: PathBuf,
        parse_pb: ProgressBar,
        embed_pb: ProgressBar,
    ) -> Result<FileStats> {
        debug!("Indexing file: {:?}", path);
        let mut stats = FileStats::default();

        // Read file content
        let content = tokio_fs::read_to_string(&path).await?;
        stats.lines = content.lines().count();

        // Very rough heuristics for functions/classes counts per common languages
        let ext = path.extension().and_then(OsStr::to_str).unwrap_or("");
        let (fn_regex, class_regex) = match ext {
            "rs" => (Some(Regex::new(r"\bfn\s+\w+").unwrap()), None),
            "py" => (
                Some(Regex::new(r"\bdef\s+\w+\s*\(").unwrap()),
                Some(Regex::new(r"\bclass\s+\w+\s*:").unwrap()),
            ),
            "ts" | "js" => (
                Some(Regex::new(r"\bfunction\s+\w+|\b\w+\s*=\s*\(.*\)\s*=>").unwrap()),
                Some(Regex::new(r"\bclass\s+\w+").unwrap()),
            ),
            "go" => (Some(Regex::new(r"\bfunc\s+\w+\s*\(").unwrap()), None),
            "java" => (
                Some(Regex::new(r"\b\w+\s+\w+\s*\(.*\)\s*\{").unwrap()),
                Some(Regex::new(r"\bclass\s+\w+").unwrap()),
            ),
            "cpp" | "cc" | "cxx" | "hpp" | "h" | "c" => (
                Some(Regex::new(r"\b\w+\s+\w+\s*\(.*\)\s*\{").unwrap()),
                None,
            ),
            _ => (None, None),
        };

        parse_pb.set_message(format!("Parsing {}", path.display()));
        parse_pb.inc(1);

        if let Some(re) = fn_regex {
            stats.functions = re.find_iter(&content).count();
        }
        if let Some(re) = class_regex {
            stats.classes = re.find_iter(&content).count();
        }

        // Pretend to generate embeddings by counting tokens roughly
        embed_pb.set_message(format!("Embedding {}", path.display()));
        stats.embeddings = content.split_whitespace().count() / 100; // 1 per ~100 tokens
        embed_pb.inc(1);

        Ok(stats)
    }

    async fn collect_files(&self, path: &Path) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();

        let walker = if self.config.recursive {
            WalkDir::new(path)
        } else {
            WalkDir::new(path).max_depth(1)
        };

        for entry in walker {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                continue;
            }
            if self.should_index(path) {
                files.push(path.to_path_buf());
            }
        }

        Ok(files)
    }

    fn should_index(&self, path: &Path) -> bool {
        let file_name = path.file_name().and_then(OsStr::to_str).unwrap_or("");
        let path_str = path.to_string_lossy();

        // Exclude patterns (simple substring match)
        for pat in &self.config.exclude_patterns {
            if path_str.contains(pat) || file_name.contains(pat) {
                return false;
            }
        }

        // Include patterns (if provided, must match at least one)
        if !self.config.include_patterns.is_empty()
            && !self
                .config
                .include_patterns
                .iter()
                .any(|p| path_str.contains(p) || file_name.contains(p))
        {
            return false;
        }

        // Language filtering by extension
        if !self.config.languages.is_empty() {
            let ext = path
                .extension()
                .and_then(OsStr::to_str)
                .unwrap_or("")
                .to_lowercase();
            let lang_matches = |langs: &Vec<String>, e: &str| -> bool {
                let lang = e;
                langs.iter().any(|l| match l.as_str() {
                    "rust" | "rs" => matches!(lang, "rs"),
                    "python" | "py" => matches!(lang, "py"),
                    "js" | "javascript" | "jsx" => matches!(lang, "js" | "jsx"),
                    "ts" | "typescript" | "tsx" => matches!(lang, "ts" | "tsx"),
                    "go" => matches!(lang, "go"),
                    "java" => matches!(lang, "java"),
                    "cpp" | "c++" | "cc" | "cxx" | "hpp" | "h" | "c" => {
                        matches!(lang, "cpp" | "cc" | "cxx" | "hpp" | "h" | "c")
                    }
                    _ => false,
                })
            };
            if !lang_matches(&self.config.languages, &ext) {
                return false;
            }
        }

        // Default excludes
        for dir in [
            ".git",
            "node_modules",
            "target",
            ".codegraph",
            "dist",
            "build",
        ] {
            if path_str.contains(dir) {
                return false;
            }
        }

        true
    }

    async fn is_indexed(&self, _path: &Path) -> Result<bool> {
        Ok(self.store.project_marker(&self.project_id).await?.is_some())
    }

    async fn has_file_metadata(&self) -> Result<bool> {
        Ok(!self
            .store
            .get_file_metadata_for_project(&self.project_id)
            .await?
            .is_empty())
    }

    /// Calculate SHA-256 hash of file content
    fn calculate_file_hash(file_path: &Path) -> Result<String> {
        // Resolve symlinks to actual file
        let canonical_path = fs::canonicalize(file_path)
            .with_context(|| format!("Failed to resolve path: {:?}", file_path))?;

        let mut file = fs::File::open(&canonical_path)
            .with_context(|| format!("Failed to open file: {:?}", canonical_path))?;

        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 8192];

        loop {
            let bytes_read = file
                .read(&mut buffer)
                .with_context(|| format!("Failed to read file: {:?}", canonical_path))?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }

        Ok(hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>())
    }

    /// Detect changes between current filesystem and stored file metadata
    async fn detect_file_changes(&self, current_files: &[PathBuf]) -> Result<Vec<FileChange>> {
        let files: Vec<_> = current_files.iter().map(|p| (p.clone(), 0)).collect();
        let snapshots = codegraph_parser::SourceSnapshots::capture(
            &files,
            crate::estimation::source_memory_budget(),
            self.parser.concurrency(),
        )
        .await?;
        self.detect_snapshot_changes(current_files, &snapshots)
            .await
    }

    async fn detect_snapshot_changes(
        &self,
        current_files: &[PathBuf],
        snapshots: &codegraph_parser::SourceSnapshots,
    ) -> Result<Vec<FileChange>> {
        let stored_metadata = self
            .store
            .get_file_metadata_for_project(&self.project_id)
            .await?;

        // Create lookup map of stored files
        let stored_map: HashMap<String, FileMetadataRecord> = stored_metadata
            .into_iter()
            .map(|record| (record.file_path.clone(), record))
            .collect();

        let mut changes = Vec::new();
        let mut current_file_set = HashSet::new();

        // Check current files for additions or modifications
        for file_path in current_files {
            let file_path_str = file_path.to_string_lossy().to_string();
            current_file_set.insert(file_path_str.clone());

            let current_hash = snapshots
                .get(file_path)
                .ok_or_else(|| anyhow!("Missing source snapshot: {}", file_path.display()))?
                .content_hash
                .clone();

            match stored_map.get(&file_path_str) {
                Some(stored) => {
                    if stored.content_hash != current_hash {
                        changes.push(FileChange {
                            file_path: file_path_str,
                            change_type: FileChangeType::Modified,
                            current_hash: Some(current_hash),
                            previous_hash: Some(stored.content_hash.clone()),
                        });
                    } else {
                        changes.push(FileChange {
                            file_path: file_path_str,
                            change_type: FileChangeType::Unchanged,
                            current_hash: Some(current_hash),
                            previous_hash: Some(stored.content_hash.clone()),
                        });
                    }
                }
                None => {
                    changes.push(FileChange {
                        file_path: file_path_str,
                        change_type: FileChangeType::Added,
                        current_hash: Some(current_hash),
                        previous_hash: None,
                    });
                }
            }
        }

        // Check for deleted files
        for (stored_path, stored_record) in stored_map {
            if !current_file_set.contains(&stored_path) {
                changes.push(FileChange {
                    file_path: stored_path,
                    change_type: FileChangeType::Deleted,
                    current_hash: None,
                    previous_hash: Some(stored_record.content_hash),
                });
            }
        }

        Ok(changes)
    }

    /// Delete nodes and edges for specific files
    async fn delete_data_for_files(&self, file_paths: &[String]) -> Result<()> {
        if file_paths.is_empty() {
            return Ok(());
        }

        // Use writer to delete nodes, edges, and file metadata as one ordered operation
        self.writer_handle()?
            .enqueue_delete_nodes_by_file(file_paths.to_vec(), &self.project_id)
            .await
    }

    /// Persist file metadata for incremental indexing change tracking
    async fn persist_file_metadata(
        &self,
        files: &[PathBuf],
        nodes: &[CodeNode],
        edges: &[EdgeRelationship],
        snapshots: &codegraph_parser::SourceSnapshots,
        previous: Option<&std::collections::BTreeMap<String, String>>,
    ) -> Result<std::collections::BTreeMap<String, String>> {
        let mut file_metadata_records = Vec::new();
        let mut hashes = std::collections::BTreeMap::new();

        // Create progress bar for file metadata
        let metadata_pb = self.progress.add(ProgressBar::new(files.len() as u64));
        metadata_pb.set_style(
            ProgressStyle::with_template("{spinner:.blue} {msg} [{bar:40.cyan/blue}] {pos}/{len}")
                .unwrap()
                .progress_chars("█▓▒░  "),
        );
        metadata_pb.set_message("💾 Processing file metadata");

        // Build HashMap for O(1) lookups instead of O(N) iterations
        let mut file_stats: HashMap<String, (i64, i64)> = HashMap::new();

        // Count nodes per file - O(nodes)
        for node in nodes {
            let entry = file_stats
                .entry(node.location.file_path.clone())
                .or_insert((0, 0));
            entry.0 += 1;
        }

        // Build node-to-file mapping for edge counting - O(nodes)
        let node_file_map: HashMap<NodeId, String> = nodes
            .iter()
            .map(|n| (n.id, n.location.file_path.clone()))
            .collect();

        // Count edges per file - O(edges)
        for edge in edges {
            if let Some(file_path) = node_file_map.get(&edge.from) {
                let entry = file_stats.entry(file_path.clone()).or_insert((0, 0));
                entry.1 += 1;
            }
        }

        for file_path in files {
            let file_path_str = file_path.to_string_lossy().to_string();

            let snapshot = snapshots
                .get(file_path)
                .ok_or_else(|| anyhow!("Missing source snapshot: {}", file_path.display()))?;
            let content_hash = snapshot.content_hash.clone();
            let file_size = snapshot.size as i64;
            let modified_at = chrono::DateTime::<chrono::Utc>::from(snapshot.modified_at);

            // Get counts from HashMap - O(1) lookup
            let (node_count, edge_count) =
                file_stats.get(&file_path_str).copied().unwrap_or((0, 0));

            let hash = codegraph_core::artifact_cache::fingerprint(&(
                &file_path_str,
                &content_hash,
                file_size,
                modified_at,
                node_count,
                edge_count,
            ))?;
            let unchanged = previous.and_then(|hashes| hashes.get(&file_path_str)) == Some(&hash);
            hashes.insert(file_path_str.clone(), hash);
            metadata_pb.inc(1);
            if unchanged {
                continue;
            }
            file_metadata_records.push(FileMetadataRecord {
                file_path: file_path_str,
                project_id: self.project_id.clone(),
                content_hash,
                modified_at,
                file_size,
                last_indexed_at: chrono::Utc::now(),
                node_count,
                edge_count,
                language: None, // Will be inferred from file extension if needed
                parse_errors: None,
            });
        }

        // Batch upsert file metadata
        let updated = file_metadata_records.len();
        if updated > 0 {
            self.writer_handle()?
                .enqueue_file_metadata(file_metadata_records)
                .await?;
        }

        self.finish_bar(
            metadata_pb,
            format!("💾 File metadata complete: {} files tracked", files.len()),
        )?;

        info!("💾 Updated metadata for {updated} of {} files", files.len());
        Ok(hashes)
    }

    async fn persist_project_metadata(
        &self,
        stats: &IndexStats,
        node_count: usize,
        edge_count: usize,
        fingerprint: &str,
    ) -> Result<()> {
        let project_name = self
            .project_root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&self.project_id)
            .to_string();
        let root_path = self.project_root.to_string_lossy().to_string();
        let primary_language = self.config.languages.first().cloned();
        let record = ProjectMetadataRecord {
            metadata: serde_json::json!({
                "input_fingerprint": fingerprint,
                "graph_shape": GRAPH_SHAPE_VERSION,
                "complete": stats.complete,
                "stats": stats,
            }),
            project_id: self.project_id.clone(),
            name: project_name,
            root_path,
            primary_language,
            file_count: stats.files as i64,
            node_count: node_count as i64,
            edge_count: edge_count as i64,
            avg_coverage_score: 0.0,
            last_analyzed: chrono::Utc::now(),
            codegraph_version: env!("CARGO_PKG_VERSION").to_string(),
            organization_id: self.organization_id.clone(),
            domain: self.domain.clone(),
        };

        self.enqueue_project_metadata_record(record).await
    }

    fn annotate_node(&self, node: &mut CodeNode) {
        node.metadata
            .attributes
            .insert("project_id".to_string(), self.project_id.clone());
        if let Some(org) = &self.organization_id {
            node.metadata
                .attributes
                .insert("organization_id".to_string(), org.clone());
        }
        if let Some(repo) = &self.repository_url {
            node.metadata
                .attributes
                .insert("repository_url".to_string(), repo.clone());
        }
        if let Some(domain) = &self.domain {
            node.metadata
                .attributes
                .insert("domain".to_string(), domain.clone());
        }
    }

    async fn persist_nodes_batch(&self, chunk: &[CodeNode]) -> Result<()> {
        self.enqueue_nodes_chunk(chunk).await
    }

    async fn log_graph_counts(&self, what: &str, expected: usize) {
        match self.store.counts().await {
            Ok(counts) => {
                let stored = if what == "edges" {
                    counts.edges
                } else {
                    counts.nodes
                };
                info!("🗄️ Graph {what} persisted: {stored} (expected ≈ {expected})");
            }
            Err(e) => warn!("⚠️ Graph {what} count query failed: {e}"),
        }
    }

    /// The SQLite graph store the indexer writes to.
    pub fn graph_store(&self) -> SqliteGraphStore {
        self.store.clone()
    }

    async fn verify_file_metadata_count(&self, expected_files: usize) -> Result<()> {
        if expected_files == 0 {
            return Ok(());
        }
        let stored = self
            .store
            .get_file_metadata_for_project(&self.project_id)
            .await
            .context("Failed to verify file metadata count")?;
        if stored.len() >= expected_files {
            return Ok(());
        }
        let sample_paths: Vec<&str> = stored
            .iter()
            .take(5)
            .map(|row| row.file_path.as_str())
            .collect();
        Err(anyhow!(
            "file_metadata count {} is less than expected {} for project {}. Sample stored file_paths: {:?}",
            stored.len(),
            expected_files,
            self.project_id,
            sample_paths
        ))
    }

    async fn verify_project_metadata_present(&self) -> Result<()> {
        if self.store.project_marker(&self.project_id).await?.is_none() {
            Err(anyhow!(
                "project metadata missing for project {}",
                self.project_id
            ))
        } else {
            Ok(())
        }
    }

    fn writer_handle(&self) -> Result<&GraphWriterHandle> {
        self.writer
            .as_ref()
            .ok_or_else(|| anyhow!("Graph writer not initialized"))
    }

    async fn enqueue_nodes_chunk(&self, nodes: &[CodeNode]) -> Result<()> {
        if nodes.is_empty() {
            return Ok(());
        }
        let batch: Vec<CodeNode> = nodes.to_vec();
        self.writer_handle()?.enqueue_nodes(batch).await
    }

    async fn enqueue_edges(&self, edges: Vec<CodeEdge>) -> Result<()> {
        if edges.is_empty() {
            return Ok(());
        }
        self.writer_handle()?.enqueue_edges(edges).await
    }

    async fn enqueue_project_metadata_record(&self, record: ProjectMetadataRecord) -> Result<()> {
        self.writer_handle()?.enqueue_project_metadata(record).await
    }

    async fn flush_writer(&self) -> Result<()> {
        if let Some(writer) = &self.writer {
            writer.flush().await
        } else {
            Ok(())
        }
    }

    async fn shutdown_writer(&mut self) -> Result<()> {
        match self.writer.take() {
            Some(writer) => writer.shutdown().await,
            _ => Ok(()),
        }
    }

    async fn open_store(project_root: &Path) -> Result<SqliteGraphStore> {
        let root = project_root.to_path_buf();
        let store = tokio::task::spawn_blocking(move || SqliteGraphStore::open(&root))
            .await?
            .with_context(|| {
                format!(
                    "Failed to open graph store {}",
                    codegraph_graph::sqlite_store::db_path(project_root).display()
                )
            })?;
        info!(
            "🗄️ Graph store opened: {}",
            codegraph_graph::sqlite_store::db_path(project_root).display()
        );
        Ok(store)
    }

    /// Takes the cross-process write lock, waiting if another writer holds it.
    async fn acquire_write_lock(&self) -> Result<WriteLock> {
        if let Some(lock) = WriteLock::acquire(&self.project_root, false)? {
            return Ok(lock);
        }
        info!("⏳ Waiting for another process to release the graph write lock");
        let root = self.project_root.clone();
        tokio::task::spawn_blocking(move || WriteLock::acquire(&root, true))
            .await??
            .ok_or_else(|| anyhow!("Blocking write lock acquisition returned no lock"))
    }

    fn normalize_symbol(symbol: &str) -> String {
        symbol.trim().to_lowercase()
    }

    /// Binds a call-like edge to a symbol: exact alias, then normalized variants, then a
    /// lexical candidate. An edge marked `VERBATIM_TARGET_ATTR` keeps the text code-review-graph
    /// keeps unless a symbol has exactly that alias.
    pub(crate) fn resolve_target(
        catalog: &crate::resolution::SymbolCatalog,
        edge: &EdgeRelationship,
        normalized: &mut HashMap<String, Vec<String>>,
        lexical_cache: &mut HashMap<(NodeId, String), Option<(NodeId, f64)>>,
    ) -> (Option<NodeId>, (&'static str, f64)) {
        if edge
            .metadata
            .contains_key(codegraph_parser::languages::crg_shape::UNTYPED_RECEIVER_ATTR)
        {
            return (catalog.exact_free_function(edge), ("exact", 1.0));
        }
        let mut target = catalog.exact(edge, &[]);
        let mut provenance = (
            if target.is_some_and(|id| edge.metadata.get("target_node_id") == Some(&id.to_string()))
            {
                "definition"
            } else {
                "exact"
            },
            1.0f64,
        );
        // A path-qualified target names one symbol of one file: no other spelling or
        // look-alike is that symbol (code-review-graph settles it to the plain name instead).
        if target.is_some()
            || edge
                .metadata
                .contains_key(codegraph_parser::languages::crg_shape::VERBATIM_TARGET_ATTR)
            || catalog.is_path_qualified(&edge.to)
        {
            return (target, provenance);
        }
        let variants = normalized.entry(edge.to.clone()).or_insert_with(|| {
            let mut variants = Self::normalize_symbol_target(&edge.to);
            if edge.to.starts_with("_R") || edge.to.starts_with("_ZN") || edge.to.contains('<') {
                variants.extend(Self::normalize_rust_symbol(&edge.to));
                variants.sort();
                variants.dedup();
            }
            variants
        });
        target = catalog.exact(edge, variants);
        provenance.0 = "normalized";
        if target.is_none() && !catalog.ambiguous(&edge.to) {
            let lexical = *lexical_cache
                .entry((edge.from, edge.to.clone()))
                .or_insert_with(|| catalog.fuzzy(edge));
            if let Some((id, score)) = lexical {
                target = Some(id);
                provenance = ("lexical", score);
            }
        }
        (target, provenance)
    }

    /// The text an unresolved edge keeps. Like code-review-graph's `_settle_unbound_endpoints`,
    /// a CALLS/REFERENCES target written `<file>::<Owner>.<name>` that names no node falls back
    /// to the name the source wrote (`./m.rs::Foo.clone` -> `clone`).
    pub(crate) fn unbound_target(
        catalog: &crate::resolution::SymbolCatalog,
        edge: &EdgeRelationship,
    ) -> String {
        if matches!(edge.edge_type, EdgeType::Calls | EdgeType::References)
            && catalog.is_path_qualified(&edge.to)
            && let Some(plain) = edge
                .to
                .split_once("::")
                .and_then(|(_, symbol)| symbol.rsplit(['.', ':']).find(|part| !part.is_empty()))
        {
            return plain.to_string();
        }
        edge.to.clone()
    }

    /// Whether an unresolved edge is stored with its target text, as code-review-graph does.
    /// Only CRG edge kinds from source extraction qualify: Rust-only analyzer kinds (uses,
    /// defines, ...) and FastML edges, whose targets are pattern labels such as
    /// `java:implements `, still drop unresolved targets.
    fn keeps_unresolved_target(edge: &EdgeRelationship) -> bool {
        !edge.metadata.contains_key("fast_ml_enhancement")
            && matches!(
                edge.edge_type,
                EdgeType::Calls
                    | EdgeType::Imports
                    | EdgeType::Extends
                    | EdgeType::Implements
                    | EdgeType::References
            )
            || edge.edge_type == EdgeType::Other(codegraph_parser::languages::java::INJECTS.into())
    }

    fn normalize_symbol_target(target: &str) -> Vec<String> {
        // Normalize edge targets to match codegraph-parser emitted symbol keys.
        let mut variants = Vec::new();

        let mut base = target.trim();

        // Strip trailing macro bang
        if let Some(stripped) = base.strip_suffix('!') {
            base = stripped;
        }

        // Strip call parentheses and args: take up to first '('
        if let Some(idx) = base.find('(') {
            base = &base[..idx];
        }

        // Strip generics by removing everything from first '<' that has a matching '>'
        let mut generic_stripped = String::new();
        let mut depth = 0;
        for ch in base.chars() {
            if ch == '<' {
                depth += 1;
                continue;
            }
            if ch == '>' && depth > 0 {
                depth -= 1;
                continue;
            }
            if depth == 0 {
                generic_stripped.push(ch);
            }
        }
        let mut cleaned = generic_stripped.trim().to_string();

        // Strip trait qualification: "Type as Trait::method" -> "Type::method"
        if let Some(pos) = cleaned.find(" as ") {
            let after = &cleaned[pos + 4..];
            if let Some(idx) = after.find("::") {
                cleaned = format!("{}{}", &cleaned[..pos], &after[idx..]);
            }
        }

        // Align with parser naming: drop self./this./super./crate:: prefixes
        let prefixes = ["self::", "self.", "this.", "super::", "super.", "crate::"];
        for p in prefixes {
            if let Some(stripped) = cleaned.strip_prefix(p) {
                cleaned = stripped.to_string();
                break;
            }
        }

        // Generate both module separators used by emitters (Rust uses ::, Python/JS often .)
        let dotted = cleaned.replace("::", ".");
        let coloned = cleaned.replace('.', "::");

        let lower_clean = cleaned.to_lowercase();
        let lower_dotted = dotted.to_lowercase();
        let lower_coloned = coloned.to_lowercase();

        variants.push(cleaned.clone());
        variants.push(lower_clean.clone());
        variants.push(dotted.clone());
        variants.push(lower_dotted.clone());
        variants.push(coloned.clone());
        variants.push(lower_coloned.clone());

        // Push last path segment variants
        for candidate in [&cleaned, &dotted, &coloned] {
            if let Some(last) = candidate.rsplit(['.', ':']).next() {
                variants.push(last.to_string());
                variants.push(last.to_lowercase());
            }
        }

        variants.sort();
        variants.dedup();
        variants
    }

    #[cfg(test)]
    pub(crate) fn normalize_symbol_target_for_tests(target: &str) -> Vec<String> {
        Self::normalize_symbol_target(target)
    }

    fn normalize_rust_symbol(target: &str) -> Vec<String> {
        // Demangle if possible (rustc, then Symbolic as fallback to strip hashes)
        let demangled = try_demangle(target)
            .map(|d| d.to_string())
            .unwrap_or_else(|_| target.to_string());

        let symbolic = demangle(&demangled).into_owned();

        let mut out = Vec::new();

        // Strip generics/path args via syn
        if let Ok(path) = parse_syn_path::<SynPath>(&symbolic) {
            let mut simple = Vec::new();
            for seg in path.segments {
                let name = seg.ident.to_string();
                simple.push(name);
            }
            if !simple.is_empty() {
                let joined = simple.join("::");
                out.push(joined.clone());
                out.push(joined.to_lowercase());
                if let Some(last) = simple.last() {
                    out.push(last.clone());
                    out.push(last.to_lowercase());
                }
            }
        }

        // Fallback: return demangled text plus lowered forms
        if out.is_empty() {
            out.push(symbolic.clone());
            out.push(symbolic.to_lowercase());
        }

        out
    }

    fn normalize_js_symbol(target: &str) -> Vec<String> {
        // Lightweight heuristic until JS parser dependency is restored
        Self::normalize_symbol_target(target)
    }

    fn normalize_python_symbol(target: &str) -> Vec<String> {
        // Fallback: simple split heuristics for Python until AST parser is re-enabled
        Self::normalize_symbol_target(target)
    }

    fn progress_enabled() -> bool {
        if let Ok(v) = std::env::var("CODEGRAPH_NO_PROGRESS") {
            let v = v.trim();
            if v == "1" || v.eq_ignore_ascii_case("true") {
                return false;
            }
        }
        if let Ok(v) = std::env::var("CODEGRAPH_PROGRESS") {
            let v = v.trim();
            if v == "0" || v.eq_ignore_ascii_case("false") {
                return false;
            }
        }
        true
    }

    fn progress_draw_target() -> ProgressDrawTarget {
        if Self::progress_enabled() {
            ProgressDrawTarget::stderr_with_hz(4)
        } else {
            ProgressDrawTarget::hidden()
        }
    }

    fn create_progress_bar(&self, total: u64, message: &str) -> ProgressBar {
        let pb = self.progress.add(ProgressBar::new(total));
        pb.set_draw_target(Self::progress_draw_target());
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.blue} [{elapsed_precise}] [{bar:34.cyan/blue}] {pos}/{len} ({percent}%) {msg} | {per_sec}/s | ETA:{eta}")
                .unwrap()
                .progress_chars("█▉▊▋▌▍▎▏ "),
        );
        pb.set_message(message.to_string());
        pb
    }

    /// Create enhanced progress bar with dual metrics for files and success rates
    fn create_dual_progress_bar(
        &self,
        total: u64,
        primary_msg: &str,
        secondary_msg: &str,
    ) -> ProgressBar {
        let pb = self.progress.add(ProgressBar::new(total));
        pb.set_draw_target(Self::progress_draw_target());
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.blue} [{elapsed_precise}] [{bar:34.cyan/blue}] {pos}/{len} {msg} | Success: {percent}% | {per_sec}/s | ETA:{eta}")
                .unwrap()
                .progress_chars("█▉▊▋▌▍▎▏ "),
        );
        pb.set_message(format!("{} | {}", primary_msg, secondary_msg));
        pb
    }

    /// Create high-performance progress bar for batch processing
    fn create_batch_progress_bar(&self, total: u64, batch_size: usize, label: &str) -> ProgressBar {
        let pb = self.progress.add(ProgressBar::new(total));
        pb.set_draw_target(Self::progress_draw_target());
        let batch_info = format!("row limit: {}", batch_size);

        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.blue} [{elapsed_precise}] [{bar:34.cyan/blue}] {pos}/{len} items | {msg} | {percent}% | {per_sec}/s | ETA:{eta}")
                .unwrap()
                .progress_chars("█▉▊▋▌▍▎▏ "),
        );
        pb.set_message(format!("{} | {}", label, batch_info));
        pb
    }

    fn finish_bar(&self, pb: ProgressBar, message: impl Into<String>) -> Result<()> {
        pb.finish_and_clear();
        if Self::progress_enabled() {
            self.progress.println(message.into())?;
        }
        Ok(())
    }

    /// Index a single file (for daemon mode incremental updates)
    /// Uses upsert semantics - no duplicate records created
    pub async fn index_single_file(&self, _path: &Path) -> Result<()> {
        self.reconcile_project(&self.project_root, false).await?;
        Ok(())
    }

    /// Reconciliation also removes obsolete definitions and rebinds unchanged callers.
    pub async fn delete_file_data(&self, _path: &Path) -> Result<()> {
        self.reconcile_project(&self.project_root, false).await?;
        Ok(())
    }

    /// Detect language from file extension
    fn detect_language(&self, path: &Path) -> Option<codegraph_core::Language> {
        use codegraph_core::Language;

        let ext = path.extension()?.to_str()?.to_lowercase();
        match ext.as_str() {
            "rs" => Some(Language::Rust),
            "py" => Some(Language::Python),
            "ts" => Some(Language::TypeScript),
            "tsx" => Some(Language::TypeScript),
            "js" => Some(Language::JavaScript),
            "jsx" => Some(Language::JavaScript),
            "go" => Some(Language::Go),
            "java" => Some(Language::Java),
            "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "c" | "h" => Some(Language::Cpp),
            "swift" => Some(Language::Swift),
            "kt" | "kts" => Some(Language::Kotlin),
            "cs" => Some(Language::CSharp),
            "rb" => Some(Language::Ruby),
            "php" => Some(Language::Php),
            "dart" => Some(Language::Dart),
            _ => None,
        }
    }

    pub fn should_reconcile(&self, path: &Path) -> bool {
        if path.components().any(|part| {
            matches!(
                part.as_os_str().to_str(),
                Some(".codegraph" | ".git" | "target" | "node_modules")
            )
        }) {
            return false;
        }
        self.should_index(path) || crate::reconciliation::support_file(path)
    }

    pub async fn watch_for_changes(&self, path: impl AsRef<Path>) -> Result<()> {
        use notify::{Event, RecursiveMode, Watcher};
        let path = path.as_ref().to_path_buf();
        let (tx, mut rx) = mpsc::channel(100);
        let debounce = std::time::Duration::from_millis(
            std::env::var("CODEGRAPH_WATCH_DEBOUNCE_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
        );
        let mut watcher =
            notify::recommended_watcher(move |res: std::result::Result<Event, notify::Error>| {
                let _ = tx.blocking_send(res);
            })?;
        watcher.watch(&path, RecursiveMode::Recursive)?;
        while let Some(first) = rx.recv().await {
            let relevant = |event: &std::result::Result<Event, notify::Error>| {
                event.as_ref().map_or(true, |event| {
                    !matches!(event.kind, notify::EventKind::Access(_))
                        && (event.need_rescan()
                            || event.paths.iter().any(|path| self.should_reconcile(path)))
                })
            };
            let mut dirty = relevant(&first);
            // Trailing debounce retains the final save, renames, deletes and overflow rescans.
            while let Ok(Some(event)) = tokio::time::timeout(debounce, rx.recv()).await {
                dirty |= relevant(&event);
            }
            if dirty && let Err(error) = self.reconcile_project(&path, false).await {
                warn!("Watch reconciliation failed: {error}");
            }
        }
        Ok(())
    }
}

impl ProjectIndexer {
    async fn handle_file_event(
        &self,
        event: notify::Event,
        _last_events: &mut std::collections::HashMap<PathBuf, std::time::Instant>,
        _debounce_ms: u64,
    ) {
        if !matches!(event.kind, notify::EventKind::Access(_))
            && (event.need_rescan() || event.paths.iter().any(|path| self.should_reconcile(path)))
        {
            if let Err(error) = self.reconcile_project(&self.project_root, false).await {
                warn!("Watch reconciliation failed: {error}");
                return;
            }
            if let Some(tx) = WATCH_TEST_NOTIFIER
                .get_or_init(|| Mutex::new(None))
                .lock()
                .unwrap()
                .as_ref()
            {
                for path in event.paths {
                    let _ = tx.send(path);
                }
            }
        }
    }

    pub async fn simulate_file_event(
        &self,
        event: notify::Event,
        last_events: &mut std::collections::HashMap<PathBuf, std::time::Instant>,
        debounce_ms: u64,
    ) {
        self.handle_file_event(event, last_events, debounce_ms)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn analyzer_requires_rust_analyzer_when_lsp_enabled() {
        let settings =
            AnalyzerSettings::for_tier(codegraph_core::config_manager::IndexingTier::Full);

        let err = ProjectIndexer::validate_analyzer_tools(
            &[codegraph_core::Language::Rust],
            settings,
            "",
            Path::new("."),
        )
        .await
        .expect_err("should fail when required tools are missing");
        assert!(err.to_string().contains("Missing rust-analyzer"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn analyzer_rejects_broken_rustup_shims_and_accepts_working_fallbacks() {
        use std::os::unix::fs::PermissionsExt;
        let project = tempfile::tempdir().unwrap();
        let broken = project.path().join("broken");
        let working = project.path().join("working");
        for directory in [&broken, &working] {
            std::fs::create_dir(directory).unwrap();
        }
        std::fs::write(project.path().join("rust-toolchain.toml"), "sentinel").unwrap();
        for (directory, script) in [
            (
                &broken,
                "#!/bin/sh\nprintf \"Unknown binary 'rust-analyzer' in official toolchain 'stable'\\n\" >&2\nexit 1\n",
            ),
            (
                &working,
                "#!/bin/sh\n[ \"$1\" = --version ] && [ -f rust-toolchain.toml ] || exit 2\nprintf 'rust-analyzer test\\n'\n",
            ),
        ] {
            let path = directory.join("rust-analyzer");
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let settings =
            AnalyzerSettings::for_tier(codegraph_core::config_manager::IndexingTier::Balanced);
        let error = ProjectIndexer::validate_analyzer_tools(
            &[codegraph_core::Language::Rust],
            settings,
            broken.to_str().unwrap(),
            project.path(),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("Unknown binary 'rust-analyzer'"), "{error}");
        assert!(error.contains("rustup component add rust-analyzer"));
        assert!(error.contains(broken.to_str().unwrap()));
        let path = std::env::join_paths([&broken, &working]).unwrap();
        ProjectIndexer::validate_analyzer_tools(
            &[codegraph_core::Language::Rust],
            settings,
            path.to_str().unwrap(),
            project.path(),
        )
        .await
        .unwrap();
        ProjectIndexer::validate_analyzer_tools(
            &[codegraph_core::Language::Rust],
            AnalyzerSettings::for_tier(codegraph_core::config_manager::IndexingTier::Fast),
            broken.to_str().unwrap(),
            project.path(),
        )
        .await
        .unwrap();
        ProjectIndexer::validate_analyzer_tools(
            &[codegraph_core::Language::Rust],
            AnalyzerSettings {
                require_tools: false,
                ..settings
            },
            broken.to_str().unwrap(),
            project.path(),
        )
        .await
        .unwrap();
    }

    #[test]
    fn verbatim_targets_skip_normalized_and_lexical_resolution() {
        let node = |name: &str| {
            let mut node = CodeNode::new(
                name.to_string(),
                Some(codegraph_core::NodeType::Function),
                Some(codegraph_core::Language::Rust),
                codegraph_core::Location {
                    file_path: "m.rs".into(),
                    line: 1,
                    column: 0,
                    end_line: None,
                    end_column: None,
                },
            );
            node.set_deterministic_id("test");
            node
        };
        let mut new = node("new");
        new.metadata
            .attributes
            .insert("method_of".into(), "Foo".into());
        let caller = node("main");
        let catalog = crate::resolution::SymbolCatalog::new(&[new.clone(), caller.clone()]);
        let mut edge = EdgeRelationship {
            from: caller.id,
            to: "HashMap::new".into(),
            edge_type: EdgeType::Calls,
            metadata: std::collections::HashMap::new(),
            span: None,
        };
        let resolve = |edge: &EdgeRelationship| {
            ProjectIndexer::resolve_target(
                &catalog,
                edge,
                &mut std::collections::HashMap::new(),
                &mut std::collections::HashMap::new(),
            )
        };
        assert_eq!(resolve(&edge), (Some(new.id), ("normalized", 1.0)));
        edge.metadata.insert(
            codegraph_parser::languages::crg_shape::VERBATIM_TARGET_ATTR.into(),
            "true".into(),
        );
        assert_eq!(resolve(&edge).0, None);
    }

    #[test]
    fn unbound_path_qualified_calls_fall_back_to_the_plain_name() {
        let node = |name: &str, kind: codegraph_core::NodeType| {
            let mut node = CodeNode::new(
                name.to_string(),
                Some(kind),
                Some(codegraph_core::Language::Rust),
                codegraph_core::Location {
                    file_path: "./m.rs".into(),
                    line: 1,
                    column: 0,
                    end_line: None,
                    end_column: None,
                },
            );
            node.set_deterministic_id("test");
            node
        };
        let file = node("./m.rs", codegraph_core::NodeType::Other("File".into()));
        let mut sig = node("sig", codegraph_core::NodeType::Function);
        sig.metadata
            .attributes
            .insert("qualified_name".into(), "./m.rs::String.sig".into());
        let catalog = crate::resolution::SymbolCatalog::new(&[file, sig.clone()]);
        let edge = EdgeRelationship {
            from: sig.id,
            to: "./m.rs::String.len".into(),
            edge_type: EdgeType::Calls,
            metadata: std::collections::HashMap::new(),
            span: None,
        };
        let resolved = ProjectIndexer::resolve_target(
            &catalog,
            &edge,
            &mut std::collections::HashMap::new(),
            &mut std::collections::HashMap::new(),
        );
        assert_eq!(resolved.0, None);
        assert_eq!(ProjectIndexer::unbound_target(&catalog, &edge), "len");
        let bare = EdgeRelationship {
            to: "HashMap::new".into(),
            ..edge.clone()
        };
        assert_eq!(
            ProjectIndexer::unbound_target(&catalog, &bare),
            "HashMap::new"
        );
    }

    #[test]
    fn untyped_receiver_calls_bind_only_free_functions() {
        let node = |name: &str, qualified: &str| {
            let mut node = CodeNode::new(
                name.to_string(),
                Some(codegraph_core::NodeType::Function),
                Some(codegraph_core::Language::Rust),
                codegraph_core::Location {
                    file_path: "./m.rs".into(),
                    line: 1,
                    column: 0,
                    end_line: None,
                    end_column: None,
                },
            );
            node.metadata
                .attributes
                .insert("qualified_name".into(), qualified.into());
            node.set_deterministic_id("test");
            node
        };
        let mut method = node("a", "./m.rs::Foo.a");
        method
            .metadata
            .attributes
            .insert("parent_name".into(), "Foo".into());
        let free = node("b", "./m.rs::b");
        let caller = node("main", "./m.rs::main");
        let catalog =
            crate::resolution::SymbolCatalog::new(&[method, free.clone(), caller.clone()]);
        let call = |to: &str| EdgeRelationship {
            from: caller.id,
            to: to.into(),
            edge_type: EdgeType::Calls,
            metadata: std::collections::HashMap::from([(
                codegraph_parser::languages::crg_shape::UNTYPED_RECEIVER_ATTR.to_string(),
                "true".to_string(),
            )]),
            span: None,
        };
        let resolve = |edge: &EdgeRelationship| {
            ProjectIndexer::resolve_target(
                &catalog,
                edge,
                &mut std::collections::HashMap::new(),
                &mut std::collections::HashMap::new(),
            )
            .0
        };
        assert_eq!(resolve(&call("a")), None);
        assert_eq!(resolve(&call("aa")), None);
        assert_eq!(resolve(&call("b")), Some(free.id));
    }

    #[test]
    fn fast_tier_filters_noisy_edges() {
        let mut edges = vec![
            EdgeRelationship {
                from: NodeId::new_v4(),
                to: "a".to_string(),
                edge_type: EdgeType::Calls,
                metadata: std::collections::HashMap::new(),
                span: None,
            },
            EdgeRelationship {
                from: NodeId::new_v4(),
                to: "b".to_string(),
                edge_type: EdgeType::Uses,
                metadata: std::collections::HashMap::new(),
                span: None,
            },
            EdgeRelationship {
                from: NodeId::new_v4(),
                to: "c".to_string(),
                edge_type: EdgeType::References,
                metadata: std::collections::HashMap::new(),
                span: None,
            },
            EdgeRelationship {
                from: NodeId::new_v4(),
                to: "d".to_string(),
                edge_type: EdgeType::Other("flows_to".to_string()),
                metadata: std::collections::HashMap::new(),
                span: None,
            },
        ];

        let removed = filter_edges_for_tier(
            codegraph_core::config_manager::IndexingTier::Fast,
            &mut edges,
        );
        assert_eq!(removed, 2);
        assert!(edges.iter().any(|e| e.edge_type == EdgeType::Calls));
        assert!(
            edges
                .iter()
                .any(|e| e.edge_type == EdgeType::Other("flows_to".to_string()))
        );
        assert!(!edges.iter().any(|e| e.edge_type == EdgeType::Uses));
        assert!(!edges.iter().any(|e| e.edge_type == EdgeType::References));
    }

    #[test]
    fn metadata_text_formatter_never_truncates_unicode_or_long_source() {
        let content = "let café = \"🚀\";\n".repeat(1000);
        let node = CodeNode::new(
            "fixture",
            Some(NodeType::Function),
            Some(codegraph_core::Language::Rust),
            codegraph_core::Location {
                file_path: "fixture.rs".into(),
                line: 1,
                column: 0,
                end_line: None,
                end_column: None,
            },
        )
        .with_content(content.clone());
        assert!(prepare_node_text(&node).ends_with(&content));
    }
}

pub fn prepare_node_text(node: &CodeNode) -> String {
    let lang = node
        .language
        .as_ref()
        .map(|l| format!("{:?}", l))
        .unwrap_or_else(|| "unknown".to_string());
    let kind = node
        .node_type
        .as_ref()
        .map(|t| format!("{:?}", t))
        .unwrap_or_else(|| "unknown".to_string());
    let mut text = format!("{} {} {}", lang, kind, node.name);
    if let Some(c) = &node.content {
        text.push(' ');
        text.push_str(c);
    }

    text
}

pub fn simple_text_embedding(text: &str, dimension: usize) -> Vec<f32> {
    let mut embedding = vec![0.0f32; dimension];
    let mut hash = 5381u32;
    for b in text.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(b as u32);
    }
    let mut state = hash;
    for value in &mut embedding {
        state = state.wrapping_mul(1103515245).wrapping_add(12345);
        *value = ((state as f32 / u32::MAX as f32) - 0.5) * 2.0;
    }
    embedding
}

pub fn normalize(v: &[f32]) -> Vec<f32> {
    let mut out = v.to_vec();
    let norm: f32 = out.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut out {
            *x /= norm;
        }
    }
    out
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct IndexStats {
    pub cached_files: usize,
    pub index_ms: u64,
    pub startup_ms: u64,
    pub phase_ms: std::collections::BTreeMap<String, u64>,
    pub writer_jobs_acked: u64,
    pub writer_rows_acked: u64,
    pub writer_payload_bytes_acked: u64,
    pub source_read_operations: u64,
    pub source_read_bytes: u64,
    pub source_spilled_bytes: u64,
    pub graph_complete: bool,
    pub vector_index_ready: bool,
    pub embedding_status: String,
    pub semantic_status: String,
    pub complete: bool,
    pub inference_texts: u64,
    pub inference_tokens: u64,
    pub embedding_cache_hits: u64,
    pub files: usize,
    pub skipped: usize,
    pub lines: usize,
    pub functions: usize,
    pub classes: usize,
    pub structs: usize,
    pub traits: usize,
    pub embeddings: usize,
    pub errors: usize,
    // Extended stats for detailed reporting
    pub nodes: usize,
    pub edges: usize,
    pub chunks: usize,
    pub embedding_dimension: usize,
    pub embedding_provider: String,
    pub resolved_edges: usize,
    pub unresolved_edges: usize,
    pub resolution_rate: f64,
    // Analyzer stats
    pub analyzers_enabled: bool,
    pub build_context_nodes: usize,
    pub build_context_edges: usize,
    pub lsp_nodes_enriched: usize,
    pub lsp_edges_resolved: usize,
    pub docs_attached: usize,
    pub export_edges_added: usize,
    pub reexport_edges_added: usize,
    pub feature_enables_edges_added: usize,
    pub uses_edges_derived: usize,
    pub module_nodes_added: usize,
    pub module_contains_edges_added: usize,
    pub module_import_edges_added: usize,
    pub dataflow_variable_nodes_added: usize,
    pub dataflow_defines_edges_added: usize,
    pub dataflow_uses_edges_added: usize,
    pub dataflow_flows_to_edges_added: usize,
    pub dataflow_returns_edges_added: usize,
    pub dataflow_mutates_edges_added: usize,
    pub doc_nodes_added: usize,
    pub document_edges_added: usize,
    pub specification_edges_added: usize,
    pub package_cycles_detected: usize,
    pub boundary_violations_added: usize,
}

impl IndexStats {
    fn merge(&mut self, other: FileStats) {
        self.files += 1;
        self.lines += other.lines;
        self.functions += other.functions;
        self.classes += other.classes;
        self.structs += other.structs;
        self.traits += other.traits;
        self.embeddings += other.embeddings;
    }
}

#[derive(Debug, Default, Clone)]
struct FileStats {
    lines: usize,
    functions: usize,
    classes: usize,
    structs: usize,
    traits: usize,
    embeddings: usize,
}
