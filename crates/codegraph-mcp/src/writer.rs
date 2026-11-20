// ABOUTME: Bounded asynchronous graph writer over the SQLite store with durable ordering barriers.
// ABOUTME: One write connection applies batches in order; byte and row limits bound queue memory.

use anyhow::{Result, anyhow};
use codegraph_core::CodeNode;
use codegraph_graph::{
    FileMetadataRecord, ProjectMetadataRecord, edge::CodeEdge, sqlite_store::SqliteGraphStore,
};
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use tokio::task::JoinHandle;

enum Job {
    Nodes(Vec<CodeNode>),
    Edges(Vec<CodeEdge>),
    FileMetadata(Vec<FileMetadataRecord>),
    DeleteFiles { paths: Vec<String>, project: String },
    ProjectMetadata(ProjectMetadataRecord),
    Flush(oneshot::Sender<Result<()>>),
    Shutdown(oneshot::Sender<Result<()>>),
}

struct Envelope {
    job: Job,
    _memory: Option<OwnedSemaphorePermit>,
    bytes: u64,
}

#[derive(Clone, Copy)]
struct Limits {
    rows: usize,
    bytes: usize,
    queue_bytes: usize,
}

impl Limits {
    fn from_env() -> Self {
        fn value(name: &str, default: usize) -> usize {
            std::env::var(name)
                .ok()
                .and_then(|s| s.parse().ok())
                .filter(|v| *v > 0)
                .unwrap_or(default)
        }
        let queue_bytes =
            value("CODEGRAPH_WRITE_QUEUE_BYTES", 32 * 1024 * 1024).clamp(1024, u32::MAX as usize);
        Self {
            rows: value("CODEGRAPH_DB_BATCH_ROWS", 512),
            bytes: value("CODEGRAPH_DB_BATCH_BYTES", 4 * 1024 * 1024).min(queue_bytes),
            queue_bytes,
        }
    }
}

struct ByteCounter(usize);
impl std::io::Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn encoded_size(value: &impl Serialize) -> Result<usize> {
    let mut counter = ByteCounter(0);
    serde_json::to_writer(&mut counter, value)?;
    Ok(counter.0)
}

#[derive(Default)]
struct WriteMetrics {
    jobs: AtomicU64,
    rows: AtomicU64,
    bytes: AtomicU64,
}
impl Job {
    fn rows(&self) -> usize {
        match self {
            Self::Nodes(rows) => rows.len(),
            Self::Edges(rows) => rows.len(),
            Self::FileMetadata(rows) => rows.len(),
            Self::ProjectMetadata(_) => 1,
            Self::DeleteFiles { paths, .. } => paths.len(),
            _ => 0,
        }
    }
}
async fn execute(
    job: Job,
    store: &SqliteGraphStore,
    metrics: &WriteMetrics,
    bytes: u64,
) -> Result<()> {
    let rows = job.rows();
    match job {
        Job::Nodes(rows) => store.upsert_nodes_batch(&rows).await?,
        Job::Edges(rows) => store.upsert_edges_batch(&rows).await?,
        Job::FileMetadata(rows) => store.upsert_file_metadata_batch(&rows).await?,
        Job::DeleteFiles { paths, project } => {
            store.delete_data_for_files(&project, &paths).await?
        }
        Job::ProjectMetadata(row) => store.upsert_project_metadata(row).await?,
        Job::Flush(_) | Job::Shutdown(_) => unreachable!("barriers handled by writer"),
    }
    metrics.jobs.fetch_add(1, Ordering::Relaxed);
    metrics.rows.fetch_add(rows as u64, Ordering::Relaxed);
    metrics.bytes.fetch_add(bytes, Ordering::Relaxed);
    Ok(())
}

fn record_error(result: Result<()>, error: &mut Option<String>) {
    if let Err(err) = result {
        tracing::error!("Graph write failed: {err}");
        error.get_or_insert_with(|| err.to_string());
    }
}

pub(crate) struct GraphWriterHandle {
    tx: mpsc::Sender<Envelope>,
    memory: Arc<Semaphore>,
    limits: Limits,
    join: JoinHandle<()>,
    metrics: Arc<WriteMetrics>,
}

impl GraphWriterHandle {
    /// SQLite has one writer per database, so jobs run one at a time in arrival order.
    pub(crate) fn new(store: SqliteGraphStore) -> Self {
        let limits = Limits::from_env();
        let memory = Arc::new(Semaphore::new(limits.queue_bytes));
        let (tx, mut rx) = mpsc::channel::<Envelope>(8);
        let metrics = Arc::new(WriteMetrics::default());
        let worker_metrics = metrics.clone();
        let join = tokio::spawn(async move {
            let mut error = None;
            while let Some(envelope) = rx.recv().await {
                let Envelope {
                    job,
                    _memory,
                    bytes,
                } = envelope;
                match job {
                    Job::Flush(response) => {
                        let _ =
                            response.send(error.take().map_or(Ok(()), |e: String| Err(anyhow!(e))));
                    }
                    Job::Shutdown(response) => {
                        let _ =
                            response.send(error.take().map_or(Ok(()), |e: String| Err(anyhow!(e))));
                        break;
                    }
                    job @ (Job::DeleteFiles { .. }
                    | Job::FileMetadata(_)
                    | Job::ProjectMetadata(_)) => {
                        // Completion metadata must never mark a failed write set current.
                        if error.is_none() {
                            record_error(
                                execute(job, &store, &worker_metrics, bytes).await,
                                &mut error,
                            );
                        }
                    }
                    job => {
                        record_error(
                            execute(job, &store, &worker_metrics, bytes).await,
                            &mut error,
                        );
                    }
                }
                drop(_memory);
            }
        });
        Self {
            tx,
            memory,
            limits,
            join,
            metrics,
        }
    }

    pub(crate) fn metrics(&self) -> (u64, u64, u64) {
        (
            self.metrics.jobs.load(Ordering::Relaxed),
            self.metrics.rows.load(Ordering::Relaxed),
            self.metrics.bytes.load(Ordering::Relaxed),
        )
    }

    async fn send(&self, job: Job, bytes: usize) -> Result<()> {
        if bytes > self.limits.queue_bytes {
            return Err(anyhow!("Write exceeds queue byte budget: {bytes}"));
        }
        let permit = self
            .memory
            .clone()
            .acquire_many_owned(bytes.max(1) as u32)
            .await?;
        self.tx
            .send(Envelope {
                job,
                _memory: Some(permit),
                bytes: bytes as u64,
            })
            .await
            .map_err(|_| anyhow!("Graph writer unavailable"))
    }

    async fn batch<T: Serialize>(&self, rows: Vec<T>, wrap: impl Fn(Vec<T>) -> Job) -> Result<()> {
        let mut batch = Vec::new();
        let mut bytes = 2usize;
        for row in rows {
            // Count encoded bytes without allocating another serialized batch.
            let size = encoded_size(&row)? + 1;
            if size + 2 > self.limits.bytes {
                return Err(anyhow!(
                    "One graph record needs {} bytes; increase CODEGRAPH_DB_BATCH_BYTES (current {})",
                    size + 2,
                    self.limits.bytes
                ));
            }
            if !batch.is_empty()
                && (batch.len() >= self.limits.rows || bytes + size > self.limits.bytes)
            {
                self.send(wrap(std::mem::take(&mut batch)), bytes).await?;
                bytes = 2;
            }
            bytes += size;
            batch.push(row);
        }
        if !batch.is_empty() {
            self.send(wrap(batch), bytes).await?;
        }
        Ok(())
    }

    pub(crate) async fn enqueue_nodes(&self, rows: Vec<CodeNode>) -> Result<()> {
        self.batch(rows, Job::Nodes).await
    }
    pub(crate) async fn enqueue_edges(&self, rows: Vec<CodeEdge>) -> Result<()> {
        self.batch(rows, Job::Edges).await
    }
    pub(crate) async fn enqueue_file_metadata(&self, rows: Vec<FileMetadataRecord>) -> Result<()> {
        self.batch(rows, Job::FileMetadata).await
    }
    pub(crate) async fn enqueue_delete_nodes_by_file(
        &self,
        paths: Vec<String>,
        project: &str,
    ) -> Result<()> {
        self.batch(paths, |paths| Job::DeleteFiles {
            paths,
            project: project.to_string(),
        })
        .await
    }
    pub(crate) async fn enqueue_project_metadata(&self, row: ProjectMetadataRecord) -> Result<()> {
        let bytes = encoded_size(&row)?;
        self.send(Job::ProjectMetadata(row), bytes).await
    }
    pub(crate) async fn flush(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.send(Job::Flush(tx), 1).await?;
        rx.await
            .map_err(|_| anyhow!("Graph writer ended before acknowledgement"))?
    }
    pub(crate) async fn shutdown(self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.send(Job::Shutdown(tx), 1).await?;
        let result = rx
            .await
            .map_err(|_| anyhow!("Graph writer ended before acknowledgement"))?;
        self.join.await?;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_core::{Language, Location, NodeType};

    fn fixture() -> (GraphWriterHandle, SqliteGraphStore) {
        let store = SqliteGraphStore::open_in_memory(std::path::Path::new("/test")).unwrap();
        // A real constraint failure: SQLite aborts the batch transaction that inserts `bad`.
        store
            .with_conn_blocking(|conn| {
                conn.execute_batch(
                    "CREATE TRIGGER reject_bad BEFORE INSERT ON nodes WHEN NEW.name = 'bad' \
                     BEGIN SELECT RAISE(ABORT, 'rejected node bad'); END;",
                )?;
                Ok(())
            })
            .unwrap();
        let mut writer = GraphWriterHandle::new(store.clone());
        writer.limits.rows = 1;
        writer.limits.bytes = 2048;
        (writer, store)
    }

    fn node(name: &str) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            Some(NodeType::Function),
            Some(Language::Rust),
            Location {
                file_path: "a.rs".into(),
                line: 1,
                column: 0,
                end_line: None,
                end_column: None,
            },
        )
    }

    fn metadata() -> ProjectMetadataRecord {
        ProjectMetadataRecord {
            metadata: serde_json::json!({}),
            project_id: "test".into(),
            name: "test".into(),
            root_path: "/test".into(),
            primary_language: None,
            file_count: 1,
            node_count: 1,
            edge_count: 0,
            avg_coverage_score: 1.0,
            last_analyzed: chrono::Utc::now(),
            codegraph_version: "test".into(),
            organization_id: None,
            domain: None,
        }
    }

    #[tokio::test]
    async fn failed_writes_do_not_advance_metadata_and_flush_allows_retry() {
        let (writer, store) = fixture();
        writer
            .enqueue_nodes(vec![node("good"), node("bad")])
            .await
            .unwrap();
        writer.enqueue_project_metadata(metadata()).await.unwrap();
        assert!(writer.flush().await.is_err());
        assert!(store.project_marker("test").await.unwrap().is_none());
        writer.enqueue_nodes(vec![node("retry")]).await.unwrap();
        writer.enqueue_project_metadata(metadata()).await.unwrap();
        writer.flush().await.unwrap();
        assert!(store.project_marker("test").await.unwrap().is_some());
        writer.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn oversized_records_fail_before_entering_queue() {
        let (writer, _) = fixture();
        let large = node("large").with_content("x".repeat(4096));
        assert!(
            writer
                .enqueue_nodes(vec![large])
                .await
                .unwrap_err()
                .to_string()
                .contains("CODEGRAPH_DB_BATCH_BYTES")
        );
        writer.flush().await.unwrap();
        writer.shutdown().await.unwrap();
    }
}
