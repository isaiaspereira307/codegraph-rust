// ABOUTME: Tool executor for the graph analysis tools over the SQLite graph
// ABOUTME: Validates tool parameters, runs GraphQuery calls and caches their results

use crate::git_changes::changed_files;
use crate::review::{ReviewOptions, build_review, churn_counts, minimal_view};
use std::path::PathBuf;

/// Tools whose answer depends on the git work tree, which the database change marker
/// (`GraphQuery::change_marker`, part of every cache key) does not track.
const UNCACHED_TOOLS: &[&str] = &[
    "detect_changes",
    "get_review_context",
    "get_minimal_context",
    "get_affected_flows",
];

/// `, showing N of M` when a list was cut.
fn shown_of(shown: usize, total: usize) -> String {
    if shown < total {
        format!(", showing {shown} of {total}")
    } else {
        String::new()
    }
}

fn is_uncached(tool_name: &str) -> bool {
    UNCACHED_TOOLS.contains(&tool_name)
}

use codegraph_graph::graph_query::{GraphQuery, QueryPattern, TraverseDirection};
use codegraph_mcp_core::debug_logger::DebugLogger;
use codegraph_mcp_core::error::{McpError, Result};
use lru::LruCache;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use std::num::NonZeroUsize;
use std::sync::Arc;
use tracing::{debug, info, warn};

const TOOL_PROGRESS_LOG_TARGET: &str = "codegraph::mcp::tools";

use crate::graph_tool_schemas::GraphToolSchemas;

/// Statistics about LRU cache performance
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CacheStats {
    /// Number of cache hits (successful lookups)
    pub hits: u64,
    /// Number of cache misses (lookups that required a graph query)
    pub misses: u64,
    /// Number of entries evicted due to LRU policy
    pub evictions: u64,
    /// Current number of entries in cache
    pub current_size: usize,
    /// Maximum cache size (capacity)
    pub max_size: usize,
}

impl CacheStats {
    /// Calculate cache hit rate as percentage
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }
}

/// Executor for graph analysis tools
/// Receives tool calls and runs the matching GraphQuery method
pub struct GraphToolExecutor {
    graph_query: Arc<GraphQuery>,
    /// LRU cache for tool results (function_name + params → result)
    cache: Arc<Mutex<LruCache<String, JsonValue>>>,
    /// Cache statistics for observability
    cache_stats: Arc<Mutex<CacheStats>>,
    /// Whether caching is enabled
    cache_enabled: bool,
    /// Maximum result size in bytes (derived from LLM context window)
    /// Prevents sending oversized results that would exceed model limits
    max_result_bytes: usize,
}

/// Default max result bytes when context window not specified (~200KB)
const DEFAULT_MAX_RESULT_BYTES: usize = 200_000;

/// Ceiling for one tool result regardless of the configured context window. A window of a
/// million tokens does not make a multi-megabyte tool result useful to the model.
const MAX_RESULT_BYTES_CEILING: usize = 200_000;

/// Longest `content` a result row carries. Rows keep their file path and line range, so the
/// client can read the rest; whole documents and impl blocks otherwise dominate the context.
const DEFAULT_CONTENT_SNIPPET_CHARS: usize = 2_000;

/// `CODEGRAPH_TOOL_CONTENT_CHARS` overrides the snippet length; 0 disables shortening.
fn content_snippet_chars() -> usize {
    std::env::var("CODEGRAPH_TOOL_CONTENT_CHARS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(DEFAULT_CONTENT_SNIPPET_CHARS)
}

/// Shorten long `content` strings anywhere in a tool result to a leading snippet that ends
/// on a line boundary where possible, recording the original length next to it.
fn shorten_content(value: &mut JsonValue, max_chars: usize) {
    if max_chars == 0 {
        return;
    }
    match value {
        JsonValue::Array(items) => items
            .iter_mut()
            .for_each(|item| shorten_content(item, max_chars)),
        JsonValue::Object(map) => {
            let original_chars = match map.get("content") {
                Some(JsonValue::String(content)) if content.chars().count() > max_chars => {
                    Some(content.chars().count())
                }
                _ => None,
            };
            if let Some(original_chars) = original_chars {
                if let Some(JsonValue::String(content)) = map.get_mut("content") {
                    let cut = content
                        .char_indices()
                        .nth(max_chars)
                        .map_or(content.len(), |(index, _)| index);
                    // Prefer ending on a complete line when that keeps most of the snippet.
                    let cut = content[..cut]
                        .rfind('\n')
                        .filter(|line_end| *line_end >= cut / 2)
                        .unwrap_or(cut);
                    content.truncate(cut);
                }
                map.insert("content_truncated".to_string(), JsonValue::Bool(true));
                map.insert("content_chars".to_string(), json!(original_chars));
            }
            map.iter_mut()
                .filter(|(key, _)| key.as_str() != "content")
                .for_each(|(_, child)| shorten_content(child, max_chars));
        }
        _ => {}
    }
}

impl GraphToolExecutor {
    /// Create a new tool executor
    /// Derives max result size from the agent's context window (environment, then
    /// `[llm] context_window` in the config file, then the default)
    pub fn new(graph_query: Arc<GraphQuery>) -> Self {
        let context_window = codegraph_core::config_manager::ConfigManager::agent_context_window();

        Self::with_context_window(graph_query, context_window)
    }

    /// Create a new tool executor with explicit context window for result size limiting
    /// max_result_bytes = context_window * 2 (about half the window at 4 bytes/token), capped at
    /// `MAX_RESULT_BYTES_CEILING`
    pub fn with_context_window(graph_query: Arc<GraphQuery>, context_window: usize) -> Self {
        // Calculate max result bytes: use ~50% of context window, assuming ~4 chars per token
        // This leaves room for system prompt, conversation history, and response
        let max_result_bytes = context_window
            .saturating_mul(2)
            .min(MAX_RESULT_BYTES_CEILING);

        Self::with_limits(graph_query, true, 100, max_result_bytes)
    }

    /// Create a new tool executor with custom cache and result size configuration
    pub fn with_limits(
        graph_query: Arc<GraphQuery>,
        cache_enabled: bool,
        cache_size: usize,
        max_result_bytes: usize,
    ) -> Self {
        let capacity = NonZeroUsize::new(cache_size).unwrap_or(NonZeroUsize::new(100).unwrap());
        let cache = Arc::new(Mutex::new(LruCache::new(capacity)));
        let cache_stats = Arc::new(Mutex::new(CacheStats {
            hits: 0,
            misses: 0,
            evictions: 0,
            current_size: 0,
            max_size: cache_size,
        }));

        info!(
            "GraphToolExecutor initialized with max_result_bytes: {} ({:.1}MB)",
            max_result_bytes,
            max_result_bytes as f64 / 1_000_000.0
        );

        Self {
            graph_query,
            cache,
            cache_stats,
            cache_enabled,
            max_result_bytes,
        }
    }

    /// Create a new tool executor with custom cache configuration (legacy compatibility)
    pub fn with_cache(
        graph_query: Arc<GraphQuery>,
        cache_enabled: bool,
        cache_size: usize,
    ) -> Self {
        Self::with_limits(
            graph_query,
            cache_enabled,
            cache_size,
            DEFAULT_MAX_RESULT_BYTES,
        )
    }

    /// Get current cache statistics
    pub fn cache_stats(&self) -> CacheStats {
        self.cache_stats.lock().clone()
    }

    /// Expose the underlying graph queries for read-only summaries
    pub fn graph_query(&self) -> Arc<GraphQuery> {
        self.graph_query.clone()
    }

    /// Clear the cache and reset statistics
    pub fn clear_cache(&self) {
        let mut cache = self.cache.lock();
        cache.clear();
        let mut stats = self.cache_stats.lock();
        stats.hits = 0;
        stats.misses = 0;
        stats.evictions = 0;
        stats.current_size = 0;
    }

    /// Truncate result if it exceeds max_result_bytes to prevent context window overflow
    /// Returns truncated result with warning metadata if truncation occurred
    fn truncate_if_oversized(&self, tool_name: &str, result: JsonValue) -> JsonValue {
        let serialized = result.to_string();
        let result_bytes = serialized.len();

        if result_bytes <= self.max_result_bytes {
            return result;
        }

        // Result is oversized - need to truncate
        let overflow_ratio = result_bytes as f64 / self.max_result_bytes as f64;
        tracing::warn!(
            tool = tool_name,
            result_bytes = result_bytes,
            max_bytes = self.max_result_bytes,
            overflow_ratio = format!("{:.1}x", overflow_ratio),
            "Tool result exceeds max_result_bytes limit, truncating"
        );

        // Strategy: Try to truncate the "result" field if it's an array
        if let Some(result_array) = result.get("result").and_then(|r| r.as_array()) {
            // Calculate how many items we can keep
            let item_count = result_array.len();
            if item_count == 0 {
                return result;
            }

            // Estimate bytes per item (rough average)
            let bytes_per_item = result_bytes / item_count;
            let max_items = self.max_result_bytes / bytes_per_item.max(1);
            let keep_items = max_items.min(item_count).max(1); // Keep at least 1

            // Create truncated result
            let truncated_array: Vec<JsonValue> =
                result_array.iter().take(keep_items).cloned().collect();
            let truncated_count = item_count - keep_items;

            tracing::info!(
                tool = tool_name,
                original_items = item_count,
                kept_items = keep_items,
                truncated_items = truncated_count,
                "Truncated result array to fit context window"
            );

            // Reconstruct the result with truncation metadata
            let mut truncated_result = result.clone();
            if let Some(obj) = truncated_result.as_object_mut() {
                obj.insert("result".to_string(), JsonValue::Array(truncated_array));
                obj.insert(
                    "_truncated".to_string(),
                    json!({
                        "original_items": item_count,
                        "kept_items": keep_items,
                        "truncated_items": truncated_count,
                        "reason": "Result exceeded context window limit",
                        "max_bytes": self.max_result_bytes
                    }),
                );
            }

            return truncated_result;
        }

        // Fallback: If we can't smart-truncate, just return with a warning
        // This shouldn't happen often since most results are arrays
        tracing::error!(
            tool = tool_name,
            result_bytes = result_bytes,
            "Cannot truncate non-array result, returning as-is (may cause context overflow)"
        );
        result
    }

    /// Generate a cache key from project, database change marker, tool name and parameters.
    /// Any commit to the store moves the marker, so entries of an older state never hit.
    fn cache_key(
        project_id: &str,
        marker: (i64, i64),
        tool_name: &str,
        parameters: &JsonValue,
    ) -> String {
        format!(
            "{}@{}.{}:{}:{}",
            project_id, marker.0, marker.1, tool_name, parameters
        )
    }

    /// Execute a tool call from LLM
    ///
    /// # Arguments
    /// * `tool_name` - Name of the tool to execute
    /// * `parameters` - JSON parameters for the tool
    ///
    /// # Returns
    /// JSON result from the tool execution
    pub async fn execute(&self, tool_name: &str, parameters: JsonValue) -> Result<JsonValue> {
        log_tool_call_start(tool_name, &parameters);

        let exec_result: Result<JsonValue> = async {
            // Validate tool exists
            let _schema = GraphToolSchemas::get_by_name(tool_name)
                .ok_or_else(|| McpError::Protocol(format!("Unknown tool: {}", tool_name)))?;

            let project_id = self.graph_query.project_id();

            // Check cache if enabled. The key carries the change marker read before the
            // query runs: a commit during the query only files the entry under a marker
            // that never comes back, so a stale answer is never served.
            let cache_key = if self.cache_enabled && !is_uncached(tool_name) {
                let marker = self
                    .graph_query
                    .change_marker()
                    .await
                    .map_err(|e| McpError::Protocol(format!("graph store unavailable: {e}")))?;
                Some(Self::cache_key(&project_id, marker, tool_name, &parameters))
            } else {
                None
            };
            if let Some(cache_key) = &cache_key {
                // Try cache lookup
                {
                    let mut cache = self.cache.lock();
                    if let Some(cached_result) = cache.get(cache_key) {
                        // Cache hit
                        let mut stats = self.cache_stats.lock();
                        stats.hits += 1;
                        debug!("Cache hit for {}: {}", tool_name, cache_key);
                        let cached = cached_result.clone();
                        log_tool_call_finish(tool_name, &cached);
                        return Ok(cached);
                    }
                }

                // Cache miss - record it
                {
                    let mut stats = self.cache_stats.lock();
                    stats.misses += 1;
                }
                debug!("Cache miss for {}: {}", tool_name, cache_key);
            }

            // Execute based on tool name
            let result = match tool_name {
                "find_nodes_by_name" => self.execute_find_nodes_by_name(parameters.clone()).await?,
                "search_nodes" => self.execute_search_nodes(parameters.clone()).await?,
                "list_graph_stats" => self.execute_list_graph_stats().await?,
                "find_large_functions" => {
                    self.execute_find_large_functions(parameters.clone())
                        .await?
                }
                "query_graph" => self.execute_query_graph(parameters.clone()).await?,
                "traverse_graph" => self.execute_traverse_graph(parameters.clone()).await?,
                "get_impact_radius" => self.execute_get_impact_radius(parameters.clone()).await?,
                "detect_changes" => self.execute_detect_changes(parameters.clone()).await?,
                "get_review_context" => self.execute_review(parameters.clone(), false).await?,
                "get_minimal_context" => self.execute_review(parameters.clone(), true).await?,
                "list_flows" => self.execute_list_flows(parameters.clone()).await?,
                "get_flow" => self.execute_get_flow(parameters.clone()).await?,
                "get_affected_flows" => self.execute_get_affected_flows(parameters.clone()).await?,
                "list_communities" => self.execute_list_communities(parameters.clone()).await?,
                "get_community" => self.execute_get_community(parameters.clone()).await?,
                "get_architecture_overview" => {
                    self.execute_get_architecture_overview(parameters.clone())
                        .await?
                }
                "get_transitive_dependencies" => {
                    self.execute_get_transitive_dependencies(parameters.clone())
                        .await?
                }
                "detect_circular_dependencies" => {
                    self.execute_detect_circular_dependencies(parameters.clone())
                        .await?
                }
                "trace_call_chain" => self.execute_trace_call_chain(parameters.clone()).await?,
                "calculate_coupling_metrics" => {
                    self.execute_calculate_coupling_metrics(parameters.clone())
                        .await?
                }
                "get_hub_nodes" => self.execute_get_hub_nodes(parameters.clone()).await?,
                "get_bridge_nodes" => self.execute_get_bridge_nodes(parameters.clone()).await?,
                "get_knowledge_gaps" => self.execute_get_knowledge_gaps(parameters.clone()).await?,
                "get_suggested_questions" => self.execute_get_suggested_questions().await?,
                "get_surprising_connections" => {
                    self.execute_get_surprising_connections(parameters.clone())
                        .await?
                }
                "get_reverse_dependencies" => {
                    self.execute_get_reverse_dependencies(parameters.clone())
                        .await?
                }
                "find_complexity_hotspots" => {
                    self.execute_find_complexity_hotspots(parameters.clone())
                        .await?
                }
                _ => {
                    return Err(
                        McpError::Protocol(format!("Tool not implemented: {}", tool_name)).into(),
                    );
                }
            };

            // Shape the result for a model: content snippets first, then the size limit.
            let mut result = result;
            shorten_content(&mut result, content_snippet_chars());
            let result = self.truncate_if_oversized(tool_name, result);

            // Cache the result if enabled
            if let Some(cache_key) = cache_key {
                let mut cache = self.cache.lock();
                let was_evicted = cache.len() >= cache.cap().get();
                cache.put(cache_key, result.clone());

                // Update stats
                let mut stats = self.cache_stats.lock();
                if was_evicted {
                    stats.evictions += 1;
                }
                stats.current_size = cache.len();
            }

            Ok(result)
        }
        .await;

        match exec_result {
            Ok(result) => {
                log_tool_call_finish(tool_name, &result);
                Ok(result)
            }
            Err(err) => {
                warn!(
                    target: TOOL_PROGRESS_LOG_TARGET,
                    tool = tool_name,
                    parameters = %parameters,
                    error = %err,
                    "Tool call failed"
                );
                DebugLogger::log_tool_error(tool_name, &parameters, &format!("{}", err));
                Err(err)
            }
        }
    }

    /// Execute get_transitive_dependencies
    /// Execute find_nodes_by_name - resolve a symbol name to node IDs
    async fn execute_find_nodes_by_name(&self, params: JsonValue) -> Result<JsonValue> {
        let name = params["name"]
            .as_str()
            .ok_or_else(|| McpError::Protocol("Missing name".to_string()))?;
        let limit = params["limit"].as_i64().unwrap_or(10).clamp(1, 50) as usize;

        let result = self
            .graph_query
            .find_nodes_by_name(name, limit)
            .await
            .map_err(|e| McpError::Protocol(format!("find_nodes_by_name failed: {}", e)))?;

        Ok(json!({
            "tool": "find_nodes_by_name",
            "parameters": {
                "name": name,
                "limit": limit
            },
            "result": result
        }))
    }

    /// Execute search_nodes - full-text search over the FTS5 index
    async fn execute_search_nodes(&self, params: JsonValue) -> Result<JsonValue> {
        let query = params["query"]
            .as_str()
            .ok_or_else(|| McpError::Protocol("Missing query".to_string()))?;
        let limit = params["limit"].as_i64().unwrap_or(10).clamp(1, 50) as usize;

        let result = self
            .graph_query
            .search_nodes(query, limit)
            .await
            .map_err(|e| McpError::Protocol(format!("search_nodes failed: {}", e)))?;

        Ok(json!({
            "tool": "search_nodes",
            "parameters": {
                "query": query,
                "limit": limit
            },
            "result": result
        }))
    }

    /// Node id for a call: `node_id` as given, else the best `name` match (exact name wins).
    /// `Ok(None)` when the name matches nothing; an error when neither is given.
    async fn target_node_id(&self, params: &JsonValue) -> Result<Option<String>> {
        if let Some(id) = params["node_id"].as_str() {
            return Ok(Some(id.to_string()));
        }
        let name = params["name"]
            .as_str()
            .ok_or_else(|| McpError::Protocol("Missing node_id or name".to_string()))?;
        let candidates = self
            .graph_query
            .search_nodes(name, 10)
            .await
            .map_err(|e| McpError::Protocol(format!("node lookup failed: {}", e)))?;
        Ok(candidates
            .iter()
            .find(|n| n.name == name)
            .or(candidates.first())
            .map(|n| n.id.clone()))
    }

    /// Execute query_graph - one pattern from a node given by id or by name
    async fn execute_query_graph(&self, params: JsonValue) -> Result<JsonValue> {
        let pattern_name = params["pattern"]
            .as_str()
            .ok_or_else(|| McpError::Protocol("Missing pattern".to_string()))?;
        let pattern = QueryPattern::parse(pattern_name).ok_or_else(|| {
            McpError::Protocol(format!(
                "Unknown pattern '{pattern_name}'; expected callers_of, callees_of, imports_of, importers_of or tests_for"
            ))
        })?;
        let limit = params["limit"].as_i64().unwrap_or(20).clamp(1, 100) as usize;

        let Some(node_id) = self.target_node_id(&params).await? else {
            return Ok(no_match("query_graph", &params));
        };
        let result = self
            .graph_query
            .query_graph(pattern, &node_id, limit)
            .await
            .map_err(|e| McpError::Protocol(format!("query_graph failed: {}", e)))?;

        Ok(json!({
            "tool": "query_graph",
            "parameters": { "pattern": pattern.name(), "node_id": node_id, "limit": limit },
            "result": result
        }))
    }

    /// Execute get_review_context (`minimal` = false) or get_minimal_context (`minimal` = true)
    async fn execute_review(&self, params: JsonValue, minimal: bool) -> Result<JsonValue> {
        let opts = ReviewOptions {
            base_ref: params["base_ref"].as_str().unwrap_or("HEAD").to_string(),
            depth: params["depth"].as_i64().unwrap_or(3).clamp(1, 10) as i32,
            max_nodes: params["max_nodes"].as_i64().unwrap_or(50).clamp(1, 500) as usize,
            max_snippet_lines: params["max_snippet_lines"]
                .as_i64()
                .unwrap_or(20)
                .clamp(1, 100) as usize,
            include_churn: params["include_churn"].as_bool().unwrap_or(true),
        };
        let tool = if minimal {
            "get_minimal_context"
        } else {
            "get_review_context"
        };
        let ctx = build_review(&self.graph_query, &opts)
            .await
            .map_err(|e| McpError::Protocol(format!("{tool} failed: {e}")))?;
        let result = if minimal {
            minimal_view(&ctx)
        } else {
            serde_json::to_value(&ctx)?
        };
        Ok(json!({
            "tool": tool,
            "parameters": {
                "base_ref": opts.base_ref,
                "depth": opts.depth,
                "max_nodes": opts.max_nodes,
                "max_snippet_lines": opts.max_snippet_lines,
                "include_churn": opts.include_churn
            },
            "result": result
        }))
    }

    /// Execute list_flows - stored execution flows by criticality
    async fn execute_list_flows(&self, params: JsonValue) -> Result<JsonValue> {
        let sort_by = params["sort_by"]
            .as_str()
            .unwrap_or("criticality")
            .to_string();
        let limit = params["limit"].as_i64().unwrap_or(50).clamp(1, 200) as usize;
        let kind = params["kind"]
            .as_str()
            .filter(|k| !k.is_empty())
            .map(str::to_string);
        let minimal = params["detail_level"].as_str() == Some("minimal");
        let (flows, total) = self
            .graph_query
            .list_flows(&sort_by, limit, kind.as_deref())
            .await
            .map_err(|e| McpError::Protocol(format!("list_flows failed: {e:#}")))?;
        let shown = flows.len();
        let rows: Vec<JsonValue> = flows
            .iter()
            .map(|f| {
                if minimal {
                    json!({"name": f.name, "criticality": f.criticality, "node_count": f.node_count})
                } else {
                    serde_json::to_value(f).unwrap_or(JsonValue::Null)
                }
            })
            .collect();
        Ok(json!({
            "tool": "list_flows",
            "parameters": { "sort_by": sort_by, "limit": limit, "kind": kind,
                            "detail_level": if minimal { "minimal" } else { "standard" } },
            "result": {
                "summary": format!("Found {total} execution flow(s){}", shown_of(shown, total)),
                "flows": rows,
                "total": total,
                "truncated": shown < total,
            }
        }))
    }

    /// Execute get_flow - one flow with steps and optional numbered source
    async fn execute_get_flow(&self, params: JsonValue) -> Result<JsonValue> {
        let flow_id = params["flow_id"].as_i64();
        let flow_name = params["flow_name"].as_str().map(str::to_string);
        let include_source = params["include_source"].as_bool().unwrap_or(false);
        let max_steps = params["max_steps"].as_i64().unwrap_or(50).clamp(1, 200) as usize;
        let max_source_lines = params["max_source_lines"]
            .as_i64()
            .unwrap_or(400)
            .clamp(1, 2000) as usize;
        let parameters = json!({ "flow_id": flow_id, "flow_name": flow_name,
            "include_source": include_source, "max_steps": max_steps,
            "max_source_lines": max_source_lines });
        let detail = self
            .graph_query
            .get_flow(flow_id, flow_name.as_deref(), max_steps)
            .await
            .map_err(|e| McpError::Protocol(format!("get_flow failed: {e:#}")))?;
        let Some(detail) = detail else {
            return Ok(
                json!({ "tool": "get_flow", "parameters": parameters, "result": {
                "status": "not_found",
                "summary": "No flow found matching the given criteria." } }),
            );
        };
        let summary = format!(
            "Flow '{}': {} nodes, depth {}, criticality {:.4}{}",
            detail.flow.name,
            detail.flow.node_count,
            detail.flow.depth,
            detail.flow.criticality,
            shown_of(detail.steps.len(), detail.total_steps)
        );
        let mut truncated = detail.truncated;
        let mut source_truncated = false;
        let mut flow = serde_json::to_value(&detail)?;
        if include_source {
            let root = PathBuf::from(self.graph_query.project_id());
            let mut budget = max_source_lines;
            for (i, step) in detail.steps.iter().enumerate() {
                if budget == 0 {
                    truncated = true;
                    source_truncated = true;
                    break;
                }
                flow["steps"][i]["source"] = json!(crate::flows::numbered_source(
                    &root,
                    &step.file,
                    step.line_start,
                    step.line_end,
                    &mut budget
                ));
            }
        }
        flow["total_steps"] = json!(detail.total_steps);
        flow["truncated"] = json!(truncated);
        if source_truncated {
            flow["source_truncated"] = json!(true);
        }
        Ok(
            json!({ "tool": "get_flow", "parameters": parameters, "result": {
            "status": "ok", "summary": summary, "flow": flow } }),
        )
    }

    /// Execute get_affected_flows - flows through the changed files
    async fn execute_get_affected_flows(&self, params: JsonValue) -> Result<JsonValue> {
        let base_ref = params["base_ref"].as_str().unwrap_or("HEAD~1").to_string();
        let minimal = params["detail_level"].as_str() == Some("minimal");
        let max_flows = params["max_flows"].as_i64().unwrap_or(50).max(0) as usize;
        let given: Option<Vec<String>> = params["changed_files"].as_array().map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        });
        let files = match given {
            Some(files) => files,
            None => {
                let root = PathBuf::from(self.graph_query.project_id());
                let base = base_ref.clone();
                tokio::task::spawn_blocking(move || changed_files(&root, &base))
                    .await
                    .map_err(|e| McpError::Protocol(format!("git task failed: {e}")))?
                    .map_err(|e| McpError::Protocol(format!("get_affected_flows failed: {e}")))?
                    .into_iter()
                    .map(|f| f.path)
                    .collect()
            }
        };
        let parameters = json!({ "changed_files": files, "base_ref": base_ref,
            "detail_level": if minimal { "minimal" } else { "standard" }, "max_flows": max_flows });
        if files.is_empty() {
            return Ok(
                json!({ "tool": "get_affected_flows", "parameters": parameters, "result": {
                "status": "ok", "summary": "No changed files detected.",
                "affected_flows": [], "total": 0, "truncated": false } }),
            );
        }
        let mut flows = self
            .graph_query
            .affected_flows(&files)
            .await
            .map_err(|e| McpError::Protocol(format!("get_affected_flows failed: {e:#}")))?;
        let total = flows.len();
        let ceiling = if minimal { 500 } else { 25 };
        let limit = if max_flows == 0 {
            ceiling
        } else {
            max_flows.min(ceiling)
        };
        let mut truncated = total > limit;
        flows.truncate(limit);
        let shown = flows.len();
        let rows: Vec<JsonValue> = if minimal {
            flows.iter().map(crate::flows::affected_minimal).collect()
        } else {
            let (rows, cut) = crate::flows::bound_flow_steps(flows);
            truncated |= cut;
            rows
        };
        Ok(
            json!({ "tool": "get_affected_flows", "parameters": parameters, "result": {
            "status": "ok",
            "summary": format!("{total} flow(s) affected by changes in {} file(s){}",
                files.len(), shown_of(shown, total)),
            "changed_files": files, "affected_flows": rows, "total": total,
            "truncated": truncated } }),
        )
    }

    /// Execute list_communities - stored communities with capped member lists
    async fn execute_list_communities(&self, params: JsonValue) -> Result<JsonValue> {
        let sort_by = params["sort_by"].as_str().unwrap_or("size").to_string();
        let min_size = params["min_size"].as_i64().unwrap_or(0).max(0) as usize;
        let minimal = params["detail_level"].as_str() == Some("minimal");
        let max_results = params["max_results"].as_i64().unwrap_or(50).clamp(1, 200) as usize;
        let max_members = params["max_members"].as_i64().unwrap_or(10).clamp(1, 25) as usize;
        let all = self
            .graph_query
            .list_communities(&sort_by, min_size)
            .await
            .map_err(|e| McpError::Protocol(format!("list_communities failed: {e:#}")))?;
        let total = all.len();
        let rows: Vec<JsonValue> = all
            .iter()
            .take(max_results)
            .map(|c| {
                if minimal {
                    json!({"name": c.name, "size": c.size, "cohesion": c.cohesion})
                } else {
                    let mut v = serde_json::to_value(c).unwrap_or(JsonValue::Null);
                    crate::communities::cap_members(&mut v, max_members);
                    v
                }
            })
            .collect();
        let shown = rows.len();
        Ok(json!({
            "tool": "list_communities",
            "parameters": { "sort_by": sort_by, "min_size": min_size,
                "detail_level": if minimal { "minimal" } else { "standard" },
                "max_results": max_results, "max_members": max_members },
            "result": {
                "summary": format!("Found {total} communities{}", shown_of(shown, total)),
                "communities": rows, "total": total, "truncated": shown < total,
            }
        }))
    }

    /// Execute get_community - one community, optionally with member node details
    async fn execute_get_community(&self, params: JsonValue) -> Result<JsonValue> {
        let id = params["community_id"].as_i64();
        let name = params["community_name"].as_str().map(str::to_string);
        let include_members = params["include_members"].as_bool().unwrap_or(false);
        let max_members = params["max_members"].as_i64().unwrap_or(25).clamp(1, 25) as usize;
        let parameters = json!({ "community_id": id, "community_name": name,
            "include_members": include_members, "max_members": max_members });
        let found = self
            .graph_query
            .get_community(id, name.as_deref())
            .await
            .map_err(|e| McpError::Protocol(format!("get_community failed: {e:#}")))?;
        let Some(community) = found else {
            return Ok(
                json!({ "tool": "get_community", "parameters": parameters, "result": {
                "status": "not_found",
                "summary": "No community found matching the given criteria." } }),
            );
        };
        let summary = format!(
            "Community '{}': {} nodes, cohesion {:.4}",
            community.name, community.size, community.cohesion
        );
        let mut value = serde_json::to_value(&community)?;
        if include_members {
            let details = self
                .graph_query
                .community_members(community.id)
                .await
                .map_err(|e| McpError::Protocol(format!("get_community failed: {e:#}")))?;
            value["member_details"] = serde_json::to_value(details)?;
        }
        crate::communities::cap_members(&mut value, max_members);
        Ok(
            json!({ "tool": "get_community", "parameters": parameters, "result": {
            "status": "ok", "summary": summary, "community": value } }),
        )
    }

    /// Execute get_architecture_overview - communities and the coupling between them
    async fn execute_get_architecture_overview(&self, params: JsonValue) -> Result<JsonValue> {
        let minimal = params["detail_level"].as_str().unwrap_or("minimal") != "standard";
        let max_results = params["max_results"].as_i64().unwrap_or(100).clamp(1, 200) as usize;
        let max_members = params["max_members"].as_i64().unwrap_or(10).clamp(1, 25) as usize;
        let overview = self
            .graph_query
            .architecture_overview()
            .await
            .map_err(|e| McpError::Protocol(format!("get_architecture_overview failed: {e:#}")))?;
        let mut value = if minimal {
            crate::communities::minimal_overview(&overview)
        } else {
            let mut v = serde_json::to_value(&overview)?;
            if let Some(list) = v["communities"].as_array_mut() {
                for c in list {
                    crate::communities::cap_members(c, max_members);
                }
            }
            v
        };
        let mut truncated = false;
        let mut cross_total = 0;
        let mut warn_total = 0;
        for (key, total) in [
            ("cross_community_edges", &mut cross_total),
            ("warnings", &mut warn_total),
        ] {
            if let Some(rows) = value[key].as_array_mut() {
                *total = rows.len();
                if rows.len() > max_results {
                    rows.truncate(max_results);
                    truncated = true;
                }
            }
        }
        let shown = value["cross_community_edges"]
            .as_array()
            .map_or(0, Vec::len);
        let label = if minimal {
            "community pairs"
        } else {
            "cross-community edges"
        };
        value["summary"] = json!(format!(
            "Architecture: {} communities, {cross_total} {label}{}, {warn_total} warning(s)",
            overview.communities.len(),
            shown_of(shown, cross_total)
        ));
        value["status"] = json!("ok");
        value["cross_community_edges_total"] = json!(cross_total);
        value["truncated"] = json!(truncated);
        Ok(json!({
            "tool": "get_architecture_overview",
            "parameters": { "detail_level": if minimal { "minimal" } else { "standard" },
                "max_results": max_results, "max_members": max_members },
            "result": value
        }))
    }

    /// Execute detect_changes - nodes touched by `git diff -U0 <base_ref>` in the project
    async fn execute_detect_changes(&self, params: JsonValue) -> Result<JsonValue> {
        let base_ref = params["base_ref"].as_str().unwrap_or("HEAD").to_string();
        let include_churn = params["include_churn"].as_bool().unwrap_or(false);
        let root = PathBuf::from(self.graph_query.project_id());
        let base = base_ref.clone();
        let git_root = root.clone();
        let files = tokio::task::spawn_blocking(move || changed_files(&git_root, &base))
            .await
            .map_err(|e| McpError::Protocol(format!("detect_changes failed: {}", e)))?
            .map_err(|e| McpError::Protocol(format!("detect_changes failed: {}", e)))?;

        let ranges: Vec<(String, Vec<(i32, i32)>)> = files
            .iter()
            .map(|f| (f.path.clone(), f.lines.clone()))
            .collect();
        let nodes = self
            .graph_query
            .nodes_at_lines(&ranges)
            .await
            .map_err(|e| McpError::Protocol(format!("detect_changes failed: {}", e)))?;

        let (churn, churn_status) = churn_counts(&root, include_churn).await;
        let ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
        let risks = self
            .graph_query
            .risk_scores(&ids, &churn)
            .await
            .map_err(|e| McpError::Protocol(format!("detect_changes failed: {}", e)))?;
        let mut scored: Vec<(f64, JsonValue)> = Vec::with_capacity(nodes.len());
        for node in &nodes {
            let score = risks.get(&node.id).map_or(0.0, |r| r.0);
            let mut value = serde_json::to_value(node)?;
            if let Some(obj) = value.as_object_mut() {
                obj.insert("risk_score".into(), json!(score));
            }
            scored.push((score, value));
        }
        let risk_score = scored.iter().map(|s| s.0).fold(0.0, f64::max);
        let mut priorities: Vec<&(f64, JsonValue)> = scored.iter().collect();
        // Stable: ties keep the changed-node order.
        priorities.sort_by(|a, b| b.0.total_cmp(&a.0));
        let review_priorities: Vec<&JsonValue> =
            priorities.into_iter().take(10).map(|s| &s.1).collect();

        let changed_files: Vec<JsonValue> = files
            .iter()
            .map(|f| json!({ "path": f.path, "lines": f.lines }))
            .collect();
        let changed_nodes: Vec<&JsonValue> = scored.iter().map(|s| &s.1).collect();
        Ok(json!({
            "tool": "detect_changes",
            "parameters": { "base_ref": base_ref, "include_churn": include_churn },
            "result": {
                "base_ref": base_ref,
                "risk_score": risk_score,
                "churn_status": churn_status,
                "changed_files": changed_files,
                "changed_nodes": changed_nodes,
                "review_priorities": review_priorities,
            }
        }))
    }

    /// Execute get_impact_radius - dependents of changed nodes or files, with tests
    async fn execute_get_impact_radius(&self, params: JsonValue) -> Result<JsonValue> {
        let strings = |key: &str| -> Vec<String> {
            params[key]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        let node_ids = strings("node_ids");
        let files = strings("files");
        if node_ids.is_empty() && files.is_empty() {
            return Err(McpError::Protocol("Missing node_ids or files".to_string()).into());
        }
        let depth = params["depth"].as_i64().unwrap_or(3) as i32;
        let max_nodes = params["max_nodes"].as_i64().unwrap_or(200).clamp(1, 1000) as usize;

        let result = self
            .graph_query
            .get_impact_radius(&node_ids, &files, depth, max_nodes)
            .await
            .map_err(|e| McpError::Protocol(format!("get_impact_radius failed: {}", e)))?;

        Ok(json!({
            "tool": "get_impact_radius",
            "parameters": {
                "node_ids": node_ids,
                "files": files,
                "depth": depth,
                "max_nodes": max_nodes
            },
            "result": result
        }))
    }

    /// Execute traverse_graph - breadth-first walk over dependency edges
    async fn execute_traverse_graph(&self, params: JsonValue) -> Result<JsonValue> {
        let direction_name = params["direction"].as_str().unwrap_or("out");
        let direction = TraverseDirection::parse(direction_name).ok_or_else(|| {
            McpError::Protocol(format!(
                "Unknown direction '{direction_name}'; expected out, in or both"
            ))
        })?;
        let edge_type = params["edge_type"].as_str();
        let max_depth = params["max_depth"].as_i64().unwrap_or(3) as i32;
        let max_nodes = params["max_nodes"].as_i64().unwrap_or(100).clamp(1, 500) as usize;

        let Some(node_id) = self.target_node_id(&params).await? else {
            return Ok(no_match("traverse_graph", &params));
        };
        let result = self
            .graph_query
            .traverse_graph(&node_id, direction, edge_type, max_depth, max_nodes)
            .await
            .map_err(|e| McpError::Protocol(format!("traverse_graph failed: {}", e)))?;

        Ok(json!({
            "tool": "traverse_graph",
            "parameters": {
                "node_id": node_id,
                "direction": direction_name,
                "edge_type": edge_type,
                "max_depth": max_depth,
                "max_nodes": max_nodes
            },
            "result": result
        }))
    }

    /// Execute find_large_functions - functions by line span, read from SQLite
    async fn execute_find_large_functions(&self, params: JsonValue) -> Result<JsonValue> {
        let min_lines = params["min_lines"].as_i64().unwrap_or(50).max(1);
        let limit = params["limit"].as_i64().unwrap_or(20).clamp(1, 100) as usize;

        let result = self
            .graph_query
            .find_large_functions(min_lines, limit)
            .await
            .map_err(|e| McpError::Protocol(format!("find_large_functions failed: {}", e)))?;

        Ok(json!({
            "tool": "find_large_functions",
            "parameters": {
                "min_lines": min_lines,
                "limit": limit
            },
            "result": result
        }))
    }

    /// Execute list_graph_stats - graph-wide counts read from SQLite
    async fn execute_list_graph_stats(&self) -> Result<JsonValue> {
        let result = self
            .graph_query
            .graph_stats()
            .await
            .map_err(|e| McpError::Protocol(format!("list_graph_stats failed: {}", e)))?;

        Ok(json!({
            "tool": "list_graph_stats",
            "parameters": {},
            "result": result
        }))
    }

    async fn execute_get_transitive_dependencies(&self, params: JsonValue) -> Result<JsonValue> {
        let node_id = params["node_id"]
            .as_str()
            .ok_or_else(|| McpError::Protocol("Missing node_id".to_string()))?;

        // Default to "Calls" if edge_type not provided (for LATS compatibility)
        let edge_type = params["edge_type"].as_str().unwrap_or("Calls");

        let depth = params["depth"].as_i64().unwrap_or(3) as i32;

        let result = self
            .graph_query
            .get_transitive_dependencies(node_id, edge_type, depth)
            .await
            .map_err(|e| {
                McpError::Protocol(format!("get_transitive_dependencies failed: {}", e))
            })?;

        Ok(json!({
            "tool": "get_transitive_dependencies",
            "parameters": {
                "node_id": node_id,
                "edge_type": edge_type,
                "depth": depth
            },
            "result": result
        }))
    }

    /// Execute detect_circular_dependencies
    async fn execute_detect_circular_dependencies(&self, params: JsonValue) -> Result<JsonValue> {
        let edge_type = params["edge_type"]
            .as_str()
            .ok_or_else(|| McpError::Protocol("Missing edge_type".to_string()))?;

        let result = self
            .graph_query
            .detect_circular_dependencies(edge_type)
            .await
            .map_err(|e| {
                McpError::Protocol(format!("detect_circular_dependencies failed: {}", e))
            })?;

        Ok(json!({
            "tool": "detect_circular_dependencies",
            "parameters": {
                "edge_type": edge_type
            },
            "result": result
        }))
    }

    /// Execute trace_call_chain
    async fn execute_trace_call_chain(&self, params: JsonValue) -> Result<JsonValue> {
        // Accept both "from_node" (canonical) and "node_id" (common pattern) for compatibility
        let from_node = params["from_node"]
            .as_str()
            .or_else(|| params["node_id"].as_str())
            .ok_or_else(|| McpError::Protocol("Missing from_node or node_id".to_string()))?;

        let max_depth = params["max_depth"].as_i64().unwrap_or(5) as i32;

        let result = self
            .graph_query
            .trace_call_chain(from_node, max_depth)
            .await
            .map_err(|e| McpError::Protocol(format!("trace_call_chain failed: {}", e)))?;

        Ok(json!({
            "tool": "trace_call_chain",
            "parameters": {
                "from_node": from_node,
                "max_depth": max_depth
            },
            "result": result
        }))
    }

    /// Execute calculate_coupling_metrics
    async fn execute_calculate_coupling_metrics(&self, params: JsonValue) -> Result<JsonValue> {
        let node_id = params["node_id"]
            .as_str()
            .ok_or_else(|| McpError::Protocol("Missing node_id".to_string()))?;

        // Optional project_id override to reduce mismatches between indexing and runtime
        let gf = if let Some(project_id) = params["project_id"].as_str() {
            self.graph_query.with_project_id(project_id)
        } else {
            (*self.graph_query).clone()
        };

        let result = gf
            .calculate_coupling_metrics(node_id)
            .await
            .map_err(|e| McpError::Protocol(format!("calculate_coupling_metrics failed: {}", e)))?;

        Ok(json!({
            "tool": "calculate_coupling_metrics",
            "parameters": {
                "node_id": node_id
            },
            "result": result
        }))
    }

    /// Execute get_hub_nodes - most connected nodes, CRG-shaped
    async fn execute_get_hub_nodes(&self, params: JsonValue) -> Result<JsonValue> {
        let top_n = params["top_n"].as_i64().unwrap_or(10).clamp(1, 100) as usize;
        let min_degree = params["min_degree"].as_i64().map(|d| d.max(1) as usize);
        let minimal = params["detail_level"].as_str() == Some("minimal");
        let mut hubs = self
            .graph_query
            .hub_nodes()
            .await
            .map_err(|e| McpError::Protocol(format!("get_hub_nodes failed: {e:#}")))?;
        if let Some(min) = min_degree {
            hubs.retain(|h| h.total_degree >= min);
        }
        let total = hubs.len();
        hubs.truncate(top_n);
        let rows: Vec<JsonValue> = hubs
            .iter()
            .map(|h| {
                if minimal {
                    // qualified_name stays so the answer chains into the node tools.
                    json!({
                        "name": h.name,
                        "qualified_name": h.qualified_name,
                        "kind": h.kind,
                        "total_degree": h.total_degree
                    })
                } else {
                    serde_json::to_value(h).unwrap_or(JsonValue::Null)
                }
            })
            .collect();
        Ok(json!({
            "tool": "get_hub_nodes",
            "parameters": { "top_n": top_n, "min_degree": min_degree,
                            "detail_level": if minimal { "minimal" } else { "standard" } },
            "result": {
                "status": "ok",
                "summary": format!("{total} hub node(s) ranked by degree{}", shown_of(rows.len(), total)),
                "hub_nodes": rows, "count": rows.len(), "total": total,
                "truncated": rows.len() < total,
            }
        }))
    }

    /// Execute get_bridge_nodes - betweenness centrality chokepoints
    async fn execute_get_bridge_nodes(&self, params: JsonValue) -> Result<JsonValue> {
        let top_n = params["top_n"].as_i64().unwrap_or(10).clamp(1, 100) as usize;
        let minimal = params["detail_level"].as_str() == Some("minimal");
        let mut bridges = self
            .graph_query
            .bridge_nodes()
            .await
            .map_err(|e| McpError::Protocol(format!("get_bridge_nodes failed: {e:#}")))?;
        let total = bridges.len();
        bridges.truncate(top_n);
        let rows: Vec<JsonValue> = bridges
            .iter()
            .map(|b| {
                if minimal {
                    json!({"name": b.name, "kind": b.kind, "betweenness": b.betweenness})
                } else {
                    serde_json::to_value(b).unwrap_or(JsonValue::Null)
                }
            })
            .collect();
        Ok(json!({
            "tool": "get_bridge_nodes",
            "parameters": { "top_n": top_n,
                            "detail_level": if minimal { "minimal" } else { "standard" } },
            "result": {
                "status": "ok",
                "summary": format!("{total} bridge node(s) ranked by betweenness{}", shown_of(rows.len(), total)),
                "bridge_nodes": rows, "count": rows.len(), "total": total,
                "truncated": rows.len() < total,
            }
        }))
    }

    /// Execute get_knowledge_gaps - structural weaknesses by category, CRG-shaped
    async fn execute_get_knowledge_gaps(&self, params: JsonValue) -> Result<JsonValue> {
        const MINIMAL_FIELDS: [&str; 6] = [
            "name",
            "qualified_name",
            "community_id",
            "size",
            "degree",
            "load_time",
        ];
        let max = params["max_per_category"]
            .as_i64()
            .unwrap_or(15)
            .clamp(1, 50) as usize;
        let minimal = params["detail_level"].as_str() == Some("minimal");
        let raw = self
            .graph_query
            .knowledge_gaps()
            .await
            .map_err(|e| McpError::Protocol(format!("get_knowledge_gaps failed: {e:#}")))?;
        let raw = serde_json::to_value(&raw)?;
        let mut summary = serde_json::Map::new();
        let mut gaps = serde_json::Map::new();
        let mut truncated = false;
        let mut total_gaps = 0usize;
        for (category, rows) in raw.as_object().into_iter().flatten() {
            let rows = rows.as_array().cloned().unwrap_or_default();
            total_gaps += rows.len();
            summary.insert(category.clone(), json!(rows.len()));
            truncated |= rows.len() > max;
            let visible: Vec<JsonValue> = rows
                .into_iter()
                .take(max)
                .map(|row| {
                    if !minimal {
                        return row;
                    }
                    let kept: serde_json::Map<String, JsonValue> = MINIMAL_FIELDS
                        .iter()
                        .filter_map(|k| row.get(*k).map(|v| (k.to_string(), v.clone())))
                        .collect();
                    JsonValue::Object(kept)
                })
                .collect();
            gaps.insert(category.clone(), JsonValue::Array(visible));
        }
        Ok(json!({
            "tool": "get_knowledge_gaps",
            "parameters": { "max_per_category": max,
                            "detail_level": if minimal { "minimal" } else { "standard" } },
            "result": {
                "status": "ok",
                "summary": summary, "gaps": gaps, "total_gaps": total_gaps,
                "truncated": truncated,
            }
        }))
    }

    /// Execute get_suggested_questions - review prompts from the graph analyses
    async fn execute_get_suggested_questions(&self) -> Result<JsonValue> {
        let questions =
            self.graph_query.suggested_questions().await.map_err(|e| {
                McpError::Protocol(format!("get_suggested_questions failed: {e:#}"))
            })?;
        let count_of = |p: &str| questions.iter().filter(|q| q.priority == p).count();
        Ok(json!({
            "tool": "get_suggested_questions",
            "parameters": {},
            "result": {
                "status": "ok",
                "summary": format!("{} review question(s)", questions.len()),
                "questions": questions,
                "count": questions.len(),
                "by_priority": { "high": count_of("high"), "medium": count_of("medium"),
                                 "low": count_of("low") },
            }
        }))
    }

    /// Execute get_surprising_connections - edges ranked by surprise score
    async fn execute_get_surprising_connections(&self, params: JsonValue) -> Result<JsonValue> {
        let top_n = params["top_n"].as_i64().unwrap_or(15).clamp(1, 100) as usize;
        let minimal = params["detail_level"].as_str() == Some("minimal");
        let mut surprises = self
            .graph_query
            .surprising_connections()
            .await
            .map_err(|e| McpError::Protocol(format!("get_surprising_connections failed: {e:#}")))?;
        let total = surprises.len();
        surprises.truncate(top_n);
        let rows: Vec<JsonValue> = surprises
            .iter()
            .map(|s| {
                if minimal {
                    json!({"source": s.source, "target": s.target, "edge_kind": s.edge_kind,
                           "surprise_score": s.surprise_score})
                } else {
                    serde_json::to_value(s).unwrap_or(JsonValue::Null)
                }
            })
            .collect();
        Ok(json!({
            "tool": "get_surprising_connections",
            "parameters": { "top_n": top_n,
                            "detail_level": if minimal { "minimal" } else { "standard" } },
            "result": {
                "status": "ok",
                "summary": format!("{total} surprising connection(s) ranked by surprise score{}",
                                   shown_of(rows.len(), total)),
                "surprising_connections": rows, "count": rows.len(), "total": total,
                "truncated": rows.len() < total,
            }
        }))
    }

    /// Execute get_reverse_dependencies
    async fn execute_get_reverse_dependencies(&self, params: JsonValue) -> Result<JsonValue> {
        let node_id = params["node_id"]
            .as_str()
            .ok_or_else(|| McpError::Protocol("Missing node_id".to_string()))?;

        // Default to "Calls" if edge_type not provided (for LATS compatibility)
        let edge_type = params["edge_type"].as_str().unwrap_or("Calls");

        let depth = params["depth"].as_i64().unwrap_or(3) as i32;

        let result = self
            .graph_query
            .get_reverse_dependencies(node_id, edge_type, depth)
            .await
            .map_err(|e| McpError::Protocol(format!("get_reverse_dependencies failed: {}", e)))?;

        Ok(json!({
            "tool": "get_reverse_dependencies",
            "parameters": {
                "node_id": node_id,
                "edge_type": edge_type,
                "depth": depth
            },
            "result": result
        }))
    }

    /// Execute find_complexity_hotspots - find functions with high complexity and coupling
    async fn execute_find_complexity_hotspots(&self, params: JsonValue) -> Result<JsonValue> {
        let min_complexity = params["min_complexity"].as_f64().unwrap_or(5.0) as f32;
        let limit = params["limit"].as_i64().unwrap_or(20) as i32;

        let result = self
            .graph_query
            .get_complexity_hotspots(min_complexity, limit)
            .await
            .map_err(|e| McpError::Protocol(format!("find_complexity_hotspots failed: {}", e)))?;

        Ok(json!({
            "tool": "find_complexity_hotspots",
            "parameters": {
                "min_complexity": min_complexity,
                "limit": limit
            },
            "result": result
        }))
    }

    /// Get all available tool schemas for registration
    pub fn get_tool_schemas() -> Vec<crate::ToolSchema> {
        GraphToolSchemas::all()
    }

    /// Get tool names for listing
    pub fn get_tool_names() -> Vec<String> {
        GraphToolSchemas::tool_names()
    }
}

fn log_tool_call_start(tool_name: &str, parameters: &JsonValue) {
    info!(
        target: TOOL_PROGRESS_LOG_TARGET,
        tool = tool_name,
        "Tool call started"
    );
    debug!(
        target: TOOL_PROGRESS_LOG_TARGET,
        tool = tool_name,
        "Tool input payload: {}",
        parameters
    );

    // Debug logging to file if enabled
    DebugLogger::log_tool_start(tool_name, parameters);
}

fn log_tool_call_finish(tool_name: &str, result: &JsonValue) {
    info!(
        target: TOOL_PROGRESS_LOG_TARGET,
        tool = tool_name,
        result_bytes = result.to_string().len(),
        "Tool call completed"
    );
    debug!(
        target: TOOL_PROGRESS_LOG_TARGET,
        tool = tool_name,
        "Tool output payload: {}",
        result
    );

    // Debug logging to file if enabled
    DebugLogger::log_tool_finish(tool_name, result);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shorten_content_keeps_location_and_marks_truncation() {
        let long = (0..200)
            .map(|i| format!("line {i} of a long document"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut result = json!({
            "tool": "trace_call_chain",
            "result": [
                { "name": "doc", "file_path": "README.md", "start_line": 1, "content": long },
                { "name": "small", "content": "fn small() {}" },
                { "node": { "name": "nested", "content": "x".repeat(5_000) } }
            ]
        });

        shorten_content(&mut result, 300);

        let rows = result["result"].as_array().unwrap();
        let snippet = rows[0]["content"].as_str().unwrap();
        assert!(snippet.chars().count() <= 300, "{}", snippet.len());
        assert!(
            snippet.ends_with("of a long document"),
            "ends on a full line"
        );
        assert_eq!(rows[0]["content_truncated"], true);
        assert!(rows[0]["content_chars"].as_u64().unwrap() > 4_000);
        assert_eq!(rows[0]["file_path"], "README.md");
        assert_eq!(rows[1]["content"], "fn small() {}");
        assert!(rows[1].get("content_truncated").is_none());
        assert_eq!(rows[2]["node"]["content"].as_str().unwrap().len(), 300);
        assert_eq!(rows[2]["node"]["content_truncated"], true);
    }

    #[test]
    fn test_shorten_content_respects_char_boundaries_and_zero_disables() {
        let mut result = json!({ "content": "é".repeat(50) });
        shorten_content(&mut result, 10);
        assert_eq!(result["content"].as_str().unwrap().chars().count(), 10);

        let mut untouched = json!({ "content": "y".repeat(50) });
        shorten_content(&mut untouched, 0);
        assert_eq!(untouched["content"].as_str().unwrap().len(), 50);
    }

    #[test]
    fn test_tool_schemas_available() {
        let schemas = GraphToolExecutor::get_tool_schemas();
        assert_eq!(schemas.len(), 27);
    }

    #[test]
    fn test_tool_names() {
        let names = GraphToolExecutor::get_tool_names();
        assert_eq!(names.len(), 27);
        assert!(names.contains(&"get_review_context".to_string()));
        assert!(names.contains(&"detect_changes".to_string()));
        assert!(names.contains(&"get_impact_radius".to_string()));
        assert!(names.contains(&"find_large_functions".to_string()));
        assert!(names.contains(&"query_graph".to_string()));
        assert!(names.contains(&"traverse_graph".to_string()));
        assert!(names.contains(&"search_nodes".to_string()));
        assert!(names.contains(&"list_graph_stats".to_string()));
        assert!(names.contains(&"get_transitive_dependencies".to_string()));
        assert!(names.contains(&"find_complexity_hotspots".to_string()));
    }

    #[test]
    fn test_parameter_extraction() {
        let params = json!({
            "node_id": "nodes:123",
            "edge_type": "Calls",
            "depth": 5
        });

        assert_eq!(params["node_id"].as_str().unwrap(), "nodes:123");
        assert_eq!(params["edge_type"].as_str().unwrap(), "Calls");
        assert_eq!(params["depth"].as_i64().unwrap(), 5);
    }

    // === Cache Tests ===

    #[test]
    fn test_cache_key_generation() {
        let params1 = json!({
            "node_id": "nodes:123",
            "edge_type": "Calls",
            "depth": 3
        });
        let params2 = json!({
            "node_id": "nodes:123",
            "edge_type": "Calls",
            "depth": 3
        });
        let params3 = json!({
            "node_id": "nodes:456",
            "edge_type": "Calls",
            "depth": 3
        });

        let project = "proj-a";

        let key1 =
            GraphToolExecutor::cache_key(project, (1, 0), "get_transitive_dependencies", &params1);
        let key2 =
            GraphToolExecutor::cache_key(project, (1, 0), "get_transitive_dependencies", &params2);
        let key3 =
            GraphToolExecutor::cache_key(project, (1, 0), "get_transitive_dependencies", &params3);

        // Same params should generate same key
        assert_eq!(key1, key2);
        // Different params should generate different key
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_cache_key_includes_project_scope() {
        let params = json!({
            "node_id": "nodes:123"
        });

        let key_a = GraphToolExecutor::cache_key("proj-a", (1, 0), "get_hub_nodes", &params);
        let key_b = GraphToolExecutor::cache_key("proj-b", (1, 0), "get_hub_nodes", &params);

        assert_ne!(key_a, key_b);
        assert!(
            key_a.starts_with("proj-a@"),
            "Project scope should prefix cache key"
        );
    }

    #[test]
    fn test_cache_stats_initialization() {
        let stats = CacheStats {
            hits: 0,
            misses: 0,
            evictions: 0,
            current_size: 0,
            max_size: 100,
        };

        assert_eq!(stats.hit_rate(), 0.0);
    }

    #[test]
    fn test_cache_stats_hit_rate_calculation() {
        let stats = CacheStats {
            hits: 75,
            misses: 25,
            evictions: 5,
            current_size: 50,
            max_size: 100,
        };

        assert_eq!(stats.hit_rate(), 75.0);
    }

    #[test]
    fn test_cache_stats_hit_rate_no_requests() {
        let stats = CacheStats {
            hits: 0,
            misses: 0,
            evictions: 0,
            current_size: 0,
            max_size: 100,
        };

        assert_eq!(stats.hit_rate(), 0.0);
    }

    #[test]
    fn test_log_tool_call_start_captures_info_and_debug() {
        let logs = capture_logs(|| {
            let params = serde_json::json!({
                "node_id": "nodes:123",
                "edge_type": "Calls"
            });
            log_tool_call_start("get_transitive_dependencies", &params);
        });

        assert!(logs.contains("Tool call started"));
        assert!(logs.contains("Tool input payload"));
    }

    #[test]
    fn test_log_tool_call_finish_captures_info_and_debug() {
        let logs = capture_logs(|| {
            let result = serde_json::json!({
                "tool": "detect_cycles",
                "result": "ok"
            });
            log_tool_call_finish("detect_cycles", &result);
        });

        assert!(logs.contains("Tool call completed"));
        assert!(logs.contains("Tool output payload"));
    }

    fn capture_logs<F>(f: F) -> String
    where
        F: FnOnce(),
    {
        use std::io::Write;
        use std::sync::{Arc, Mutex};
        use tracing::subscriber::with_default;
        use tracing_subscriber::EnvFilter;

        #[derive(Clone)]
        struct BufferWriter {
            inner: Arc<Mutex<Vec<u8>>>,
        }

        impl BufferWriter {
            fn new() -> Self {
                Self {
                    inner: Arc::new(Mutex::new(Vec::new())),
                }
            }

            fn into_string(&self) -> String {
                let bytes = self.inner.lock().unwrap().clone();
                String::from_utf8(bytes).unwrap()
            }
        }

        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufferWriter {
            type Writer = BufferGuard;

            fn make_writer(&'a self) -> Self::Writer {
                BufferGuard {
                    inner: self.inner.clone(),
                }
            }
        }

        struct BufferGuard {
            inner: Arc<Mutex<Vec<u8>>>,
        }

        impl Write for BufferGuard {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.inner.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let writer = BufferWriter::new();

        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new("debug"))
            .with_ansi(false)
            .without_time()
            .with_writer(writer.clone())
            .finish();

        with_default(subscriber, f);

        writer.into_string()
    }

    fn insert_node(store: &codegraph_graph::sqlite_store::SqliteGraphStore, name: &str) {
        store
            .with_conn_blocking(|conn| {
                conn.execute_batch(&format!(
                    "INSERT INTO nodes (kind, name, qualified_name, file_path, line_start, \
                     line_end, extra, updated_at) \
                     VALUES ('Function', '{name}', 'm.py::{name}', 'm.py', 1, 2, '{{}}', 0)"
                ))?;
                Ok(())
            })
            .unwrap();
    }

    fn hit_names(result: &JsonValue) -> Vec<String> {
        let mut names: Vec<String> = result["result"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["name"].as_str().unwrap().to_string())
            .collect();
        names.sort();
        names
    }

    /// A cached answer is not served once the store changes, whether the write came
    /// through the executor's own connection or another one (`codegraph update`).
    #[tokio::test]
    async fn cached_results_follow_database_changes() {
        use codegraph_graph::sqlite_store::SqliteGraphStore;
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteGraphStore::open(dir.path()).unwrap();
        insert_node(&store, "alpha_one");
        let executor =
            GraphToolExecutor::with_cache(Arc::new(GraphQuery::new(store.clone())), true, 10);
        let params = json!({ "name": "alpha" });

        let first = executor
            .execute("find_nodes_by_name", params.clone())
            .await
            .unwrap();
        assert_eq!(hit_names(&first), ["alpha_one"]);
        executor
            .execute("find_nodes_by_name", params.clone())
            .await
            .unwrap();
        assert_eq!(
            executor.cache_stats().hits,
            1,
            "unchanged store answers from cache"
        );

        // Same connection: data_version stays, total_changes moves.
        insert_node(&store, "alpha_two");
        let second = executor
            .execute("find_nodes_by_name", params.clone())
            .await
            .unwrap();
        assert_eq!(hit_names(&second), ["alpha_one", "alpha_two"]);

        // Another connection: data_version moves.
        let other = SqliteGraphStore::open(dir.path()).unwrap();
        insert_node(&other, "alpha_three");
        let third = executor
            .execute("find_nodes_by_name", params)
            .await
            .unwrap();
        assert_eq!(hit_names(&third), ["alpha_one", "alpha_three", "alpha_two"]);
        assert_eq!(executor.cache_stats().hits, 1);
    }

    #[tokio::test]
    async fn suggested_questions_on_an_empty_database() {
        use codegraph_graph::sqlite_store::SqliteGraphStore;
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteGraphStore::open(dir.path()).unwrap();
        let executor = GraphToolExecutor::with_cache(Arc::new(GraphQuery::new(store)), false, 10);
        let out = executor
            .execute("get_suggested_questions", json!({}))
            .await
            .unwrap();
        let result = &out["result"];
        assert_eq!(result["questions"], json!([]));
        assert_eq!(result["count"], 0);
        assert_eq!(
            result["by_priority"],
            json!({ "high": 0, "medium": 0, "low": 0 })
        );
    }

    /// `detail_level=minimal` keeps `qualified_name`, which the node tools accept as node_id.
    #[tokio::test]
    async fn minimal_hubs_keep_a_chainable_qualified_name() {
        use codegraph_graph::sqlite_store::SqliteGraphStore;
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteGraphStore::open(dir.path()).unwrap();
        for name in ["hub", "a", "b", "c"] {
            insert_node(&store, name);
        }
        store
            .with_conn_blocking(|conn| {
                for caller in ["a", "b", "c"] {
                    conn.execute_batch(&format!(
                        "INSERT INTO edges (kind, source_qualified, target_qualified, \
                         file_path, line, updated_at) \
                         VALUES ('CALLS', 'm.py::{caller}', 'm.py::hub', 'm.py', 1, 0)"
                    ))?;
                }
                Ok(())
            })
            .unwrap();
        let executor = GraphToolExecutor::with_cache(Arc::new(GraphQuery::new(store)), false, 10);
        let out = executor
            .execute(
                "get_hub_nodes",
                json!({ "detail_level": "minimal", "top_n": 1 }),
            )
            .await
            .unwrap();
        let hub = &out["result"]["hub_nodes"][0];
        assert_eq!(hub["qualified_name"], "m.py::hub", "{out}");
        let callers = executor
            .execute(
                "get_reverse_dependencies",
                json!({ "node_id": hub["qualified_name"], "edge_type": "Calls", "depth": 1 }),
            )
            .await
            .unwrap();
        assert_eq!(
            callers["result"].as_array().map(Vec::len),
            Some(3),
            "{callers}"
        );
    }
}

/// Result for a call whose `name` matched no node.
fn no_match(tool: &str, params: &JsonValue) -> JsonValue {
    json!({
        "tool": tool,
        "parameters": { "name": params["name"] },
        "result": null,
        "message": "no node matches the given name"
    })
}
