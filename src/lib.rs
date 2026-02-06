//! WolfScale - Distributed MariaDB Synchronization Manager
//!
//! A high-performance, Rust-based distributed database synchronization manager
//! that keeps multiple MariaDB databases in sync using a Write-Ahead Log (WAL)
//! for coordinated writes, schema changes, and deletes across multiple servers.
//!
//! # Architecture
//!
//! WolfScale uses a leader-based replication model where one node coordinates
//! all writes and replicates them to follower nodes. This ensures strong
//! consistency while maintaining high performance.
//!
//! # Features
//!
//! - High-performance Write-Ahead Log (WAL) with optional compression
//! - Leader election and automatic failover
//! - Node drop/rejoin handling with automatic catch-up
//! - Schema change (ALTER TABLE, CREATE, DROP) propagation
//! - Snowflake ID generation for distributed primary keys
//! - HTTP API for write operations
//! - Record-level tracking for precise synchronization

pub mod config;
pub mod error;
pub mod wal;
pub mod state;
pub mod replication;
pub mod executor;
pub mod network;
pub mod api;
pub mod id;
pub mod proxy;
pub mod binlog;
pub mod tuning;
pub mod lb;

pub use config::WolfScaleConfig;
pub use error::{Error, Result};

/// Re-export commonly used types
pub mod prelude {
    pub use crate::config::WolfScaleConfig;
    pub use crate::error::{Error, Result};
    pub use crate::wal::LogEntry;
    pub use crate::wal::{WalWriter, WalReader};
    pub use crate::state::{NodeState, StateTracker, ClusterMembership};
    pub use crate::replication::Message;
    pub use crate::id::SnowflakeId;
}
