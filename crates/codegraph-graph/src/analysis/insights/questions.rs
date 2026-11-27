// ABOUTME: Suggested review questions built from the other insight analyses.
// ABOUTME: Ports generate_suggested_questions from code-review-graph analysis.py.

use super::super::AnalysisGraph;
use super::{bridge_nodes, hub_nodes, knowledge_gaps, surprising_connections};
use crate::graph_query::CommunityRow;
use serde::Serialize;
use std::collections::HashSet;

/// One generated review question.
#[derive(Debug, Clone, Serialize)]
pub struct Question {
    pub category: &'static str,
    pub question: String,
    pub target: String,
    pub priority: &'static str,
}

/// Review questions from the other analyses (`generate_suggested_questions` in the CRG): bridges,
/// untested hubs, cross-community surprises, thin communities, untested hotspots, in that order.
pub fn suggested_questions(g: &AnalysisGraph, communities: &[CommunityRow]) -> Vec<Question> {
    let mut out = Vec::new();
    for b in bridge_nodes(g).into_iter().take(3) {
        out.push(Question {
            category: "bridge_node",
            question: format!(
                "'{}' is a critical connector between multiple code regions. Is it adequately \
                 tested and documented?",
                b.name
            ),
            target: b.qualified_name,
            priority: "high",
        });
    }
    let tested: HashSet<&str> = g
        .edges
        .iter()
        .filter(|e| e.kind == "TESTED_BY")
        .map(|e| e.source.as_str())
        .collect();
    for h in hub_nodes(g).into_iter().take(3) {
        if !tested.contains(h.qualified_name.as_str()) {
            out.push(Question {
                category: "hub_risk",
                question: format!(
                    "Hub node '{}' has {} connections but no direct test coverage. Should it be \
                     tested?",
                    h.name, h.total_degree
                ),
                target: h.qualified_name,
                priority: "high",
            });
        }
    }
    for s in surprising_connections(g).into_iter().take(3) {
        if s.reasons.contains(&"cross-community") {
            let cid = |c: Option<i64>| c.map_or("None".to_string(), |c| c.to_string());
            out.push(Question {
                category: "surprising_connection",
                question: format!(
                    "'{}' (community {}) calls '{}' (community {}). Is this coupling intentional?",
                    s.source,
                    cid(s.source_community),
                    s.target,
                    cid(s.target_community)
                ),
                target: s.source_qualified,
                priority: "medium",
            });
        }
    }
    let gaps = knowledge_gaps(g, communities, None);
    for c in gaps.thin_communities.into_iter().take(2) {
        out.push(Question {
            category: "thin_community",
            question: format!(
                "Community '{}' has only {} member(s). Should it be merged with a neighbor?",
                c.name, c.size
            ),
            target: format!("community:{}", c.community_id),
            priority: "low",
        });
    }
    for h in gaps.untested_hotspots.into_iter().take(2) {
        out.push(Question {
            category: "untested_hotspot",
            question: format!(
                "'{}' has {} connections but no test coverage. Is this a risk?",
                h.name, h.degree
            ),
            target: h.qualified_name,
            priority: "medium",
        });
    }
    out
}
