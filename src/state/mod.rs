//! State Management Module
//!
//! Handles persistent state tracking for nodes, including
//! applied LSN tracking and cluster membership.

mod tracker;
mod membership;
pub mod election;

pub use tracker::StateTracker;
pub use membership::{NodeState, NodeStatus, NodeRole, ClusterMembership, ClusterSummary};
pub use election::{ElectionCoordinator, ElectionConfig, ElectionState};

