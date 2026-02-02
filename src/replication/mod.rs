//! Replication Module
//!
//! Handles log replication between leader and follower nodes.

pub mod protocol;
mod leader;
mod follower;

pub use protocol::{Message, FrameHeader};
pub use leader::LeaderNode;
pub use follower::FollowerNode;

/// Configuration for replication
#[derive(Debug, Clone)]
pub struct ReplicationConfig {
    /// Maximum entries per batch
    pub max_batch_entries: usize,
    /// Heartbeat interval in milliseconds
    pub heartbeat_interval_ms: u64,
    /// Replication timeout in milliseconds
    pub replication_timeout_ms: u64,
}

impl Default for ReplicationConfig {
    fn default() -> Self {
        Self {
            max_batch_entries: 1000,
            heartbeat_interval_ms: 500,
            replication_timeout_ms: 5000,
        }
    }
}

/// Common trait for replication nodes
#[async_trait::async_trait]
pub trait ReplicationNode: Send + Sync {
    /// Get the node ID
    fn node_id(&self) -> &str;

    /// Check if this node is the leader
    fn is_leader(&self) -> bool;

    /// Start the replication process
    async fn start(&self) -> crate::Result<()>;

    /// Stop the replication process
    async fn stop(&self) -> crate::Result<()>;
}
