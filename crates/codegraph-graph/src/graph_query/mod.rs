// ABOUTME: Graph tool queries over an in-memory petgraph snapshot of the SQLite store.
// ABOUTME: Semantics follow the former SurrealQL fn::* functions (depth limits, defaults).

mod analysis_queries;
mod changes;
mod coverage;
mod impact;
mod large;
mod query;
mod search;
mod snapshot;
mod stats;
mod traverse;

pub use analysis_queries::{FlowDetail, FlowRow, FlowStep};
pub use coverage::NodeCoverage;
pub use impact::{ImpactRadius, ImpactedNode};
pub use query::{QueryGraphResult, QueryPattern};
pub use traverse::{TraversalResult, TraverseDirection, TraversedNode};
pub mod types;

use crate::sqlite_store::SqliteGraphStore;
use crate::sqlite_store::mapping::{DEPENDENCY_EDGE_KINDS, edge_kind_from_input};
use anyhow::{Result, anyhow};
use petgraph::Direction::{Incoming, Outgoing};
use petgraph::graph::NodeIndex;
use snapshot::Snapshot;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use types::*;

const CALLS: &str = "CALLS";
const MAX_CALLERS: usize = 10;
const MAX_HUBS: usize = 50;

type Cached = Option<((i64, i64), Arc<Snapshot>)>;

/// Read side of the graph tools. Clones share the snapshot cache.
#[derive(Clone)]
pub struct GraphQuery {
    store: SqliteGraphStore,
    cache: Arc<Mutex<Cached>>,
}

fn safe_depth(depth: i32, default: i32) -> i32 {
    if (1..=10).contains(&depth) {
        depth
    } else {
        default
    }
}

fn is_dependency(kind: &str) -> bool {
    DEPENDENCY_EDGE_KINDS.contains(&kind)
}

fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

impl GraphQuery {
    pub fn new(store: SqliteGraphStore) -> Self {
        Self {
            store,
            cache: Arc::new(Mutex::new(None)),
        }
    }

    pub fn store(&self) -> &SqliteGraphStore {
        &self.store
    }

    /// Cache and log key: the project root this database belongs to.
    pub fn project_id(&self) -> String {
        self.store.root().display().to_string()
    }

    /// One database holds one project, so the id does not change what is queried.
    pub fn with_project_id(&self, _project_id: &str) -> Self {
        self.clone()
    }

    /// Current snapshot; rebuilt when this connection or any other committed a change.
    async fn snapshot(&self) -> Result<Arc<Snapshot>> {
        let cache = self.cache.clone();
        self.store
            .with_conn(move |conn| {
                let key: (i64, i64) = (
                    conn.query_row("PRAGMA data_version", [], |r| r.get(0))?,
                    conn.query_row("SELECT total_changes()", [], |r| r.get(0))?,
                );
                let mut cached = cache.lock().expect("snapshot cache poisoned");
                if let Some((k, snap)) = cached.as_ref()
                    && *k == key
                {
                    return Ok(snap.clone());
                }
                let snap = Arc::new(Snapshot::load(conn)?);
                *cached = Some((key, snap.clone()));
                Ok(snap)
            })
            .await
    }

    pub async fn find_nodes_by_name(
        &self,
        needle: &str,
        limit: usize,
    ) -> Result<Vec<NodeReference>> {
        let snap = self.snapshot().await?;
        let max = if limit == 0 { 10 } else { limit };
        let lowered = needle.to_lowercase();
        let mut hits: Vec<NodeIndex> = snap
            .graph
            .node_indices()
            .filter(|&ix| {
                let n = snap.node(ix);
                n.name.to_lowercase().contains(&lowered) || n.file_path.contains(needle)
            })
            .collect();
        // Name matches before path-only matches, and symbols before File nodes (whose
        // absolute-path names would otherwise sort first); then by name and id.
        let rank = |ix: NodeIndex| {
            let n = snap.node(ix);
            (
                !n.name.to_lowercase().contains(&lowered),
                n.kind.as_deref() == Some("File"),
            )
        };
        hits.sort_by(|a, b| {
            rank(*a)
                .cmp(&rank(*b))
                .then_with(|| snap.order_key(*a).cmp(&snap.order_key(*b)))
        });
        Ok(hits
            .into_iter()
            .take(max)
            .map(|ix| snap.reference(ix))
            .collect())
    }

    async fn dependencies(
        &self,
        node_id: &str,
        edge_type: &str,
        depth: i32,
        dir: petgraph::Direction,
    ) -> Result<Vec<DependencyNode>> {
        let snap = self.snapshot().await?;
        let Some(start) = snap.resolve(node_id) else {
            return Ok(Vec::new());
        };
        let kind = edge_kind_from_input(edge_type);
        Ok(snap
            .expand(start, &kind, dir, safe_depth(depth, 3))
            .into_iter()
            .map(|(ix, d)| {
                let info = snap.info(ix);
                DependencyNode {
                    id: info.id,
                    name: info.name,
                    kind: info.kind,
                    location: info.location,
                    language: info.language,
                    content: info.content,
                    metadata: info.metadata,
                    dependency_depth: (dir == Outgoing).then_some(d),
                    dependent_depth: (dir == Incoming).then_some(d),
                }
            })
            .collect())
    }

    pub async fn get_transitive_dependencies(
        &self,
        node_id: &str,
        edge_type: &str,
        depth: i32,
    ) -> Result<Vec<DependencyNode>> {
        self.dependencies(node_id, edge_type, depth, Outgoing).await
    }

    pub async fn get_reverse_dependencies(
        &self,
        node_id: &str,
        edge_type: &str,
        depth: i32,
    ) -> Result<Vec<DependencyNode>> {
        self.dependencies(node_id, edge_type, depth, Incoming).await
    }

    /// Pairs A→B and B→A over one edge kind, reported once with `node1_id < node2_id`.
    pub async fn detect_circular_dependencies(
        &self,
        edge_type: &str,
    ) -> Result<Vec<CircularDependency>> {
        let snap = self.snapshot().await?;
        let kind = edge_kind_from_input(edge_type);
        let mut pairs = HashSet::new();
        for ix in snap.graph.node_indices() {
            for target in snap.neighbours(ix, Outgoing, |k| k == kind) {
                let (a, b) = (snap.node(ix), snap.node(target));
                if ix != target
                    && a.id < b.id
                    && snap
                        .neighbours(target, Outgoing, |k| k == kind)
                        .contains(&ix)
                {
                    pairs.insert((ix, target));
                }
            }
        }
        let mut cycles: Vec<CircularDependency> = pairs
            .into_iter()
            .map(|(a, b)| CircularDependency {
                node1_id: snap.node(a).id.clone(),
                node2_id: snap.node(b).id.clone(),
                node1: snap.info(a),
                node2: snap.info(b),
                dependency_type: kind.clone(),
            })
            .collect();
        cycles.sort_by(|x, y| (&x.node1_id, &x.node2_id).cmp(&(&y.node1_id, &y.node2_id)));
        Ok(cycles)
    }

    /// Breadth-first walk over CALLS with the name path from the start and up to ten callers.
    pub async fn trace_call_chain(
        &self,
        from_node: &str,
        max_depth: i32,
    ) -> Result<Vec<CallChainNode>> {
        let snap = self.snapshot().await?;
        let Some(start) = snap.resolve(from_node) else {
            return Ok(Vec::new());
        };
        let mut visited = HashSet::from([start]);
        let mut frontier = vec![(start, vec![snap.node(start).name.clone()])];
        let mut chain = Vec::new();
        for depth in 1..=safe_depth(max_depth, 5) {
            let mut next = Vec::new();
            for (ix, path) in &frontier {
                for callee in snap.neighbours(*ix, Outgoing, |k| k == CALLS) {
                    if visited.insert(callee) {
                        let mut path = path.clone();
                        path.push(snap.node(callee).name.clone());
                        next.push((callee, path));
                    }
                }
            }
            for (ix, path) in &next {
                let info = snap.info(*ix);
                let callers = snap
                    .neighbours(*ix, Incoming, |k| k == CALLS)
                    .into_iter()
                    .take(MAX_CALLERS)
                    .map(|c| snap.caller(c))
                    .collect();
                chain.push(CallChainNode {
                    id: info.id,
                    name: info.name,
                    kind: info.kind,
                    location: info.location,
                    language: info.language,
                    content: info.content,
                    metadata: info.metadata,
                    call_depth: Some(depth),
                    call_path: Some(path.clone()),
                    called_by: Some(callers),
                });
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        Ok(chain)
    }

    pub async fn calculate_coupling_metrics(&self, node_id: &str) -> Result<CouplingMetricsResult> {
        let snap = self.snapshot().await?;
        let ix = snap
            .resolve(node_id)
            .ok_or_else(|| anyhow!("Node not found: {node_id}"))?;
        let dependents = snap.neighbours(ix, Incoming, is_dependency);
        let dependencies = snap.neighbours(ix, Outgoing, is_dependency);
        let afferent = dependents.len() as i32;
        let efferent = dependencies.len() as i32;
        let total = afferent + efferent;
        let instability = if total > 0 {
            round(efferent as f64 / total as f64, 6)
        } else {
            0.0
        };
        let category = if instability < 0.3 {
            "stable"
        } else if instability > 0.7 {
            "unstable"
        } else {
            "balanced"
        };
        Ok(CouplingMetricsResult {
            node: snap.info(ix),
            metrics: CouplingMetrics {
                afferent_coupling: afferent,
                efferent_coupling: efferent,
                total_coupling: total,
                instability,
                stability: 1.0 - instability,
                is_stable: instability < 0.3,
                is_unstable: instability > 0.7,
                coupling_category: category.to_string(),
            },
            dependents: dependents.into_iter().map(|d| snap.reference(d)).collect(),
            dependencies: dependencies
                .into_iter()
                .map(|d| snap.reference(d))
                .collect(),
        })
    }

    /// Nodes with at least `min_degree` dependency edges (default 5), top 50 by total degree.
    /// Like code-review-graph's `find_hub_nodes`, File nodes are never hubs.
    pub async fn get_hub_nodes(&self, min_degree: i32) -> Result<Vec<HubNode>> {
        let snap = self.snapshot().await?;
        let threshold = if min_degree > 0 { min_degree } else { 5 };
        let mut hubs: Vec<HubNode> = snap
            .graph
            .node_indices()
            .filter(|&ix| snap.node(ix).kind.as_deref() != Some("File"))
            .filter_map(|ix| {
                let incoming = snap.degree_by_kind(ix, Incoming, is_dependency);
                let outgoing = snap.degree_by_kind(ix, Outgoing, is_dependency);
                let afferent: i32 = incoming.iter().map(|(_, c)| c).sum();
                let efferent: i32 = outgoing.iter().map(|(_, c)| c).sum();
                let to_counts = |rows: Vec<(String, i32)>| {
                    rows.into_iter()
                        .map(|(edge_type, count)| EdgeTypeCount { edge_type, count })
                        .collect()
                };
                (afferent + efferent >= threshold).then(|| HubNode {
                    node_id: snap.node(ix).id.clone(),
                    node: snap.summary(ix),
                    afferent_degree: afferent,
                    efferent_degree: efferent,
                    total_degree: afferent + efferent,
                    incoming_by_type: to_counts(incoming),
                    outgoing_by_type: to_counts(outgoing),
                })
            })
            .collect();
        hubs.sort_by(|a, b| {
            b.total_degree
                .cmp(&a.total_degree)
                .then_with(|| (&a.node.name, &a.node_id).cmp(&(&b.node.name, &b.node_id)))
        });
        hubs.truncate(MAX_HUBS);
        Ok(hubs)
    }

    /// Functions with complexity ≥ `min_complexity` (default 5), ranked by
    /// complexity × (dependents + 1); `limit` 1..=100, default 20.
    pub async fn get_complexity_hotspots(
        &self,
        min_complexity: f32,
        limit: i32,
    ) -> Result<Vec<ComplexityHotspot>> {
        let snap = self.snapshot().await?;
        let limit = if (1..=100).contains(&limit) {
            limit as usize
        } else {
            20
        };
        let threshold = if min_complexity > 0.0 {
            min_complexity
        } else {
            5.0
        };
        let mut candidates: Vec<(NodeIndex, f32)> = snap
            .graph
            .node_indices()
            .filter_map(|ix| {
                let n = snap.node(ix);
                let complexity = n.complexity?;
                (n.kind.as_deref() == Some("Function") && complexity >= threshold)
                    .then_some((ix, complexity))
            })
            .collect();
        candidates.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then_with(|| snap.order_key(a.0).cmp(&snap.order_key(b.0)))
        });
        candidates.truncate(limit * 2);
        let degree = |ix, dir| -> i32 {
            snap.degree_by_kind(ix, dir, is_dependency)
                .iter()
                .map(|(_, c)| c)
                .sum()
        };
        let mut spots: Vec<ComplexityHotspot> = candidates
            .into_iter()
            .map(|(ix, complexity)| {
                let n = snap.node(ix);
                let afferent = degree(ix, Incoming);
                let efferent = degree(ix, Outgoing);
                let total = afferent + efferent;
                let instability = if total > 0 {
                    round(efferent as f64 / total as f64, 4) as f32
                } else {
                    0.0
                };
                ComplexityHotspot {
                    id: n.id.clone(),
                    name: n.name.clone(),
                    kind: n.kind.clone(),
                    language: n.language.clone(),
                    file_path: Some(n.file_path.clone()),
                    start_line: n.start_line,
                    end_line: n.end_line,
                    complexity,
                    afferent_coupling: afferent,
                    efferent_coupling: efferent,
                    instability,
                    risk_score: complexity * (afferent + 1) as f32,
                }
            })
            .collect();
        spots.sort_by(|a, b| b.risk_score.total_cmp(&a.risk_score));
        spots.truncate(limit);
        Ok(spots)
    }

    pub async fn count_nodes_for_project(&self) -> Result<usize> {
        Ok(self.snapshot().await?.graph.node_count())
    }
}
