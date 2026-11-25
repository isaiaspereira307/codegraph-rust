// ABOUTME: Maps CodeGraph node/edge kinds and identities onto code-review-graph conventions.
// ABOUTME: Pure functions; see docs/architecture/storage-sqlite.md for the tables.

use codegraph_core::{CodeNode, EdgeType, Language, NodeType};
use std::path::Path;

/// Edge kinds that count as a dependency for hubs, coupling and hotspots: the CRG names of
/// the old `$dependency_edge_types` list.
pub const DEPENDENCY_EDGE_KINDS: &[&str] = &[
    "CALLS",
    "DEFINES",
    "IMPORTS_FROM",
    "USES",
    "INHERITS",
    "IMPLEMENTS",
    "REFERENCES",
    "DEPENDS_ON",
    "EXPORTS",
    "REEXPORTS",
    "ENABLES",
    "GENERATES",
    "FLOWS_TO",
    "RETURNS",
    "CAPTURES",
    "MUTATES",
];

pub fn crg_node_kind(kind: &NodeType) -> String {
    match kind {
        NodeType::Function => "Function",
        NodeType::Class | NodeType::Struct | NodeType::Trait | NodeType::Interface => "Class",
        NodeType::Enum | NodeType::Type => "Type",
        NodeType::Module => "Module",
        NodeType::Directory => "Directory",
        NodeType::Variable => "Variable",
        NodeType::Import => "Import",
        NodeType::Other(other) => return other.clone(),
    }
    .to_string()
}

/// The Rust variant name, kept in `cg_nodes.rust_kind`.
pub fn rust_node_kind(kind: &NodeType) -> String {
    match kind {
        NodeType::Other(other) => other.clone(),
        other => format!("{other:?}"),
    }
}

pub fn crg_edge_kind(kind: &EdgeType) -> String {
    edge_kind_from_input(&kind.to_string())
}

/// Accepts a CodeGraph edge name (`calls`, `imports`) or a CRG kind (`CALLS`, `IMPORTS_FROM`).
pub fn edge_kind_from_input(input: &str) -> String {
    match input.trim().to_ascii_lowercase().as_str() {
        "imports" | "imports_from" => "IMPORTS_FROM".to_string(),
        "extends" | "inherits" => "INHERITS".to_string(),
        other => other.to_ascii_uppercase(),
    }
}

pub fn language_name(language: &Language) -> String {
    match language {
        Language::Other(other) => other.to_ascii_lowercase(),
        other => format!("{other:?}").to_ascii_lowercase(),
    }
}

/// Absolute form of a node's path; relative paths are taken against the project root.
pub fn absolute_path(root: &Path, file_path: &str) -> String {
    let path = Path::new(file_path);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path.strip_prefix("./").unwrap_or(path))
    };
    joined.to_string_lossy().into_owned()
}

/// `<abs_path>` for file-like nodes, `<abs_path>::<symbol>` otherwise.
pub fn qualified_name(root: &Path, node: &CodeNode) -> String {
    let path = absolute_path(root, &node.location.file_path);
    match symbol(root, node) {
        Some(symbol) => format!("{path}::{symbol}"),
        None => path,
    }
}

/// The part of the qualified name after the path, or `None` for a file or directory node.
pub fn symbol(root: &Path, node: &CodeNode) -> Option<String> {
    if matches!(node.node_type, Some(NodeType::Directory)) {
        return None;
    }
    let raw = &node.location.file_path;
    let path = absolute_path(root, raw);
    // The CRG-shape pass stores a root-relative, normalized key; older nodes (and LSP
    // enrichment) may still carry the raw or absolute spelling. All three mean the file.
    let key = codegraph_core::canonical_file_key(root, raw);
    let attr = node.metadata.attributes.get("qualified_name");
    let symbol = match attr {
        Some(qn) if qn == raw || *qn == path || *qn == key => return None,
        Some(qn) => [raw.as_str(), path.as_str(), key.as_str()]
            .iter()
            .find_map(|prefix| qn.strip_prefix(&format!("{prefix}::")))
            .unwrap_or(qn)
            .to_string(),
        None => node.name.to_string(),
    };
    Some(symbol)
}

const MAX_INDEXED_TOKEN_CHARS: usize = 200;

/// CRG's `node_index_tokens`: camelCase splits of name, parent and file stem that FTS5's
/// unicode61 tokenizer cannot produce itself.
pub fn name_tokens(kind: &str, name: &str, parent: Option<&str>, file_path: &str) -> String {
    let stem = Path::new(file_path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = if kind == "File" { "" } else { name };
    let mut out: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for value in [Some(name), parent, Some(stem.as_str())]
        .into_iter()
        .flatten()
    {
        for run in value
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|r| !r.is_empty())
        {
            let parts = camel_parts(run);
            if parts.len() < 2 {
                continue;
            }
            for part in parts {
                if seen.insert(part.to_ascii_lowercase()) {
                    out.push(part.to_string());
                }
            }
        }
    }
    let joined = out.join(" ");
    joined.chars().take(MAX_INDEXED_TOKEN_CHARS).collect()
}

/// Splits at `(?<=[a-z0-9])(?=[A-Z])` and `(?<=[A-Z])(?=[A-Z][a-z])`.
fn camel_parts(run: &str) -> Vec<&str> {
    let bytes = run.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    for i in 1..bytes.len() {
        let (prev, cur) = (bytes[i - 1], bytes[i]);
        let lower_to_upper =
            (prev.is_ascii_lowercase() || prev.is_ascii_digit()) && cur.is_ascii_uppercase();
        let acronym_end = prev.is_ascii_uppercase()
            && cur.is_ascii_uppercase()
            && bytes.get(i + 1).is_some_and(|n| n.is_ascii_lowercase());
        if lower_to_upper || acronym_end {
            parts.push(&run[start..i]);
            start = i;
        }
    }
    parts.push(&run[start..]);
    parts
}
