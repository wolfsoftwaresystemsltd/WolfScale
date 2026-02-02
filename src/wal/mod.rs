//! Write-Ahead Log Module
//!
//! High-performance, append-only log for durable storage
//! and replication of database operations.

pub mod entry;
mod segment;
mod writer;
mod reader;

pub use entry::{LogEntry, PrimaryKey, Value, EntryHeader, Lsn, WalEntry};
pub use segment::Segment;
pub use writer::WalWriter;
pub use reader::WalReader;

use std::path::PathBuf;

/// WAL directory structure
pub struct WalPaths {
    pub base_dir: PathBuf,
}

impl WalPaths {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Get path for a specific segment file
    pub fn segment_path(&self, segment_id: u64) -> PathBuf {
        self.base_dir.join(format!("wal_{:020}.log", segment_id))
    }

    /// Get path for the segment index
    pub fn index_path(&self) -> PathBuf {
        self.base_dir.join("wal_index.db")
    }

    /// Ensure WAL directory exists
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.base_dir)
    }
}
