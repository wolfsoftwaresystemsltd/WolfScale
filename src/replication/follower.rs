//! Follower Node Implementation
//!
//! Handles follower responsibilities: receiving replicated entries,
//! applying them to the local database, and requesting sync on gaps.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tokio::time::interval;

use super::protocol::Message;
use super::ReplicationConfig;
use crate::wal::entry::{Lsn, WalEntry};
use crate::wal::WalWriter;
use crate::state::{ClusterMembership, StateTracker, ElectionCoordinator, ElectionConfig, ElectionState};
use crate::executor::MariaDbExecutor;
use crate::error::{Error, Result};

/// Follower node state
pub struct FollowerNode {
    /// Node ID
    node_id: String,
    /// WAL writer (for local persistence)
    wal_writer: WalWriter,
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
    /// Last applied LSN
    last_applied_lsn: RwLock<Lsn>,
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
    /// Channel to receive entries from message loop (due to Send trait constraints)
    entry_rx: tokio::sync::Mutex<Option<mpsc::Receiver<Vec<crate::wal::entry::WalEntry>>>>,
}

impl FollowerNode {
    /// Create a new follower node
    pub fn new(
        node_id: String,
        wal_writer: WalWriter,
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
            wal_writer,
            state_tracker,
            cluster,
            executor,
            config,
            term: RwLock::new(1),
            leader_id: RwLock::new(None),
            last_applied_lsn: RwLock::new(0),
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
        wal_writer: WalWriter,
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
    pub async fn set_entry_receiver(&self, rx: mpsc::Receiver<Vec<crate::wal::entry::WalEntry>>) {
        *self.entry_rx.lock().await = Some(rx);
    }

    /// Start the follower loop
    pub async fn start(&self) -> Result<()> {
        // Load last applied LSN from state
        let last_lsn = self.state_tracker.last_applied_lsn().await?;
        *self.last_applied_lsn.write().await = last_lsn;

        // Start monitoring loop
        let check_interval = Duration::from_millis(self.config.heartbeat_interval_ms / 2);
        let mut interval_ticker = interval(check_interval);

        loop {
            if *self.shutdown.read().await {
                break;
            }

            // Take the receiver out for this iteration
            let mut entry_rx_guard = self.entry_rx.lock().await;
            let maybe_entries = if let Some(ref mut rx) = *entry_rx_guard {
                // Try to receive without blocking
                rx.try_recv().ok()
            } else {
                None
            };
            drop(entry_rx_guard);

            // Process any received entries
            if let Some(entries) = maybe_entries {
                tracing::info!("Processing {} replicated entries from leader", entries.len());
                for entry in &entries {
                    if let Err(e) = self.apply_entry(entry).await {
                        // Log but continue - non-fatal errors shouldn't stop replication
                        tracing::warn!("Failed to apply entry LSN {}: {} - continuing", entry.header.lsn, e);
                    }
                    // Always update our LSN - even if the entry failed, we've "processed" it
                    // This prevents re-processing the same entries forever
                    let _ = self.cluster.record_heartbeat(&self.node_id, entry.header.lsn).await;
                }
            }

            tokio::select! {
                _ = interval_ticker.tick() => {
                    // Check for leader timeout
                    self.check_leader_timeout().await?;
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
        // Persist to local WAL first
        if let Err(e) = self.wal_writer.append(entry.entry.clone()).await {
            tracing::error!("Failed to persist replicated entry to WAL: {}", e);
            return Err(e);
        }

        // Execute against database
        if let Err(e) = self.executor.execute_entry(&entry.entry).await {
            // Check if this is a non-fatal SQL error (e.g. key constraint, etc.)
            // For now, we log it and return it, but the caller (handle_append_entries)
            // will decide whether to continue.
            return Err(e);
        }

        // Record as applied
        if let Some(table) = entry.entry.table_name() {
            let pk_str = format!("{:?}", entry.header.lsn); 
            let _ = self.state_tracker
                .record_applied(entry.header.lsn, table, &pk_str)
                .await;
        }

        // Update our own LSN in cluster membership so it shows in status
        let _ = self.cluster.record_heartbeat(&self.node_id, entry.header.lsn).await;

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
