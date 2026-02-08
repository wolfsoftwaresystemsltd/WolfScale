//! Cluster module for WolfDisk
//!
//! Handles leader election, cluster state management, and node coordination.

pub mod state;

pub use state::{ClusterManager, ClusterState, PeerInfo};
