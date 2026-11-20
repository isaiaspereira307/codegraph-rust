-- ABOUTME: Rust-only tables kept beside the CRG schema in the same graph.db.
-- ABOUTME: CRG ignores them; they hold stable SHA node/edge ids and indexer state.

CREATE TABLE IF NOT EXISTS cg_nodes (
    uuid TEXT PRIMARY KEY,
    qualified_name TEXT NOT NULL,
    file_path TEXT NOT NULL,
    rust_kind TEXT,
    complexity REAL,
    content TEXT,
    attributes TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_cg_nodes_qn ON cg_nodes(qualified_name);
CREATE INDEX IF NOT EXISTS idx_cg_nodes_file ON cg_nodes(file_path);

CREATE TABLE IF NOT EXISTS cg_edges (
    uuid TEXT PRIMARY KEY,
    edge_id INTEGER NOT NULL,
    from_uuid TEXT NOT NULL,
    to_uuid TEXT NOT NULL,
    file_path TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_cg_edges_from ON cg_edges(from_uuid);
CREATE INDEX IF NOT EXISTS idx_cg_edges_to ON cg_edges(to_uuid);
CREATE INDEX IF NOT EXISTS idx_cg_edges_edge ON cg_edges(edge_id);

CREATE TABLE IF NOT EXISTS cg_file_metadata (
    project_id TEXT NOT NULL,
    file_path TEXT NOT NULL,
    record TEXT NOT NULL,
    PRIMARY KEY (project_id, file_path)
);

CREATE TABLE IF NOT EXISTS cg_project_metadata (
    project_id TEXT PRIMARY KEY,
    record TEXT NOT NULL
);
