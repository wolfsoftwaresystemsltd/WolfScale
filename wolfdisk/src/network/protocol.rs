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

    // === Replication (leader -> follower) ===
    /// Full file sync with chunk data (for writes)
    FileSync(FileSyncMsg),
    /// Rename a file or directory
    RenameFile(RenameFileMsg),
    /// Set file/directory attributes (chmod/chown)
    SetAttr(SetAttrMsg),

    // === Client Operations ===
    /// Client requesting file read (forwarded to leader if needed)
    ReadRequest(ReadRequestMsg),
    /// Client requesting file write (forwarded to leader)
    WriteRequest(WriteRequestMsg),
    /// Response to client request
    ClientResponse(ClientResponseMsg),

    // === File Operations (forwarded from followers to leader) ===
    /// Create a file
    CreateFile(CreateFileMsg),
    /// Create a directory
    CreateDir(CreateDirMsg),
    /// Delete a file
    DeleteFile(DeleteFileMsg),
    /// Delete a directory
    DeleteDir(DeleteDirMsg),
    /// Create a symbolic link
    CreateSymlink(CreateSymlinkMsg),
    /// Response to file operation
    FileOpResponse(FileOpResponseMsg),
    /// Get file/directory attributes (thin client)
    GetAttr(GetAttrMsg),
    /// Get file/directory attributes response
    GetAttrResponse(GetAttrResponseMsg),
    /// Read directory contents (thin client)
    ReadDir(ReadDirMsg),
    /// Read directory response
    ReadDirResponse(ReadDirResponseMsg),
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
    /// File or directory renamed
    Rename {
        from_path: String,
        to_path: String,
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
    /// Paths deleted since the requested version (for delta sync)
    pub deleted_paths: Vec<String>,
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

/// Create file request (follower -> leader)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFileMsg {
    pub path: String,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
}

/// Create directory request (follower -> leader)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDirMsg {
    pub path: String,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
}

/// Delete file request (follower -> leader)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteFileMsg {
    pub path: String,
}

/// Delete directory request (follower -> leader)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteDirMsg {
    pub path: String,
}

/// Response to file operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOpResponseMsg {
    pub success: bool,
    pub error: Option<String>,
}

/// Get file attributes request (thin-client)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetAttrMsg {
    pub path: String,
}

/// Get file attributes response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetAttrResponseMsg {
    pub exists: bool,
    pub is_dir: bool,
    pub size: u64,
    pub permissions: u32,
    pub uid: u32,
    pub gid: u32,
    pub modified_ms: u64,
    pub created_ms: u64,
    pub accessed_ms: u64,
}

/// Read directory request (thin-client)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadDirMsg {
    pub path: String,
}

/// Directory entry in ReadDirResponse
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirEntryMsg {
    pub name: String,
    pub is_dir: bool,
}

/// Read directory response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadDirResponseMsg {
    pub success: bool,
    pub entries: Vec<DirEntryMsg>,
    pub error: Option<String>,
}

/// Full file sync with chunk data (for writes and initial sync)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSyncMsg {
    /// File path
    pub path: String,
    /// Whether this is a directory
    pub is_dir: bool,
    /// File size in bytes
    pub size: u64,
    /// File permissions
    pub permissions: u32,
    /// Owner user ID
    pub uid: u32,
    /// Owner group ID
    pub gid: u32,
    /// Modification time (ms since epoch)
    pub modified_ms: u64,
    /// Chunk references
    pub chunks: Vec<ChunkRefMsg>,
    /// Actual chunk data (hash -> data)
    pub chunk_data: Vec<ChunkWithData>,
}

/// Chunk with its actual data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkWithData {
    pub hash: [u8; 32],
    pub data: Vec<u8>,
}

/// Rename file/directory message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameFileMsg {
    pub from_path: String,
    pub to_path: String,
}

/// Set file attributes message (chmod/chown)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetAttrMsg {
    pub path: String,
    pub permissions: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub size: Option<u64>,
    pub modified_ms: Option<u64>,
}

/// Create symbolic link message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSymlinkMsg {
    /// Path where the symlink will be created
    pub link_path: String,
    /// Target path the symlink points to
    pub target: String,
}

/// Serialize a message for transmission
pub fn encode_message(msg: &Message) -> Result<Vec<u8>, bincode::Error> {
    bincode::serialize(msg)
}

/// Deserialize a message from bytes
pub fn decode_message(data: &[u8]) -> Result<Message, bincode::Error> {
    bincode::deserialize(data)
}

