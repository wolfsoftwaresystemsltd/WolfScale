//! Leader Election
//!
//! Implements Raft-style leader election with randomized timeouts
//! to achieve automatic failover when the leader goes down.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use rand::Rng;

use crate::wal::entry::Lsn;
use crate::state::{ClusterMembership, NodeRole, StateTracker};
use crate::replication::Message;
use crate::error::Result;

/// Election state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElectionState {
    /// Following a leader
    Follower,
    /// Running for election
    Candidate,
    /// Won the election
    Leader,
}

/// Election configuration
#[derive(Debug, Clone)]
pub struct ElectionConfig {
    /// Minimum election timeout in milliseconds
    pub timeout_min_ms: u64,
    /// Maximum election timeout in milliseconds
    pub timeout_max_ms: u64,
}

impl Default for ElectionConfig {
    fn default() -> Self {
        Self {
            timeout_min_ms: 1500,
            timeout_max_ms: 3000,
        }
    }
}

/// Election coordinator manages leader election
pub struct ElectionCoordinator {
    /// This node's ID
    node_id: String,
    /// Current state
    state: RwLock<ElectionState>,
    /// Current term
    term: RwLock<u64>,
    /// Who we voted for in current term
    voted_for: RwLock<Option<String>>,
    /// Votes received (when candidate)
    votes_received: RwLock<Vec<String>>,
    /// Last heartbeat from leader
    last_heartbeat: RwLock<Instant>,
    /// Current election timeout
    election_timeout: RwLock<Duration>,
    /// Election configuration
    config: ElectionConfig,
    /// Cluster membership
    cluster: Arc<ClusterMembership>,
    /// State tracker for persistence
    state_tracker: Arc<StateTracker>,
    /// Last log LSN (for vote comparison)
    last_log_lsn: RwLock<Lsn>,
    /// Message sender
    message_tx: mpsc::Sender<(String, Message)>,
}

impl ElectionCoordinator {
    /// Create a new election coordinator
    pub fn new(
        node_id: String,
        cluster: Arc<ClusterMembership>,
        state_tracker: Arc<StateTracker>,
        config: ElectionConfig,
        message_tx: mpsc::Sender<(String, Message)>,
    ) -> Self {
        Self {
            node_id,
            state: RwLock::new(ElectionState::Follower),
            term: RwLock::new(1),
            voted_for: RwLock::new(None),
            votes_received: RwLock::new(Vec::new()),
            last_heartbeat: RwLock::new(Instant::now()),
            election_timeout: RwLock::new(Self::random_timeout(&config)),
            config,
            cluster,
            state_tracker,
            last_log_lsn: RwLock::new(0),
            message_tx,
        }
    }

    /// Generate a random election timeout
    fn random_timeout(config: &ElectionConfig) -> Duration {
        let mut rng = rand::thread_rng();
        let ms = rng.gen_range(config.timeout_min_ms..=config.timeout_max_ms);
        Duration::from_millis(ms)
    }

    /// Reset the election timer (called on valid heartbeat)
    pub async fn reset_timer(&self) {
        *self.last_heartbeat.write().await = Instant::now();
        *self.election_timeout.write().await = Self::random_timeout(&self.config);
    }

    /// Update the last log LSN
    pub async fn set_last_log_lsn(&self, lsn: Lsn) {
        *self.last_log_lsn.write().await = lsn;
    }

    /// Get current state
    pub async fn state(&self) -> ElectionState {
        *self.state.read().await
    }

    /// Get current term
    pub async fn term(&self) -> u64 {
        *self.term.read().await
    }

    /// Check if election timeout has expired
    pub async fn check_timeout(&self) -> bool {
        let last = *self.last_heartbeat.read().await;
        let timeout = *self.election_timeout.read().await;
        last.elapsed() > timeout
    }

    /// Start an election
    pub async fn start_election(&self) -> Result<()> {
        // Increment term
        let new_term = {
            let mut term = self.term.write().await;
            *term += 1;
            *term
        };

        // Transition to candidate
        *self.state.write().await = ElectionState::Candidate;

        // Vote for ourselves
        *self.voted_for.write().await = Some(self.node_id.clone());
        {
            let mut votes = self.votes_received.write().await;
            votes.clear();
            votes.push(self.node_id.clone());
        }

        // Update cluster role
        self.cluster.update_node(&self.node_id, |node| {
            node.role = NodeRole::Candidate;
        }).await?;

        // Persist term and vote
        self.state_tracker.set_current_term(new_term).await?;
        self.state_tracker.set_voted_for(Some(&self.node_id)).await?;

        // Reset election timer
        *self.election_timeout.write().await = Self::random_timeout(&self.config);
        *self.last_heartbeat.write().await = Instant::now();

        tracing::info!(
            "Starting election for term {} (node: {})",
            new_term,
            self.node_id
        );

        // Send RequestVote to all peers
        let last_lsn = *self.last_log_lsn.read().await;
        let msg = Message::RequestVote {
            term: new_term,
            candidate_id: self.node_id.clone(),
            last_log_lsn: last_lsn,
            last_log_term: new_term - 1, // Approximate
        };

        let peers = self.cluster.peers().await;
        for peer in peers {
            let _ = self.message_tx.send((peer.address.clone(), msg.clone())).await;
        }

        // Check if we're the only node (instant win)
        self.check_election_result().await?;

        Ok(())
    }

    /// Handle a vote request from another candidate
    pub async fn handle_vote_request(
        &self,
        term: u64,
        candidate_id: &str,
        last_log_lsn: Lsn,
        _last_log_term: u64,
    ) -> Result<Message> {
        let current_term = *self.term.read().await;

        // If term is stale, reject
        if term < current_term {
            return Ok(Message::VoteResponse {
                node_id: self.node_id.clone(),
                term: current_term,
                vote_granted: false,
            });
        }

        // If term is higher, update our term and become follower
        if term > current_term {
            self.step_down(term).await?;
        }

        // Check if we can vote for this candidate
        let voted_for = self.voted_for.read().await.clone();
        let our_last_lsn = *self.last_log_lsn.read().await;

        let can_vote = match voted_for {
            None => true,
            Some(ref id) => id == candidate_id,
        };

        // Candidate's log must be at least as up-to-date as ours
        let log_ok = last_log_lsn >= our_last_lsn;

        let vote_granted = can_vote && log_ok;

        if vote_granted {
            *self.voted_for.write().await = Some(candidate_id.to_string());
            self.state_tracker.set_voted_for(Some(candidate_id)).await?;
            self.reset_timer().await;

            tracing::info!(
                "Granting vote to {} for term {}",
                candidate_id,
                term
            );
        }

        Ok(Message::VoteResponse {
            node_id: self.node_id.clone(),
            term: *self.term.read().await,
            vote_granted,
        })
    }

    /// Handle a vote response
    pub async fn handle_vote_response(
        &self,
        voter_id: &str,
        term: u64,
        vote_granted: bool,
    ) -> Result<()> {
        let current_state = *self.state.read().await;
        let current_term = *self.term.read().await;

        // Ignore if we're not a candidate
        if current_state != ElectionState::Candidate {
            return Ok(());
        }

        // Ignore if term doesn't match
        if term != current_term {
            if term > current_term {
                self.step_down(term).await?;
            }
            return Ok(());
        }

        if vote_granted {
            let mut votes = self.votes_received.write().await;
            if !votes.contains(&voter_id.to_string()) {
                votes.push(voter_id.to_string());
                tracing::info!(
                    "Received vote from {} ({}/{})",
                    voter_id,
                    votes.len(),
                    self.cluster.quorum_size().await
                );
            }
        }

        self.check_election_result().await?;
        Ok(())
    }

    /// Check if we've won the election
    async fn check_election_result(&self) -> Result<()> {
        let current_state = *self.state.read().await;
        if current_state != ElectionState::Candidate {
            return Ok(());
        }

        let votes = self.votes_received.read().await.len();
        let quorum = self.cluster.quorum_size().await;

        if votes >= quorum {
            self.become_leader().await?;
        }

        Ok(())
    }

    /// Become the leader
    async fn become_leader(&self) -> Result<()> {
        *self.state.write().await = ElectionState::Leader;
        
        self.cluster.set_leader(&self.node_id).await?;

        tracing::info!(
            "Won election for term {}, becoming LEADER",
            *self.term.read().await
        );

        Ok(())
    }

    /// Step down to follower (saw higher term)
    pub async fn step_down(&self, new_term: u64) -> Result<()> {
        let current_term = *self.term.read().await;
        
        if new_term > current_term {
            *self.term.write().await = new_term;
            self.state_tracker.set_current_term(new_term).await?;
        }

        *self.state.write().await = ElectionState::Follower;
        *self.voted_for.write().await = None;
        self.state_tracker.set_voted_for(None).await?;

        self.cluster.update_node(&self.node_id, |node| {
            node.role = NodeRole::Follower;
        }).await?;

        self.reset_timer().await;

        tracing::info!(
            "Stepping down to follower for term {}",
            new_term
        );

        Ok(())
    }

    /// Become follower with known leader
    pub async fn become_follower(&self, term: u64, leader_id: &str) -> Result<()> {
        *self.term.write().await = term;
        *self.state.write().await = ElectionState::Follower;
        
        self.cluster.set_leader(leader_id).await?;
        self.cluster.update_node(&self.node_id, |node| {
            node.role = NodeRole::Follower;
        }).await?;

        self.reset_timer().await;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::time::Duration;

    #[tokio::test]
    async fn test_election_timeout() {
        let config = ElectionConfig {
            timeout_min_ms: 100,
            timeout_max_ms: 200,
        };

        let timeout = ElectionCoordinator::random_timeout(&config);
        assert!(timeout >= Duration::from_millis(100));
        assert!(timeout <= Duration::from_millis(200));
    }

    #[tokio::test]
    async fn test_election_coordinator_creation() {
        let dir = tempdir().unwrap();
        let (tx, _rx) = mpsc::channel(100);

        let cluster = Arc::new(ClusterMembership::new(
            "node-1".to_string(),
            "localhost:7654".to_string(),
            Duration::from_secs(1),
            Duration::from_secs(5),
        ));

        let state_tracker = Arc::new(StateTracker::new(
            dir.path().to_path_buf(),
            "node-1".to_string(),
        ).unwrap());

        let coordinator = ElectionCoordinator::new(
            "node-1".to_string(),
            cluster,
            state_tracker,
            ElectionConfig::default(),
            tx,
        );

        assert_eq!(coordinator.state().await, ElectionState::Follower);
        assert_eq!(coordinator.term().await, 1);
    }
}
