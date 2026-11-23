// ABOUTME: Response shaping for the flow tools: step budgets, minimal views and source snippets.
// ABOUTME: Pure functions over GraphQuery flow types so limits are testable without a database.

use codegraph_graph::graph_query::FlowDetail;
use serde_json::{Value as JsonValue, json};
use std::path::{Path, PathBuf};

/// Steps shared by all flows of a `get_affected_flows` answer (standard detail).
pub const AFFECTED_STEP_BUDGET: usize = 400;

/// Spends the step budget over `flows` in order (already most critical first): the tail keeps its
/// metadata and gets `steps_omitted`. Every flow reports `total_steps`. Second value: any cut.
pub fn bound_flow_steps(flows: Vec<FlowDetail>) -> (Vec<JsonValue>, bool) {
    let mut budget = AFFECTED_STEP_BUDGET;
    let mut truncated = false;
    let mut out = Vec::with_capacity(flows.len());
    for flow in flows {
        let total = flow.steps.len();
        let mut value = serde_json::to_value(&flow).unwrap_or(JsonValue::Null);
        if total > budget {
            value["steps"] = serde_json::to_value(&flow.steps[..budget]).unwrap_or(json!([]));
            value["steps_omitted"] = json!(true);
            truncated = true;
        }
        value["total_steps"] = json!(total);
        budget -= total.min(budget);
        out.push(value);
    }
    (out, truncated)
}

/// `{id, name, criticality, depth, node_count, file_count}` for minimal detail.
pub fn affected_minimal(flow: &FlowDetail) -> JsonValue {
    let f = &flow.flow;
    json!({
        "id": f.id, "name": f.name, "criticality": f.criticality,
        "depth": f.depth, "node_count": f.node_count, "file_count": f.file_count,
    })
}

/// Lines `start..=end` of `file` as `"{n}: {line}"`, limited to `budget` lines, which is reduced by
/// the lines emitted. Relative paths resolve against `root`. Unreadable files give a marker text.
pub fn numbered_source(
    root: &Path,
    file: &str,
    start: Option<i64>,
    end: Option<i64>,
    budget: &mut usize,
) -> Option<String> {
    let path = Path::new(file);
    let full: PathBuf = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let Ok(text) = std::fs::read_to_string(&full) else {
        return Some("(could not read file)".to_string());
    };
    let lines: Vec<&str> = text.lines().collect();
    let from = (start.unwrap_or(1).max(1) - 1) as usize;
    let to = (end.unwrap_or(lines.len() as i64).max(0) as usize)
        .min(lines.len())
        .min(from + *budget);
    if from >= to {
        return Some(String::new());
    }
    *budget -= to - from;
    Some(
        (from..to)
            .map(|i| format!("{}: {}", i + 1, lines[i]))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_graph::graph_query::{FlowDetail, FlowRow, FlowStep};

    fn flow(id: i64, steps: usize) -> FlowDetail {
        FlowDetail {
            flow: FlowRow {
                id,
                name: format!("flow{id}"),
                entry_point_id: id,
                depth: 2,
                node_count: steps as i64,
                file_count: 1,
                criticality: 0.5,
                path: vec![],
                created_at: String::new(),
                updated_at: String::new(),
            },
            steps: (0..steps)
                .map(|i| FlowStep {
                    node_id: i as i64,
                    name: format!("n{i}"),
                    kind: "Function".into(),
                    file: "f.py".into(),
                    line_start: Some(1),
                    line_end: Some(2),
                    qualified_name: format!("f.py::n{i}"),
                })
                .collect(),
            total_steps: steps,
            truncated: false,
        }
    }

    #[test]
    fn step_budget_marks_steps_omitted() {
        let (out, cut) = bound_flow_steps(vec![flow(1, 300), flow(2, 200), flow(3, 5)]);
        assert!(cut);
        assert_eq!(out[0]["steps"].as_array().unwrap().len(), 300);
        assert!(out[0].get("steps_omitted").is_none());
        assert_eq!(out[1]["steps"].as_array().unwrap().len(), 100);
        assert_eq!(out[1]["steps_omitted"], true);
        assert_eq!(out[1]["total_steps"], 200);
        assert_eq!(out[2]["steps"].as_array().unwrap().len(), 0);
        assert_eq!(out[2]["steps_omitted"], true);
    }

    #[test]
    fn step_budget_leaves_small_sets_alone() {
        let (out, cut) = bound_flow_steps(vec![flow(1, 3)]);
        assert!(!cut);
        assert_eq!(out[0]["steps"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn minimal_projection_keeps_only_listed_fields() {
        let v = affected_minimal(&flow(7, 4));
        assert_eq!(
            v,
            serde_json::json!({"id": 7, "name": "flow7", "criticality": 0.5,
                               "depth": 2, "node_count": 4, "file_count": 1})
        );
    }

    #[test]
    fn numbered_source_spends_the_shared_budget() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.py"), "a\nb\nc\nd\n").unwrap();
        let mut budget = 3;
        let first = numbered_source(dir.path(), "f.py", Some(1), Some(2), &mut budget).unwrap();
        assert_eq!(first, "1: a\n2: b");
        assert_eq!(budget, 1);
        let second = numbered_source(dir.path(), "f.py", Some(3), Some(4), &mut budget).unwrap();
        assert_eq!(second, "3: c");
        assert_eq!(budget, 0);
    }

    #[test]
    fn numbered_source_reports_unreadable_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut budget = 5;
        let text = numbered_source(dir.path(), "missing.py", Some(1), Some(2), &mut budget);
        assert_eq!(text.as_deref(), Some("(could not read file)"));
        assert_eq!(budget, 5);
    }
}
