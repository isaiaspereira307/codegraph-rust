// ABOUTME: Reads stored analysis (flows) from the CRG tables: list, detail and affected-by-files.
// ABOUTME: Works on databases written by code-review-graph too; it needs only nodes/flows tables.

use super::GraphQuery;
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct FlowRow {
    pub id: i64,
    pub name: String,
    pub entry_point_id: i64,
    pub depth: i64,
    pub node_count: i64,
    pub file_count: i64,
    pub criticality: f64,
    pub path: Vec<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlowStep {
    pub node_id: i64,
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line_start: Option<i64>,
    pub line_end: Option<i64>,
    pub qualified_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlowDetail {
    #[serde(flatten)]
    pub flow: FlowRow,
    pub steps: Vec<FlowStep>,
    pub total_steps: usize,
    pub truncated: bool,
}

const FLOW_COLUMNS: &str = "f.id, f.name, f.entry_point_id, f.depth, f.node_count, f.file_count, \
     f.criticality, f.path_json, f.created_at, f.updated_at";

fn flow_row(r: &rusqlite::Row) -> rusqlite::Result<FlowRow> {
    let path_json: String = r.get(7)?;
    Ok(FlowRow {
        id: r.get(0)?,
        name: r.get(1)?,
        entry_point_id: r.get(2)?,
        depth: r.get(3)?,
        node_count: r.get(4)?,
        file_count: r.get(5)?,
        criticality: r.get(6)?,
        path: serde_json::from_str(&path_json).unwrap_or_default(),
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

fn flow_detail(conn: &Connection, flow: FlowRow, max_steps: usize) -> Result<FlowDetail> {
    let total: i64 = conn.query_row(
        "SELECT count(*) FROM flow_memberships WHERE flow_id = ?1",
        [flow.id],
        |r| r.get(0),
    )?;
    let mut stmt = conn.prepare(
        "SELECT n.id, n.name, n.kind, n.file_path, n.line_start, n.line_end, n.qualified_name \
         FROM flow_memberships m JOIN nodes n ON n.id = m.node_id \
         WHERE m.flow_id = ?1 ORDER BY m.position LIMIT ?2",
    )?;
    let steps = stmt
        .query_map(
            params![flow.id, max_steps.min(i64::MAX as usize) as i64],
            |r| {
                Ok(FlowStep {
                    node_id: r.get(0)?,
                    name: r.get(1)?,
                    kind: r.get(2)?,
                    file: r.get(3)?,
                    line_start: r.get(4)?,
                    line_end: r.get(5)?,
                    qualified_name: r.get(6)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let total_steps = total as usize;
    Ok(FlowDetail {
        flow,
        truncated: total_steps > steps.len(),
        steps,
        total_steps,
    })
}

impl GraphQuery {
    /// Stored flows, sorted by `sort_by` (criticality, depth, node_count, file_count: descending;
    /// name: ascending; anything else is criticality), ties by id. `kind` keeps flows whose
    /// entry node has that kind. Returns the page and the total matching count.
    pub async fn list_flows(
        &self,
        sort_by: &str,
        limit: usize,
        kind: Option<&str>,
    ) -> Result<(Vec<FlowRow>, usize)> {
        let order = match sort_by {
            "depth" => "f.depth DESC",
            "node_count" => "f.node_count DESC",
            "file_count" => "f.file_count DESC",
            "name" => "f.name ASC",
            _ => "f.criticality DESC",
        };
        let kind = kind.map(str::to_string);
        self.store().with_conn(move |conn| {
            let filter = "(?1 IS NULL OR EXISTS (SELECT 1 FROM nodes n WHERE n.id = f.entry_point_id \
                          AND n.kind = ?1))";
            let total: i64 = conn.query_row(
                &format!("SELECT count(*) FROM flows f WHERE {filter}"),
                params![kind],
                |r| r.get(0),
            )?;
            let mut stmt = conn.prepare(&format!(
                "SELECT {FLOW_COLUMNS} FROM flows f WHERE {filter} ORDER BY {order}, f.id ASC LIMIT ?2"
            ))?;
            let rows = stmt
                .query_map(params![kind, limit as i64], flow_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok((rows, total as usize))
        })
        .await
    }

    /// One flow with its steps. `id` wins; otherwise `name` is a case-insensitive substring match
    /// over the 500 most critical flows (first hit).
    pub async fn get_flow(
        &self,
        id: Option<i64>,
        name: Option<&str>,
        max_steps: usize,
    ) -> Result<Option<FlowDetail>> {
        let name = name.map(str::to_lowercase);
        self.store().with_conn(move |conn| {
            let flow = if let Some(id) = id {
                conn.query_row(
                    &format!("SELECT {FLOW_COLUMNS} FROM flows f WHERE f.id = ?1"),
                    [id],
                    flow_row,
                )
                .optional()?
            } else if let Some(needle) = name {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {FLOW_COLUMNS} FROM flows f ORDER BY f.criticality DESC, f.id ASC LIMIT 500"
                ))?;
                let rows = stmt
                    .query_map([], flow_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows.into_iter()
                    .find(|f| f.name.to_lowercase().contains(&needle))
            } else {
                None
            };
            flow.map(|f| flow_detail(conn, f, max_steps)).transpose()
        })
        .await
    }

    /// Flows that touch any node of the given files (repo-relative or absolute paths), most
    /// critical first, ties by id, each with all its steps.
    pub async fn affected_flows(&self, files: &[String]) -> Result<Vec<FlowDetail>> {
        if files.is_empty() {
            return Ok(Vec::new());
        }
        let files = files.to_vec();
        self.store().with_conn(move |conn| {
            let mut ids = std::collections::BTreeSet::new();
            let mut stmt = conn.prepare(
                "SELECT DISTINCT m.flow_id FROM flow_memberships m JOIN nodes n ON n.id = m.node_id \
                 WHERE n.file_path = ?1 OR substr(n.file_path, -length(?1) - 1) = '/' || ?1",
            )?;
            for file in &files {
                for id in stmt.query_map([file], |r| r.get::<_, i64>(0))? {
                    ids.insert(id?);
                }
            }
            let mut flows = Vec::new();
            let mut by_id = conn.prepare(&format!(
                "SELECT {FLOW_COLUMNS} FROM flows f WHERE f.id = ?1"
            ))?;
            for id in ids {
                if let Some(flow) = by_id.query_row([id], flow_row).optional()? {
                    flows.push(flow_detail(conn, flow, i64::MAX as usize)?);
                }
            }
            flows.sort_by(|a, b| {
                b.flow
                    .criticality
                    .total_cmp(&a.flow.criticality)
                    .then(a.flow.id.cmp(&b.flow.id))
            });
            Ok(flows)
        })
        .await
    }
}
