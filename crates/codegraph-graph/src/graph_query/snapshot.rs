// ABOUTME: Immutable petgraph copy of the SQLite graph, keyed by node uuid and qualified name.
// ABOUTME: Built from nodes ⟕ cg_nodes and edges; text INHERITS/IMPLEMENTS bind by name, else skip.

use super::heritage::{HeritageNode, is_heritage, last_segment, resolve_heritage};
use super::types::{CallerInfo, NodeInfo, NodeLocation, NodeReference};
use anyhow::Result;
use petgraph::Direction;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};

pub(crate) struct NodeData {
    pub id: String,
    /// Stored qualified name; text edge targets are matched against it.
    pub qualified: String,
    pub name: String,
    pub kind: Option<String>,
    pub file_path: String,
    pub start_line: Option<i32>,
    pub end_line: Option<i32>,
    pub language: Option<String>,
    pub content: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub complexity: Option<f32>,
}

pub(crate) struct Snapshot {
    pub graph: DiGraph<NodeData, String>,
    index: HashMap<String, NodeIndex>,
    /// Bare base name → number of text INHERITS/IMPLEMENTS edges naming it that matched
    /// several type nodes and so were left unbound.
    #[allow(dead_code)] // read by query_graph's `inheritors_of` pattern
    pub ambiguous_heritage: HashMap<String, usize>,
}

impl Snapshot {
    pub fn load(conn: &Connection) -> Result<Self> {
        let mut graph = DiGraph::new();
        let mut index = HashMap::new();
        let mut by_qn = HashMap::new();
        let mut stmt = conn.prepare(
            "SELECT n.qualified_name, n.name, n.kind, n.file_path, n.line_start, n.line_end, \
             n.language, c.uuid, c.content, c.attributes, c.complexity \
             FROM nodes n LEFT JOIN cg_nodes c ON c.qualified_name = n.qualified_name",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let qualified: String = row.get(0)?;
            let uuid: Option<String> = row.get(7)?;
            let attributes: Option<String> = row.get(9)?;
            let ix = graph.add_node(NodeData {
                id: uuid.clone().unwrap_or_else(|| qualified.clone()),
                qualified: qualified.clone(),
                name: row.get(1)?,
                kind: row.get(2)?,
                file_path: row.get(3)?,
                start_line: row.get(4)?,
                end_line: row.get(5)?,
                language: row.get(6)?,
                content: row.get(8)?,
                metadata: attributes.and_then(|a| serde_json::from_str(&a).ok()),
                complexity: row.get::<_, Option<f64>>(10)?.map(|c| c as f32),
            });
            if let Some(uuid) = uuid {
                index.insert(uuid, ix);
            }
            index.insert(qualified.clone(), ix);
            by_qn.insert(qualified, ix);
        }
        // Edges whose target is a qualified name bind directly. Text heritage targets
        // (`Base`, `unittest.TestCase`) are resolved by name below; other edges to unknown
        // targets are skipped.
        let mut heritage: Vec<(String, usize, String)> = Vec::new();
        // File path → file paths of the File nodes it imports (IMPORTS_FROM File → File).
        let mut imports: HashMap<String, Vec<String>> = HashMap::new();
        let mut stmt =
            conn.prepare("SELECT kind, source_qualified, target_qualified FROM edges")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let (kind, source, target): (String, String, String) =
                (row.get(0)?, row.get(1)?, row.get(2)?);
            let Some(&s) = by_qn.get(&source) else {
                continue;
            };
            match by_qn.get(&target) {
                Some(&t) => {
                    let is_file = |ix: NodeIndex| graph[ix].kind.as_deref() == Some("File");
                    if kind == "IMPORTS_FROM" && is_file(s) && is_file(t) {
                        imports
                            .entry(graph[s].file_path.clone())
                            .or_default()
                            .push(graph[t].file_path.clone());
                    }
                    graph.add_edge(s, t, kind);
                }
                None if is_heritage(&kind) => {
                    heritage.push((kind, s.index(), target));
                }
                None => {}
            }
        }
        let mut ambiguous_heritage = HashMap::new();
        if !heritage.is_empty() {
            let nodes: Vec<HeritageNode<'_>> = graph
                .node_weights()
                .map(|n| HeritageNode {
                    qualified: &n.qualified,
                    name: &n.name,
                    kind: n.kind.as_deref(),
                    file_path: &n.file_path,
                    language: n.language.as_deref(),
                })
                .collect();
            let imported = |ix: usize| {
                imports
                    .get(nodes[ix].file_path)
                    .cloned()
                    .unwrap_or_default()
            };
            // One call per kind, so every derived edge keeps the kind it was written with;
            // each call indexes the candidates once.
            let mut derived = Vec::new();
            for kind in ["INHERITS", "IMPLEMENTS"] {
                let edges: Vec<(usize, String)> = heritage
                    .iter()
                    .filter(|(k, _, _)| *k == kind)
                    .map(|(_, s, t)| (*s, t.clone()))
                    .collect();
                if edges.is_empty() {
                    continue;
                }
                let r = resolve_heritage(&nodes, &edges, &imported);
                derived.extend(r.bound.into_iter().map(|(s, t)| (s, t, kind)));
                for (_, text) in r.ambiguous {
                    *ambiguous_heritage
                        .entry(last_segment(&text).to_string())
                        .or_insert(0) += 1;
                }
            }
            for (s, t, kind) in derived {
                graph.add_edge(NodeIndex::new(s), NodeIndex::new(t), kind.to_string());
            }
        }
        Ok(Self {
            graph,
            index,
            ambiguous_heritage,
        })
    }

    /// Accepts a uuid, a qualified name, or the SurrealDB forms `nodes:id`, ``nodes:`id` ``
    /// and `nodes:⟨id⟩` (with anything after the closing quote ignored).
    pub fn resolve(&self, input: &str) -> Option<NodeIndex> {
        let s = input.trim();
        let s = s.strip_prefix("nodes:").unwrap_or(s);
        let s = if let Some(rest) = s.strip_prefix('`') {
            rest.split('`').next().unwrap_or(rest)
        } else if let Some(rest) = s.strip_prefix('⟨') {
            rest.split('⟩').next().unwrap_or(rest)
        } else {
            s
        };
        self.index.get(s).copied()
    }

    pub fn node(&self, ix: NodeIndex) -> &NodeData {
        &self.graph[ix]
    }

    fn location(&self, ix: NodeIndex) -> NodeLocation {
        let n = self.node(ix);
        NodeLocation {
            file_path: n.file_path.clone(),
            start_line: n.start_line,
            end_line: n.end_line,
        }
    }

    pub fn info(&self, ix: NodeIndex) -> NodeInfo {
        let n = self.node(ix);
        NodeInfo {
            id: n.id.clone(),
            name: n.name.clone(),
            kind: n.kind.clone(),
            location: Some(self.location(ix)),
            language: n.language.clone(),
            content: n.content.clone(),
            metadata: n.metadata.clone(),
        }
    }

    /// Identity and location only, for ranked lists.
    pub fn summary(&self, ix: NodeIndex) -> NodeInfo {
        NodeInfo {
            content: None,
            metadata: None,
            ..self.info(ix)
        }
    }

    pub fn reference(&self, ix: NodeIndex) -> NodeReference {
        let n = self.node(ix);
        NodeReference {
            id: n.id.clone(),
            name: n.name.clone(),
            kind: n.kind.clone(),
            location: Some(self.location(ix)),
        }
    }

    pub fn caller(&self, ix: NodeIndex) -> CallerInfo {
        let n = self.node(ix);
        CallerInfo {
            id: n.id.clone(),
            name: n.name.clone(),
            kind: n.kind.clone(),
        }
    }

    /// Sort key that makes every listing deterministic.
    pub fn order_key(&self, ix: NodeIndex) -> (&str, &str) {
        let n = self.node(ix);
        (n.name.as_str(), n.id.as_str())
    }

    /// Distinct neighbours over edges whose kind passes `keep`, sorted by name.
    pub fn neighbours(
        &self,
        ix: NodeIndex,
        dir: Direction,
        keep: impl Fn(&str) -> bool,
    ) -> Vec<NodeIndex> {
        let mut seen = HashSet::new();
        let mut out: Vec<NodeIndex> = self
            .graph
            .edges_directed(ix, dir)
            .filter(|e| keep(e.weight()))
            .map(|e| {
                if dir == Direction::Outgoing {
                    e.target()
                } else {
                    e.source()
                }
            })
            .filter(|n| seen.insert(*n))
            .collect();
        out.sort_by(|a, b| self.order_key(*a).cmp(&self.order_key(*b)));
        out
    }

    /// Edge rows (not distinct neighbours) whose kind passes `keep`, grouped by kind.
    pub fn degree_by_kind(
        &self,
        ix: NodeIndex,
        dir: Direction,
        keep: impl Fn(&str) -> bool,
    ) -> Vec<(String, i32)> {
        let mut counts: HashMap<&str, i32> = HashMap::new();
        for e in self.graph.edges_directed(ix, dir) {
            if keep(e.weight()) {
                *counts.entry(e.weight().as_str()).or_default() += 1;
            }
        }
        let mut out: Vec<(String, i32)> = counts
            .into_iter()
            .map(|(k, c)| (k.to_string(), c))
            .collect();
        out.sort();
        out
    }

    /// Breadth-first walk over one edge kind; each node keeps the first depth it was reached at.
    pub fn expand(
        &self,
        start: NodeIndex,
        kind: &str,
        dir: Direction,
        max_depth: i32,
    ) -> Vec<(NodeIndex, i32)> {
        let mut visited = HashSet::from([start]);
        let mut frontier = vec![start];
        let mut reached = Vec::new();
        for depth in 1..=max_depth {
            let mut next = Vec::new();
            for &n in &frontier {
                for m in self.neighbours(n, dir, |k| k == kind) {
                    if visited.insert(m) {
                        next.push(m);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            next.sort_by(|a, b| self.order_key(*a).cmp(&self.order_key(*b)));
            reached.extend(next.iter().map(|&n| (n, depth)));
            frontier = next;
        }
        reached
    }
}
