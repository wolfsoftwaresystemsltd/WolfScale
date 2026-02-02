//! Replication Protocol
//!
//! Defines the wire protocol for communication between nodes.

use serde::{Deserialize, Serialize};

use crate::wal::entry::{Lsn, WalEntry};
use crate::state::NodeState;

/// Protocol messages for node communication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    // ========== Heartbeat/Health ==========
    /// Heartbeat from leader
    Heartbeat {
        term: u64,
        leader_id: String,
        commit_lsn: Lsn,
    },

    /// Heartbeat response
    HeartbeatResponse {
        node_id: String,
        term: u64,
        last_applied_lsn: Lsn,
        success: bool,
    },

    // ========== Log Replication ==========
    /// Append entries request (from leader to followers)
    AppendEntries {
        term: u64,
        leader_id: String,
        prev_lsn: Lsn,
        prev_term: u64,
        entries: Vec<WalEntry>,
        leader_commit_lsn: Lsn,
    },

    /// Append entries response
    AppendEntriesResponse {
        node_id: String,
        term: u64,
        success: bool,
        match_lsn: Lsn,
    },

    // ========== Leader Election ==========
    /// Request vote (from candidate)
    RequestVote {
        term: u64,
        candidate_id: String,
        last_log_lsn: Lsn,
        last_log_term: u64,
    },

    /// Vote response
    VoteResponse {
        node_id: String,
        term: u64,
        vote_granted: bool,
    },

    // ========== Synchronization ==========
    /// Request to sync entries (from follower to leader)
    SyncRequest {
        node_id: String,
        from_lsn: Lsn,
        max_entries: usize,
    },

    /// Sync response with entries
    SyncResponse {
        from_lsn: Lsn,
        entries: Vec<WalEntry>,
        has_more: bool,
    },

    /// Full sync request (for nodes that are too far behind)
    FullSyncRequest {
        node_id: String,
    },

    /// Full sync start (leader indicates tables to sync)
    FullSyncStart {
        tables: Vec<String>,
        snapshot_lsn: Lsn,
    },

    /// Full sync chunk (table data)
    FullSyncChunk {
        table: String,
        data: Vec<u8>,
        is_last: bool,
    },

    /// Full sync complete
    FullSyncComplete {
        snapshot_lsn: Lsn,
    },

    // ========== Cluster Membership ==========
    /// Join cluster request
    JoinRequest {
        node_id: String,
        address: String,
    },

    /// Join cluster response
    JoinResponse {
        success: bool,
        leader_id: Option<String>,
        leader_address: Option<String>,
        current_term: u64,
        message: Option<String>,
    },

    /// Leave cluster request
    LeaveRequest {
        node_id: String,
    },

    /// Leave cluster response
    LeaveResponse {
        success: bool,
    },

    /// Cluster state update (broadcast by leader)
    ClusterStateUpdate {
        term: u64,
        leader_id: String,
        nodes: Vec<NodeState>,
    },

    // ========== Status ==========
    /// Status request
    StatusRequest,

    /// Status response
    StatusResponse {
        node_id: String,
        is_leader: bool,
        term: u64,
        last_applied_lsn: Lsn,
        commit_lsn: Lsn,
        leader_id: Option<String>,
    },

    // ========== Write Forwarding ==========
    /// Forward write to leader
    WriteForward {
        entry: WalEntry,
        client_id: String,
    },

    /// Write forward response
    WriteForwardResponse {
        success: bool,
        lsn: Option<Lsn>,
        error: Option<String>,
    },

    // ========== Error ==========
    /// Error response
    Error {
        code: ErrorCode,
        message: String,
    },
}

/// Error codes for protocol errors
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    /// Not the leader
    NotLeader,
    /// Term mismatch
    TermMismatch,
    /// Node not found
    NodeNotFound,
    /// Log inconsistency
    LogInconsistency,
    /// Timeout
    Timeout,
    /// Internal error
    Internal,
}

impl Message {
    /// Serialize message to bytes
    pub fn serialize(&self) -> Result<Vec<u8>, bincode::Error> {
        bincode::serialize(self)
    }

    /// Deserialize message from bytes
    pub fn deserialize(bytes: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(bytes)
    }

    /// Get the message type name (for logging)
    pub fn type_name(&self) -> &'static str {
        match self {
            Message::Heartbeat { .. } => "Heartbeat",
            Message::HeartbeatResponse { .. } => "HeartbeatResponse",
            Message::AppendEntries { .. } => "AppendEntries",
            Message::AppendEntriesResponse { .. } => "AppendEntriesResponse",
            Message::RequestVote { .. } => "RequestVote",
            Message::VoteResponse { .. } => "VoteResponse",
            Message::SyncRequest { .. } => "SyncRequest",
            Message::SyncResponse { .. } => "SyncResponse",
            Message::FullSyncRequest { .. } => "FullSyncRequest",
            Message::FullSyncStart { .. } => "FullSyncStart",
            Message::FullSyncChunk { .. } => "FullSyncChunk",
            Message::FullSyncComplete { .. } => "FullSyncComplete",
            Message::JoinRequest { .. } => "JoinRequest",
            Message::JoinResponse { .. } => "JoinResponse",
            Message::LeaveRequest { .. } => "LeaveRequest",
            Message::LeaveResponse { .. } => "LeaveResponse",
            Message::ClusterStateUpdate { .. } => "ClusterStateUpdate",
            Message::StatusRequest => "StatusRequest",
            Message::StatusResponse { .. } => "StatusResponse",
            Message::WriteForward { .. } => "WriteForward",
            Message::WriteForwardResponse { .. } => "WriteForwardResponse",
            Message::Error { .. } => "Error",
        }
    }
}

/// Frame header for length-prefixed messages
#[derive(Debug, Clone, Copy)]
pub struct FrameHeader {
    /// Message length
    pub length: u32,
    /// Message checksum
    pub checksum: u32,
}

impl FrameHeader {
    /// Header size in bytes
    pub const SIZE: usize = 8;

    /// Create a new frame header
    pub fn new(data: &[u8]) -> Self {
        Self {
            length: data.len() as u32,
            checksum: crc32fast::hash(data),
        }
    }

    /// Serialize header to bytes
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut bytes = [0u8; Self::SIZE];
        bytes[0..4].copy_from_slice(&self.length.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.checksum.to_le_bytes());
        bytes
    }

    /// Deserialize header from bytes
    pub fn from_bytes(bytes: &[u8; Self::SIZE]) -> Self {
        Self {
            length: u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            checksum: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_serialization() {
        let msg = Message::Heartbeat {
            term: 1,
            leader_id: "node-1".to_string(),
            commit_lsn: 100,
        };

        let bytes = msg.serialize().unwrap();
        let restored = Message::deserialize(&bytes).unwrap();

        match restored {
            Message::Heartbeat { term, leader_id, commit_lsn } => {
                assert_eq!(term, 1);
                assert_eq!(leader_id, "node-1");
                assert_eq!(commit_lsn, 100);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_frame_header() {
        let data = b"test message data";
        let header = FrameHeader::new(data);
        let bytes = header.to_bytes();
        let restored = FrameHeader::from_bytes(&bytes);

        assert_eq!(header.length, restored.length);
        assert_eq!(header.checksum, restored.checksum);
    }
}
