use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

pub type NodeId = Uuid;
pub type EdgeId = Uuid;

/// Encode hash bytes as lowercase hexadecimal without intermediate allocations.
pub fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        text.push(HEX[(byte >> 4) as usize] as char);
        text.push(HEX[(byte & 0x0f) as usize] as char);
    }
    text
}

/// Generate a deterministic node ID from stable attributes.
/// Same code entity = same ID across indexing runs, enabling proper UPSERT behavior.
pub fn generate_node_id(
    project_id: &str,
    file_path: &str,
    name: &str,
    node_type: &str,
    start_line: u32,
) -> NodeId {
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(b"|");
    hasher.update(file_path.as_bytes());
    hasher.update(b"|");
    hasher.update(name.as_bytes());
    hasher.update(b"|");
    hasher.update(node_type.as_bytes());
    hasher.update(b"|");
    hasher.update(start_line.to_le_bytes());

    let hash = hasher.finalize();
    // Convert first 16 bytes to UUID format for compatibility with existing storage
    Uuid::from_slice(&hash[..16]).expect("SHA256 hash should always produce at least 16 bytes")
}

/// Stable file key for ID derivation: the path relative to `project_root` with `/`
/// separators, so `.`, `./a.py`, `a.py` and `/abs/root/a.py` all hash the same.
/// Paths outside the root keep their canonical absolute form.
pub fn canonical_file_key(project_root: &std::path::Path, file_path: &str) -> String {
    use std::path::{Component, Path, PathBuf};

    fn lexical(path: &Path) -> PathBuf {
        let mut out = PathBuf::new();
        for comp in path.components() {
            match comp {
                Component::CurDir => {}
                Component::ParentDir => {
                    out.pop();
                }
                other => out.push(other.as_os_str()),
            }
        }
        out
    }

    let raw = Path::new(file_path);
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        project_root.join(raw)
    };
    let abs = std::fs::canonicalize(&joined).unwrap_or_else(|_| lexical(&joined));
    let root = std::fs::canonicalize(project_root).unwrap_or_else(|_| lexical(project_root));
    let key = match abs.strip_prefix(&root) {
        Ok(rel) => rel,
        Err(_) => abs.as_path(),
    };
    key.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, bincode::Encode, bincode::Decode,
)]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Java,
    Cpp,
    // Revolutionary universal language support
    Swift,
    Kotlin,
    CSharp,
    Ruby,
    Php,
    Dart,
    Other(String),
}

#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, bincode::Encode, bincode::Decode,
)]
pub enum NodeType {
    Function,
    Struct,
    Enum,
    Trait,
    Module,
    Directory,
    Variable,
    Import,
    Class,
    Interface,
    Type,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum EdgeType {
    Calls,
    Defines,
    Uses,
    Imports,
    Extends,
    Implements,
    Contains,
    #[default]
    References,
    Other(String),
}

impl fmt::Display for EdgeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            EdgeType::Calls => "calls",
            EdgeType::Defines => "defines",
            EdgeType::Uses => "uses",
            EdgeType::Imports => "imports",
            EdgeType::Extends => "extends",
            EdgeType::Implements => "implements",
            EdgeType::Contains => "contains",
            EdgeType::References => "references",
            EdgeType::Other(s) => s.as_str(),
        };
        write!(f, "{}", s)
    }
}

impl FromStr for EdgeType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "calls" => Ok(EdgeType::Calls),
            "defines" => Ok(EdgeType::Defines),
            "uses" => Ok(EdgeType::Uses),
            "imports" => Ok(EdgeType::Imports),
            "extends" => Ok(EdgeType::Extends),
            "implements" => Ok(EdgeType::Implements),
            "contains" => Ok(EdgeType::Contains),
            "references" => Ok(EdgeType::References),
            other => Ok(EdgeType::Other(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]
pub struct Location {
    pub file_path: String,
    pub line: u32,
    pub column: u32,
    pub end_line: Option<u32>,
    pub end_column: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode, Default)]
pub struct Span {
    pub start_byte: u32,
    pub end_byte: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub attributes: HashMap<String, String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Universal extraction result for single-pass node + edge generation (FASTEST approach)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionResult {
    pub nodes: Vec<crate::CodeNode>,
    pub edges: Vec<EdgeRelationship>,
}

/// Represents a relationship between code entities extracted during AST traversal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeRelationship {
    pub from: NodeId,
    pub to: String, // Symbol name to be resolved to NodeId later
    pub edge_type: EdgeType,
    pub metadata: HashMap<String, String>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChangeEvent {
    Created(String),  // file path
    Modified(String), // file path
    Deleted(String),  // file path
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePayload {
    pub event: ChangeEvent,
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delta {
    pub file_path: String,
    pub changes: Vec<String>,
}

#[cfg(test)]
mod canonical_file_key_tests {
    use super::{canonical_file_key, generate_node_id};
    use std::fs;

    #[test]
    fn spellings_of_the_same_file_share_one_key_and_id() {
        let root = std::env::temp_dir().join(format!("cg-file-key-{}", std::process::id()));
        fs::create_dir_all(root.join("pkg")).unwrap();
        fs::write(root.join("pkg/a.py"), "x = 1\n").unwrap();

        let abs = root.join("pkg/a.py").to_string_lossy().into_owned();
        let keys = [
            canonical_file_key(&root, "pkg/a.py"),
            canonical_file_key(&root, "./pkg/a.py"),
            canonical_file_key(&root, &abs),
            canonical_file_key(&root, "pkg/../pkg/a.py"),
        ];
        assert!(keys.iter().all(|k| k == "pkg/a.py"), "{keys:?}");

        let ids: Vec<_> = keys
            .iter()
            .map(|k| generate_node_id("p", k, "a.py", "File", 1))
            .collect();
        assert!(ids.windows(2).all(|w| w[0] == w[1]));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn paths_outside_the_root_keep_an_absolute_key() {
        let root = std::env::temp_dir().join(format!("cg-file-key-out-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let outside = std::env::temp_dir().join("cg-file-key-elsewhere.py");
        fs::write(&outside, "").unwrap();
        let key = canonical_file_key(&root, &outside.to_string_lossy());
        assert!(key.starts_with('/') || key.contains(':'), "{key}");
        fs::remove_dir_all(&root).unwrap();
        fs::remove_file(&outside).unwrap();
    }
}
