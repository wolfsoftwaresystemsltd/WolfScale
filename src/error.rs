//! WolfScale Error Types

use thiserror::Error;

/// Result type alias for WolfScale operations
pub type Result<T> = std::result::Result<T, Error>;

/// WolfScale error types
#[derive(Error, Debug)]
pub enum Error {
    // Configuration errors
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Invalid configuration file: {0}")]
    ConfigParse(#[from] toml::de::Error),

    // WAL errors
    #[error("WAL error: {0}")]
    Wal(String),

    #[error("WAL segment not found: {0}")]
    WalSegmentNotFound(u64),

    #[error("WAL entry corrupted at LSN {lsn}: {reason}")]
    WalCorrupted { lsn: u64, reason: String },

    #[error("WAL serialization error: {0}")]
    WalSerialization(#[from] bincode::Error),

    // Database errors
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Schema error: {0}")]
    Schema(String),

    #[error("Query execution failed: {0}")]
    QueryExecution(String),

    // Replication errors
    #[error("Replication error: {0}")]
    Replication(String),

    #[error("Not leader: current leader is {0}")]
    NotLeader(String),

    #[error("No leader available")]
    NoLeader,

    #[error("Node not found: {0}")]
    NodeNotFound(String),

    #[error("Quorum not reached: {reached}/{required}")]
    QuorumNotReached { reached: usize, required: usize },

    // Network errors
    #[error("Network error: {0}")]
    Network(String),

    #[error("Connection failed to {address}: {reason}")]
    ConnectionFailed { address: String, reason: String },

    #[error("Connection timeout to {0}")]
    ConnectionTimeout(String),

    // State errors
    #[error("State error: {0}")]
    State(String),

    #[error("Node state corrupted: {0}")]
    StateCorrupted(String),

    // I/O errors
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    // Sync errors
    #[error("Sync failed: node {node_id} is behind by {entries_behind} entries")]
    SyncFailed { node_id: String, entries_behind: u64 },

    #[error("Catch-up required from LSN {from} to {to}")]
    CatchUpRequired { from: u64, to: u64 },

    // Internal errors
    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Operation cancelled")]
    Cancelled,

    #[error("Shutdown in progress")]
    ShuttingDown,
}

impl Error {
    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Error::ConnectionTimeout(_)
                | Error::QuorumNotReached { .. }
                | Error::Network(_)
        )
    }

    /// Check if this error indicates the node should step down from leadership
    pub fn should_step_down(&self) -> bool {
        matches!(
            self,
            Error::QuorumNotReached { .. } | Error::Network(_)
        )
    }
}
