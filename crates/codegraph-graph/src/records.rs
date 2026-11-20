// ABOUTME: Indexer bookkeeping records shared by every storage backend.
// ABOUTME: File metadata drives incremental change detection; project metadata marks readiness.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMetadataRecord {
    pub metadata: JsonValue,
    pub project_id: String,
    pub name: String,
    pub root_path: String,
    pub primary_language: Option<String>,
    pub file_count: i64,
    pub node_count: i64,
    pub edge_count: i64,
    pub avg_coverage_score: f32,
    pub last_analyzed: DateTime<Utc>,
    pub codegraph_version: String,
    pub organization_id: Option<String>,
    pub domain: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadataRecord {
    pub file_path: String,
    pub project_id: String,
    pub content_hash: String,
    pub modified_at: DateTime<Utc>,
    pub file_size: i64,
    pub last_indexed_at: DateTime<Utc>,
    pub node_count: i64,
    pub edge_count: i64,
    pub language: Option<String>,
    pub parse_errors: Option<Vec<String>>,
}

impl FileMetadataRecord {
    /// Generate a unique record ID based on project_id and file_path
    pub fn id(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.project_id.as_bytes());
        hasher.update(b":");
        hasher.update(self.file_path.as_bytes());
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    }
}
