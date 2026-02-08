//! WolfDisk - Distributed File System
//!
//! A distributed file system for Linux that provides easy-to-use
//! shared and replicated storage across multiple nodes.

pub mod config;
pub mod error;
pub mod fuse;
pub mod storage;
pub mod network;
pub mod cluster;
pub mod replication;

pub use config::{Config, NodeRole, ReplicationMode};
pub use cluster::{ClusterManager, ClusterState};
pub use replication::{ReplicationManager, SyncState};
pub use error::{Error, Result};
