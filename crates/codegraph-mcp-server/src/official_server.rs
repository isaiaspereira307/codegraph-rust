// ABOUTME: MCP server implementation for CodeGraph code intelligence tools
// ABOUTME: Exposes deterministic graph analysis tools over the MCP protocol (no LLM in the loop)

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{
        CallToolResult, ContentBlock as Content, GetPromptRequestParams, GetPromptResponse,
        GetPromptResult, ListPromptsResult, PaginatedRequestParams, Prompt, PromptMessage,
        Role as PromptMessageRole, ServerCapabilities, ServerConfig,
    },
    service::RequestContext,
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::OnceCell;

use crate::prompts::{INITIAL_INSTRUCTIONS, INITIAL_INSTRUCTIONS_PROMPT_NAME};
use codegraph_graph::graph_query::GraphQuery;
use codegraph_graph::sqlite_store::{SqliteGraphStore, db_path};
use codegraph_mcp_tools::GraphToolExecutor;

/// Dependency relationship followed by the graph traversal tools.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub enum EdgeKind {
    Calls,
    Imports,
    Uses,
    Extends,
    Implements,
    References,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct FindNodesRequest {
    /// Symbol name or fragment to search for (e.g. "parse_file", "Config")
    pub name: String,
    /// Maximum number of results (1-50, default 10)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SearchNodesRequest {
    /// Words to search for; every word must match (e.g. "parse file")
    pub query: String,
    /// Maximum number of results (1-50, default 10)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct FindLargeFunctionsRequest {
    /// Minimum number of lines in the function (default 50)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_lines: Option<u32>,
    /// Maximum number of results (1-100, default 20)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// `list_graph_stats` takes no parameters.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ListGraphStatsRequest {}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct DependencyRequest {
    /// Node ID returned by find_nodes_by_name
    pub node_id: String,
    /// Relationship to follow
    pub edge_type: EdgeKind,
    /// Maximum traversal depth (1-5, default 3)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CycleRequest {
    /// Relationship to check for cycles
    pub edge_type: EdgeKind,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CallChainRequest {
    /// Node ID of the function to start from
    pub from_node: String,
    /// Maximum call depth (1-10, default 5)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct NodeRequest {
    /// Node ID returned by find_nodes_by_name
    pub node_id: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct HubRequest {
    /// Minimum incoming + outgoing connections to qualify as a hub (default 5)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_degree: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct HotspotRequest {
    /// Minimum cyclomatic complexity (default 5.0)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_complexity: Option<f64>,
    /// Maximum number of results (1-100, default 20)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// CodeGraph MCP server. The project store is opened on the first tool call and kept
/// for the lifetime of the server.
#[derive(Clone)]
pub struct CodeGraphMCPServer {
    executor: Arc<OnceCell<Arc<GraphToolExecutor>>>,
}

impl Default for CodeGraphMCPServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router]
impl CodeGraphMCPServer {
    pub fn new() -> Self {
        Self {
            executor: Arc::new(OnceCell::new()),
        }
    }

    #[tool(
        description = "Find code nodes whose name contains the given text. Returns node IDs, names, kinds and file locations. Use this first to obtain the node_id the other tools need."
    )]
    async fn find_nodes_by_name(
        &self,
        params: Parameters<FindNodesRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("find_nodes_by_name", &params.0).await
    }

    #[tool(
        description = "Full-text search over code nodes (names, paths, signatures, docstrings) with stemming and camelCase splitting. Ranks name matches first. Returns node IDs and locations."
    )]
    async fn search_nodes(
        &self,
        params: Parameters<SearchNodesRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("search_nodes", &params.0).await
    }

    #[tool(
        description = "Graph-wide counts: nodes and edges per kind, number of files, languages and the time of the last build. Takes no parameters."
    )]
    async fn list_graph_stats(
        &self,
        params: Parameters<ListGraphStatsRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("list_graph_stats", &params.0).await
    }

    #[tool(
        description = "List functions spanning at least `min_lines` lines (default 50), longest first. Returns node IDs, locations and line counts."
    )]
    async fn find_large_functions(
        &self,
        params: Parameters<FindLargeFunctionsRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("find_large_functions", &params.0).await
    }

    #[tool(
        description = "List what a node depends on, following the given relationship recursively up to `depth`."
    )]
    async fn get_transitive_dependencies(
        &self,
        params: Parameters<DependencyRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("get_transitive_dependencies", &params.0)
            .await
    }

    #[tool(
        description = "List what depends on a node (its dependents), following the given relationship backwards up to `depth`. Use before changing a symbol to see what it affects."
    )]
    async fn get_reverse_dependencies(
        &self,
        params: Parameters<DependencyRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("get_reverse_dependencies", &params.0).await
    }

    #[tool(description = "Find dependency cycles for the given relationship.")]
    async fn detect_circular_dependencies(
        &self,
        params: Parameters<CycleRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("detect_circular_dependencies", &params.0)
            .await
    }

    #[tool(description = "Follow the calls made from a function, up to `max_depth`.")]
    async fn trace_call_chain(
        &self,
        params: Parameters<CallChainRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("trace_call_chain", &params.0).await
    }

    #[tool(
        description = "Coupling metrics for a node: afferent (incoming), efferent (outgoing) and instability Ce/(Ce+Ca)."
    )]
    async fn calculate_coupling_metrics(
        &self,
        params: Parameters<NodeRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("calculate_coupling_metrics", &params.0)
            .await
    }

    #[tool(
        description = "Highly connected nodes (degree >= min_degree), sorted by total degree. Finds central components and god objects."
    )]
    async fn get_hub_nodes(
        &self,
        params: Parameters<HubRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("get_hub_nodes", &params.0).await
    }

    #[tool(
        description = "Functions ranked by risk (cyclomatic complexity x afferent coupling). Use to prioritize review and refactoring."
    )]
    async fn find_complexity_hotspots(
        &self,
        params: Parameters<HotspotRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("find_complexity_hotspots", &params.0).await
    }
}

impl CodeGraphMCPServer {
    async fn call_tool(
        &self,
        name: &str,
        params: &impl Serialize,
    ) -> Result<CallToolResult, McpError> {
        let params = serde_json::to_value(params)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        let result = self.call_graph_tool(name, params).await?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result)
                .unwrap_or_else(|_| "Error formatting tool result".to_string()),
        )]))
    }

    /// Run a graph tool by name without an MCP transport (used by `codegraph tool`).
    pub async fn call_graph_tool(&self, name: &str, params: Value) -> Result<Value, McpError> {
        let executor = self
            .executor
            .get_or_try_init(open_executor)
            .await
            .map_err(|e| McpError::internal_error(e, None))?;
        executor
            .execute(name, params)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))
    }
}

/// Open the project's graph database and build the tool executor.
///
/// Reads `<cwd>/.code-review-graph/graph.db` (created empty if absent), the database
/// `codegraph index` and code-review-graph write.
async fn open_executor() -> Result<Arc<GraphToolExecutor>, String> {
    let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let store = SqliteGraphStore::open(&project_root)
        .map_err(|e| format!("Failed to open graph database: {e:#}"))?;
    let query = Arc::new(GraphQuery::new(store));
    match query.count_nodes_for_project().await {
        Ok(0) => tracing::warn!(
            "{} has zero nodes indexed; run `codegraph index` in this project.",
            db_path(&project_root).display()
        ),
        Ok(count) => tracing::info!("{} has {} indexed nodes", project_root.display(), count),
        Err(e) => tracing::warn!("Could not verify project data presence: {e}"),
    }
    Ok(Arc::new(GraphToolExecutor::new(query)))
}

#[tool_handler]
impl ServerHandler for CodeGraphMCPServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .build(),
        )
        .with_instructions(INITIAL_INSTRUCTIONS)
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, McpError>> + Send + '_ {
        async move {
            Ok(ListPromptsResult {
                prompts: vec![initial_instructions_prompt()],
                next_cursor: None,
                ..Default::default()
            })
        }
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetPromptResponse, McpError>> + Send + '_ {
        let name = request.name.clone();
        async move {
            match name.as_str() {
                INITIAL_INSTRUCTIONS_PROMPT_NAME => Ok(GetPromptResult::new(vec![
                    PromptMessage::new_text(
                        PromptMessageRole::User,
                        "Please read the CodeGraph usage guide below before using CodeGraph tools.",
                    ),
                    PromptMessage::new_text(PromptMessageRole::Assistant, INITIAL_INSTRUCTIONS),
                ])
                .with_description("CodeGraph usage guide: which graph tool answers which question")
                .into()),
                _ => Err(McpError::invalid_params(
                    format!("Unknown prompt: {}", name),
                    None,
                )),
            }
        }
    }
}

fn initial_instructions_prompt() -> Prompt {
    Prompt::new(
        INITIAL_INSTRUCTIONS_PROMPT_NAME,
        Some("CodeGraph usage guide: which graph tool answers which question"),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_instructions_prompt_name_is_mcp_compatible() {
        assert_eq!(
            initial_instructions_prompt().name,
            "codegraph:initial_instructions"
        );
    }

    #[test]
    fn every_graph_tool_is_exposed_over_mcp() {
        let mut exposed: Vec<String> = CodeGraphMCPServer::tool_router()
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        exposed.sort();
        let mut expected = GraphToolExecutor::get_tool_names();
        expected.sort();
        assert_eq!(exposed, expected);
    }

    #[test]
    fn optional_parameters_are_omitted_so_executor_defaults_apply() {
        let params = serde_json::to_value(DependencyRequest {
            node_id: "nodes:abc".into(),
            edge_type: EdgeKind::Calls,
            depth: None,
        })
        .unwrap();
        assert_eq!(
            params,
            serde_json::json!({"node_id": "nodes:abc", "edge_type": "Calls"})
        );
    }
}
