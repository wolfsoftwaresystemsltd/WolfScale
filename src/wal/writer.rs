//! WAL Writer
//!
//! High-performance, batched writer for the Write-Ahead Log.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, RwLock};

use super::entry::{LogEntry, Lsn, WalEntry};
use super::segment::Segment;
use super::WalPaths;
use crate::config::WalConfig;
use crate::error::{Error, Result};

/// Write request sent to the writer task
struct WriteRequest {
    entry: LogEntry,
    response: oneshot::Sender<Result<Lsn>>,
}

/// WAL Writer handle
///
/// This is a cloneable handle to the underlying writer task.
#[derive(Clone)]
pub struct WalWriter {
    /// Channel to send write requests
    sender: mpsc::Sender<WriteRequest>,
    /// Shared state for reading current LSN
    state: Arc<RwLock<WriterState>>,
}

/// Shared writer state
struct WriterState {
    /// Current LSN
    current_lsn: Lsn,
    /// Current term (for replication)
    current_term: u64,
    /// Node ID
    node_id: String,
}

/// Internal writer that manages segments
struct WriterInner {
    /// WAL paths
    paths: WalPaths,
    /// Configuration
    config: WalConfig,
    /// Current active segment
    current_segment: Option<Segment>,
    /// Write buffer for batching
    buffer: VecDeque<(WalEntry, oneshot::Sender<Result<Lsn>>)>,
    /// Last flush time
    last_flush: Instant,
    /// Shared state
    state: Arc<RwLock<WriterState>>,
}

impl WalWriter {
    /// Create a new WAL writer
    pub async fn new(
        data_dir: PathBuf,
        config: WalConfig,
        node_id: String,
    ) -> Result<Self> {
        let paths = WalPaths::new(data_dir.join("wal"));
        paths.ensure_dirs()?;

        // Find the last LSN from existing segments
        let last_lsn = Self::find_last_lsn(&paths).await?;

        let state = Arc::new(RwLock::new(WriterState {
            current_lsn: last_lsn,
            current_term: 1,
            node_id,
        }));

        let (sender, receiver) = mpsc::channel(10000);

        let inner = WriterInner {
            paths,
            config,
            current_segment: None,
            buffer: VecDeque::new(),
            last_flush: Instant::now(),
            state: Arc::clone(&state),
        };

        // Spawn writer task
        tokio::spawn(Self::writer_task(inner, receiver));

        Ok(Self { sender, state })
    }

    /// Find the last LSN from existing segments
    async fn find_last_lsn(paths: &WalPaths) -> Result<Lsn> {
        let segments = super::segment::list_segments(&paths.base_dir)?;
        
        if let Some(last_path) = segments.last() {
            let mut segment = Segment::open(last_path.clone(), 64, true)?;
            let mut last_lsn = segment.first_lsn();
            
            for result in segment.iter() {
                if let Ok(entry) = result {
                    last_lsn = entry.header.lsn;
                }
            }
            
            Ok(last_lsn)
        } else {
            Ok(0)
        }
    }

    /// Append an entry to the WAL
    pub async fn append(&self, entry: LogEntry) -> Result<Lsn> {
        let (tx, rx) = oneshot::channel();

        self.sender
            .send(WriteRequest { entry, response: tx })
            .await
            .map_err(|_| Error::Wal("Writer task terminated".into()))?;

        rx.await.map_err(|_| Error::Wal("Write cancelled".into()))?
    }

    /// Append multiple entries atomically
    pub async fn append_batch(&self, entries: Vec<LogEntry>) -> Result<Vec<Lsn>> {
        let mut lsns = Vec::with_capacity(entries.len());
        for entry in entries {
            lsns.push(self.append(entry).await?);
        }
        Ok(lsns)
    }

    /// Get the current LSN
    pub async fn current_lsn(&self) -> Lsn {
        self.state.read().await.current_lsn
    }

    /// Get the current term
    pub async fn current_term(&self) -> u64 {
        self.state.read().await.current_term
    }

    /// Set the current term (for leader election)
    pub async fn set_term(&self, term: u64) {
        self.state.write().await.current_term = term;
    }

    /// Force flush the buffer
    pub async fn flush(&self) -> Result<()> {
        // Send a no-op entry to trigger flush
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(WriteRequest {
                entry: LogEntry::Noop,
                response: tx,
            })
            .await
            .map_err(|_| Error::Wal("Writer task terminated".into()))?;

        rx.await.map_err(|_| Error::Wal("Flush cancelled".into()))?.map(|_| ())
    }

    /// Writer task that processes write requests
    async fn writer_task(
        mut inner: WriterInner,
        mut receiver: mpsc::Receiver<WriteRequest>,
    ) {
        let flush_interval = Duration::from_millis(inner.config.flush_interval_ms);
        let batch_size = inner.config.batch_size;

        loop {
            // Wait for next request or flush timeout
            let timeout = flush_interval.saturating_sub(inner.last_flush.elapsed());
            
            tokio::select! {
                Some(request) = receiver.recv() => {
                    // Skip no-op entries but still trigger flush
                    if !request.entry.is_noop() {
                        // Allocate LSN
                        let lsn = {
                            let mut state = inner.state.write().await;
                            state.current_lsn += 1;
                            state.current_lsn
                        };

                        let state = inner.state.read().await;
                        let wal_entry = WalEntry::new(
                            lsn,
                            state.current_term,
                            state.node_id.clone(),
                            request.entry,
                        );
                        drop(state);

                        inner.buffer.push_back((wal_entry, request.response));
                    } else {
                        // No-op entry, just acknowledge
                        let _ = request.response.send(Ok(0));
                    }

                    // Flush if batch is full
                    if inner.buffer.len() >= batch_size {
                        if let Err(e) = inner.flush_buffer().await {
                            tracing::error!("WAL flush failed: {}", e);
                        }
                    }
                }
                _ = tokio::time::sleep(timeout) => {
                    // Flush on timeout
                    if !inner.buffer.is_empty() {
                        if let Err(e) = inner.flush_buffer().await {
                            tracing::error!("WAL flush failed: {}", e);
                        }
                    }
                }
            }
        }
    }
}

impl WriterInner {
    /// Flush the write buffer to disk
    async fn flush_buffer(&mut self) -> Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        // Ensure we have an active segment
        self.ensure_segment()?;

        let mut responses = Vec::new();

        while let Some((entry, response)) = self.buffer.pop_front() {
            let lsn = entry.header.lsn;

            // Check if segment needs rotation
            let needs_rotation = {
                let segment = self.current_segment.as_ref().unwrap();
                !segment.has_space(8192)
            };

            if needs_rotation {
                // Seal and rotate segment
                self.current_segment.as_mut().unwrap().seal()?;
                let new_segment = Segment::create(
                    self.paths.segment_path(lsn),
                    lsn,
                    self.config.segment_size_mb,
                    self.config.compression,
                )?;
                self.current_segment = Some(new_segment);
            }

            // Now append to segment
            let segment = self.current_segment.as_mut().unwrap();
            match segment.append(&entry) {
                Ok(_) => {
                    responses.push((response, Ok(lsn)));
                }
                Err(e) => {
                    responses.push((response, Err(e)));
                }
            }
        }

        // Sync if configured
        if self.config.fsync {
            if let Some(segment) = self.current_segment.as_ref() {
                segment.sync()?;
            }
        }

        // Send responses
        for (response, result) in responses {
            let _ = response.send(result);
        }

        self.last_flush = Instant::now();
        Ok(())
    }

    /// Ensure we have an active segment
    fn ensure_segment(&mut self) -> Result<()> {
        if self.current_segment.is_none() {
            // Find existing segments or create new one
            let segments = super::segment::list_segments(&self.paths.base_dir)?;

            if let Some(last_path) = segments.last() {
                let segment = Segment::open(
                    last_path.clone(),
                    self.config.segment_size_mb,
                    self.config.compression,
                )?;

                if !segment.is_sealed() && segment.has_space(8192) {
                    self.current_segment = Some(segment);
                    return Ok(());
                }
            }

            // Create new segment
            let state = futures::executor::block_on(self.state.read());
            let next_lsn = state.current_lsn + 1;
            drop(state);

            let segment = Segment::create(
                self.paths.segment_path(next_lsn),
                next_lsn,
                self.config.segment_size_mb,
                self.config.compression,
            )?;
            self.current_segment = Some(segment);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wal::entry::{Value, PrimaryKey};
    use tempfile::tempdir;

    fn test_config() -> WalConfig {
        WalConfig {
            batch_size: 10,
            flush_interval_ms: 100,
            compression: true,
            segment_size_mb: 1,
            retention_hours: 0,
            fsync: false,
        }
    }

    #[tokio::test]
    async fn test_writer_basic() {
        let dir = tempdir().unwrap();
        let writer = WalWriter::new(
            dir.path().to_path_buf(),
            test_config(),
            "test-node".to_string(),
        )
        .await
        .unwrap();

        let entry = LogEntry::Insert {
            table: "users".to_string(),
            columns: vec!["id".to_string(), "name".to_string()],
            values: vec![Value::Int(1), Value::String("Alice".to_string())],
            primary_key: PrimaryKey::Int(1),
        };

        let lsn = writer.append(entry).await.unwrap();
        assert_eq!(lsn, 1);

        let current = writer.current_lsn().await;
        assert_eq!(current, 1);
    }

    #[tokio::test]
    async fn test_writer_multiple_entries() {
        let dir = tempdir().unwrap();
        let writer = WalWriter::new(
            dir.path().to_path_buf(),
            test_config(),
            "test-node".to_string(),
        )
        .await
        .unwrap();

        for i in 1..=100 {
            let entry = LogEntry::Insert {
                table: "test".to_string(),
                columns: vec!["id".to_string()],
                values: vec![Value::Int(i)],
                primary_key: PrimaryKey::Int(i),
            };
            let lsn = writer.append(entry).await.unwrap();
            assert_eq!(lsn, i as u64);
        }

        writer.flush().await.unwrap();
    }
}
