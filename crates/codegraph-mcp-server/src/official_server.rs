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
pub struct QueryGraphRequest {
    /// callers_of, callees_of, imports_of, importers_of, tests_for or inheritors_of
    pub pattern: String,
    /// Node ID from find_nodes_by_name or search_nodes; takes precedence over name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// Symbol name, used when node_id is not given
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Maximum number of results (1-100, default 20)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ReviewRequest {
    /// Commit, branch or tag to diff against (default HEAD)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_ref: Option<String>,
    /// Levels of dependents to walk (1-10, default 3)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
    /// Maximum impacted nodes (1-500, default 50)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_nodes: Option<u32>,
    /// Lines per changed-node snippet (1-100, default 20); get_review_context only
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_snippet_lines: Option<u32>,
    /// Add the 90-day git commit churn of each file to the risk score (default true)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_churn: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ListFlowsRequest {
    /// criticality (default), depth, node_count, file_count or name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_by: Option<String>,
    /// Maximum flows (1-200, default 50)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Only flows whose entry node has this kind (Function, Test)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// standard (default) or minimal
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_level: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct GetFlowRequest {
    /// Flow id from list_flows
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_id: Option<i64>,
    /// Partial, case-insensitive name; ignored when flow_id is given
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_name: Option<String>,
    /// Add numbered source to each step (default false)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_source: Option<bool>,
    /// Steps to return (1-200, default 50)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<u32>,
    /// Total source lines across steps (1-2000, default 400)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_source_lines: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct AffectedFlowsRequest {
    /// Repo-relative changed paths; detected from git when omitted
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_files: Option<Vec<String>>,
    /// Commit, branch or tag to diff against (default HEAD~1)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_ref: Option<String>,
    /// standard (default) or minimal
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_level: Option<String>,
    /// Flows to return (default 50, capped by detail level)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_flows: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ListCommunitiesRequest {
    /// size (default), cohesion or name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_by: Option<String>,
    /// Smallest community to include (default 0)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_size: Option<u32>,
    /// standard (default) or minimal
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_level: Option<String>,
    /// Communities to return (1-200, default 50)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
    /// Members listed per community (1-25, default 10)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_members: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct GetCommunityRequest {
    /// Community id from list_communities
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_id: Option<i64>,
    /// Partial, case-insensitive name; ignored when community_id is given
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_name: Option<String>,
    /// Add member node details (default false)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_members: Option<bool>,
    /// Members to list (1-25, default 25)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_members: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ArchitectureOverviewRequest {
    /// minimal (default) or standard
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_level: Option<String>,
    /// Cross-community rows and warnings (1-200, default 100)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
    /// standard only: members per community (1-25, default 10)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_members: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct DetectChangesRequest {
    /// Commit, branch or tag to diff against (default HEAD)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_ref: Option<String>,
    /// Add the 90-day git commit churn of each file to the risk score (default true)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_churn: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct GetImpactRadiusRequest {
    /// Changed node IDs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_ids: Option<Vec<String>>,
    /// Changed file paths (exact or suffix match)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<String>>,
    /// Levels of dependents to walk (1-10, default 3)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
    /// Maximum impacted nodes to return (1-1000, default 200)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_nodes: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TraverseGraphRequest {
    /// Node ID from find_nodes_by_name or search_nodes; takes precedence over name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// Symbol name, used when node_id is not given
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// out, in or both (default out)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    /// Single edge kind such as Calls; all dependency kinds when omitted
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_type: Option<String>,
    /// Levels to walk (1-10, default 3)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u32>,
    /// Maximum nodes to return (1-500, default 100)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_nodes: Option<u32>,
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
pub struct BuildOrUpdateGraphRequest {
    /// Project directory (default: the server's working directory)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Indexing tier: fast (default), balanced or full
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_tier: Option<String>,
}

const INDEX_TIERS: &[&str] = &["fast", "balanced", "full"];

/// Arguments for the `codegraph update` child process behind `build_or_update_graph`.
fn update_command_args(path: &str, index_tier: Option<&str>) -> Vec<String> {
    let mut args = vec!["update".to_string(), path.to_string()];
    if let Some(tier) = index_tier {
        args.push("--index-tier".to_string());
        args.push(tier.to_string());
    }
    args
}

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
    /// Hubs to return (1-100, default 10)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_n: Option<u32>,
    /// standard (default) or minimal (name, qualified_name, kind, total_degree)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_level: Option<String>,
    /// Optional: only nodes with at least this many incoming + outgoing connections
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_degree: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct BridgeRequest {
    /// Bridges to return (1-100, default 10)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_n: Option<u32>,
    /// standard (default) or minimal (name, kind, betweenness)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_level: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct KnowledgeGapsRequest {
    /// Entries per gap category (1-50, default 15)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_per_category: Option<u32>,
    /// standard (default) or minimal (identifying fields only, no file paths)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_level: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SurprisingRequest {
    /// Edges to return (1-100, default 15)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_n: Option<u32>,
    /// standard (default) or minimal (source, target, edge_kind, surprise_score)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_level: Option<String>,
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
        description = "Direct relationships of one node: callers_of, callees_of, imports_of, importers_of, tests_for or inheritors_of. Give node_id, or name for the best match. Returns the target, matches and `unresolved`: edges whose other end names no node, or, for inheritors_of, heritage edges ambiguous between this node and other same-named types."
    )]
    async fn query_graph(
        &self,
        params: Parameters<QueryGraphRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("query_graph", &params.0).await
    }

    #[tool(
        description = "Breadth-first walk from a node over dependency edges, out, in or both, bounded by max_depth and max_nodes. Returns reached nodes nearest first, with depth and a truncated flag."
    )]
    async fn traverse_graph(
        &self,
        params: Parameters<TraverseGraphRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("traverse_graph", &params.0).await
    }

    #[tool(
        description = "Blast radius of a change: transitive dependents of the given node_ids or files, grouped by depth, with affected files and the tests covering them."
    )]
    async fn get_impact_radius(
        &self,
        params: Parameters<GetImpactRadiusRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("get_impact_radius", &params.0).await
    }

    #[tool(
        description = "Graph nodes touched by the current git diff against base_ref (default HEAD): changed files, changed line ranges, the nodes whose spans overlap them with a 0-1 risk_score, the overall risk_score and the 10 riskiest as review_priorities."
    )]
    async fn detect_changes(
        &self,
        params: Parameters<DetectChangesRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("detect_changes", &params.0).await
    }

    #[tool(
        description = "Review context for the current git diff in one call: changed nodes by risk_score (0-1, with risk_signals), overall risk level, impact radius by depth, affected files, covering tests and snippets."
    )]
    async fn get_review_context(
        &self,
        params: Parameters<ReviewRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("get_review_context", &params.0).await
    }

    #[tool(
        description = "Compact review context for the current git diff: changed node IDs with risk_score (0-1), overall risk level, affected files and counts. No snippets or impact list."
    )]
    async fn get_minimal_context(
        &self,
        params: Parameters<ReviewRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("get_minimal_context", &params.0).await
    }

    #[tool(
        description = "List execution flows (call chains from entry points such as route handlers, CLI commands and main), most critical first."
    )]
    async fn list_flows(
        &self,
        params: Parameters<ListFlowsRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("list_flows", &params.0).await
    }

    #[tool(
        description = "One execution flow with its steps (function, file, lines); optionally numbered source. Pass flow_id or a partial flow_name."
    )]
    async fn get_flow(
        &self,
        params: Parameters<GetFlowRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("get_flow", &params.0).await
    }

    #[tool(
        description = "Execution flows that pass through the changed files (from git diff when changed_files is omitted), most critical first."
    )]
    async fn get_affected_flows(
        &self,
        params: Parameters<AffectedFlowsRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("get_affected_flows", &params.0).await
    }

    #[tool(
        description = "Architectural chokepoints: nodes with the highest betweenness centrality (on the shortest paths between many others)."
    )]
    async fn get_bridge_nodes(
        &self,
        params: Parameters<BridgeRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("get_bridge_nodes", &params.0).await
    }

    #[tool(
        description = "Structural weaknesses: isolated nodes, thin communities, untested hotspots, single-file communities and import cycles (load_time says whether a loop can fail as the files load)."
    )]
    async fn get_knowledge_gaps(
        &self,
        params: Parameters<KnowledgeGapsRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("get_knowledge_gaps", &params.0).await
    }

    #[tool(
        description = "Unexpected coupling: edges scored by cross-community, cross-language, peripheral-to-hub, cross-test-boundary and unusual edge kind, highest first."
    )]
    async fn get_surprising_connections(
        &self,
        params: Parameters<SurprisingRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("get_surprising_connections", &params.0)
            .await
    }

    #[tool(
        description = "Review questions generated from the graph: bridge nodes, untested hubs, cross-community surprises, thin communities and untested hotspots."
    )]
    async fn get_suggested_questions(&self) -> Result<CallToolResult, McpError> {
        self.call_tool("get_suggested_questions", &serde_json::json!({}))
            .await
    }

    #[tool(
        description = "List communities: clusters of closely related functions and classes with size, cohesion and a sample of members."
    )]
    async fn list_communities(
        &self,
        params: Parameters<ListCommunitiesRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("list_communities", &params.0).await
    }

    #[tool(
        description = "One community by community_id or partial community_name; include_members adds member node details."
    )]
    async fn get_community(
        &self,
        params: Parameters<GetCommunityRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("get_community", &params.0).await
    }

    #[tool(
        description = "Architecture overview from communities: the edges between them and warnings for heavily coupled pairs."
    )]
    async fn get_architecture_overview(
        &self,
        params: Parameters<ArchitectureOverviewRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.call_tool("get_architecture_overview", &params.0).await
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
        description = "Most connected nodes (in + out edges), highest total degree first. Finds central components and god objects. Each hub's qualified_name (kept in detail_level=minimal) is accepted as node_id by the other node tools."
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

    #[tool(
        description = "Build the project graph from scratch or update it incrementally (changed files only). Runs `codegraph update` in a child process with captured output; returns ok, exit code and the last output lines. Fails if another writer holds the graph lock. Use after editing, before detect_changes or get_review_context, when the graph may be stale."
    )]
    async fn build_or_update_graph(
        &self,
        params: Parameters<BuildOrUpdateGraphRequest>,
    ) -> Result<CallToolResult, McpError> {
        let request = params.0;
        if let Some(tier) = request
            .index_tier
            .as_deref()
            .filter(|tier| !INDEX_TIERS.contains(tier))
        {
            return Err(McpError::invalid_params(
                format!("index_tier must be one of fast, balanced, full (got {tier})"),
                None,
            ));
        }
        let path = match request.path {
            Some(path) => path,
            None => std::env::current_dir()
                .map_err(|e| McpError::internal_error(e.to_string(), None))?
                .to_string_lossy()
                .into_owned(),
        };
        let exe =
            std::env::current_exe().map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = tokio::process::Command::new(exe)
            .args(update_command_args(&path, request.index_tier.as_deref()))
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        // The child committed through another connection; the cache keys already move with
        // `data_version`, but drop the old entries rather than let them age out of the LRU.
        if let Some(executor) = self.executor.get() {
            executor.clear_cache();
        }

        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        let lines: Vec<&str> = text.lines().collect();
        let tail = lines[lines.len().saturating_sub(40)..].join("\n");
        let result = serde_json::json!({
            "ok": output.status.success(),
            "exit_code": output.status.code(),
            "path": path,
            "output_tail": tail,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result)
                .unwrap_or_else(|_| "Error formatting tool result".to_string()),
        )]))
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
        // Served by the server itself (writes the graph via a child process), not the executor.
        expected.push("build_or_update_graph".to_string());
        expected.sort();
        assert_eq!(exposed, expected);
    }

    #[test]
    fn build_or_update_graph_builds_update_args() {
        assert_eq!(update_command_args("/repo", None), vec!["update", "/repo"]);
        assert_eq!(
            update_command_args("/repo", Some("full")),
            vec!["update", "/repo", "--index-tier", "full"]
        );
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
