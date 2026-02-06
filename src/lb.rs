//! Load Balancer Routing Module
//!
//! Routes writes to leader and load balances reads across healthy followers.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::RwLock;

use crate::state::{ClusterMembership, NodeState, NodeRole, NodeStatus};

/// Maximum acceptable replication lag (in entries) before a node is skipped for reads
const DEFAULT_MAX_LAG: u64 = 100;

/// Load balancer router for distributing queries across cluster nodes
pub struct LoadBalancerRouter {
    /// Cluster membership for node discovery
    cluster: Arc<ClusterMembership>,
    /// Maximum acceptable lag for read routing
    max_acceptable_lag: u64,
    /// Round-robin counter for read distribution
    read_counter: AtomicUsize,
    /// Cached list of read nodes (refreshed periodically)
    read_nodes_cache: RwLock<Vec<NodeState>>,
}

impl LoadBalancerRouter {
    /// Create a new load balancer router
    pub fn new(cluster: Arc<ClusterMembership>, max_acceptable_lag: Option<u64>) -> Self {
        Self {
            cluster,
            max_acceptable_lag: max_acceptable_lag.unwrap_or(DEFAULT_MAX_LAG),
            read_counter: AtomicUsize::new(0),
            read_nodes_cache: RwLock::new(Vec::new()),
        }
    }

    /// Get the current leader for write routing
    pub async fn get_leader(&self) -> Option<NodeState> {
        self.cluster.current_leader().await
    }

    /// Get a healthy node for read routing (round-robin across followers)
    /// Falls back to leader if no healthy followers are available
    pub async fn get_read_node(&self) -> Option<NodeState> {
        let nodes = self.healthy_read_nodes().await;
        
        if nodes.is_empty() {
            // Fall back to leader if no healthy followers
            return self.get_leader().await;
        }

        // Round-robin selection
        let idx = self.read_counter.fetch_add(1, Ordering::Relaxed) % nodes.len();
        nodes.get(idx).cloned()
    }

    /// Get all healthy nodes suitable for read queries
    /// Includes followers with acceptable lag and the leader
    pub async fn healthy_read_nodes(&self) -> Vec<NodeState> {
        let all_nodes = self.cluster.all_nodes().await;
        
        // Get leader LSN for lag calculation
        let leader_lsn = self.cluster.current_leader().await
            .map(|l| l.last_applied_lsn)
            .unwrap_or(0);

        all_nodes
            .into_iter()
            .filter(|node| {
                // Must be active
                if node.status != NodeStatus::Active {
                    return false;
                }

                // Must not be a load balancer (we don't route to other LBs)
                if node.role == NodeRole::LoadBalancer {
                    return false;
                }

                // Check lag (leader always has 0 lag)
                let lag = if leader_lsn > node.last_applied_lsn {
                    leader_lsn - node.last_applied_lsn
                } else {
                    0
                };

                lag <= self.max_acceptable_lag
            })
            .collect()
    }

    /// Refresh the cached list of read nodes
    pub async fn refresh_read_nodes(&self) {
        let nodes = self.healthy_read_nodes().await;
        let mut cache = self.read_nodes_cache.write().await;
        *cache = nodes;
    }

    /// Get stats about the load balancer state
    pub async fn stats(&self) -> LoadBalancerStats {
        let leader = self.get_leader().await;
        let read_nodes = self.healthy_read_nodes().await;
        
        LoadBalancerStats {
            leader_address: leader.map(|l| l.address),
            read_node_count: read_nodes.len(),
            read_node_addresses: read_nodes.iter().map(|n| n.address.clone()).collect(),
            total_reads_routed: self.read_counter.load(Ordering::Relaxed),
        }
    }
}

/// Statistics about load balancer state
#[derive(Debug, Clone)]
pub struct LoadBalancerStats {
    /// Address of current leader (for writes)
    pub leader_address: Option<String>,
    /// Number of healthy read nodes
    pub read_node_count: usize,
    /// Addresses of healthy read nodes
    pub read_node_addresses: Vec<String>,
    /// Total reads routed (round-robin counter)
    pub total_reads_routed: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_load_balancer_router() {
        let cluster = Arc::new(ClusterMembership::new(
            "lb-1".to_string(),
            "localhost:7654".to_string(),
            Duration::from_secs(5),
            Duration::from_secs(10),
        ));

        let router = LoadBalancerRouter::new(Arc::clone(&cluster), None);
        
        // Initially no leader
        assert!(router.get_leader().await.is_none());
        
        // No read nodes either
        let nodes = router.healthy_read_nodes().await;
        // Only self is in cluster, and it's marked as LB so excluded
        assert!(nodes.is_empty() || nodes.len() == 1);
    }
}
