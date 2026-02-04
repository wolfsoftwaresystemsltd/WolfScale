//! Cluster Membership Management
//!
//! Tracks node states, health, and cluster membership.

use std::collections::HashMap;
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::wal::entry::Lsn;
use crate::error::Result;

/// Node status in the cluster
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    /// Node is joining the cluster
    Joining,
    /// Node is syncing (catching up with the leader)
    Syncing,
    /// Node is active and up-to-date
    Active,
    /// Node is lagging behind (missed heartbeats)
    Lagging,
    /// Node has been dropped from the cluster
    Dropped,
    /// Node is offline/unreachable
    Offline,
    /// Node needs full database migration (too far behind for WAL catch-up)
    NeedsMigration,
}

impl std::fmt::Display for NodeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeStatus::Joining => write!(f, "JOINING"),
            NodeStatus::Syncing => write!(f, "SYNCING"),
            NodeStatus::Active => write!(f, "ACTIVE"),
            NodeStatus::Lagging => write!(f, "LAGGING"),
            NodeStatus::Dropped => write!(f, "DROPPED"),
            NodeStatus::Offline => write!(f, "OFFLINE"),
            NodeStatus::NeedsMigration => write!(f, "NEEDS_MIGRATION"),
        }
    }
}

/// Role of a node in the cluster
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeRole {
    /// Node is the cluster leader
    Leader,
    /// Node is a follower
    Follower,
    /// Node is a candidate (during election)
    Candidate,
}

impl std::fmt::Display for NodeRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeRole::Leader => write!(f, "LEADER"),
            NodeRole::Follower => write!(f, "FOLLOWER"),
            NodeRole::Candidate => write!(f, "CANDIDATE"),
        }
    }
}

/// State of a single node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeState {
    /// Unique node identifier
    pub id: String,
    /// Node address (host:port)
    pub address: String,
    /// Current status
    pub status: NodeStatus,
    /// Current role
    pub role: NodeRole,
    /// Last applied LSN
    pub last_applied_lsn: Lsn,
    /// Last heartbeat time (not serialized)
    #[serde(skip)]
    pub last_heartbeat: Option<Instant>,
    /// When the node joined
    pub joined_at: chrono::DateTime<chrono::Utc>,
    /// Replication lag in entries
    pub replication_lag: u64,
}

impl NodeState {
    /// Create a new node state
    pub fn new(id: String, address: String) -> Self {
        Self {
            id,
            address,
            status: NodeStatus::Joining,
            role: NodeRole::Follower,
            last_applied_lsn: 0,
            last_heartbeat: None,
            joined_at: chrono::Utc::now(),
            replication_lag: 0,
        }
    }

    /// Check if the node is healthy (received heartbeat recently)
    pub fn is_healthy(&self, timeout: Duration) -> bool {
        match self.last_heartbeat {
            Some(last) => last.elapsed() < timeout,
            None => false,
        }
    }

    /// Update heartbeat time
    pub fn touch(&mut self) {
        self.last_heartbeat = Some(Instant::now());
    }

    /// Time since last heartbeat
    pub fn time_since_heartbeat(&self) -> Option<Duration> {
        self.last_heartbeat.map(|t| t.elapsed())
    }
}

/// Cluster membership tracker
pub struct ClusterMembership {
    /// This node's ID
    node_id: String,
    /// All known nodes (including self)
    nodes: RwLock<HashMap<String, NodeState>>,
    /// Heartbeat timeout
    heartbeat_timeout: Duration,
    /// Election timeout
    election_timeout: Duration,
}

impl ClusterMembership {
    /// Create a new cluster membership tracker
    pub fn new(
        node_id: String,
        address: String,
        heartbeat_timeout: Duration,
        election_timeout: Duration,
    ) -> Self {
        let mut nodes = HashMap::new();
        let mut self_node = NodeState::new(node_id.clone(), address);
        self_node.status = NodeStatus::Active;
        self_node.touch();
        nodes.insert(node_id.clone(), self_node);

        Self {
            node_id,
            nodes: RwLock::new(nodes),
            heartbeat_timeout,
            election_timeout,
        }
    }

    /// Get this node's ID
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Add a peer node
    pub async fn add_peer(&self, id: String, address: String) -> Result<()> {
        let mut nodes = self.nodes.write().await;
        
        // First, remove any synthetic peer with the same address
        // Synthetic IDs are formatted as "peer-{address-with-dashes}"
        let synthetic_id = format!("peer-{}", address.replace(':', "-"));
        nodes.remove(&synthetic_id);
        
        // Also remove any other nodes with the same address but different ID
        // (except if they're already using the same ID we're adding)
        let nodes_to_remove: Vec<String> = nodes.iter()
            .filter(|(existing_id, node)| {
                node.address == address && *existing_id != &id && existing_id.starts_with("peer-")
            })
            .map(|(k, _)| k.clone())
            .collect();
        
        for remove_id in nodes_to_remove {
            nodes.remove(&remove_id);
        }
        
        if !nodes.contains_key(&id) {
            nodes.insert(id.clone(), NodeState::new(id, address));
        }
        Ok(())
    }

    /// Remove a peer node
    pub async fn remove_peer(&self, id: &str) -> Result<Option<NodeState>> {
        let mut nodes = self.nodes.write().await;
        Ok(nodes.remove(id))
    }

    /// Get a node's state
    pub async fn get_node(&self, id: &str) -> Option<NodeState> {
        let nodes = self.nodes.read().await;
        nodes.get(id).cloned()
    }

    /// Get this node's state
    pub async fn get_self(&self) -> NodeState {
        let nodes = self.nodes.read().await;
        nodes.get(&self.node_id).cloned().expect("Self node must exist")
    }

    /// Update a node's state
    pub async fn update_node<F>(&self, id: &str, f: F) -> Result<()>
    where
        F: FnOnce(&mut NodeState),
    {
        let mut nodes = self.nodes.write().await;
        if let Some(node) = nodes.get_mut(id) {
            f(node);
        }
        Ok(())
    }

    /// Record a heartbeat from a node
    pub async fn record_heartbeat(&self, id: &str, lsn: Lsn) -> Result<()> {
        let mut nodes = self.nodes.write().await;
        if let Some(node) = nodes.get_mut(id) {
            let old_lsn = node.last_applied_lsn;
            node.touch();
            // Only update LSN if the new value is higher - prevents PeerHeartbeat from resetting progress
            if lsn > node.last_applied_lsn {
                node.last_applied_lsn = lsn;
                tracing::trace!("record_heartbeat: updated node '{}' lsn {} -> {}", id, old_lsn, lsn);
            }
            
            // Update status based on current state
            match node.status {
                NodeStatus::Joining => {
                    // Node has responded - it's now active
                    node.status = NodeStatus::Active;
                }
                NodeStatus::Syncing if node.replication_lag == 0 => {
                    node.status = NodeStatus::Active;
                }
                NodeStatus::Lagging => {
                    node.status = NodeStatus::Syncing;
                }
                NodeStatus::Offline => {
                    // Node came back online
                    node.status = NodeStatus::Active;
                }
                _ => {}
            }
        } else {
            tracing::warn!("record_heartbeat: node '{}' NOT FOUND in cluster membership!", id);
        }
        Ok(())
    }

    /// Check for timed-out nodes and update their status
    /// Only times out nodes we've actually received heartbeats from (i.e., the leader)
    /// Nodes learned from membership lists without direct heartbeats are not timed out
    pub async fn check_timeouts(&self) -> Vec<String> {
        let mut nodes = self.nodes.write().await;
        let mut timed_out = Vec::new();

        for (id, node) in nodes.iter_mut() {
            if id == &self.node_id {
                continue; // Skip self
            }

            // Only timeout nodes we've actually received heartbeats from
            // Nodes learned from leader's membership list don't have heartbeats recorded
            if node.last_heartbeat.is_none() {
                continue; // Never received heartbeat from this node, skip
            }

            if !node.is_healthy(self.heartbeat_timeout) {
                if node.status == NodeStatus::Active {
                    node.status = NodeStatus::Lagging;
                    timed_out.push(id.clone());
                } else if node.status == NodeStatus::Lagging {
                    // Check if should be marked as dropped
                    if let Some(since) = node.time_since_heartbeat() {
                        if since > self.election_timeout * 3 {
                            node.status = NodeStatus::Dropped;
                            // Clear leader role when dropped - forces re-election
                            if node.role == NodeRole::Leader {
                                node.role = NodeRole::Follower;
                            }
                            timed_out.push(id.clone());
                        }
                    }
                }
            }
        }

        timed_out
    }

    /// Get all active nodes
    pub async fn active_nodes(&self) -> Vec<NodeState> {
        let nodes = self.nodes.read().await;
        nodes
            .values()
            .filter(|n| n.status == NodeStatus::Active)
            .cloned()
            .collect()
    }

    /// Get all peer nodes (excluding self)
    pub async fn peers(&self) -> Vec<NodeState> {
        let nodes = self.nodes.read().await;
        nodes
            .values()
            .filter(|n| n.id != self.node_id)
            // Note: we include synthetic peers (peer-*) because they're needed for 
            // initial replication before followers identify themselves with their real IDs
            .cloned()
            .collect()
    }

    /// Get all real peer nodes (excluding self and synthetic peers)
    /// Use this for building membership lists in PeerHeartbeat messages
    pub async fn real_peers(&self) -> Vec<NodeState> {
        let nodes = self.nodes.read().await;
        nodes
            .values()
            .filter(|n| n.id != self.node_id && !n.id.starts_with("peer-"))
            .cloned()
            .collect()
    }

    /// Get all nodes (excluding synthetic peers)
    pub async fn all_nodes(&self) -> Vec<NodeState> {
        let nodes = self.nodes.read().await;
        nodes.values()
            .filter(|n| !n.id.starts_with("peer-"))  // Filter out synthetic peers
            .cloned()
            .collect()
    }

    /// Get the current leader (if known)
    pub async fn current_leader(&self) -> Option<NodeState> {
        let nodes = self.nodes.read().await;
        nodes.values().find(|n| n.role == NodeRole::Leader).cloned()
    }

    /// Set a node as the leader
    pub async fn set_leader(&self, leader_id: &str) -> Result<()> {
        let mut nodes = self.nodes.write().await;
        
        // Remove leader role from all nodes
        for node in nodes.values_mut() {
            if node.role == NodeRole::Leader {
                node.role = NodeRole::Follower;
            }
        }

        // Set new leader
        if let Some(node) = nodes.get_mut(leader_id) {
            node.role = NodeRole::Leader;
        }

        Ok(())
    }

    /// Get the cluster size (total nodes)
    pub async fn size(&self) -> usize {
        self.nodes.read().await.len()
    }

    /// Get quorum size (majority)
    pub async fn quorum_size(&self) -> usize {
        let size = self.size().await;
        (size / 2) + 1
    }

    /// Check if we have quorum (at least 2 active nodes = leader + one follower)
    pub async fn has_quorum(&self) -> bool {
        let active = self.active_nodes().await.len();
        // We have quorum if there are at least 2 active nodes
        // (leader can replicate writes to at least one follower)
        active >= 2
    }

    /// Update replication lag for all nodes
    pub async fn update_replication_lag(&self, leader_lsn: Lsn) {
        let mut nodes = self.nodes.write().await;
        for node in nodes.values_mut() {
            if leader_lsn > node.last_applied_lsn {
                node.replication_lag = leader_lsn - node.last_applied_lsn;
            } else {
                node.replication_lag = 0;
            }
        }
    }

    /// Get nodes that need to catch up
    pub async fn nodes_needing_sync(&self) -> Vec<NodeState> {
        let nodes = self.nodes.read().await;
        nodes
            .values()
            .filter(|n| {
                n.id != self.node_id
                    && (n.status == NodeStatus::Syncing
                        || n.status == NodeStatus::Joining
                        || n.replication_lag > 0)
            })
            .cloned()
            .collect()
    }

    /// Mark a node as rejoined
    pub async fn mark_rejoined(&self, id: &str) -> Result<()> {
        let mut nodes = self.nodes.write().await;
        if let Some(node) = nodes.get_mut(id) {
            node.status = NodeStatus::Syncing;
            node.touch();
        }
        Ok(())
    }

    /// Get cluster summary
    pub async fn summary(&self) -> ClusterSummary {
        let nodes = self.nodes.read().await;
        // Filter out synthetic peers for accurate count
        let real_nodes: Vec<_> = nodes.values()
            .filter(|n| !n.id.starts_with("peer-"))
            .collect();
        
        let mut summary = ClusterSummary {
            total_nodes: real_nodes.len(),
            active_nodes: 0,
            syncing_nodes: 0,
            lagging_nodes: 0,
            dropped_nodes: 0,
            leader_id: None,
        };

        for node in real_nodes {
            match node.status {
                NodeStatus::Active => summary.active_nodes += 1,
                NodeStatus::Syncing => summary.syncing_nodes += 1,
                NodeStatus::Lagging => summary.lagging_nodes += 1,
                NodeStatus::Dropped => summary.dropped_nodes += 1,
                _ => {}
            }

            if node.role == NodeRole::Leader {
                summary.leader_id = Some(node.id.clone());
            }
        }

        summary
    }
}

/// Cluster summary information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterSummary {
    pub total_nodes: usize,
    pub active_nodes: usize,
    pub syncing_nodes: usize,
    pub lagging_nodes: usize,
    pub dropped_nodes: usize,
    pub leader_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cluster_membership() {
        let cluster = ClusterMembership::new(
            "node-1".to_string(),
            "localhost:7654".to_string(),
            Duration::from_secs(1),
            Duration::from_secs(5),
        );

        // Add peers
        cluster.add_peer("node-2".to_string(), "localhost:7655".to_string()).await.unwrap();
        cluster.add_peer("node-3".to_string(), "localhost:7656".to_string()).await.unwrap();

        assert_eq!(cluster.size().await, 3);
        assert_eq!(cluster.quorum_size().await, 2);
    }

    #[tokio::test]
    async fn test_leader_election() {
        let cluster = ClusterMembership::new(
            "node-1".to_string(),
            "localhost:7654".to_string(),
            Duration::from_secs(1),
            Duration::from_secs(5),
        );

        cluster.add_peer("node-2".to_string(), "localhost:7655".to_string()).await.unwrap();

        // No leader initially
        assert!(cluster.current_leader().await.is_none());

        // Set leader
        cluster.set_leader("node-1").await.unwrap();
        let leader = cluster.current_leader().await.unwrap();
        assert_eq!(leader.id, "node-1");
        assert_eq!(leader.role, NodeRole::Leader);
    }

    #[tokio::test]
    async fn test_heartbeat_and_timeout() {
        let cluster = ClusterMembership::new(
            "node-1".to_string(),
            "localhost:7654".to_string(),
            Duration::from_millis(100),
            Duration::from_millis(500),
        );

        cluster.add_peer("node-2".to_string(), "localhost:7655".to_string()).await.unwrap();
        
        // Set node to Active status so check_timeouts will process it
        cluster.update_node("node-2", |node| {
            node.status = NodeStatus::Active;
        }).await.unwrap();
        
        cluster.record_heartbeat("node-2", 10).await.unwrap();

        // Node should be healthy
        let node = cluster.get_node("node-2").await.unwrap();
        assert!(node.is_healthy(Duration::from_millis(100)));

        // Wait for timeout
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Check timeouts
        let timed_out = cluster.check_timeouts().await;
        assert!(!timed_out.is_empty());
    }
}
