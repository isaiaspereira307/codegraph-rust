// ABOUTME: LLM tool schemas for graph analysis tools
// ABOUTME: JSON schemas for agentic tool calling - defines parameters and descriptions for LLM consumption

use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

/// Tool schema for LLM function calling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: JsonValue,
}

/// Collection of all graph analysis tool schemas
pub struct GraphToolSchemas;

impl GraphToolSchemas {
    /// Get all tool schemas for registration with LLM
    pub fn all() -> Vec<ToolSchema> {
        vec![
            Self::find_nodes_by_name(),
            Self::get_transitive_dependencies(),
            Self::detect_circular_dependencies(),
            Self::trace_call_chain(),
            Self::calculate_coupling_metrics(),
            Self::get_hub_nodes(),
            Self::get_reverse_dependencies(),
            Self::find_complexity_hotspots(),
            Self::search_nodes(),
            Self::list_graph_stats(),
            Self::find_large_functions(),
            Self::query_graph(),
        ]
    }

    /// Schema for query_graph (callers, callees, imports, importers and tests of one node)
    pub fn query_graph() -> ToolSchema {
        ToolSchema {
            name: "query_graph".to_string(),
            description: "Direct relationships of one node. Patterns: callers_of (CALLS into the \
                   node), callees_of (CALLS out of it), imports_of (files it imports), \
                   importers_of (files importing it), tests_for (tests that cover it). Give \
                   node_id, or name to pick the best match. Returns the target, matches sorted \
                   by name, the total, and how many text targets were ignored as unresolved."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "enum": ["callers_of", "callees_of", "imports_of", "importers_of", "tests_for"],
                        "description": "Relationship to follow"
                    },
                    "node_id": {
                        "type": "string",
                        "description": "Node ID returned by find_nodes_by_name or search_nodes"
                    },
                    "name": {
                        "type": "string",
                        "description": "Symbol name, used when node_id is not given; exact matches win"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results (1-100, defaults to 20)",
                        "minimum": 1,
                        "maximum": 100,
                        "default": 20
                    }
                },
                "required": ["pattern"]
            }),
        }
    }

    /// Schema for find_large_functions (functions by line span, longest first)
    pub fn find_large_functions() -> ToolSchema {
        ToolSchema {
            name: "find_large_functions".to_string(),
            description: "Functions whose body spans at least min_lines lines, longest first. \
                   Use it to find refactoring candidates. Returns node IDs, names, locations \
                   and line counts."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "min_lines": {
                        "type": "integer",
                        "description": "Minimum number of lines in the function (defaults to 50)",
                        "minimum": 1,
                        "default": 50
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results (1-100, defaults to 20)",
                        "minimum": 1,
                        "maximum": 100,
                        "default": 20
                    }
                },
                "required": []
            }),
        }
    }

    /// Schema for list_graph_stats (graph-wide counts; takes no parameters)
    pub fn list_graph_stats() -> ToolSchema {
        ToolSchema {
            name: "list_graph_stats".to_string(),
            description: "Graph-wide counts: nodes and edges per kind, number of files, \
                   languages and the time of the last build. Use it to check that the index \
                   exists and is fresh before relying on other graph tools."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
            }),
        }
    }

    /// Schema for search_nodes (full-text search over the FTS5 index)
    pub fn search_nodes() -> ToolSchema {
        ToolSchema {
            name: "search_nodes".to_string(),
            description: "Full-text search over code nodes: names, qualified names, file paths, \
                   signatures and docstrings, with stemming (\"parsing\" finds \"parse\") and \
                   camelCase splitting (\"password\" finds \"hashPassword\"). Ranks name matches \
                   first. Returns node IDs, names, kinds and locations."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Words to search for; every word must match (e.g., 'parse file')"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results (1-50, defaults to 10)",
                        "minimum": 1,
                        "maximum": 50,
                        "default": 10
                    }
                },
                "required": ["query"]
            }),
        }
    }

    /// Schema for find_nodes_by_name function (entrypoint for obtaining node IDs)
    pub fn find_nodes_by_name() -> ToolSchema {
        ToolSchema {
            name: "find_nodes_by_name".to_string(),
            description: "Find code nodes whose name contains the given text. \
                   Returns node IDs, names, kinds and file locations. \
                   Use this first to obtain the node_id required by the other graph tools."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Symbol name or fragment to search for (e.g., 'parse_file', 'Config')"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results (1-50, defaults to 10)",
                        "minimum": 1,
                        "maximum": 50,
                        "default": 10
                    }
                },
                "required": ["name"]
            }),
        }
    }

    /// Schema for get_transitive_dependencies function
    pub fn get_transitive_dependencies() -> ToolSchema {
        ToolSchema {
            name: "get_transitive_dependencies".to_string(),
            description: "Get all transitive dependencies of a code node up to specified depth. \
                Follows dependency edges recursively to find all nodes this node depends on. \
                Useful for impact analysis and understanding dependency chains."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_id": {
                        "type": "string",
                        "description": "The ID of the code node to analyze (e.g., 'nodes:123' or ID extracted from search results)"
                    },
                    "edge_type": {
                        "type": "string",
                        "description": "Type of dependency relationship to follow",
                    "enum": ["Calls", "Imports", "Uses", "Extends", "Implements", "References"]
                    },
                    "depth": {
                        "type": "integer",
                        "description": "Maximum traversal depth (1-10, defaults to 3 if not specified)",
                        "minimum": 1,
                    "maximum": 5,
                    "default": 3,
                    "description": "Maximum traversal depth (1-5). Values above 5 are clamped server-side."
                }
            },
                "required": ["node_id", "edge_type"]
            }),
        }
    }

    /// Schema for detect_circular_dependencies function
    pub fn detect_circular_dependencies() -> ToolSchema {
        ToolSchema {
            name: "detect_circular_dependencies".to_string(),
            description: "Detect circular dependencies in the codebase for a given edge type. \
                Returns pairs of nodes that have bidirectional relationships (A depends on B and B depends on A). \
                Critical for identifying architectural issues and potential cyclic import problems.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "edge_type": {
                        "type": "string",
                        "description": "Type of dependency relationship to analyze for cycles",
                        "enum": ["Calls", "Imports", "Uses", "Extends", "Implements", "References"]
                    }
                },
                "required": ["edge_type"]
            }),
        }
    }

    /// Schema for trace_call_chain function
    pub fn trace_call_chain() -> ToolSchema {
        ToolSchema {
            name: "trace_call_chain".to_string(),
            description:
                "Trace the execution call chain starting from a specific function or method. \
                Follows 'Calls' edges recursively to map which functions are invoked. \
                Essential for understanding control flow and execution paths through the codebase."
                    .to_string(),
            parameters: json!({
                    "type": "object",
                    "properties": {
                        "node_id": {
                            "type": "string",
                            "description": "The ID of the starting function/method node (extracted from search results)"
                        },
                        "max_depth": {
                            "type": "integer",
                        "description": "Maximum call chain depth to traverse (1-5; values above 5 are clamped server-side)",
                        "minimum": 1,
                        "maximum": 5,
                        "default": 5
                    }
                },
                "required": ["node_id"]
            }),
        }
    }

    /// Schema for calculate_coupling_metrics function
    pub fn calculate_coupling_metrics() -> ToolSchema {
        ToolSchema {
            name: "calculate_coupling_metrics".to_string(),
            description: "Calculate architectural coupling metrics for a code node. \
                Returns afferent coupling (Ca = incoming dependencies), efferent coupling (Ce = outgoing dependencies), \
                and instability (I = Ce/(Ce+Ca), where 0=stable, 1=unstable). \
                Use this to assess architectural quality and identify problematic coupling patterns.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_id": {
                        "type": "string",
                        "description": "The ID of the code node to analyze (extracted from search results)"
                    }
                },
                "required": ["node_id"]
            }),
        }
    }

    /// Schema for get_hub_nodes function
    pub fn get_hub_nodes() -> ToolSchema {
        ToolSchema {
            name: "get_hub_nodes".to_string(),
            description: "Identify highly connected hub nodes in the code graph with degree >= min_degree. \
                Returns nodes sorted by total degree (incoming + outgoing connections) in descending order. \
                Useful for finding architectural hotspots, central components, and potential bottlenecks or god objects.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "min_degree": {
                        "type": "integer",
                        "description": "Minimum total degree (incoming + outgoing connections) to qualify as a hub (defaults to 5)",
                        "minimum": 1,
                        "default": 5
                    }
                },
                "required": []
            }),
        }
    }

    /// Schema for get_reverse_dependencies function
    pub fn get_reverse_dependencies() -> ToolSchema {
        ToolSchema {
            name: "get_reverse_dependencies".to_string(),
            description: "Get all reverse dependencies (dependents) of a code node - nodes that depend ON this node. \
                Follows incoming dependency edges recursively up to specified depth. \
                Critical for change impact analysis - shows what will be affected if you modify this node.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_id": {
                        "type": "string",
                        "description": "The ID of the code node to analyze (extracted from search results)"
                    },
                    "edge_type": {
                        "type": "string",
                        "description": "Type of dependency relationship to follow backwards",
                        "enum": ["Calls", "Imports", "Uses", "Extends", "Implements", "References"]
                    },
                    "depth": {
                        "type": "integer",
                        "description": "Maximum traversal depth for reverse dependencies (1-5; values above 5 are clamped server-side)",
                        "minimum": 1,
                        "maximum": 5,
                        "default": 3
                    }
                },
            "required": ["node_id", "edge_type"]
        }),
    }
    }

    /// Schema for find_complexity_hotspots function
    pub fn find_complexity_hotspots() -> ToolSchema {
        ToolSchema {
            name: "find_complexity_hotspots".to_string(),
            description: "Find functions with high cyclomatic complexity combined with coupling metrics. \
                Returns functions ranked by risk_score (complexity × afferent_coupling). \
                Use for refactoring prioritization, technical debt assessment, and code review focus. \
                High risk_score indicates code that is both complex AND widely depended upon.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "min_complexity": {
                        "type": "number",
                        "description": "Minimum cyclomatic complexity threshold (defaults to 5.0, lower for more results)",
                        "minimum": 1.0,
                        "default": 5.0
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results (1-100, defaults to 20)",
                        "minimum": 1,
                        "maximum": 100,
                        "default": 20
                    }
                },
                "required": []
            }),
        }
    }

    /// Get schema by name
    pub fn get_by_name(name: &str) -> Option<ToolSchema> {
        Self::all().into_iter().find(|s| s.name == name)
    }

    /// Get all tool names
    pub fn tool_names() -> Vec<String> {
        Self::all().into_iter().map(|s| s.name).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_schemas_valid() {
        let schemas = GraphToolSchemas::all();
        assert_eq!(schemas.len(), 12, "Should have exactly 12 tool schemas");

        for schema in schemas {
            assert!(!schema.name.is_empty(), "Schema name should not be empty");
            assert!(
                !schema.description.is_empty(),
                "Schema description should not be empty"
            );
            assert!(
                schema.parameters.is_object(),
                "Schema parameters should be an object"
            );
        }
    }

    #[test]
    fn test_get_by_name() {
        let schema = GraphToolSchemas::get_by_name("get_transitive_dependencies");
        assert!(schema.is_some());
        assert_eq!(schema.unwrap().name, "get_transitive_dependencies");

        let missing = GraphToolSchemas::get_by_name("nonexistent");
        assert!(missing.is_none());
    }

    #[test]
    fn test_tool_names() {
        let names = GraphToolSchemas::tool_names();
        assert_eq!(names.len(), 12);
        assert!(names.contains(&"find_large_functions".to_string()));
        assert!(names.contains(&"query_graph".to_string()));
        assert!(names.contains(&"search_nodes".to_string()));
        assert!(names.contains(&"list_graph_stats".to_string()));
        assert!(names.contains(&"find_nodes_by_name".to_string()));
        assert!(names.contains(&"get_transitive_dependencies".to_string()));
        assert!(names.contains(&"detect_circular_dependencies".to_string()));
        assert!(names.contains(&"trace_call_chain".to_string()));
        assert!(names.contains(&"calculate_coupling_metrics".to_string()));
        assert!(names.contains(&"get_hub_nodes".to_string()));
        assert!(names.contains(&"get_reverse_dependencies".to_string()));
        assert!(names.contains(&"find_complexity_hotspots".to_string()));
    }

    #[test]
    fn test_schema_serialization() {
        let schema = GraphToolSchemas::get_transitive_dependencies();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(json.contains("get_transitive_dependencies"));
        assert!(json.contains("node_id"));
        assert!(json.contains("edge_type"));
    }
}
