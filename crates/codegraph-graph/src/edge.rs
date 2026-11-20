use codegraph_core::{EdgeId, EdgeType, NodeId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Metadata key carrying the verbatim target of an unresolved edge.
pub const UNRESOLVED_TARGET_KEY: &str = "unresolved_target";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeEdge {
    pub id: EdgeId,
    pub from: NodeId,
    pub to: NodeId,
    pub edge_type: EdgeType,
    pub weight: f64,
    pub metadata: HashMap<String, String>,
    pub project_id: Option<String>,
}

impl CodeEdge {
    pub fn new(from: NodeId, to: NodeId, edge_type: EdgeType) -> Self {
        Self {
            id: EdgeId::new_v4(),
            from,
            to,
            edge_type,
            weight: 1.0,
            metadata: HashMap::new(),
            project_id: None,
        }
    }

    /// An edge whose target names no indexed node. It is stored, as code-review-graph does,
    /// with `target_qualified` set to `target` verbatim. `to` is a placeholder id derived from
    /// the text, so occurrences of one target share it; it never names a node.
    pub fn unresolved(from: NodeId, target: &str, edge_type: EdgeType) -> Self {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(
            serde_json::to_vec(&("unresolved-target-v1", target)).expect("serializable target"),
        );
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&digest[..16]);
        Self::new(from, NodeId::from_bytes(bytes), edge_type)
            .with_metadata(UNRESOLVED_TARGET_KEY.into(), target.into())
    }

    /// The target text of an edge built by [`CodeEdge::unresolved`].
    pub fn unresolved_target(&self) -> Option<&str> {
        self.metadata.get(UNRESOLVED_TARGET_KEY).map(String::as_str)
    }

    pub fn with_weight(mut self, weight: f64) -> Self {
        self.weight = weight;
        self
    }

    pub fn with_metadata(mut self, key: String, value: String) -> Self {
        self.metadata.insert(key, value);
        self
    }

    pub fn with_project_id(mut self, project_id: impl Into<String>) -> Self {
        self.project_id = Some(project_id.into());
        self
    }

    /// Stable identity includes provenance, including source span/occurrence metadata.
    /// Re-indexing an occurrence updates it; distinct call sites remain distinct edges.
    pub fn set_deterministic_id(&mut self, project_id: &str) {
        use sha2::{Digest, Sha256};
        let metadata: std::collections::BTreeMap<_, _> = self.metadata.iter().collect();
        let identity = serde_json::to_vec(&(
            "edge-v1",
            project_id,
            self.from,
            self.to,
            self.edge_type.to_string(),
            metadata,
        ))
        .expect("serializable edge identity");
        let digest = Sha256::digest(identity);
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&digest[..16]);
        self.id = EdgeId::from_bytes(bytes);
        self.project_id = Some(project_id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_stable_scoped_and_preserves_occurrences() {
        let mut edge = CodeEdge::new(NodeId::new_v4(), NodeId::new_v4(), EdgeType::Calls)
            .with_metadata("source_span".into(), "1:4".into());
        edge.set_deterministic_id("project");
        let mut again = edge.clone();
        again.id = EdgeId::new_v4();
        again.set_deterministic_id("project");
        assert_eq!(edge.id, again.id);
        again.metadata.insert("source_span".into(), "10:14".into());
        again.set_deterministic_id("project");
        assert_ne!(edge.id, again.id);
        again = edge.clone();
        again.set_deterministic_id("other-project");
        assert_ne!(edge.id, again.id);
    }
}
