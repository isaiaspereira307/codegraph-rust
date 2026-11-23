// ABOUTME: Result types returned by the graph tools (dependencies, call chains, hubs, ...).
// ABOUTME: Serialized as-is into MCP/CLI tool responses.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyNode {
    pub id: String,
    pub name: String,
    pub kind: Option<String>,
    pub location: Option<NodeLocation>,
    pub language: Option<String>,
    #[serde(
        default,
        deserialize_with = "codegraph_core::deserialize_content_string"
    )]
    pub content: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub dependency_depth: Option<i32>,
    pub dependent_depth: Option<i32>,
}

/// Circular dependency pair
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircularDependency {
    pub node1_id: String,
    pub node2_id: String,
    pub node1: NodeInfo,
    pub node2: NodeInfo,
    pub dependency_type: String,
}

/// Call chain node with caller information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallChainNode {
    pub id: String,
    pub name: String,
    pub kind: Option<String>,
    pub location: Option<NodeLocation>,
    pub language: Option<String>,
    #[serde(
        default,
        deserialize_with = "codegraph_core::deserialize_content_string"
    )]
    pub content: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub call_depth: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_path: Option<Vec<String>>,
    pub called_by: Option<Vec<CallerInfo>>,
}

/// Coupling metrics result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouplingMetricsResult {
    pub node: NodeInfo,
    pub metrics: CouplingMetrics,
    pub dependents: Vec<NodeReference>,
    pub dependencies: Vec<NodeReference>,
}

/// Coupling metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouplingMetrics {
    pub afferent_coupling: i32,
    pub efferent_coupling: i32,
    pub total_coupling: i32,
    pub instability: f64,
    pub stability: f64,
    pub is_stable: bool,
    pub is_unstable: bool,
    pub coupling_category: String,
}

/// Hub node with degree information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubNode {
    pub node_id: String,
    pub node: NodeInfo,
    pub afferent_degree: i32,
    pub efferent_degree: i32,
    pub total_degree: i32,
    pub incoming_by_type: Vec<EdgeTypeCount>,
    pub outgoing_by_type: Vec<EdgeTypeCount>,
}

/// Directory summary for bootstrap
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectorySummary {
    pub dir_id: String,
    pub name: String,
    pub full_path: String,
    pub depth: Option<i32>,
    pub file_count: i32,
}

/// Edge type count
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeTypeCount {
    pub edge_type: String,
    pub count: i32,
}

/// Node information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub id: String,
    pub name: String,
    pub kind: Option<String>,
    pub location: Option<NodeLocation>,
    pub language: Option<String>,
    #[serde(
        default,
        deserialize_with = "codegraph_core::deserialize_content_string"
    )]
    pub content: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

/// Node reference (minimal info)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeReference {
    pub id: String,
    pub name: String,
    pub kind: Option<String>,
    pub location: Option<NodeLocation>,
}

/// Caller information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallerInfo {
    pub id: String,
    pub name: String,
    pub kind: Option<String>,
}

/// Node location information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeLocation {
    pub file_path: String,
    pub start_line: Option<i32>,
    pub end_line: Option<i32>,
}

/// Complexity hotspot with risk metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityHotspot {
    pub id: String,
    pub name: String,
    pub kind: Option<String>,
    pub language: Option<String>,
    pub file_path: Option<String>,
    pub start_line: Option<i32>,
    pub end_line: Option<i32>,
    pub complexity: f32,
    pub afferent_coupling: i32,
    pub efferent_coupling: i32,
    pub instability: f32,
    pub risk_score: f32,
}

/// Graph-wide counts, read with GROUP BY over the SQLite tables.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphStats {
    pub total_nodes: usize,
    pub total_edges: usize,
    /// Node count per kind (File, Function, Class, Test, ...), sorted by kind.
    pub nodes_by_kind: BTreeMap<String, usize>,
    /// Edge count per kind (CALLS, IMPORTS_FROM, CONTAINS, ...), sorted by kind.
    pub edges_by_kind: BTreeMap<String, usize>,
    /// Distinct file paths that own at least one node.
    pub files: usize,
    /// Distinct non-empty node languages, sorted.
    pub languages: Vec<String>,
    /// `metadata.last_updated` as written by the last finalize; `None` on an empty database.
    pub last_updated: Option<String>,
}
