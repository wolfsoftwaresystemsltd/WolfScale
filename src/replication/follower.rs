//! Follower Node Implementation
//!
//! Handles follower responsibilities: receiving replicated entries,
//! applying them to the local database, and requesting sync on gaps.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};

use super::protocol::Message;
use super::ReplicationConfig;
use crate::wal::entry::{Lsn, WalEntry};
use crate::state::{ClusterMembership, StateTracker, ElectionCoordinator, ElectionConfig, ElectionState};
use crate::executor::MariaDbExecutor;
use crate::error::{Error, Result};

/// Batch of entries to replicate with metadata for ACK
#[derive(Clone)]
pub struct ReplicationBatch {
    pub entries: Vec<WalEntry>,
    pub term: u64,
    pub leader_id: String,
    pub leader_address: String,
}

/// Follower node state
pub struct FollowerNode {
    /// Node ID
    node_id: String,
    /// State tracker
    state_tracker: Arc<StateTracker>,
    /// Cluster membership
    cluster: Arc<ClusterMembership>,
    /// Database executor
    executor: Arc<MariaDbExecutor>,
    /// Replication configuration  
    config: ReplicationConfig,
    /// Current term
    term: RwLock<u64>,
    /// Current leader ID
    leader_id: RwLock<Option<String>>,
    /// Last applied LSN (Arc for sharing with spawned tasks)
    last_applied_lsn: Arc<RwLock<Lsn>>,
    /// Last heartbeat time
    last_heartbeat: RwLock<Instant>,
    /// Message sender for outbound messages
    message_tx: mpsc::Sender<(String, Message)>,
    /// Shutdown signal
    shutdown: RwLock<bool>,
    /// Election coordinator
    election: Arc<ElectionCoordinator>,
    /// Disable automatic election (manual promotion only)
    disable_auto_election: bool,
    /// Was this node previously a leader (prevents auto re-election)
    was_leader: RwLock<bool>,
    /// Channel to receive entries from message loop (message loop can't call us directly - not Send)
    entry_rx: tokio::sync::Mutex<Option<mpsc::Receiver<ReplicationBatch>>>,
}

impl FollowerNode {
    /// Create a new follower node
    pub fn new(
        node_id: String,
        _wal_writer: crate::wal::WalWriter,  // kept for API compatibility but unused
        state_tracker: Arc<StateTracker>,
        cluster: Arc<ClusterMembership>,
        executor: Arc<MariaDbExecutor>,
        config: ReplicationConfig,
        message_tx: mpsc::Sender<(String, Message)>,
        election_config: ElectionConfig,
        disable_auto_election: bool,
    ) -> Self {
        let election = Arc::new(ElectionCoordinator::new(
            node_id.clone(),
            cluster.clone(),
            state_tracker.clone(),
            election_config,
            message_tx.clone(),
        ));

        Self {
            node_id,
            state_tracker,
            cluster,
            executor,
            config,
            term: RwLock::new(1),
            leader_id: RwLock::new(None),
            last_applied_lsn: Arc::new(RwLock::new(0)),
            last_heartbeat: RwLock::new(Instant::now()),
            message_tx,
            shutdown: RwLock::new(false),
            election,
            disable_auto_election,
            was_leader: RwLock::new(false),
            entry_rx: tokio::sync::Mutex::new(None),
        }
    }

    /// Create with was_leader flag set (for rejoining nodes)
    pub fn new_rejoining(
        node_id: String,
        wal_writer: crate::wal::WalWriter,
        state_tracker: Arc<StateTracker>,
        cluster: Arc<ClusterMembership>,
        executor: Arc<MariaDbExecutor>,
        config: ReplicationConfig,
        message_tx: mpsc::Sender<(String, Message)>,
        election_config: ElectionConfig,
        disable_auto_election: bool,
    ) -> Self {
        let mut node = Self::new(
            node_id,
            wal_writer,
            state_tracker,
            cluster,
            executor,
            config,
            message_tx,
            election_config,
            disable_auto_election,
        );
        // Mark as previously a leader - won't auto-elect
        node.was_leader = RwLock::new(true);
        node
    }

    /// Set the entry receiver channel (for receiving entries from message loop)
    pub async fn set_entry_receiver(&self, rx: mpsc::Receiver<ReplicationBatch>) {
        *self.entry_rx.lock().await = Some(rx);
    }

    /// Start the follower loop
    /// 
    /// This loop handles both entry processing AND election timeouts.
    /// Entries come via channel from message loop (message loop can't call us directly - not Send)
    pub async fn start(&self) -> Result<()> {
        // Load last applied LSN from state
        let last_lsn = self.state_tracker.last_applied_lsn().await?;
        *self.last_applied_lsn.write().await = last_lsn;
        tracing::info!("Follower starting with last_applied_lsn={}", last_lsn);

        // Monitoring loop - process entries AND check for leader timeouts
        let heartbeat_check = Duration::from_millis(self.config.heartbeat_interval_ms * 2);
        let mut loop_count: u64 = 0;
        
        loop {
            loop_count += 1;
            if loop_count % 1000 == 1 {
                tracing::debug!("Follower loop iteration {}", loop_count);
            }
            
            if *self.shutdown.read().await {
                break;
            }

            // Try to receive entries from channel
            let maybe_batch = {
                let mut guard = self.entry_rx.lock().await;
                if let Some(ref mut rx) = *guard {
                    rx.try_recv().ok()
                } else {
                    // Log once if channel not connected
                    static LOGGED_NO_CHANNEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                    if !LOGGED_NO_CHANNEL.swap(true, std::sync::atomic::Ordering::Relaxed) {
                        tracing::error!("Follower entry_rx channel not connected!");
                    }
                    None
                }
            };

            // Process entries inline (no spawning - simpler and prevents memory issues)
            if let Some(batch) = maybe_batch {
                tracing::info!("Processing batch of {} entries (LSN {} to {})", 
                    batch.entries.len(),
                    batch.entries.first().map(|e| e.header.lsn).unwrap_or(0),
                    batch.entries.last().map(|e| e.header.lsn).unwrap_or(0));
                    
                // Process inline using the helper function
                process_batch_background(
                    batch,
                    Arc::clone(&self.executor),
                    Arc::clone(&self.cluster),
                    self.message_tx.clone(),
                    self.node_id.clone(),
                    Arc::clone(&self.last_applied_lsn),
                ).await;
                
                // Immediately check for more entries (no sleep)
                continue;
            }

            // No entries available, wait a bit before checking again
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(10)) => {
                    // Short sleep between polls
                }
                _ = tokio::time::sleep(heartbeat_check) => {
                    // Check for leader timeout periodically
                    if let Err(e) = self.check_leader_timeout().await {
                        tracing::warn!("Leader timeout check failed: {}", e);
                    }
                }
            }
        }

        Ok(())
    }

    /// Stop the follower
    pub async fn stop(&self) -> Result<()> {
        *self.shutdown.write().await = true;
        Ok(())
    }

    /// Reset the election timer (called when heartbeat is received)
    pub async fn reset_election_timer(&self) {
        self.election.reset_timer().await;
        *self.last_heartbeat.write().await = Instant::now();
    }

    /// Handle a heartbeat from the leader
    pub async fn handle_heartbeat(
        &self,
        term: u64,
        leader_id: String,
        commit_lsn: Lsn,
    ) -> Result<Message> {
        // Update term if needed
        let mut current_term = self.term.write().await;
        if term > *current_term {
            *current_term = term;
            self.state_tracker.set_current_term(term).await?;
        }
        drop(current_term);

        // Update leader
        *self.leader_id.write().await = Some(leader_id.clone());
        self.cluster.set_leader(&leader_id).await?;

        // Update heartbeat time and reset election timer
        *self.last_heartbeat.write().await = Instant::now();
        self.election.reset_timer().await;

        // Check if we need to sync
        let last_applied = *self.last_applied_lsn.read().await;
        if last_applied < commit_lsn {
            // Request sync
            self.request_sync(last_applied + 1).await?;
        }

        // Send response
        Ok(Message::HeartbeatResponse {
            node_id: self.node_id.clone(),
            term: *self.term.read().await,
            last_applied_lsn: last_applied,
            success: true,
        })
    }

    /// Handle an append entries request from the leader
    pub async fn handle_append_entries(
        &self,
        term: u64,
        leader_id: String,
        prev_lsn: Lsn,
        _prev_term: u64,
        entries: Vec<WalEntry>,
        _leader_commit_lsn: Lsn,
    ) -> Result<Message> {
        let current_term = *self.term.read().await;

        // Reject if term is stale
        if term < current_term {
            return Ok(Message::AppendEntriesResponse {
                node_id: self.node_id.clone(),
                term: current_term,
                success: false,
                match_lsn: 0,
            });
        }

        // Update term and leader
        if term > current_term {
            *self.term.write().await = term;
            self.state_tracker.set_current_term(term).await?;
        }
        *self.leader_id.write().await = Some(leader_id.clone());
        *self.last_heartbeat.write().await = Instant::now();

        // Check log consistency
        let last_applied = *self.last_applied_lsn.read().await;
        
        if prev_lsn > 0 && prev_lsn != last_applied {
            // Gap detected, need to sync
            tracing::warn!(
                "Log gap detected: prev_lsn={}, last_applied={}",
                prev_lsn,
                last_applied
            );
            
            // Request missing entries
            self.request_sync(last_applied + 1).await?;

            return Ok(Message::AppendEntriesResponse {
                node_id: self.node_id.clone(),
                term: *self.term.read().await,
                success: false,
                match_lsn: last_applied,
            });
        }

        // Apply entries
        let mut match_lsn = last_applied;
        for entry in entries {
            if entry.header.lsn <= last_applied {
                continue;
            }

            // Apply to local log and database
            match self.apply_entry(&entry).await {
                Ok(_) => {
                    match_lsn = entry.header.lsn;
                }
                Err(e) => {
                    // Log error but continue to next entry if it's an application error
                    // This prevents a single bad query from stopping all replication
                    tracing::error!("Follower failed to apply entry LSN {}: {}", entry.header.lsn, e);
                    
                    // We still advance match_lsn because we've "processed" this entry (even if with error)
                    // and we don't want the leader to keep sending it.
                    match_lsn = entry.header.lsn;
                }
            }
        }

        // Update last applied if we moved forward
        if match_lsn > last_applied {
            *self.last_applied_lsn.write().await = match_lsn;
            let _ = self.state_tracker.set_last_applied_lsn(match_lsn).await;
        }

        Ok(Message::AppendEntriesResponse {
            node_id: self.node_id.clone(),
            term: *self.term.read().await,
            success: true,
            match_lsn,
        })
    }

    /// Handle a sync response from the leader
    pub async fn handle_sync_response(
        &self,
        _from_lsn: Lsn,
        entries: Vec<WalEntry>,
        has_more: bool,
    ) -> Result<()> {
        let last_applied = *self.last_applied_lsn.read().await;

        let mut new_applied = last_applied;
        for entry in entries {
            if entry.header.lsn <= last_applied {
                continue;
            }

            match self.apply_entry(&entry).await {
                Ok(_) => {
                    new_applied = entry.header.lsn;
                }
                Err(e) => {
                    tracing::error!("Failed to apply synced entry {}: {}", entry.header.lsn, e);
                    break;
                }
            }
        }

        if new_applied > last_applied {
            *self.last_applied_lsn.write().await = new_applied;
            self.state_tracker.set_last_applied_lsn(new_applied).await?;
        }

        // Request more if needed
        if has_more {
            self.request_sync(new_applied + 1).await?;
        }

        Ok(())
    }

    /// Apply entries to database
    async fn apply_entry(&self, entry: &WalEntry) -> Result<()> {
        // NOTE: We do NOT append to local WAL for replicated entries.
        // The entries already have LSNs assigned by the leader.
        // We just execute the SQL - state tracking happens in the caller.
        // This keeps apply_entry as fast as possible.

        // Execute against database
        if let Err(e) = self.executor.execute_entry(&entry.entry).await {
            // Check if this is a non-fatal SQL error (e.g. key constraint, etc.)
            // For now, we log it and return it, but the caller (handle_append_entries)
            // will decide whether to continue.
            return Err(e);
        }

        Ok(())
    }

    /// Request sync from leader
    async fn request_sync(&self, from_lsn: Lsn) -> Result<()> {
        let leader_id = self.leader_id.read().await.clone();

        if let Some(leader) = leader_id {
            let msg = Message::SyncRequest {
                node_id: self.node_id.clone(),
                from_lsn,
                max_entries: self.config.max_batch_entries,
            };

            self.message_tx.send((leader, msg)).await
                .map_err(|_| Error::Network("Failed to send sync request".into()))?;
        }

        Ok(())
    }

    /// Check for leader timeout (may trigger election)
    async fn check_leader_timeout(&self) -> Result<()> {
        // Check if election timeout has expired
        if !self.election.check_timeout().await {
            return Ok(());
        }

        let election_state = self.election.state().await;
        let leader = self.leader_id.read().await.clone();

        match election_state {
            ElectionState::Follower => {
                // Leader timeout detected
                if leader.is_some() {
                    tracing::warn!(
                        "Leader timeout detected, no heartbeat for election timeout period"
                    );
                    *self.leader_id.write().await = None;
                }

                // Check if we should start an election
                if self.disable_auto_election {
                    tracing::info!("Auto-election disabled, waiting for manual promotion");
                    return Ok(());
                }

                // Don't auto-elect if we were previously a leader (prevent flip-flopping)
                if *self.was_leader.read().await {
                    tracing::info!(
                        "Was previously leader, not auto-electing (use manual promotion)"
                    );
                    return Ok(());
                }

                // Start election
                tracing::info!("Starting leader election");
                self.election.start_election().await?;
            }
            ElectionState::Candidate => {
                // Election timed out, start a new one
                tracing::info!("Election timed out, starting new election");
                self.election.start_election().await?;
            }
            ElectionState::Leader => {
                // We won! Nothing to do here, main.rs should detect this
            }
        }

        Ok(())
    }

    /// Handle a vote request from another candidate
    pub async fn handle_vote_request(
        &self,
        term: u64,
        candidate_id: &str,
        last_log_lsn: Lsn,
        last_log_term: u64,
    ) -> Result<Message> {
        self.election
            .handle_vote_request(term, candidate_id, last_log_lsn, last_log_term)
            .await
    }

    /// Handle a vote response
    pub async fn handle_vote_response(
        &self,
        voter_id: &str,
        term: u64,
        vote_granted: bool,
    ) -> Result<()> {
        self.election
            .handle_vote_response(voter_id, term, vote_granted)
            .await
    }

    /// Check if we've become the leader
    pub async fn is_leader(&self) -> bool {
        self.election.state().await == ElectionState::Leader
    }

    /// Get the election coordinator for external access
    pub fn election(&self) -> Arc<ElectionCoordinator> {
        self.election.clone()
    }

    /// Allow this node to participate in elections (for manual promotion)
    pub async fn enable_election_participation(&self) {
        *self.was_leader.write().await = false;
    }

    /// Get the current term
    pub async fn current_term(&self) -> u64 {
        *self.term.read().await
    }

    /// Get the last applied LSN
    pub async fn last_applied_lsn(&self) -> Lsn {
        *self.last_applied_lsn.read().await
    }

    /// Get the current leader ID
    pub async fn leader_id(&self) -> Option<String> {
        self.leader_id.read().await.clone()
    }

    /// Check if connected to leader
    pub async fn is_connected(&self) -> bool {
        let last_heartbeat = *self.last_heartbeat.read().await;
        let timeout = Duration::from_millis(self.config.heartbeat_interval_ms * 2);
        last_heartbeat.elapsed() < timeout
    }
}

/// Standalone batch processing function for background execution.
/// This runs inline to avoid blocking the follower's main loop.
async fn process_batch_background(
    batch: ReplicationBatch,
    executor: Arc<MariaDbExecutor>,
    cluster: Arc<ClusterMembership>,
    message_tx: mpsc::Sender<(String, Message)>,
    node_id: String,
    last_applied_lsn: Arc<RwLock<Lsn>>,
) {
    let current_lsn = *last_applied_lsn.read().await;
    let batch_max_lsn = batch.entries.last().map(|e| e.header.lsn).unwrap_or(0);
    
    // Skip if entirely stale
    if batch_max_lsn <= current_lsn {
        tracing::debug!("Skipping stale batch (max_lsn={} <= current={})", batch_max_lsn, current_lsn);
        send_ack_background(&message_tx, &batch, current_lsn, &node_id).await;
        return;
    }
    
    tracing::info!("Background processing {} entries (LSN {} to {}), current position: {}", 
        batch.entries.len(),
        batch.entries.first().map(|e| e.header.lsn).unwrap_or(0),
        batch_max_lsn,
        current_lsn);
    
    let mut highest_applied = current_lsn;
    let mut processed = 0usize;
    let mut skipped = 0usize;
    
    for entry in &batch.entries {
        // Skip already-applied entries
        if entry.header.lsn <= highest_applied {
            skipped += 1;
            continue;
        }
        
        // Execute the SQL - log what we're doing
        let sql_stmts = entry.entry.to_sql();
        let sql_preview = if sql_stmts.is_empty() {
            "noop".to_string()
        } else {
            let first = &sql_stmts[0];
            if first.len() > 100 { format!("{}...", &first[..100]) } else { first.clone() }
        };
        tracing::debug!("Executing LSN {}: {}", entry.header.lsn, sql_preview);
        
        let start = std::time::Instant::now();
        if let Err(e) = executor.execute_entry(&entry.entry).await {
            tracing::warn!("Entry LSN {} failed: {} - continuing", entry.header.lsn, e);
        }
        let elapsed = start.elapsed();
        if elapsed > std::time::Duration::from_secs(1) {
            tracing::warn!("Entry LSN {} took {:.1}s to execute: {}", entry.header.lsn, elapsed.as_secs_f64(), sql_preview);
        }
        
        // Mark as processed regardless of success/failure
        highest_applied = entry.header.lsn;
        processed += 1;
        
        // Batch state updates: save every 100 entries for performance
        if processed % 100 == 0 {
            *last_applied_lsn.write().await = highest_applied;
            tracing::info!("Progress: {} entries processed, at LSN {}", processed, highest_applied);
        }
    }
    
    // Final state update to ensure we don't lose progress
    *last_applied_lsn.write().await = highest_applied;
    
    tracing::info!("Batch complete: processed={}, skipped={}, lsn={}", processed, skipped, highest_applied);
    
    // Update cluster membership
    let _ = cluster.record_heartbeat(&node_id, highest_applied).await;
    
    // Send ACK to leader
    send_ack_background(&message_tx, &batch, highest_applied, &node_id).await;
}

/// Send ACK in background processing
#[allow(dead_code)]
async fn send_ack_background(
    message_tx: &mpsc::Sender<(String, Message)>,
    batch: &ReplicationBatch,
    match_lsn: Lsn,
    node_id: &str,
) {
    let ack = Message::AppendEntriesResponse {
        node_id: node_id.to_string(),
        term: batch.term,
        success: true,
        match_lsn,
    };
    
    if let Err(e) = message_tx.send((batch.leader_address.clone(), ack)).await {
        tracing::error!("Failed to send ACK: {}", e);
    } else {
        tracing::info!("ACK sent to {} with lsn={}", batch.leader_address, match_lsn);
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
    async fn test_follower_creation() {
        let dir = tempdir().unwrap();
        let (tx, _rx) = mpsc::channel(100);

        let wal_writer = WalWriter::new(
            dir.path().to_path_buf(),
            test_wal_config(),
            "follower".to_string(),
        ).await.unwrap();

        let state_tracker = Arc::new(StateTracker::new(
            dir.path().join("state"),
            "follower".to_string(),
        ).unwrap());

        let cluster = Arc::new(ClusterMembership::new(
            "follower".to_string(),
            "localhost:7655".to_string(),
            Duration::from_secs(1),
            Duration::from_secs(5),
        ));

        let executor = Arc::new(MariaDbExecutor::new_mock());

        let _follower = FollowerNode::new(
            "follower".to_string(),
            wal_writer,
            state_tracker,
            cluster,
            executor,
            ReplicationConfig::default(),
            tx,
            ElectionConfig::default(),
            false,
        );
    }
}
