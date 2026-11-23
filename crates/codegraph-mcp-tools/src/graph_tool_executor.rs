// ABOUTME: Tool executor for the graph analysis tools over the SQLite graph
// ABOUTME: Validates tool parameters, runs GraphQuery calls and caches their results

use codegraph_graph::graph_query::GraphQuery;
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

    /// Generate a cache key from project, tool name, and parameters
    fn cache_key(project_id: &str, tool_name: &str, parameters: &JsonValue) -> String {
        // Create deterministic key from project + function name + serialized params
        format!("{}:{}:{}", project_id, tool_name, parameters.to_string())
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

            // Check cache if enabled
            if self.cache_enabled {
                let cache_key = Self::cache_key(&project_id, tool_name, &parameters);

                // Try cache lookup
                {
                    let mut cache = self.cache.lock();
                    if let Some(cached_result) = cache.get(&cache_key) {
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
            if self.cache_enabled {
                let cache_key = Self::cache_key(&project_id, tool_name, &parameters);
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

    /// Execute get_hub_nodes
    async fn execute_get_hub_nodes(&self, params: JsonValue) -> Result<JsonValue> {
        let min_degree = params["min_degree"].as_i64().unwrap_or(5) as i32;

        let result = self
            .graph_query
            .get_hub_nodes(min_degree)
            .await
            .map_err(|e| McpError::Protocol(format!("get_hub_nodes failed: {}", e)))?;

        Ok(json!({
            "tool": "get_hub_nodes",
            "parameters": {
                "min_degree": min_degree
            },
            "result": result
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
        assert_eq!(schemas.len(), 10);
    }

    #[test]
    fn test_tool_names() {
        let names = GraphToolExecutor::get_tool_names();
        assert_eq!(names.len(), 10);
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

        let key1 = GraphToolExecutor::cache_key(project, "get_transitive_dependencies", &params1);
        let key2 = GraphToolExecutor::cache_key(project, "get_transitive_dependencies", &params2);
        let key3 = GraphToolExecutor::cache_key(project, "get_transitive_dependencies", &params3);

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

        let key_a = GraphToolExecutor::cache_key("proj-a", "get_hub_nodes", &params);
        let key_b = GraphToolExecutor::cache_key("proj-b", "get_hub_nodes", &params);

        assert_ne!(key_a, key_b);
        assert!(
            key_a.starts_with("proj-a:"),
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
}
