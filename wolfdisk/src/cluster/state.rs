//! Cluster state management and leader election for WolfDisk

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::thread;

use tracing::{info, warn};

use crate::config::{Config, NodeRole};
use crate::network::discovery::{Discovery, DiscoveredPeer};

/// Cluster state for this node
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterState {
    /// Just started, finding peers
    Discovering,
    /// Following a leader
    Following,
    /// This node is the leader
    Leading,
    /// Client-only mode (no replication)
    Client,
    /// Standalone node (no peers configured)
    Standalone,
}

/// Information about a known peer
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub node_id: String,
    pub address: String,
    pub is_leader: bool,
    pub last_seen: Instant,
}

/// Cluster manager - handles leader election and state
pub struct ClusterManager {
    config: Config,
    node_id: String,
    state: Arc<RwLock<ClusterState>>,
    leader_id: Arc<RwLock<Option<String>>>,
    peers: Arc<RwLock<HashMap<String, PeerInfo>>>,
    discovery: Option<Discovery>,
    term: Arc<RwLock<u64>>,
    running: Arc<RwLock<bool>>,
    /// Last time we received a heartbeat from the leader
    last_leader_heartbeat: Arc<RwLock<Instant>>,
    /// Current index version (for sync)
    index_version: Arc<RwLock<u64>>,
}

impl ClusterManager {
    /// Create a new cluster manager
    pub fn new(config: Config) -> Self {
        let node_id = config.node.id.clone();
        let initial_state = match config.node.role {
            NodeRole::Client => ClusterState::Client,
            _ => ClusterState::Discovering,
        };
        
        Self {
            config,
            node_id,
            state: Arc::new(RwLock::new(initial_state)),
            leader_id: Arc::new(RwLock::new(None)),
            peers: Arc::new(RwLock::new(HashMap::new())),
            discovery: None,
            term: Arc::new(RwLock::new(0)),
            running: Arc::new(RwLock::new(false)),
            last_leader_heartbeat: Arc::new(RwLock::new(Instant::now())),
            index_version: Arc::new(RwLock::new(0)),
        }
    }

    /// Get current cluster state
    pub fn state(&self) -> ClusterState {
        *self.state.read().unwrap()
    }

    /// Check if this node is the leader
    pub fn is_leader(&self) -> bool {
        self.state() == ClusterState::Leading
    }

    /// Get the current leader's node ID
    pub fn leader_id(&self) -> Option<String> {
        self.leader_id.read().unwrap().clone()
    }

    /// Get the current leader's address (for forwarding operations)
    pub fn leader_address(&self) -> Option<String> {
        let leader = self.leader_id.read().unwrap().clone()?;
        let peers = self.peers.read().unwrap();
        peers.get(&leader).map(|p| p.address.clone())
    }

    /// Get this node's ID
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Get list of known peers
    pub fn peers(&self) -> Vec<PeerInfo> {
        self.peers.read().unwrap().values().cloned().collect()
    }

    /// Get current index version
    pub fn index_version(&self) -> u64 {
        *self.index_version.read().unwrap()
    }

    /// Increment and return the new index version (called on writes)
    pub fn increment_index_version(&self) -> u64 {
        let mut v = self.index_version.write().unwrap();
        *v += 1;
        *v
    }

    /// Set the index version (used during sync)
    pub fn set_index_version(&self, version: u64) {
        *self.index_version.write().unwrap() = version;
    }

    /// Record that we received a heartbeat from the leader
    pub fn receive_leader_heartbeat(&self) {
        *self.last_leader_heartbeat.write().unwrap() = Instant::now();
    }

    /// Check if leader heartbeat has timed out (2 seconds for fast failover)
    pub fn is_leader_timeout(&self) -> bool {
        self.last_leader_heartbeat.read().unwrap().elapsed() > Duration::from_secs(2)
    }

    /// Get current term
    pub fn term(&self) -> u64 {
        *self.term.read().unwrap()
    }

    /// Start the cluster manager
    pub fn start(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        *self.running.write().unwrap() = true;
        
        // Skip discovery for client-only mode and single-node mode
        if self.config.node.role == NodeRole::Client {
            info!("Starting in client-only mode (no replication)");
            *self.state.write().unwrap() = ClusterState::Client;
            return Ok(());
        }

        // Start discovery if enabled
        if self.config.cluster.discovery.is_some() || !self.config.cluster.peers.is_empty() {
            let discovery = Discovery::new(
                self.node_id.clone(),
                self.config.node.bind.clone(),
                self.config.node.role,
            );
            discovery.start()?;
            self.discovery = Some(discovery);
            
            info!("Discovery started for node {}", self.node_id);
        }

        // Start election monitor in background
        self.start_election_monitor();

        Ok(())
    }

    /// Start the election monitor thread
    fn start_election_monitor(&self) {
        let node_id = self.node_id.clone();
        let config_role = self.config.node.role;
        let state = Arc::clone(&self.state);
        let leader_id = Arc::clone(&self.leader_id);
        let peers = Arc::clone(&self.peers);
        let running = Arc::clone(&self.running);
        let term = Arc::clone(&self.term);
        let last_leader_heartbeat = Arc::clone(&self.last_leader_heartbeat);
        let discovery_delay = Duration::from_secs(3); // Wait for discovery
        let leader_timeout = Duration::from_secs(2); // Fast failover
        let peer_stale_threshold = Duration::from_secs(4);

        thread::spawn(move || {
            info!("Election monitor started for node {}", node_id);
            
            // Give discovery time to find peers
            thread::sleep(discovery_delay);
            
            while *running.read().unwrap() {
                // Update state based on discovered peers
                let current_state = *state.read().unwrap();
                
                match current_state {
                    ClusterState::Discovering | ClusterState::Following => {
                        // Check if we should become leader
                        let peers_snapshot = peers.read().unwrap();
                        let active_peers: Vec<_> = peers_snapshot.values()
                            .filter(|p| p.last_seen.elapsed() < peer_stale_threshold)
                            .collect();
                        
                        // Find current leader
                        let current_leader = active_peers.iter()
                            .find(|p| p.is_leader)
                            .map(|p| p.node_id.clone());
                        
                        // Check if leader heartbeat has timed out
                        let heartbeat_timed_out = last_leader_heartbeat.read().unwrap()
                            .elapsed() > leader_timeout;
                        
                        if let Some(leader) = current_leader {
                            // We have a visible leader - reset heartbeat timer
                            *last_leader_heartbeat.write().unwrap() = std::time::Instant::now();
                            *leader_id.write().unwrap() = Some(leader.clone());
                            if current_state == ClusterState::Discovering {
                                info!("Found leader: {}", leader);
                                *state.write().unwrap() = ClusterState::Following;
                            }
                        } else if heartbeat_timed_out || current_leader.is_none() {
                            // No leader or leader timed out - should we become leader?
                            // Rule: lowest node ID becomes leader (deterministic election)
                            let should_be_leader = if config_role == NodeRole::Leader {
                                // Explicit leader role
                                true
                            } else if config_role == NodeRole::Follower {
                                // Explicit follower role - never become leader
                                false
                            } else {
                                // Auto role - lowest ID wins
                                let all_ids: Vec<_> = active_peers.iter()
                                    .map(|p| p.node_id.as_str())
                                    .collect();
                                
                                all_ids.is_empty() || 
                                    all_ids.iter().all(|id| node_id.as_str() < *id)
                            };
                            
                            if should_be_leader {
                                info!("Becoming leader (term {}) - previous leader timed out", *term.read().unwrap());
                                *term.write().unwrap() += 1;
                                *leader_id.write().unwrap() = Some(node_id.clone());
                                *state.write().unwrap() = ClusterState::Leading;
                            }
                        }
                    }
                    ClusterState::Leading => {
                        // Check for higher-priority node that should be leader
                        let peers_snapshot = peers.read().unwrap();
                        let higher_priority = peers_snapshot.values()
                            .filter(|p| p.last_seen.elapsed() < peer_stale_threshold)
                            .any(|p| p.is_leader && p.node_id < node_id);
                        
                        if higher_priority {
                            warn!("Stepping down - higher priority leader found");
                            *state.write().unwrap() = ClusterState::Following;
                            // Reset heartbeat timer since we're now following
                            *last_leader_heartbeat.write().unwrap() = std::time::Instant::now();
                        }
                    }
                    ClusterState::Client | ClusterState::Standalone => {
                        // Client/Standalone mode - no election participation
                    }
                }
                
                thread::sleep(Duration::from_secs(1));
            }
        });
    }

    /// Update peer information (called from discovery)
    pub fn update_peer(&self, peer: DiscoveredPeer) {
        let is_leader = peer.is_leader;
        let info = PeerInfo {
            node_id: peer.node_id.clone(),
            address: peer.address,
            is_leader,
            last_seen: peer.last_seen,
        };
        
        self.peers.write().unwrap().insert(peer.node_id, info);
        
        // Update discovery with our leader status
        if let Some(ref discovery) = self.discovery {
            discovery.set_leader(self.is_leader());
        }
    }

    /// Stop the cluster manager
    pub fn stop(&self) {
        *self.running.write().unwrap() = false;
        if let Some(ref discovery) = self.discovery {
            discovery.stop();
        }
    }
}
