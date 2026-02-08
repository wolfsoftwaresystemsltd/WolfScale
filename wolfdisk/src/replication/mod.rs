//! Replication logic for WolfDisk
//!
//! Handles chunk and index replication between nodes.
//! Supports two modes:
//! - Shared: Single leader accepts writes, followers sync from leader
//! - Replicated: Quorum-based writes for high availability

pub mod sync;

pub use sync::{ReplicationManager, SyncState};
