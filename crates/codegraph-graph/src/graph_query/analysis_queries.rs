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

#[derive(Debug, Clone, Serialize)]
pub struct CommunityRow {
    pub id: i64,
    pub name: String,
    pub level: i64,
    pub cohesion: f64,
    pub size: i64,
    pub dominant_language: String,
    pub description: String,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemberNode {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub qualified_name: String,
    pub file_path: String,
    pub line_start: Option<i64>,
    pub line_end: Option<i64>,
    pub language: Option<String>,
    pub is_test: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrossCommunityEdge {
    pub source_community: i64,
    pub target_community: i64,
    pub edge_kind: String,
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchitectureOverview {
    pub communities: Vec<CommunityRow>,
    pub cross_community_edges: Vec<CrossCommunityEdge>,
    pub warnings: Vec<String>,
}

fn test_community_regex() -> &'static regex::Regex {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?i)(^test[-/]|[-/]test([:/]|$)|it:should|describe:|spec[-/]|[-/]spec$)",
        )
        .expect("valid regex")
    });
    &RE
}

fn read_communities(
    conn: &Connection,
    sort_by: &str,
    min_size: usize,
) -> Result<Vec<CommunityRow>> {
    let (column, dir) = match sort_by {
        "cohesion" => ("cohesion", "DESC"),
        "name" => ("name", "ASC"),
        _ => ("size", "DESC"),
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT id, name, level, cohesion, size, dominant_language, description FROM communities \
         WHERE size >= ?1 ORDER BY {column} {dir}, id ASC"
    ))?;
    let mut rows = stmt
        .query_map([min_size as i64], |r| {
            Ok(CommunityRow {
                id: r.get(0)?,
                name: crate::analysis::sanitize_name(&r.get::<_, String>(1)?),
                level: r.get(2)?,
                cohesion: r.get(3)?,
                size: r.get(4)?,
                dominant_language: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                description: crate::analysis::sanitize_name(
                    &r.get::<_, Option<String>>(6)?.unwrap_or_default(),
                ),
                members: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut members =
        conn.prepare("SELECT qualified_name FROM nodes WHERE community_id = ?1 ORDER BY id")?;
    for row in &mut rows {
        row.members = members
            .query_map([row.id], |r| r.get::<_, String>(0))?
            .map(|m| m.map(|qn| crate::analysis::sanitize_name(&qn)))
            .collect::<rusqlite::Result<Vec<_>>>()?;
    }
    Ok(rows)
}

impl GraphQuery {
    /// Stored communities; `sort_by` is size or cohesion (descending) or name (ascending), anything
    /// else is size; ties by id. Each carries its member qualified names in node id order.
    pub async fn list_communities(
        &self,
        sort_by: &str,
        min_size: usize,
    ) -> Result<Vec<CommunityRow>> {
        let sort_by = sort_by.to_string();
        self.store()
            .with_conn(move |conn| read_communities(conn, &sort_by, min_size))
            .await
    }

    /// `id` wins; otherwise the largest community whose name contains `name`, ignoring case.
    pub async fn get_community(
        &self,
        id: Option<i64>,
        name: Option<&str>,
    ) -> Result<Option<CommunityRow>> {
        let needle = name.map(str::to_lowercase);
        self.store()
            .with_conn(move |conn| {
                let all = read_communities(conn, "size", 0)?;
                Ok(match (id, needle) {
                    (Some(id), _) => all.into_iter().find(|c| c.id == id),
                    (None, Some(n)) => all.into_iter().find(|c| c.name.to_lowercase().contains(&n)),
                    _ => None,
                })
            })
            .await
    }

    /// Member nodes of a community, in node id order.
    pub async fn community_members(&self, id: i64) -> Result<Vec<MemberNode>> {
        self.store()
            .with_conn(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, kind, name, qualified_name, file_path, line_start, line_end, language, \
                     is_test FROM nodes WHERE community_id = ?1 ORDER BY id",
                )?;
                let rows = stmt
                    .query_map([id], |r| {
                        Ok(MemberNode {
                            id: r.get(0)?,
                            kind: r.get(1)?,
                            name: r.get(2)?,
                            qualified_name: r.get(3)?,
                            file_path: r.get(4)?,
                            line_start: r.get(5)?,
                            line_end: r.get(6)?,
                            language: r.get(7)?,
                            is_test: r.get::<_, Option<i64>>(8)?.unwrap_or(0) != 0,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Communities, the edges (other than TESTED_BY) that cross between them, and a warning for
    /// every pair with more than 10 such edges unless a side looks test-dominated by name.
    pub async fn architecture_overview(&self) -> Result<ArchitectureOverview> {
        self.store()
            .with_conn(move |conn| {
                let communities = read_communities(conn, "size", 0)?;
                let mut owner: std::collections::HashMap<&str, i64> =
                    std::collections::HashMap::new();
                for c in &communities {
                    for qn in &c.members {
                        owner.insert(qn.as_str(), c.id);
                    }
                }
                let mut stmt = conn.prepare(
                    "SELECT kind, source_qualified, target_qualified FROM edges \
                     WHERE kind != 'TESTED_BY' ORDER BY id",
                )?;
                let mut cross = Vec::new();
                let mut counts: std::collections::BTreeMap<(i64, i64), usize> = Default::default();
                for row in stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })? {
                    let (kind, source, target) = row?;
                    let (Some(&a), Some(&b)) =
                        (owner.get(source.as_str()), owner.get(target.as_str()))
                    else {
                        continue;
                    };
                    if a == b {
                        continue;
                    }
                    *counts.entry((a.min(b), a.max(b))).or_default() += 1;
                    cross.push(CrossCommunityEdge {
                        source_community: a,
                        target_community: b,
                        edge_kind: kind,
                        source: crate::analysis::sanitize_name(&source),
                        target: crate::analysis::sanitize_name(&target),
                    });
                }
                let names: std::collections::HashMap<i64, &str> = communities
                    .iter()
                    .map(|c| (c.id, c.name.as_str()))
                    .collect();
                let mut ordered: Vec<_> = counts.into_iter().collect();
                ordered.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                let mut warnings = Vec::new();
                for ((c1, c2), count) in ordered {
                    if count <= 10 {
                        continue;
                    }
                    let n1 = names
                        .get(&c1)
                        .map_or_else(|| format!("community-{c1}"), |n| n.to_string());
                    let n2 = names
                        .get(&c2)
                        .map_or_else(|| format!("community-{c2}"), |n| n.to_string());
                    if test_community_regex().is_match(&n1) || test_community_regex().is_match(&n2)
                    {
                        continue;
                    }
                    warnings.push(format!(
                        "High coupling ({count} edges) between '{n1}' and '{n2}'"
                    ));
                }
                Ok(ArchitectureOverview {
                    communities,
                    cross_community_edges: cross,
                    warnings,
                })
            })
            .await
    }
}

impl GraphQuery {
    /// Every hub candidate, most connected first (see `analysis::insights::hub_nodes`).
    pub async fn hub_nodes(&self) -> Result<Vec<crate::analysis::insights::HubRow>> {
        self.store()
            .with_conn(|conn| {
                Ok(crate::analysis::insights::hub_nodes(
                    &crate::analysis::AnalysisGraph::load(conn)?,
                ))
            })
            .await
    }

    /// Every node with positive betweenness, highest first (exact to 5000 vertices, then sampled).
    pub async fn bridge_nodes(&self) -> Result<Vec<crate::analysis::insights::BridgeRow>> {
        self.store()
            .with_conn(|conn| {
                Ok(crate::analysis::insights::bridge_nodes(
                    &crate::analysis::AnalysisGraph::load(conn)?,
                ))
            })
            .await
    }

    /// Structural weaknesses: isolated nodes, thin and single-file communities, untested
    /// hotspots and import cycles (paths relative to the project root).
    pub async fn knowledge_gaps(&self) -> Result<crate::analysis::insights::KnowledgeGaps> {
        let root = self.store().root().to_string_lossy().into_owned();
        self.store()
            .with_conn(move |conn| {
                let graph = crate::analysis::AnalysisGraph::load(conn)?;
                let communities = read_communities(conn, "size", 0)?;
                Ok(crate::analysis::insights::knowledge_gaps(
                    &graph,
                    &communities,
                    Some(&root),
                ))
            })
            .await
    }

    /// Every edge with a positive surprise score, highest first.
    pub async fn surprising_connections(
        &self,
    ) -> Result<Vec<crate::analysis::insights::SurprisingRow>> {
        self.store()
            .with_conn(|conn| {
                Ok(crate::analysis::insights::surprising_connections(
                    &crate::analysis::AnalysisGraph::load(conn)?,
                ))
            })
            .await
    }
}
