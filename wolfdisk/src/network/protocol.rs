//! Network protocol messages for WolfDisk cluster communication

use serde::{Deserialize, Serialize};

/// Protocol message types for inter-node communication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    // === Discovery & Heartbeat ===
    /// Node announcing its presence
    Announce(AnnounceMsg),
    /// Heartbeat from leader
    Heartbeat(HeartbeatMsg),

    // === Chunk Operations ===
    /// Request to store a chunk
    StoreChunk(StoreChunkMsg),
    /// Response to store chunk request
    StoreChunkAck(StoreChunkAckMsg),
    /// Request to retrieve a chunk
    GetChunk(GetChunkMsg),
    /// Response with chunk data
    ChunkData(ChunkDataMsg),
    /// Request to delete a chunk
    DeleteChunk(DeleteChunkMsg),

    // === Index Operations ===
    /// Update to file index (file created/modified/deleted)
    IndexUpdate(IndexUpdateMsg),
    /// Request full index sync
    SyncRequest(SyncRequestMsg),
    /// Response with full index
    SyncResponse(SyncResponseMsg),

    // === Client Operations ===
    /// Client requesting file read (forwarded to leader if needed)
    ReadRequest(ReadRequestMsg),
    /// Client requesting file write (forwarded to leader)
    WriteRequest(WriteRequestMsg),
    /// Response to client request
    ClientResponse(ClientResponseMsg),
}

/// Node announcement for discovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnounceMsg {
    pub node_id: String,
    pub address: String,
    pub role: NodeRoleInfo,
}

/// Role information for announcements
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum NodeRoleInfo {
    Leader,
    Follower,
    Client,
    Unknown,
}

/// Heartbeat from leader
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatMsg {
    pub leader_id: String,
    pub term: u64,
    pub index_version: u64,
    pub chunk_count: u64,
}

/// Store chunk request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreChunkMsg {
    pub hash: [u8; 32],
    pub data: Vec<u8>,
}

/// Store chunk acknowledgment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreChunkAckMsg {
    pub hash: [u8; 32],
    pub success: bool,
    pub error: Option<String>,
}

/// Get chunk request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetChunkMsg {
    pub hash: [u8; 32],
}

/// Chunk data response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkDataMsg {
    pub hash: [u8; 32],
    pub data: Option<Vec<u8>>,
    pub error: Option<String>,
}

/// Delete chunk request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteChunkMsg {
    pub hash: [u8; 32],
}

/// Index update message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexUpdateMsg {
    pub version: u64,
    pub operation: IndexOperation,
}

/// Type of index operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IndexOperation {
    /// File created or updated
    Upsert {
        path: String,
        size: u64,
        modified_ms: u64,
        permissions: u32,
        chunks: Vec<ChunkRefMsg>,
    },
    /// File or directory deleted
    Delete { path: String },
    /// Directory created
    Mkdir {
        path: String,
        permissions: u32,
    },
}

/// Chunk reference in protocol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRefMsg {
    pub hash: [u8; 32],
    pub offset: u64,
    pub size: u32,
}

/// Request for full index sync
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRequestMsg {
    pub from_version: u64,
}

/// Full index sync response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResponseMsg {
    pub current_version: u64,
    pub entries: Vec<IndexEntryMsg>,
}

/// Index entry in sync response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntryMsg {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified_ms: u64,
    pub permissions: u32,
    pub chunks: Vec<ChunkRefMsg>,
}

/// Client read request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadRequestMsg {
    pub path: String,
    pub offset: u64,
    pub size: u32,
}

/// Client write request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteRequestMsg {
    pub path: String,
    pub offset: u64,
    pub data: Vec<u8>,
}

/// Client response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientResponseMsg {
    pub success: bool,
    pub data: Option<Vec<u8>>,
    pub error: Option<String>,
}

/// Serialize a message for transmission
pub fn encode_message(msg: &Message) -> Result<Vec<u8>, bincode::Error> {
    bincode::serialize(msg)
}

/// Deserialize a message from bytes
pub fn decode_message(data: &[u8]) -> Result<Message, bincode::Error> {
    bincode::deserialize(data)
}
