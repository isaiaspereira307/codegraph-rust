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

/// Parameters shared by the two review tools.
fn review_parameters() -> JsonValue {
    json!({
        "type": "object",
        "properties": {
            "base_ref": {
                "type": "string",
                "description": "Commit, branch or tag to diff against (defaults to HEAD)",
                "default": "HEAD"
            },
            "depth": {
                "type": "integer",
                "description": "Levels of dependents to walk (1-10, defaults to 3)",
                "minimum": 1,
                "maximum": 10,
                "default": 3
            },
            "max_nodes": {
                "type": "integer",
                "description": "Maximum impacted nodes (1-500, defaults to 50)",
                "minimum": 1,
                "maximum": 500,
                "default": 50
            },
            "max_snippet_lines": {
                "type": "integer",
                "description": "Lines per changed-node snippet (1-100, defaults to 20); get_review_context only",
                "minimum": 1,
                "maximum": 100,
                "default": 20
            }
        }
    })
}

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
            Self::traverse_graph(),
            Self::get_impact_radius(),
            Self::detect_changes(),
            Self::get_review_context(),
            Self::get_minimal_context(),
            Self::list_flows(),
            Self::get_flow(),
            Self::get_affected_flows(),
            Self::list_communities(),
            Self::get_community(),
            Self::get_architecture_overview(),
            Self::get_bridge_nodes(),
            Self::get_knowledge_gaps(),
            Self::get_surprising_connections(),
            Self::get_suggested_questions(),
        ]
    }

    /// Schema for list_flows (stored execution flows by criticality)
    pub fn list_flows() -> ToolSchema {
        ToolSchema {
            name: "list_flows".to_string(),
            description: "List execution flows (call chains from an entry point such as a route \
                   handler, CLI command or main), most critical first. Criticality blends file \
                   spread, unresolved calls, security-sensitive names, test gaps and depth."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "sort_by": {
                        "type": "string",
                        "description": "criticality (default), depth, node_count, file_count or name",
                        "default": "criticality"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum flows (1-200, defaults to 50)",
                        "minimum": 1, "maximum": 200, "default": 50
                    },
                    "kind": {
                        "type": "string",
                        "description": "Only flows whose entry node has this kind (Function, Test)"
                    },
                    "detail_level": {
                        "type": "string",
                        "description": "standard (default) or minimal (name, criticality, node_count)",
                        "enum": ["standard", "minimal"], "default": "standard"
                    }
                },
                "required": []
            }),
        }
    }

    /// Schema for get_flow (one flow with its steps and optional source)
    pub fn get_flow() -> ToolSchema {
        ToolSchema {
            name: "get_flow".to_string(),
            description: "One execution flow with every step (function, file, lines). Pass \
                   flow_id from list_flows, or flow_name for a case-insensitive partial match. \
                   include_source adds numbered source for each step within a line budget."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "flow_id": { "type": "integer", "description": "Flow id from list_flows" },
                    "flow_name": { "type": "string", "description": "Partial name; ignored when flow_id is given" },
                    "include_source": { "type": "boolean", "default": false },
                    "max_steps": {
                        "type": "integer", "description": "Steps to return (1-200, defaults to 50)",
                        "minimum": 1, "maximum": 200, "default": 50
                    },
                    "max_source_lines": {
                        "type": "integer",
                        "description": "Total source lines across steps (1-2000, defaults to 400)",
                        "minimum": 1, "maximum": 2000, "default": 400
                    }
                },
                "required": []
            }),
        }
    }

    /// Schema for get_affected_flows (flows through changed files)
    pub fn get_affected_flows() -> ToolSchema {
        ToolSchema {
            name: "get_affected_flows".to_string(),
            description: "Execution flows that pass through the changed files, most critical \
                   first. Without changed_files it uses `git diff` against base_ref. Use it in \
                   review to see which entry points a change reaches."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "changed_files": {
                        "type": "array", "items": { "type": "string" },
                        "description": "Repo-relative paths; detected from git when omitted"
                    },
                    "base_ref": {
                        "type": "string",
                        "description": "Commit, branch or tag to diff against (defaults to HEAD~1)",
                        "default": "HEAD~1"
                    },
                    "detail_level": {
                        "type": "string",
                        "description": "standard (steps, shared 400-step budget, max 25 flows) or minimal (max 500 flows)",
                        "enum": ["standard", "minimal"], "default": "standard"
                    },
                    "max_flows": {
                        "type": "integer", "description": "Flows to return (defaults to 50, then capped by detail level)",
                        "minimum": 0, "default": 50
                    }
                },
                "required": []
            }),
        }
    }

    /// Schema for list_communities (stored code clusters)
    pub fn list_communities() -> ToolSchema {
        ToolSchema {
            name: "list_communities".to_string(),
            description: "List communities: clusters of closely related functions and classes \
                   (Leiden over calls, imports and inheritance, or directory grouping), with \
                   size, cohesion and a sample of members."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "sort_by": { "type": "string", "description": "size (default), cohesion or name", "default": "size" },
                    "min_size": { "type": "integer", "minimum": 0, "default": 0, "description": "Smallest community to include" },
                    "detail_level": { "type": "string", "enum": ["standard", "minimal"], "default": "standard",
                        "description": "minimal returns name, size and cohesion only" },
                    "max_results": { "type": "integer", "minimum": 1, "maximum": 200, "default": 50 },
                    "max_members": { "type": "integer", "minimum": 1, "maximum": 25, "default": 10,
                        "description": "Members listed per community; size keeps the true count" }
                },
                "required": []
            }),
        }
    }

    /// Schema for get_community (one community, optionally with member nodes)
    pub fn get_community() -> ToolSchema {
        ToolSchema {
            name: "get_community".to_string(),
            description: "One community by community_id, or by a partial community_name (largest \
                   match first). include_members adds member node details."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "community_id": { "type": "integer" },
                    "community_name": { "type": "string", "description": "Case-insensitive partial name; ignored when community_id is given" },
                    "include_members": { "type": "boolean", "default": false },
                    "max_members": { "type": "integer", "minimum": 1, "maximum": 25, "default": 25 }
                },
                "required": []
            }),
        }
    }

    /// Schema for get_architecture_overview (communities and coupling between them)
    pub fn get_architecture_overview() -> ToolSchema {
        ToolSchema {
            name: "get_architecture_overview".to_string(),
            description: "Architecture overview from the community structure: communities, the \
                   edges that cross between them and warnings for heavily coupled pairs. \
                   minimal (default) aggregates edges per community pair."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "detail_level": { "type": "string", "enum": ["minimal", "standard"], "default": "minimal" },
                    "max_results": { "type": "integer", "minimum": 1, "maximum": 200, "default": 100,
                        "description": "Cross-community rows and warnings to return" },
                    "max_members": { "type": "integer", "minimum": 1, "maximum": 25, "default": 10,
                        "description": "standard only: members listed per community" }
                },
                "required": []
            }),
        }
    }

    /// Schema for get_review_context (changed nodes by risk, impact radius, snippets)
    pub fn get_review_context() -> ToolSchema {
        ToolSchema {
            name: "get_review_context".to_string(),
            description: "Review context for the current git diff in one call: changed files, \
                   changed nodes ordered by risk (dependents plus one when untested), the \
                   transitive impact by depth, affected files, covering tests, and a snippet of \
                   each changed node. Use it instead of reading the changed files."
                .to_string(),
            parameters: review_parameters(),
        }
    }

    /// Schema for get_minimal_context (same input; ids, files and counts only)
    pub fn get_minimal_context() -> ToolSchema {
        ToolSchema {
            name: "get_minimal_context".to_string(),
            description: "Compact review context for the current git diff: changed node IDs with \
                   risk, changed and affected files, and counts. Same input as \
                   get_review_context, without snippets or the impact list."
                .to_string(),
            parameters: review_parameters(),
        }
    }

    /// Schema for detect_changes (nodes touched by the work tree's diff against a base ref)
    pub fn detect_changes() -> ToolSchema {
        ToolSchema {
            name: "detect_changes".to_string(),
            description: "Nodes touched by the current git diff: runs `git diff -U0 <base_ref>` \
                   in the project and returns the changed files, their changed line ranges, and \
                   the graph nodes whose spans overlap them. Staged and unstaged tracked changes \
                   count; untracked files do not. Fails when the project is not a git work tree."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "base_ref": {
                        "type": "string",
                        "description": "Commit, branch or tag to diff against (defaults to HEAD)",
                        "default": "HEAD"
                    }
                }
            }),
        }
    }

    /// Schema for get_impact_radius (transitive dependents of changed nodes or files, and tests)
    pub fn get_impact_radius() -> ToolSchema {
        ToolSchema {
            name: "get_impact_radius".to_string(),
            description: "Blast radius of a change: the transitive dependents of the given nodes \
                   or of every node in the given files, grouped by depth, with the affected \
                   files and the tests that cover them. Give node_ids, files, or both."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Changed node IDs"
                    },
                    "files": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Changed file paths (exact or suffix match)"
                    },
                    "depth": {
                        "type": "integer",
                        "description": "Levels of dependents to walk (1-10, defaults to 3)",
                        "minimum": 1,
                        "maximum": 10,
                        "default": 3
                    },
                    "max_nodes": {
                        "type": "integer",
                        "description": "Maximum impacted nodes to return (1-1000, defaults to 200)",
                        "minimum": 1,
                        "maximum": 1000,
                        "default": 200
                    }
                }
            }),
        }
    }

    /// Schema for traverse_graph (breadth-first walk from one node)
    pub fn traverse_graph() -> ToolSchema {
        ToolSchema {
            name: "traverse_graph".to_string(),
            description: "Breadth-first walk from a node over dependency edges (CALLS, \
                   IMPORTS_FROM, INHERITS, ...), following out, in or both directions. Returns \
                   reached nodes nearest first with their depth. Give node_id, or name for the \
                   best match. `truncated` is true when max_nodes stopped the walk."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_id": {
                        "type": "string",
                        "description": "Node ID returned by find_nodes_by_name or search_nodes"
                    },
                    "name": {
                        "type": "string",
                        "description": "Symbol name, used when node_id is not given; exact matches win"
                    },
                    "direction": {
                        "type": "string",
                        "enum": ["out", "in", "both"],
                        "description": "out follows what the node depends on, in what depends on it",
                        "default": "out"
                    },
                    "edge_type": {
                        "type": "string",
                        "description": "Single edge kind such as Calls or ImportsFrom; all dependency kinds when omitted"
                    },
                    "max_depth": {
                        "type": "integer",
                        "description": "Levels to walk (1-10, defaults to 3)",
                        "minimum": 1,
                        "maximum": 10,
                        "default": 3
                    },
                    "max_nodes": {
                        "type": "integer",
                        "description": "Maximum nodes to return (1-500, defaults to 100)",
                        "minimum": 1,
                        "maximum": 500,
                        "default": 100
                    }
                }
            }),
        }
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
            description: "Most connected nodes (in + out edges of every kind), highest total \
                degree first. Hubs are architectural hotspots: changing one has a wide blast \
                radius. Returns name, qualified name, kind, file, degrees and community id."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "top_n": {
                        "type": "integer",
                        "description": "Hubs to return (1-100, defaults to 10)",
                        "minimum": 1, "maximum": 100, "default": 10
                    },
                    "detail_level": {
                        "type": "string", "enum": ["standard", "minimal"], "default": "standard",
                        "description": "minimal returns name, kind and total_degree only"
                    },
                    "min_degree": {
                        "type": "integer",
                        "description": "Optional: only nodes with at least this total degree",
                        "minimum": 1
                    }
                },
                "required": []
            }),
        }
    }

    /// Schema for get_bridge_nodes (betweenness centrality)
    pub fn get_bridge_nodes() -> ToolSchema {
        ToolSchema {
            name: "get_bridge_nodes".to_string(),
            description: "Architectural chokepoints: nodes with the highest betweenness \
                centrality, i.e. on the shortest paths between many other nodes. If they break, \
                several regions of the code lose connectivity. Exact up to 5000 vertices, \
                sampled (deterministically) above."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "top_n": {
                        "type": "integer", "description": "Bridges to return (1-100, defaults to 10)",
                        "minimum": 1, "maximum": 100, "default": 10
                    },
                    "detail_level": {
                        "type": "string", "enum": ["standard", "minimal"], "default": "standard",
                        "description": "minimal returns name, kind and betweenness only"
                    }
                },
                "required": []
            }),
        }
    }

    /// Schema for get_knowledge_gaps (structural weaknesses)
    pub fn get_knowledge_gaps() -> ToolSchema {
        ToolSchema {
            name: "get_knowledge_gaps".to_string(),
            description: "Structural weaknesses: isolated nodes (degree <= 1), thin communities \
                (< 3 members), untested hotspots (degree >= 5, no TESTED_BY), single-file \
                communities and import cycles (files that import each other; load_time says \
                whether a loop can fail as the files load). summary and total_gaps report \
                untruncated counts."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "max_per_category": {
                        "type": "integer",
                        "description": "Entries per gap category (1-50, defaults to 15)",
                        "minimum": 1, "maximum": 50, "default": 15
                    },
                    "detail_level": {
                        "type": "string", "enum": ["standard", "minimal"], "default": "standard",
                        "description": "minimal drops file paths and keeps identifying fields only"
                    }
                },
                "required": []
            }),
        }
    }

    /// Schema for get_surprising_connections (unexpected coupling)
    pub fn get_surprising_connections() -> ToolSchema {
        ToolSchema {
            name: "get_surprising_connections".to_string(),
            description: "Unexpected architectural coupling: edges scored by cross-community \
                (+0.3), cross-language (+0.2), peripheral-to-hub (+0.2), cross-test-boundary \
                (+0.15) and unusual edge kind (+0.15), highest score first."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "top_n": {
                        "type": "integer", "description": "Edges to return (1-100, defaults to 15)",
                        "minimum": 1, "maximum": 100, "default": 15
                    },
                    "detail_level": {
                        "type": "string", "enum": ["standard", "minimal"], "default": "standard",
                        "description": "minimal returns source, target, edge_kind and surprise_score only"
                    }
                },
                "required": []
            }),
        }
    }

    /// Schema for get_suggested_questions (review prompts)
    pub fn get_suggested_questions() -> ToolSchema {
        ToolSchema {
            name: "get_suggested_questions".to_string(),
            description: "Review questions generated from the graph: bridge nodes (up to 3), \
                untested hubs (3), cross-community surprises (3), thin communities (2) and \
                untested hotspots (2). Each has category, question, target and priority."
                .to_string(),
            parameters: json!({ "type": "object", "properties": {}, "required": [] }),
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
        assert_eq!(schemas.len(), 27, "Should have exactly 27 tool schemas");

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
        assert_eq!(names.len(), 27);
        for flow_tool in [
            "list_flows",
            "get_flow",
            "get_affected_flows",
            "list_communities",
            "get_community",
            "get_architecture_overview",
            "get_bridge_nodes",
            "get_knowledge_gaps",
            "get_surprising_connections",
            "get_suggested_questions",
        ] {
            assert!(names.contains(&flow_tool.to_string()), "{flow_tool}");
        }
        assert!(names.contains(&"get_review_context".to_string()));
        assert!(names.contains(&"get_minimal_context".to_string()));
        assert!(names.contains(&"detect_changes".to_string()));
        assert!(names.contains(&"get_impact_radius".to_string()));
        assert!(names.contains(&"find_large_functions".to_string()));
        assert!(names.contains(&"query_graph".to_string()));
        assert!(names.contains(&"traverse_graph".to_string()));
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
