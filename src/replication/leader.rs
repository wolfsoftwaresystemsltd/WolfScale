//! Leader Node Implementation
//!
//! Handles leader responsibilities: accepting writes, replicating to followers,
//! and managing cluster membership.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock, oneshot};
use tokio::time::interval;

use crate::wal::entry::{Lsn, LogEntry};
use crate::wal::{WalWriter, WalReader};
use crate::replication::{Message, ReplicationConfig};
use crate::executor::MariaDbExecutor;
use crate::state::{ClusterMembership, StateTracker, NodeStatus};
use crate::error::{Error, Result};

/// Type alias for pending writes map
type PendingWritesMap = HashMap<Lsn, PendingWrite>;
/// Type alias for LSN tracking maps
type LsnMap = HashMap<String, Lsn>;

/// Pending write request
#[allow(dead_code)]
struct PendingWrite {
    lsn: Lsn,
    acks: usize,
    required_acks: usize,
    response: oneshot::Sender<Result<Lsn>>,
}

/// Leader node state
pub struct LeaderNode {
    /// Node ID
    node_id: String,
    /// WAL writer
    wal_writer: WalWriter,
    /// WAL reader (for sync)
    wal_reader: Arc<RwLock<WalReader>>,
    /// State tracker
    state_tracker: Arc<StateTracker>,
    /// Cluster membership
    cluster: Arc<ClusterMembership>,
    /// Replication configuration
    config: ReplicationConfig,
    /// Current term
    term: RwLock<u64>,
    /// Commit LSN (highest LSN acknowledged by quorum)
    commit_lsn: RwLock<Lsn>,
    /// Pending writes awaiting acknowledgment
    pending_writes: RwLock<PendingWritesMap>,
    /// Next LSN to send to each follower
    next_lsn: RwLock<LsnMap>,
    /// Match LSN for each follower (highest acknowledged)
    match_lsn: RwLock<LsnMap>,
    /// Message sender for outbound messages
    message_tx: mpsc::Sender<(String, Message)>,
    /// Database executor for health checks
    executor: Option<Arc<MariaDbExecutor>>,
    /// Shutdown signal
    shutdown: RwLock<bool>,
    /// Track pending replication per peer - (sent_up_to_lsn, sent_at_time)
    /// If pending, don't send more until ACK received
    pending_replication: RwLock<std::collections::HashMap<String, (Lsn, std::time::Instant)>>,
}

impl LeaderNode {
    /// Create a new leader node
    pub fn new(
        node_id: String,
        wal_writer: WalWriter,
        wal_reader: WalReader,
        state_tracker: Arc<StateTracker>,
        cluster: Arc<ClusterMembership>,
        config: ReplicationConfig,
        message_tx: mpsc::Sender<(String, Message)>,
        executor: Option<Arc<MariaDbExecutor>>,
    ) -> Self {
        Self {
            node_id,
            wal_writer,
            wal_reader: Arc::new(RwLock::new(wal_reader)),
            state_tracker,
            cluster,
            config,
            term: RwLock::new(1),
            commit_lsn: RwLock::new(0),
            pending_writes: RwLock::new(HashMap::new()),
            next_lsn: RwLock::new(HashMap::new()),
            match_lsn: RwLock::new(HashMap::new()),
            message_tx,
            executor,
            shutdown: RwLock::new(false),
            pending_replication: RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Start the leader loop
    pub async fn start(&self) -> Result<()> {
        tracing::debug!("Leader replication loop starting");
        
        // Set ourselves as leader
        self.cluster.set_leader(&self.node_id).await?;

        // Initialize our own LSN from WAL - critical for restart recovery
        let current_lsn = self.wal_writer.current_lsn().await;
        if current_lsn > 0 {
            tracing::info!("Leader initializing with WAL at LSN {}", current_lsn);
            let _ = self.cluster.record_heartbeat(&self.node_id, current_lsn).await;
        }

        // Initialize follower state
        self.initialize_follower_state().await?;

        // DECOUPLED: Heartbeat runs at configured interval (for health monitoring)
        // Replication runs much faster (10ms) to maximize throughput
        let heartbeat_interval = Duration::from_millis(self.config.heartbeat_interval_ms);
        let replication_interval = Duration::from_millis(10); // 100 replication cycles/sec max
        
        let mut heartbeat_ticker = interval(heartbeat_interval);
        let mut replication_ticker = interval(replication_interval);
        let mut db_check_counter = 0u64;

        loop {
            if *self.shutdown.read().await {
                break;
            }

            tokio::select! {
                // Heartbeat path: health checks and cluster membership
                _ = heartbeat_ticker.tick() => {
                    // Check database health every 5 heartbeats
                    db_check_counter += 1;
                    if db_check_counter % 5 == 0 {
                        if !self.is_database_healthy().await {
                            tracing::error!("Local database is unhealthy - leader stepping down");
                            return Err(Error::DatabaseUnavailable);
                        }
                    }
                    
                    self.send_heartbeats().await?;
                    self.check_commit_progress().await?;
                    
                    // Check if we should yield to a higher-priority node
                    if let Some(higher_priority_node) = self.check_for_priority_yield().await {
                        tracing::info!(
                            "Yielding leadership to higher-priority node: {}",
                            higher_priority_node
                        );
                        return Ok(());
                    }
                }
                // Replication path: fast loop for maximum throughput
                _ = replication_ticker.tick() => {
                    if let Err(e) = self.replicate_to_followers().await {
                        tracing::warn!("Replication error: {}", e);
                    }
                }
            }
        }

        Ok(())
    }
    
    /// Check if a higher-priority (lower-ID) node is caught up and should become leader
    async fn check_for_priority_yield(&self) -> Option<String> {
        let self_node = self.cluster.get_self().await;
        let peers = self.cluster.peers().await;
        
        for peer in peers {
            // Skip synthetic peers
            if peer.id.starts_with("peer-") {
                continue;
            }
            // Skip dropped/offline nodes
            if peer.status == crate::state::NodeStatus::Dropped 
                || peer.status == crate::state::NodeStatus::Offline {
                continue;
            }
            
            // Check if this peer has a lower ID (higher priority)
            if peer.id < self.node_id {
                // Check if they're caught up
                if peer.last_applied_lsn >= self_node.last_applied_lsn {
                    tracing::info!(
                        "Higher-priority node {} is caught up (LSN {}), should yield leadership",
                        peer.id, peer.last_applied_lsn
                    );
                    return Some(peer.id.clone());
                } else {
                    tracing::debug!(
                        "Higher-priority node {} exists but is behind (our: {}, their: {})",
                        peer.id, self_node.last_applied_lsn, peer.last_applied_lsn
                    );
                }
            }
        }
        
        None
    }
    
    /// Check if the local database is healthy (can receive writes)
    async fn is_database_healthy(&self) -> bool {
        // Use the executor to check database health via SELECT 1
        if let Some(ref executor) = self.executor {
            match executor.health_check().await {
                Ok(healthy) => healthy,
                Err(e) => {
                    tracing::error!("Database health check failed: {}", e);
                    false
                }
            }
        } else {
            // No executor available - assume healthy (e.g., during testing)
            true
        }
    }

    /// Stop the leader
    pub async fn stop(&self) -> Result<()> {
        *self.shutdown.write().await = true;
        Ok(())
    }

    /// Initialize state for tracking followers
    async fn initialize_follower_state(&self) -> Result<()> {
        let peers = self.cluster.peers().await;
        let current_lsn = self.wal_writer.current_lsn().await;

        let mut next_lsn = self.next_lsn.write().await;
        let mut match_lsn = self.match_lsn.write().await;

        for peer in peers {
            // Start by assuming followers are at the beginning
            // They will inform us of their actual position
            next_lsn.insert(peer.id.clone(), current_lsn + 1);
            match_lsn.insert(peer.id.clone(), 0);
        }

        Ok(())
    }

    /// Accept a write request
    pub async fn write(&self, entry: LogEntry) -> Result<Lsn> {
        // Append to local WAL
        let lsn = self.wal_writer.append(entry.clone()).await?;

        // Update our own LSN in cluster membership so it shows in status
        let _ = self.cluster.record_heartbeat(&self.node_id, lsn).await;

        let quorum_size = self.cluster.quorum_size().await;

        // If we're the only node, immediately commit
        if quorum_size <= 1 {
            self.advance_commit_lsn(lsn).await?;
            return Ok(lsn);
        }

        // Create pending write
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending_writes.write().await;
            pending.insert(lsn, PendingWrite {
                lsn,
                acks: 1, // Count ourselves
                required_acks: quorum_size,
                response: tx,
            });
        }

        // Trigger replication
        self.replicate_to_followers().await?;

        // Wait for quorum acknowledgment with timeout
        let timeout = Duration::from_millis(self.config.replication_timeout_ms);
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(Error::Replication("Write cancelled".into())),
            Err(_) => {
                // Remove pending write
                self.pending_writes.write().await.remove(&lsn);
                Err(Error::Replication("Replication timeout".into()))
            }
        }
    }

    /// Send heartbeats to all followers
    async fn send_heartbeats(&self) -> Result<()> {
        let term = *self.term.read().await;
        let commit_lsn = *self.commit_lsn.read().await;

        // Build membership list from all known nodes
        let peers = self.cluster.peers().await;
        let mut members: Vec<(String, String)> = peers
            .iter()
            .map(|p| (p.id.clone(), p.address.clone()))
            .collect();
        // Add self to members
        if let Some(self_node) = self.cluster.get_node(&self.node_id).await {
            members.push((self_node.id, self_node.address));
        }

        let msg = Message::Heartbeat {
            term,
            leader_id: self.node_id.clone(),
            commit_lsn,
            members,
        };

        for peer in peers {
            if peer.status != NodeStatus::Dropped {
                // Send address directly so delivery loop doesn't need to look up
                let _ = self.message_tx.send((peer.address.clone(), msg.clone())).await;
            }
        }

        Ok(())
    }

    /// Replicate entries to followers
    async fn replicate_to_followers(&self) -> Result<()> {
        let peers = self.cluster.peers().await;
        if peers.is_empty() {
            return Ok(());
        }
        
        let term = *self.term.read().await;
        let commit_lsn = *self.commit_lsn.read().await;

        // Refresh WAL index ONCE per replication cycle (not per peer)
        {
            let mut reader = self.wal_reader.write().await;
            let _ = reader.refresh_index();
        }

        for peer in peers {
            if peer.status == NodeStatus::Dropped || peer.status == NodeStatus::Offline {
                continue;
            }

            // Get FRESH peer state from cluster first - the snapshot may be stale
            let current_peer_lsn = if let Some(fresh_peer) = self.cluster.get_node(&peer.id).await {
                fresh_peer.last_applied_lsn
            } else {
                peer.last_applied_lsn // Fallback to snapshot if node disappeared
            };
            
            // Check if this peer has pending replication (waiting for ACK)
            // Skip if we're still waiting, unless it's been more than 5 seconds (stale)
            // OR the peer's LSN has changed (meaning ACK was received via cluster membership)
            {
                let mut pending = self.pending_replication.write().await;
                if let Some((pending_lsn, sent_at)) = pending.get(&peer.id) {
                    // Check if peer's LSN has advanced past our pending LSN OR went backwards (restart)
                    // This means ACK was received and processed via cluster membership
                    if current_peer_lsn >= *pending_lsn || current_peer_lsn < peer.last_applied_lsn {
                        tracing::debug!("Peer {} ACK received (lsn {} vs pending {}), clearing pending", 
                            peer.id, current_peer_lsn, pending_lsn);
                        pending.remove(&peer.id);
                        // Continue to send next batch
                    } else {
                        let elapsed = sent_at.elapsed();
                        if elapsed < std::time::Duration::from_secs(5) {
                            tracing::trace!("Skipping peer {} - pending ACK for LSN {} ({}ms ago)", 
                                peer.id, pending_lsn, elapsed.as_millis());
                            continue;
                        } else {
                            tracing::warn!("Peer {} pending ACK timed out after {}s (at lsn {}, pending {}), will retry", 
                                peer.id, elapsed.as_secs(), current_peer_lsn, pending_lsn);
                            // Clear the stale pending entry so we can retry
                            pending.remove(&peer.id);
                        }
                    }
                }
            }

            // Use the FRESH peer LSN for replication (current_peer_lsn was fetched at start of loop)
            let next = current_peer_lsn + 1;
            tracing::trace!("Peer {} has last_applied_lsn={}, will replicate from next={}", peer.id, current_peer_lsn, next);

            let reader = self.wal_reader.read().await;
            let entries = match reader.read_batch(next, self.config.max_batch_entries) {
                Ok(e) => e,
                Err(e) => {
                    tracing::error!("Failed to read WAL batch for peer {}: {}", peer.id, e);
                    continue;
                }
            };
            drop(reader);

            if entries.is_empty() {
                continue;
            }

            let batch_max_lsn = entries.last().map(|e| e.header.lsn).unwrap_or(next);
            tracing::debug!("Replicating {} entries starting at LSN {} to {}", entries.len(), next, peer.id);

            // Get prev entry info
            let (prev_lsn, prev_term) = if next > 1 {
                let reader = self.wal_reader.read().await;
                if let Ok(Some(prev_entry)) = reader.get(next - 1) {
                    (prev_entry.header.lsn, prev_entry.header.term)
                } else {
                    (0, 0)
                }
            } else {
                (0, 0)
            };

            let msg = Message::AppendEntries {
                term,
                leader_id: self.node_id.clone(),
                prev_lsn,
                prev_term,
                entries,
                leader_commit_lsn: commit_lsn,
            };

            // Mark as pending BEFORE sending
            {
                let mut pending = self.pending_replication.write().await;
                pending.insert(peer.id.clone(), (batch_max_lsn, std::time::Instant::now()));
            }

            let _ = self.message_tx.send((peer.address.clone(), msg)).await;
        }

        Ok(())
    }

    /// Handle an append entries response from a follower
    pub async fn handle_append_response(
        &self,
        node_id: &str,
        term: u64,
        success: bool,
        match_lsn: Lsn,
    ) -> Result<()> {
        // Check term
        let current_term = *self.term.read().await;
        if term > current_term {
            // We're stale, step down
            return Err(Error::Replication("Stale leader".into()));
        }

        if success {
            // Clear pending replication for this peer - they ACK'd, we can send more
            {
                let mut pending = self.pending_replication.write().await;
                pending.remove(node_id);
            }

            // Update match_lsn for this follower
            {
                let mut matches = self.match_lsn.write().await;
                matches.insert(node_id.to_string(), match_lsn);
            }

            // Update next_lsn
            {
                let mut nexts = self.next_lsn.write().await;
                nexts.insert(node_id.to_string(), match_lsn + 1);
            }

            // Update cluster membership
            self.cluster.record_heartbeat(node_id, match_lsn).await?;

            // Check if we can advance commit
            self.check_commit_progress().await?;

            // Acknowledge pending writes
            self.acknowledge_writes(match_lsn).await?;
        } else {
            // Follower rejected, decrement next_lsn and retry
            // Also clear pending so we can retry
            {
                let mut pending = self.pending_replication.write().await;
                pending.remove(node_id);
            }
            let mut nexts = self.next_lsn.write().await;
            if let Some(next) = nexts.get_mut(node_id) {
                if *next > 1 {
                    *next -= 1;
                }
            }
        }

        Ok(())
    }

    /// Handle a sync request from a follower
    pub async fn handle_sync_request(
        &self,
        node_id: &str,
        from_lsn: Lsn,
        max_entries: usize,
    ) -> Result<Message> {
        let reader = self.wal_reader.read().await;
        let entries = reader.read_batch(from_lsn, max_entries)?;
        
        let last_lsn = self.wal_writer.current_lsn().await;
        let has_more = entries.last()
            .map(|e| e.header.lsn < last_lsn)
            .unwrap_or(false);

        // Update follower state
        if !entries.is_empty() {
            let mut nexts = self.next_lsn.write().await;
            nexts.insert(
                node_id.to_string(),
                entries.last().unwrap().header.lsn + 1,
            );
        }

        Ok(Message::SyncResponse {
            from_lsn,
            entries,
            has_more,
        })
    }

    /// Check if we can advance the commit LSN
    async fn check_commit_progress(&self) -> Result<()> {
        let matches = self.match_lsn.read().await;
        let quorum_size = self.cluster.quorum_size().await;

        // Include ourselves in the count
        let current_lsn = self.wal_writer.current_lsn().await;
        let mut all_lsns: Vec<Lsn> = matches.values().copied().collect();
        all_lsns.push(current_lsn);
        all_lsns.sort_unstable();

        // Find the LSN that has quorum
        if all_lsns.len() >= quorum_size {
            let commit_index = all_lsns.len() - quorum_size;
            let new_commit = all_lsns[commit_index];

            let current_commit = *self.commit_lsn.read().await;
            if new_commit > current_commit {
                self.advance_commit_lsn(new_commit).await?;
            }
        }

        Ok(())
    }

    /// Advance the commit LSN
    async fn advance_commit_lsn(&self, lsn: Lsn) -> Result<()> {
        *self.commit_lsn.write().await = lsn;
        self.state_tracker.set_last_applied_lsn(lsn).await?;
        Ok(())
    }

    /// Acknowledge pending writes up to the given LSN
    async fn acknowledge_writes(&self, up_to_lsn: Lsn) -> Result<()> {
        let quorum_size = self.cluster.quorum_size().await;
        let mut pending = self.pending_writes.write().await;
        let matches = self.match_lsn.read().await;

        // Count acks for each pending write
        let to_ack: Vec<Lsn> = pending
            .iter()
            .filter(|(lsn, _)| **lsn <= up_to_lsn)
            .filter_map(|(lsn, _pw)| {
                let ack_count = 1 + matches.values().filter(|m| **m >= *lsn).count();
                if ack_count >= quorum_size {
                    Some(*lsn)
                } else {
                    None
                }
            })
            .collect();

        drop(matches);

        // Send acknowledgments
        for lsn in to_ack {
            if let Some(pw) = pending.remove(&lsn) {
                let _ = pw.response.send(Ok(lsn));
            }
        }

        Ok(())
    }

    /// Get the current term
    pub async fn current_term(&self) -> u64 {
        *self.term.read().await
    }

    /// Get the commit LSN
    pub async fn commit_lsn(&self) -> Lsn {
        *self.commit_lsn.read().await
    }

    /// Get pending write count
    pub async fn pending_count(&self) -> usize {
        self.pending_writes.read().await.len()
    }

    /// Handle a vote request from a candidate
    /// Leaders step down if they see a higher term
    pub async fn handle_vote_request(
        &self,
        term: u64,
        candidate_id: &str,
        last_log_lsn: Lsn,
        _last_log_term: u64,
    ) -> Result<(Message, bool)> {
        let current_term = *self.term.read().await;

        // If candidate has higher term, we must step down
        if term > current_term {
            tracing::info!(
                "Received vote request with higher term {} (ours: {}), stepping down",
                term,
                current_term
            );

            // Update term
            *self.term.write().await = term;
            self.state_tracker.set_current_term(term).await?;

            // Check if we should vote for them
            let our_lsn = self.commit_lsn().await;
            let vote_granted = last_log_lsn >= our_lsn;

            if vote_granted {
                self.state_tracker.set_voted_for(Some(candidate_id)).await?;
            }

            let response = Message::VoteResponse {
                node_id: self.node_id.clone(),
                term,
                vote_granted,
            };

            // Signal that we need to step down
            return Ok((response, true));
        }

        // Reject - our term is equal or higher
        Ok((
            Message::VoteResponse {
                node_id: self.node_id.clone(),
                term: current_term,
                vote_granted: false,
            },
            false,
        ))
    }

    /// Request this leader to step down
    pub async fn step_down(&self) -> Result<()> {
        *self.shutdown.write().await = true;
        tracing::info!("Leader stepping down");
        Ok(())
    }

    /// Get this node's ID
    pub fn node_id(&self) -> &str {
        &self.node_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use crate::config::WalConfig;

    fn test_wal_config() -> WalConfig {
        WalConfig {
            batch_size: 10,
            flush_interval_ms: 10,
            compression: false,
            segment_size_mb: 1,
            retention_hours: 0,
            fsync: false,
        }
    }

    #[tokio::test]
    async fn test_leader_creation() {
        let dir = tempdir().unwrap();
        let (tx, _rx) = mpsc::channel(100);

        let wal_writer = WalWriter::new(
            dir.path().to_path_buf(),
            test_wal_config(),
            "leader".to_string(),
        ).await.unwrap();

        let wal_reader = WalReader::new(
            dir.path().to_path_buf(),
            1,
            false,
        ).unwrap();

        let state_tracker = Arc::new(StateTracker::new(
            dir.path().join("state"),
            "leader".to_string(),
        ).unwrap());

        let cluster = Arc::new(ClusterMembership::new(
            "leader".to_string(),
            "localhost:7654".to_string(),
            Duration::from_secs(1),
            Duration::from_secs(5),
        ));

        let _leader = LeaderNode::new(
            "leader".to_string(),
            wal_writer,
            wal_reader,
            state_tracker,
            cluster,
            ReplicationConfig::default(),
            tx,
        );
    }
}
