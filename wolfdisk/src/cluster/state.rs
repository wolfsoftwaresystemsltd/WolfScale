//! Cluster state management and leader election for WolfDisk

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::thread;

use tracing::{debug, info, warn};

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
    pub is_client: bool,
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
    /// Whether initial sync from leader has completed
    /// Election monitor waits for this before allowing leader promotion
    initial_sync_complete: Arc<RwLock<bool>>,
    /// Changelog: (version, path) pairs tracking which files changed at each version
    /// Used for delta sync - leader only sends entries that changed since follower's version
    changelog: Arc<RwLock<Vec<(u64, std::path::PathBuf)>>>,
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
            initial_sync_complete: Arc::new(RwLock::new(false)),
            changelog: Arc::new(RwLock::new(Vec::new())),
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
    /// Records the changed path in the changelog for delta sync
    pub fn increment_index_version(&self, path: std::path::PathBuf) -> u64 {
        let mut v = self.index_version.write().unwrap();
        *v += 1;
        let version = *v;
        
        let mut log = self.changelog.write().unwrap();
        log.push((version, path));
        
        // Cap changelog at 10,000 entries to prevent unbounded growth
        // If a follower is more than 10,000 changes behind, it gets a full sync
        if log.len() > 10_000 {
            let drain_count = log.len() - 10_000;
            log.drain(0..drain_count);
        }
        
        version
    }
    
    /// Get the paths that changed since a given version
    /// Returns None if the changelog doesn't go back far enough (need full sync)
    pub fn changes_since(&self, from_version: u64) -> Option<Vec<std::path::PathBuf>> {
        let log = self.changelog.read().unwrap();
        
        // Check if changelog goes back far enough
        if let Some((oldest_version, _)) = log.first() {
            if from_version < *oldest_version {
                // Changelog truncated, need full sync
                return None;
            }
        }
        
        // Collect unique paths changed since from_version
        let mut changed: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
        for (v, path) in log.iter() {
            if *v > from_version {
                changed.insert(path.clone());
            }
        }
        
        Some(changed.into_iter().collect())
    }

    /// Set the index version (used during sync)
    pub fn set_index_version(&self, version: u64) {
        *self.index_version.write().unwrap() = version;
    }

    /// Mark initial sync as complete (allows election monitor to promote to leader)
    pub fn set_sync_complete(&self) {
        *self.initial_sync_complete.write().unwrap() = true;
        info!("Initial sync complete - node eligible for leader election");
    }

    /// Check if initial sync has completed
    pub fn is_sync_complete(&self) -> bool {
        *self.initial_sync_complete.read().unwrap()
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
        
        // Client mode: run discovery to find leader, but never become leader
        let is_client = self.config.node.role == NodeRole::Client;
        if is_client {
            info!("Starting in client mode (read/write but will not become leader)");
            *self.state.write().unwrap() = ClusterState::Client;
        }

        // Start discovery if enabled (including for clients!)
        if self.config.cluster.discovery.is_some() || !self.config.cluster.peers.is_empty() {
            let discovery = Discovery::new(
                self.node_id.clone(),
                self.config.node.bind.clone(),
                self.config.node.role,
            );
            discovery.start()?;
            
            // Start a sync thread to copy discovered peers to our peer map
            // AND sync our leader status to Discovery for broadcasts
            let cluster_peers = Arc::clone(&self.peers);
            let cluster_state = Arc::clone(&self.state);
            let cluster_leader_id = Arc::clone(&self.leader_id);
            let running = Arc::clone(&self.running);
            let discovery_clone = discovery.clone();
            let is_client_mode = is_client;
            
            thread::spawn(move || {
                while *running.read().unwrap() {
                    // Sync our leader status TO discovery (for broadcasts)
                    // Clients never advertise as leader
                    if !is_client_mode {
                        let is_leading = *cluster_state.read().unwrap() == ClusterState::Leading;
                        discovery_clone.set_leader(is_leading);
                    }
                    
                    // Get peers FROM discovery
                    let discovered = discovery_clone.peers();
                    
                    // Update our peer map and track leader
                    let mut peers = cluster_peers.write().unwrap();
                    for dp in discovered {
                        // If client mode, also track who the leader is
                        if is_client_mode && dp.is_leader {
                            *cluster_leader_id.write().unwrap() = Some(dp.node_id.clone());
                        }
                        
                        let is_client_peer = matches!(dp.role, crate::network::discovery::DiscoveryRole::Client);
                        peers.insert(dp.node_id.clone(), PeerInfo {
                            node_id: dp.node_id,
                            address: dp.address,
                            is_leader: dp.is_leader,
                            is_client: is_client_peer,
                            last_seen: dp.last_seen,
                        });
                    }
                    drop(peers);
                    
                    thread::sleep(Duration::from_millis(500));
                }
            });
            
            self.discovery = Some(discovery);
            
            info!("Discovery started for node {}", self.node_id);
        }

        // Start election monitor in background (but NOT for clients)
        if !is_client {
            self.start_election_monitor();
        }

        Ok(())
    }

    /// Start the election monitor thread
    /// 
    /// Election model:
    /// 1. All nodes start as followers (Discovering state)
    /// 2. Wait for discovery to find other peers
    /// 3. After discovery delay, determine leader by lowest node ID
    /// 4. If I have lowest ID -> become leader
    /// 5. Otherwise -> stay follower and wait for leader heartbeat
    fn start_election_monitor(&self) {
        let node_id = self.node_id.clone();
        let config_role = self.config.node.role;
        let state = Arc::clone(&self.state);
        let leader_id = Arc::clone(&self.leader_id);
        let peers = Arc::clone(&self.peers);
        let running = Arc::clone(&self.running);
        let term = Arc::clone(&self.term);
        let last_leader_heartbeat = Arc::clone(&self.last_leader_heartbeat);
        
        // Give discovery enough time to find all peers before electing
        let discovery_delay = Duration::from_secs(5);
        let peer_stale_threshold = Duration::from_secs(4);
        let initial_sync_complete = Arc::clone(&self.initial_sync_complete);

        thread::spawn(move || {
            info!("Election monitor started for node {} - waiting for discovery", node_id);
            
            // All nodes start as followers and wait for discovery
            thread::sleep(discovery_delay);
            
            while *running.read().unwrap() {
                let current_state = *state.read().unwrap();
                
                // Clone active peers so we don't hold the lock
                let active_peers: Vec<_> = {
                    let snapshot = peers.read().unwrap();
                    snapshot.values()
                        .filter(|p| p.last_seen.elapsed() < peer_stale_threshold)
                        .cloned()
                        .collect()
                };
                
                // Get server peer IDs for comparison (exclude clients!)
                // Clients should never be considered for leader election
                let peer_ids: Vec<&str> = active_peers.iter()
                    .filter(|p| !p.is_client)
                    .map(|p| p.node_id.as_str())
                    .collect();
                
                // Am I the lowest ID? (I should be leader if so)
                let i_am_lowest = peer_ids.is_empty() || 
                    peer_ids.iter().all(|id| node_id.as_str() < *id);
                
                // Is there a visible leader already?
                let current_leader = active_peers.iter()
                    .find(|p| p.is_leader)
                    .map(|p| p.node_id.clone());
                
                match current_state {
                    ClusterState::Discovering | ClusterState::Following => {
                        if let Some(leader) = &current_leader {
                            // There's a leader - follow them
                            *last_leader_heartbeat.write().unwrap() = std::time::Instant::now();
                            *leader_id.write().unwrap() = Some(leader.clone());
                            
                            if current_state == ClusterState::Discovering {
                                info!("Found leader: {}", leader);
                                *state.write().unwrap() = ClusterState::Following;
                            }
                            
                            // But if I have a lower ID, I should become leader
                            if i_am_lowest && node_id.as_str() < leader.as_str() {
                                info!("I have lower ID than current leader {} - taking over", leader);
                                *term.write().unwrap() += 1;
                                *leader_id.write().unwrap() = Some(node_id.clone());
                                *state.write().unwrap() = ClusterState::Leading;
                            }
                        } else if i_am_lowest && config_role != NodeRole::Follower {
                            // I have the lowest ID, but wait for initial sync before
                            // taking leadership. This ensures a restarting node first
                            // gets the latest data from the current leader.
                            if *initial_sync_complete.read().unwrap() || active_peers.is_empty() {
                                info!("Becoming leader (term {}) - I have the lowest node ID", *term.read().unwrap());
                                *term.write().unwrap() += 1;
                                *leader_id.write().unwrap() = Some(node_id.clone());
                                *state.write().unwrap() = ClusterState::Leading;
                            } else {
                                debug!("Deferring leadership: waiting for initial sync to complete");
                            }
                        } else {
                            // Not lowest ID, stay as follower and wait
                            debug!("Not becoming leader: i_am_lowest={}, config_role={:?}", i_am_lowest, config_role);
                            if current_state == ClusterState::Discovering {
                                *state.write().unwrap() = ClusterState::Following;
                            }
                        }
                    }
                    ClusterState::Leading => {
                        // Check if any peer has a lower ID - I should step down
                        if !i_am_lowest {
                            warn!("Stepping down - discovered peer with lower node ID");
                            *state.write().unwrap() = ClusterState::Following;
                            *last_leader_heartbeat.write().unwrap() = std::time::Instant::now();
                        }
                    }
                    ClusterState::Client | ClusterState::Standalone => {
                        // No election participation
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
            is_client: matches!(peer.role, crate::network::discovery::DiscoveryRole::Client),
            last_seen: peer.last_seen,
        };
        
        self.peers.write().unwrap().insert(peer.node_id, info);
        
        // Update discovery with our leader status
        if let Some(ref discovery) = self.discovery {
            discovery.set_leader(self.is_leader());
        }
    }

    /// Write cluster status to file for wolfdiskctl
    pub fn write_status_file(&self) {
        use std::time::{SystemTime, UNIX_EPOCH};
        
        let status_dir = std::path::Path::new(&self.config.node.data_dir);
        if !status_dir.exists() {
            return; // Data dir doesn't exist yet
        }
        
        let status_path = status_dir.join("cluster_status.json");
        
        let state = self.state();
        let peers = self.peers();
        
        let role = if self.is_leader() { "leader" } else if state == ClusterState::Client { "client" } else { "follower" };
        let state_str = match state {
            ClusterState::Discovering => "discovering",
            ClusterState::Following => "following",
            ClusterState::Leading => "leading",
            ClusterState::Client => "client",
            ClusterState::Standalone => "standalone",
        };
        
        let peer_statuses: Vec<serde_json::Value> = peers.iter().map(|p| {
            let peer_role = if p.is_leader { "leader" } else if p.is_client { "client" } else { "follower" };
            serde_json::json!({
                "node_id": p.node_id,
                "address": p.address,
                "role": peer_role,
                "is_leader": p.is_leader,
                "is_client": p.is_client,
                "last_seen_secs_ago": p.last_seen.elapsed().as_secs()
            })
        }).collect();
        
        let status = serde_json::json!({
            "node_id": self.node_id,
            "role": role,
            "state": state_str,
            "bind_address": self.config.node.bind,
            "leader_id": self.leader_id(),
            "index_version": self.index_version(),
            "peers": peer_statuses,
            "updated_at": SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
        });
        
        if let Ok(json) = serde_json::to_string_pretty(&status) {
            let _ = std::fs::write(&status_path, json);
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
