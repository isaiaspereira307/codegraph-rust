// ABOUTME: SQLite graph store using the code-review-graph v15 schema.
// ABOUTME: One database per repository at .code-review-graph/graph.db.

mod lock;
pub mod mapping;
mod reconcile;
mod schema;
mod write;

pub use lock::WriteLock;
pub use reconcile::GraphCounts;

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const SCHEMA_VERSION: u32 = 15;

pub fn db_path(project_root: &Path) -> PathBuf {
    project_root.join(".code-review-graph").join("graph.db")
}

/// Handle to one repository's graph database. Clones share the connection.
#[derive(Clone)]
pub struct SqliteGraphStore {
    conn: Arc<Mutex<Connection>>,
    root: PathBuf,
}

impl SqliteGraphStore {
    /// Opens `<project_root>/.code-review-graph/graph.db`, creating a v15 database if absent.
    pub fn open(project_root: &Path) -> Result<Self> {
        let path = db_path(project_root);
        std::fs::create_dir_all(path.parent().expect("db path has a parent"))
            .with_context(|| format!("creating {}", path.display()))?;
        let conn =
            Connection::open(&path).with_context(|| format!("opening {}", path.display()))?;
        Self::from_connection(conn, project_root)
    }

    /// In-memory database for tests; paths are still made absolute against `project_root`.
    pub fn open_in_memory(project_root: &Path) -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?, project_root)
    }

    fn from_connection(mut conn: Connection, project_root: &Path) -> Result<Self> {
        schema::configure(&conn)?;
        schema::ensure(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            root: project_root.to_path_buf(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Runs `f` with the connection on the blocking pool.
    pub(crate) async fn with_conn<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().expect("sqlite connection poisoned");
            f(&mut guard)
        })
        .await?
    }

    /// Synchronous access for callers already off the async runtime.
    pub fn with_conn_blocking<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut guard = self.conn.lock().expect("sqlite connection poisoned");
        f(&mut guard)
    }
}
