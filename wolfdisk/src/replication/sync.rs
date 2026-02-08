//! Replication synchronization for WolfDisk
//!
//! Handles chunk synchronization between leader and followers.

use std::sync::{Arc, RwLock};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::SystemTime;

use tracing::{debug, info, warn};

use crate::config::{Config, ReplicationMode, NodeRole};
use crate::cluster::{ClusterManager, ClusterState};
use crate::network::peer::PeerManager;
use crate::network::protocol::*;
use crate::storage::chunks::ChunkStore;
use crate::storage::index::{FileIndex, FileEntry, ChunkRef};

/// Sync state for tracking replication progress
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    /// In sync with leader/peers
    Synced,
    /// Syncing index from leader
    SyncingIndex,
    /// Syncing chunks from leader
    SyncingChunks,
    /// Waiting for leader discovery
    WaitingForLeader,
    /// Standalone node (no peers)
    Standalone,
}

/// Manages replication of chunks and index between nodes
pub struct ReplicationManager {
    config: Config,
    cluster: Arc<ClusterManager>,
    peer_manager: Option<Arc<PeerManager>>,
    chunk_store: Arc<ChunkStore>,
    file_index: Arc<RwLock<FileIndex>>,
    sync_state: Arc<RwLock<SyncState>>,
    index_version: Arc<RwLock<u64>>,
    pending_chunks: Arc<RwLock<HashSet<[u8; 32]>>>,
    running: Arc<RwLock<bool>>,
}

impl ReplicationManager {
    /// Create a new replication manager
    pub fn new(
        config: Config,
        cluster: Arc<ClusterManager>,
        chunk_store: Arc<ChunkStore>,
        file_index: Arc<RwLock<FileIndex>>,
    ) -> Self {
        Self::with_peer_manager(config, cluster, None, chunk_store, file_index)
    }

    /// Create a new replication manager with peer manager
    pub fn with_peer_manager(
        config: Config,
        cluster: Arc<ClusterManager>,
        peer_manager: Option<Arc<PeerManager>>,
        chunk_store: Arc<ChunkStore>,
        file_index: Arc<RwLock<FileIndex>>,
    ) -> Self {
        let initial_state = match config.node.role {
            NodeRole::Client => SyncState::Synced, // Clients don't replicate
            _ => SyncState::WaitingForLeader,
        };
        
        Self {
            config,
            cluster,
            peer_manager,
            chunk_store,
            file_index,
            sync_state: Arc::new(RwLock::new(initial_state)),
            index_version: Arc::new(RwLock::new(0)),
            pending_chunks: Arc::new(RwLock::new(HashSet::new())),
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Get current sync state
    pub fn sync_state(&self) -> SyncState {
        *self.sync_state.read().unwrap()
    }

    /// Get current index version
    pub fn index_version(&self) -> u64 {
        *self.index_version.read().unwrap()
    }

    /// Perform initial sync from leader (called on startup for followers)
    pub fn sync_from_leader(&self) -> Result<(), String> {
        if self.cluster.is_leader() {
            info!("This node is leader, no sync needed");
            *self.sync_state.write().unwrap() = SyncState::Synced;
            return Ok(());
        }

        let peer_manager = match &self.peer_manager {
            Some(pm) => pm,
            None => {
                warn!("No peer manager available for sync");
                return Err("No peer manager".to_string());
            }
        };

        let leader_id = self.cluster.leader_id().ok_or("No leader found")?;
        let leader_addr = self.cluster.leader_address().ok_or("No leader address")?;

        info!("Syncing from leader {} at {}", leader_id, leader_addr);
        *self.sync_state.write().unwrap() = SyncState::SyncingIndex;

        // Connect to leader and request sync
        let conn = peer_manager.get_or_connect_leader(&leader_id, &leader_addr)
            .map_err(|e| format!("Failed to connect: {}", e))?;

        let my_version = self.cluster.index_version();
        let msg = Message::SyncRequest(SyncRequestMsg { from_version: my_version });

        let response = conn.request(&msg)
            .map_err(|e| format!("Sync request failed: {}", e))?;

        match response {
            Message::SyncResponse(sync_resp) => {
                self.apply_sync_response(sync_resp)?;
                *self.sync_state.write().unwrap() = SyncState::Synced;
                info!("Sync complete - in sync with leader");
                Ok(())
            }
            _ => Err("Unexpected response from leader".to_string()),
        }
    }

    /// Apply sync response from leader
    fn apply_sync_response(&self, response: SyncResponseMsg) -> Result<(), String> {
        info!("Applying sync response: {} entries, version {}",
              response.entries.len(), response.current_version);

        let mut file_index = self.file_index.write().unwrap();
        let mut pending = self.pending_chunks.write().unwrap();

        for entry in response.entries {
            let path = PathBuf::from(&entry.path);
            let now = SystemTime::now();
            
            let chunks: Vec<ChunkRef> = entry.chunks.iter().map(|c| {
                // Track chunks we need to fetch
                if !self.chunk_store.exists(&c.hash) {
                    pending.insert(c.hash);
                }
                ChunkRef {
                    hash: c.hash,
                    offset: c.offset,
                    size: c.size,
                }
            }).collect();

            let file_entry = FileEntry {
                size: entry.size,
                is_dir: entry.is_dir,
                permissions: entry.permissions,
                uid: 0,
                gid: 0,
                created: now,
                modified: now,
                accessed: now,
                chunks,
            };

            file_index.insert(path, file_entry);
        }

        // Update our index version  
        self.cluster.set_index_version(response.current_version);
        *self.index_version.write().unwrap() = response.current_version;

        if !pending.is_empty() {
            info!("Need to fetch {} chunks from leader", pending.len());
            *self.sync_state.write().unwrap() = SyncState::SyncingChunks;
        }

        Ok(())
    }

    /// Start the replication manager
    pub fn start(&self) {
        *self.running.write().unwrap() = true;
        
        // Client-only nodes don't participate in replication
        if self.config.node.role == NodeRole::Client {
            info!("Replication disabled in client mode");
            return;
        }

        // Check if we're standalone (no peers configured)
        if self.cluster.peers().is_empty() && 
           self.config.cluster.discovery.is_none() &&
           self.config.cluster.peers.is_empty() {
            info!("Running in standalone mode (no replication)");
            *self.sync_state.write().unwrap() = SyncState::Standalone;
            return;
        }

        info!("Replication manager started");
    }

    /// Handle incoming write operation (from local or forwarded)
    /// Returns Ok if write should proceed local, Err with response if forwarded
    pub fn handle_write(
        &self,
        path: &str,
        _offset: u64,
        data: &[u8],
    ) -> Result<(), String> {
        match self.cluster.state() {
            ClusterState::Leading => {
                // We are leader - process write locally and replicate
                debug!("Leader processing write for {} ({} bytes)", path, data.len());
                Ok(())
            }
            ClusterState::Following => {
                // Forward write to leader
                if let Some(leader_id) = self.cluster.leader_id() {
                    debug!("Forwarding write to leader {}", leader_id);
                    // In full implementation, this would send via PeerManager
                    // For now, return Ok and let local write proceed
                    // (would be Err with "forwarded" response in production)
                    Ok(())
                } else {
                    Err("No leader available".to_string())
                }
            }
            ClusterState::Client => {
                // Client mode - forward to leader for all writes
                if let Some(leader_id) = self.cluster.leader_id() {
                    debug!("Client forwarding write to leader {}", leader_id);
                    Ok(())
                } else {
                    Err("Not connected to cluster".to_string())
                }
            }
            ClusterState::Discovering | ClusterState::Standalone => {
                // Still discovering or standalone - allow local write
                debug!("Local write in discovery/standalone mode");
                Ok(())
            }
        }
    }

    /// Handle incoming read request
    /// Returns true if read can proceed locally, false if should forward to leader
    pub fn handle_read(&self, _path: &str) -> bool {
        match self.cluster.state() {
            ClusterState::Leading | ClusterState::Standalone => {
                // Leader/standalone always reads locally
                true
            }
            ClusterState::Following => {
                // Follower can read locally if synced
                self.sync_state() == SyncState::Synced
            }
            ClusterState::Client => {
                // Client mode is a full proxy - all reads go to leader
                false
            }
            ClusterState::Discovering => {
                // Allow local read if we have data
                true
            }
        }
    }

    /// Replicate a chunk to followers (called after local write on leader)
    pub fn replicate_chunk(&self, hash: &[u8; 32], _data: &[u8]) {
        if !self.cluster.is_leader() {
            return;
        }

        let mode = self.config.replication.mode;
        match mode {
            ReplicationMode::Shared => {
                // Shared mode: broadcast to all followers
                debug!("Broadcasting chunk {} to followers", hex::encode(hash));
                // Would use PeerManager to send StoreChunkMsg
            }
            ReplicationMode::Replicated => {
                // Replicated mode: write to N nodes with quorum
                let factor = self.config.replication.factor;
                debug!("Replicating chunk {} to {} nodes", hex::encode(hash), factor);
                // Would use PeerManager to send to N peers and wait for quorum
            }
        }
    }

    /// Replicate index update to followers
    pub fn replicate_index_update(&self, operation: IndexOperation) {
        if !self.cluster.is_leader() {
            return;
        }

        let version = {
            let mut v = self.index_version.write().unwrap();
            *v += 1;
            *v
        };

        debug!("Replicating index update (version {})", version);
        
        let _msg = IndexUpdateMsg {
            version,
            operation,
        };
        
        // Would broadcast via PeerManager
    }

    /// Request full index sync from leader
    pub fn request_sync(&self) {
        if self.cluster.is_leader() {
            return;
        }

        if let Some(_leader_id) = self.cluster.leader_id() {
            info!("Requesting index sync from leader");
            *self.sync_state.write().unwrap() = SyncState::SyncingIndex;
            
            // Would send SyncRequestMsg via PeerManager
        }
    }

    /// Handle incoming sync response from leader
    pub fn handle_sync_response(&self, response: SyncResponseMsg) {
        info!("Received sync response: {} entries, version {}",
              response.entries.len(), response.current_version);
        
        // Update local index version
        *self.index_version.write().unwrap() = response.current_version;
        
        // Track missing chunks
        let mut pending = self.pending_chunks.write().unwrap();
        for entry in response.entries {
            for chunk in entry.chunks {
                if !self.chunk_store.exists(&chunk.hash) {
                    pending.insert(chunk.hash);
                }
            }
        }
        
        if pending.is_empty() {
            *self.sync_state.write().unwrap() = SyncState::Synced;
            info!("Sync complete - fully in sync with leader");
        } else {
            *self.sync_state.write().unwrap() = SyncState::SyncingChunks;
            info!("Sync: need to fetch {} chunks", pending.len());
        }
    }

    /// Stop the replication manager
    pub fn stop(&self) {
        *self.running.write().unwrap() = false;
    }
}
