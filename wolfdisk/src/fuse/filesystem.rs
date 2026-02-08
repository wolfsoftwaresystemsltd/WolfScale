//! WolfDisk FUSE Filesystem Implementation
//!
//! Implements the fuser::Filesystem trait to provide a mountable
//! distributed filesystem.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry,
    ReplyOpen, ReplyWrite, Request,
};
use tracing::{debug, info, warn};

use crate::cluster::ClusterManager;
use crate::config::Config;
use crate::error::Result;
use crate::network::peer::PeerManager;
use crate::network::protocol::{Message, CreateFileMsg, CreateDirMsg, DeleteFileMsg, DeleteDirMsg, IndexUpdateMsg, IndexOperation, ChunkRefMsg, FileSyncMsg, ChunkWithData, WriteRequestMsg, RenameFileMsg, CreateSymlinkMsg};
use crate::storage::{ChunkStore, FileIndex, FileEntry, InodeTable};

/// TTL for attribute caching
const TTL: Duration = Duration::from_secs(1);

/// Root inode number
const ROOT_INODE: u64 = 1;

/// WolfDisk FUSE Filesystem
pub struct WolfDiskFS {
    /// Configuration
    config: Config,

    /// Chunk storage backend (shared for replication)
    chunk_store: Arc<ChunkStore>,

    /// File metadata index (shared for replication)
    file_index: Arc<RwLock<FileIndex>>,

    /// Inode to path mapping (shared for replication)
    inode_table: Arc<RwLock<InodeTable>>,

    /// Next available inode number (shared for replication)
    next_inode: Arc<RwLock<u64>>,

    /// Open file handles (fh -> inode)
    open_files: RwLock<HashMap<u64, u64>>,

    /// Next file handle
    next_fh: RwLock<u64>,

    /// Cluster manager for leader/follower state
    cluster: Option<Arc<ClusterManager>>,

    /// Peer manager for network communication
    peer_manager: Option<Arc<PeerManager>>,
}

impl WolfDiskFS {
    /// Create a new WolfDisk filesystem (standalone mode)
    pub fn new(config: Config) -> Result<Self> {
        // Create chunk store and file index for standalone mode
        std::fs::create_dir_all(config.chunks_dir())?;
        std::fs::create_dir_all(config.index_dir())?;
        
        let chunk_store = Arc::new(ChunkStore::new(config.chunks_dir(), config.replication.chunk_size)?);
        let file_index = Arc::new(RwLock::new(FileIndex::load_or_create(&config.index_dir())?));
        
        // Build inode table from index
        let (inode_table, max_inode) = {
            let index = file_index.read().unwrap();
            InodeTable::from_index(&index)
        };
        let inode_table = Arc::new(RwLock::new(inode_table));
        let next_inode = Arc::new(RwLock::new(max_inode + 1));
        
        Self::with_cluster(config, None, None, file_index, chunk_store, inode_table, next_inode)
    }

    /// Create a new WolfDisk filesystem with cluster support
    pub fn with_cluster(
        config: Config,
        cluster: Option<Arc<ClusterManager>>,
        peer_manager: Option<Arc<PeerManager>>,
        file_index: Arc<RwLock<FileIndex>>,
        chunk_store: Arc<ChunkStore>,
        inode_table: Arc<RwLock<InodeTable>>,
        next_inode: Arc<RwLock<u64>>,
    ) -> Result<Self> {
        info!("Initializing WolfDisk filesystem");

        // Ensure data directories exist
        std::fs::create_dir_all(config.chunks_dir())?;
        std::fs::create_dir_all(config.index_dir())?;

        Ok(Self {
            config,
            chunk_store,
            file_index,
            inode_table,
            next_inode,
            open_files: RwLock::new(HashMap::new()),
            next_fh: RwLock::new(1),
            cluster,
            peer_manager,
        })
    }

    /// Check if this node is the leader (or standalone)
    fn is_leader(&self) -> bool {
        match &self.cluster {
            Some(cluster) => cluster.is_leader(),
            None => true, // Standalone mode = always "leader"
        }
    }

    /// Forward a file creation to the leader
    fn forward_create_to_leader(&self, path: &str, mode: u32, uid: u32, gid: u32) -> std::result::Result<(), i32> {
        let (cluster, peer_manager) = match (&self.cluster, &self.peer_manager) {
            (Some(c), Some(p)) => (c, p),
            _ => return Err(libc::EIO),
        };

        let leader_id = cluster.leader_id().ok_or(libc::ENOENT)?;
        let leader_addr = cluster.leader_address().ok_or(libc::ENOENT)?;

        let conn = peer_manager.get_or_connect_leader(&leader_id, &leader_addr)
            .map_err(|e| {
                warn!("Failed to connect to leader: {}", e);
                libc::EIO
            })?;

        let msg = Message::CreateFile(CreateFileMsg {
            path: path.to_string(),
            mode,
            uid,
            gid,
        });

        let response = conn.request(&msg).map_err(|e| {
            warn!("Failed to send create request to leader: {}", e);
            libc::EIO
        })?;

        match response {
            Message::FileOpResponse(resp) if resp.success => Ok(()),
            Message::FileOpResponse(resp) => {
                warn!("Leader rejected create: {:?}", resp.error);
                Err(libc::EIO)
            }
            _ => Err(libc::EIO),
        }
    }
    
    /// Forward a write operation to the leader
    fn forward_write_to_leader(&self, path: &str, offset: u64, data: &[u8]) -> std::result::Result<u32, i32> {
        let (cluster, peer_manager) = match (&self.cluster, &self.peer_manager) {
            (Some(c), Some(p)) => (c, p),
            _ => return Err(libc::EIO),
        };

        let leader_id = cluster.leader_id().ok_or(libc::ENOENT)?;
        let leader_addr = cluster.leader_address().ok_or(libc::ENOENT)?;

        info!("Forwarding write to leader {} for path: {} (offset: {}, size: {})", 
            leader_id, path, offset, data.len());

        let conn = peer_manager.get_or_connect_leader(&leader_id, &leader_addr)
            .map_err(|e| {
                warn!("Failed to connect to leader: {}", e);
                libc::EIO
            })?;

        let msg = Message::WriteRequest(WriteRequestMsg {
            path: path.to_string(),
            offset,
            data: data.to_vec(),
        });

        let response = conn.request(&msg).map_err(|e| {
            warn!("Failed to send write request to leader: {}", e);
            libc::EIO
        })?;

        match response {
            Message::ClientResponse(resp) if resp.success => {
                // Return bytes written (from data length or response)
                Ok(data.len() as u32)
            }
            Message::ClientResponse(resp) => {
                warn!("Leader rejected write: {:?}", resp.error);
                Err(libc::EIO)
            }
            _ => Err(libc::EIO),
        }
    }

    /// Forward a directory creation to the leader
    fn forward_mkdir_to_leader(&self, path: &str, mode: u32, uid: u32, gid: u32) -> std::result::Result<(), i32> {
        let (cluster, peer_manager) = match (&self.cluster, &self.peer_manager) {
            (Some(c), Some(p)) => (c, p),
            _ => return Err(libc::EIO),
        };

        let leader_id = cluster.leader_id().ok_or(libc::ENOENT)?;
        let leader_addr = cluster.leader_address().ok_or(libc::ENOENT)?;

        let conn = peer_manager.get_or_connect_leader(&leader_id, &leader_addr)
            .map_err(|_| libc::EIO)?;

        let msg = Message::CreateDir(CreateDirMsg {
            path: path.to_string(),
            mode,
            uid,
            gid,
        });

        let response = conn.request(&msg).map_err(|_| libc::EIO)?;

        match response {
            Message::FileOpResponse(resp) if resp.success => Ok(()),
            _ => Err(libc::EIO),
        }
    }

    /// Forward a file deletion to the leader
    fn forward_unlink_to_leader(&self, path: &str) -> std::result::Result<(), i32> {
        let (cluster, peer_manager) = match (&self.cluster, &self.peer_manager) {
            (Some(c), Some(p)) => (c, p),
            _ => return Err(libc::EIO),
        };

        let leader_id = cluster.leader_id().ok_or(libc::ENOENT)?;
        let leader_addr = cluster.leader_address().ok_or(libc::ENOENT)?;

        let conn = peer_manager.get_or_connect_leader(&leader_id, &leader_addr)
            .map_err(|_| libc::EIO)?;

        let msg = Message::DeleteFile(DeleteFileMsg {
            path: path.to_string(),
        });

        let response = conn.request(&msg).map_err(|_| libc::EIO)?;

        match response {
            Message::FileOpResponse(resp) if resp.success => Ok(()),
            _ => Err(libc::EIO),
        }
    }

    /// Broadcast an index update to all followers (leader only)
    fn broadcast_index_update(&self, operation: IndexOperation) {
        if !self.is_leader() {
            return;
        }
        
        if let (Some(cluster), Some(peer_manager)) = (&self.cluster, &self.peer_manager) {
            // First, ensure we're connected to all discovered peers
            let peers = cluster.peers();
            for peer in &peers {
                // Try to connect if not already connected
                if peer_manager.get(&peer.node_id).is_none() {
                    info!("Connecting to follower {} at {}", peer.node_id, peer.address);
                    if let Err(e) = peer_manager.connect(&peer.node_id, &peer.address) {
                        warn!("Failed to connect to {}: {}", peer.node_id, e);
                    }
                }
            }
            
            let version = cluster.increment_index_version();
            let msg = Message::IndexUpdate(IndexUpdateMsg {
                version,
                operation,
            });
            
            info!("Broadcasting IndexUpdate (version {}) to {} followers", version, peers.len());
            peer_manager.broadcast(&msg);
        }
    }
    
    /// Broadcast a complete file with chunk data to followers (for writes)
    fn broadcast_file_sync(&self, path: &std::path::Path, entry: &FileEntry) {
        if !self.is_leader() {
            return;
        }
        
        if let (Some(cluster), Some(peer_manager)) = (&self.cluster, &self.peer_manager) {
            // First, ensure we're connected to all discovered peers
            let peers = cluster.peers();
            for peer in &peers {
                if peer_manager.get(&peer.node_id).is_none() {
                    info!("Connecting to follower {} at {}", peer.node_id, peer.address);
                    if let Err(e) = peer_manager.connect(&peer.node_id, &peer.address) {
                        warn!("Failed to connect to {}: {}", peer.node_id, e);
                    }
                }
            }
            
            // Read all chunk data
            let mut chunk_data = Vec::new();
            for chunk in &entry.chunks {
                match self.chunk_store.get(&chunk.hash) {
                    Ok(data) => {
                        chunk_data.push(ChunkWithData {
                            hash: chunk.hash,
                            data,
                        });
                    }
                    Err(e) => {
                        warn!("Failed to read chunk for sync: {}", e);
                    }
                }
            }
            
            let modified_ms = entry.modified
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            
            let msg = Message::FileSync(FileSyncMsg {
                path: path.to_string_lossy().to_string(),
                is_dir: entry.is_dir,
                size: entry.size,
                permissions: entry.permissions,
                uid: entry.uid,
                gid: entry.gid,
                modified_ms,
                chunks: entry.chunks.iter().map(|c| ChunkRefMsg {
                    hash: c.hash,
                    offset: c.offset,
                    size: c.size,
                }).collect(),
                chunk_data,
            });
            
            info!("Broadcasting FileSync for {} ({} bytes, {} chunks) to {} followers", 
                path.display(), entry.size, entry.chunks.len(), peers.len());
            peer_manager.broadcast(&msg);
        }
    }

    /// Forward a directory deletion to the leader
    fn forward_rmdir_to_leader(&self, path: &str) -> std::result::Result<(), i32> {
        let (cluster, peer_manager) = match (&self.cluster, &self.peer_manager) {
            (Some(c), Some(p)) => (c, p),
            _ => return Err(libc::EIO),
        };

        let leader_id = cluster.leader_id().ok_or(libc::ENOENT)?;
        let leader_addr = cluster.leader_address().ok_or(libc::ENOENT)?;

        let conn = peer_manager.get_or_connect_leader(&leader_id, &leader_addr)
            .map_err(|_| libc::EIO)?;

        let msg = Message::DeleteDir(DeleteDirMsg {
            path: path.to_string(),
        });

        let response = conn.request(&msg).map_err(|_| libc::EIO)?;

        match response {
            Message::FileOpResponse(resp) if resp.success => Ok(()),
            _ => Err(libc::EIO),
        }
    }

    /// Forward a file rename to the leader
    fn forward_rename_to_leader(&self, from_path: &str, to_path: &str) -> std::result::Result<(), i32> {
        let (cluster, peer_manager) = match (&self.cluster, &self.peer_manager) {
            (Some(c), Some(p)) => (c, p),
            _ => return Err(libc::EIO),
        };

        let leader_id = cluster.leader_id().ok_or(libc::ENOENT)?;
        let leader_addr = cluster.leader_address().ok_or(libc::ENOENT)?;

        let conn = peer_manager.get_or_connect_leader(&leader_id, &leader_addr)
            .map_err(|_| libc::EIO)?;

        let msg = Message::RenameFile(RenameFileMsg {
            from_path: from_path.to_string(),
            to_path: to_path.to_string(),
        });

        let response = conn.request(&msg).map_err(|_| libc::EIO)?;

        match response {
            Message::FileOpResponse(resp) if resp.success => Ok(()),
            _ => Err(libc::EIO),
        }
    }

    /// Forward a symlink creation to the leader
    fn forward_symlink_to_leader(&self, link_path: &str, target: &str) -> std::result::Result<(), i32> {
        let (cluster, peer_manager) = match (&self.cluster, &self.peer_manager) {
            (Some(c), Some(p)) => (c, p),
            _ => return Err(libc::EIO),
        };

        let leader_id = cluster.leader_id().ok_or(libc::ENOENT)?;
        let leader_addr = cluster.leader_address().ok_or(libc::ENOENT)?;

        let conn = peer_manager.get_or_connect_leader(&leader_id, &leader_addr)
            .map_err(|_| libc::EIO)?;

        let msg = Message::CreateSymlink(CreateSymlinkMsg {
            link_path: link_path.to_string(),
            target: target.to_string(),
        });

        let response = conn.request(&msg).map_err(|_| libc::EIO)?;

        match response {
            Message::FileOpResponse(resp) if resp.success => Ok(()),
            _ => Err(libc::EIO),
        }
    }

    /// Allocate a new inode
    fn allocate_inode(&self) -> u64 {
        let mut next = self.next_inode.write().unwrap();
        let inode = *next;
        *next += 1;
        inode
    }

    /// Allocate a new file handle
    fn allocate_fh(&self) -> u64 {
        let mut next = self.next_fh.write().unwrap();
        let fh = *next;
        *next += 1;
        fh
    }

    /// Get root directory attributes
    fn root_attr(&self) -> FileAttr {
        FileAttr {
            ino: ROOT_INODE,
            size: 0,
            blocks: 0,
            atime: SystemTime::now(),
            mtime: SystemTime::now(),
            ctime: SystemTime::now(),
            crtime: SystemTime::now(),
            kind: FileType::Directory,
            perm: 0o755,
            nlink: 2,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    /// Convert FileEntry to FileAttr
    fn entry_to_attr(&self, entry: &FileEntry, inode: u64) -> FileAttr {
        FileAttr {
            ino: inode,
            size: entry.size,
            blocks: (entry.size + 511) / 512,
            atime: entry.accessed,
            mtime: entry.modified,
            ctime: entry.modified,
            crtime: entry.created,
            kind: if entry.is_dir { FileType::Directory } else { FileType::RegularFile },
            perm: entry.permissions as u16,
            nlink: if entry.is_dir { 2 } else { 1 },
            uid: entry.uid,
            gid: entry.gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }
}

impl Filesystem for WolfDiskFS {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let name_str = name.to_string_lossy();
        debug!("lookup: parent={}, name={}", parent, name_str);

        let inode_table = self.inode_table.read().unwrap();
        let file_index = self.file_index.read().unwrap();

        // Get parent path
        let parent_path = match inode_table.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        // Build child path
        let child_path = if parent_path.as_os_str().is_empty() || parent_path == std::path::Path::new("/") {
            std::path::PathBuf::from(name)
        } else {
            parent_path.join(name)
        };

        // Look up in index
        if let Some(entry) = file_index.get(&child_path) {
            if let Some(inode) = inode_table.get_inode(&child_path) {
                let attr = self.entry_to_attr(entry, inode);
                reply.entry(&TTL, &attr, 0);
                return;
            }
        }

        reply.error(libc::ENOENT);
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        debug!("getattr: ino={}", ino);

        if ino == ROOT_INODE {
            reply.attr(&TTL, &self.root_attr());
            return;
        }

        let inode_table = self.inode_table.read().unwrap();
        let file_index = self.file_index.read().unwrap();

        if let Some(path) = inode_table.get_path(ino) {
            if let Some(entry) = file_index.get(path) {
                let attr = self.entry_to_attr(entry, ino);
                reply.attr(&TTL, &attr);
                return;
            }
        }

        reply.error(libc::ENOENT);
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        debug!("read: ino={}, offset={}, size={}", ino, offset, size);

        let inode_table = self.inode_table.read().unwrap();
        let file_index = self.file_index.read().unwrap();

        let path = match inode_table.get_path(ino) {
            Some(p) => p.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let entry = match file_index.get(&path) {
            Some(e) => e.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        drop(file_index);
        drop(inode_table);

        // Read data from chunks
        match self.chunk_store.read(&entry.chunks, offset as u64, size as usize) {
            Ok(data) => reply.data(&data),
            Err(e) => {
                warn!("Read error: {}", e);
                reply.error(e.to_errno());
            }
        }
    }

    fn write(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        debug!("write: ino={}, offset={}, size={}", ino, offset, data.len());

        // Get the path for this inode
        let path = {
            let inode_table = self.inode_table.read().unwrap();
            match inode_table.get_path(ino) {
                Some(p) => p.clone(),
                None => {
                    reply.error(libc::ENOENT);
                    return;
                }
            }
        };

        // If we're a follower, forward to leader
        if !self.is_leader() {
            match self.forward_write_to_leader(&path.to_string_lossy(), offset as u64, data) {
                Ok(written) => {
                    reply.written(written);
                }
                Err(errno) => {
                    reply.error(errno);
                }
            }
            return;
        }

        // We're the leader - write locally
        let mut file_index = self.file_index.write().unwrap();

        let entry = match file_index.get_mut(&path) {
            Some(e) => e,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        // Write data to chunks
        match self.chunk_store.write(&mut entry.chunks, offset as u64, data) {
            Ok(written) => {
                // Update file size if needed
                let new_end = offset as u64 + written as u64;
                if new_end > entry.size {
                    entry.size = new_end;
                }
                entry.modified = SystemTime::now();
                
                // Clone entry for broadcast (need to do this before reply)
                let entry_clone = entry.clone();
                let path_clone = path.clone();
                
                // Drop locks before broadcast and reply
                drop(file_index);
                
                // Broadcast file sync to followers
                self.broadcast_file_sync(&path_clone, &entry_clone);
                
                reply.written(written as u32);
            }
            Err(e) => {
                warn!("Write error: {}", e);
                reply.error(e.to_errno());
            }
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        debug!("readdir: ino={}, offset={}", ino, offset);

        let inode_table = self.inode_table.read().unwrap();
        let file_index = self.file_index.read().unwrap();

        // Get directory path
        let dir_path = if ino == ROOT_INODE {
            std::path::PathBuf::new()
        } else {
            match inode_table.get_path(ino) {
                Some(p) => p.clone(),
                None => {
                    reply.error(libc::ENOENT);
                    return;
                }
            }
        };

        let mut entries = vec![
            (ino, FileType::Directory, ".".to_string()),
            (ino, FileType::Directory, "..".to_string()),
        ];

        // Find children
        for (path, entry) in file_index.iter() {
            if let Some(parent) = path.parent() {
                let parent_matches = if ino == ROOT_INODE {
                    parent.as_os_str().is_empty()
                } else {
                    parent == dir_path
                };

                if parent_matches {
                    if let Some(name) = path.file_name() {
                        let child_inode = inode_table.get_inode(path).unwrap_or(0);
                        let file_type = if entry.is_dir {
                            FileType::Directory
                        } else {
                            FileType::RegularFile
                        };
                        entries.push((child_inode, file_type, name.to_string_lossy().to_string()));
                    }
                }
            }
        }

        // Return entries starting from offset
        for (i, (inode, file_type, name)) in entries.iter().enumerate().skip(offset as usize) {
            if reply.add(*inode, (i + 1) as i64, *file_type, name) {
                break;
            }
        }

        reply.ok();
    }

    fn mkdir(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let name_str = name.to_string_lossy();
        debug!("mkdir: parent={}, name={}, mode={:o}", parent, name_str, mode);

        // Get parent path first (needed for forwarding)
        let parent_path = {
            let inode_table = self.inode_table.read().unwrap();
            if parent == ROOT_INODE {
                std::path::PathBuf::new()
            } else {
                match inode_table.get_path(parent) {
                    Some(p) => p.clone(),
                    None => {
                        reply.error(libc::ENOENT);
                        return;
                    }
                }
            }
        };

        let dir_path = parent_path.join(name);

        // If not leader, forward to leader
        if !self.is_leader() {
            info!("Forwarding mkdir to leader: {:?}", dir_path);
            match self.forward_mkdir_to_leader(&dir_path.to_string_lossy(), mode, req.uid(), req.gid()) {
                Ok(()) => {
                    // Create local entry to reflect the change (will be synced properly later)
                    let now = SystemTime::now();
                    let entry = FileEntry {
                        size: 0,
                        is_dir: true,
                        permissions: mode,
                        uid: req.uid(),
                        gid: req.gid(),
                        created: now,
                        modified: now,
                        accessed: now,
                        chunks: Vec::new(),
                        symlink_target: None,
                    };
                    let inode = self.allocate_inode();
                    self.inode_table.write().unwrap().insert(inode, dir_path.clone());
                    self.file_index.write().unwrap().insert(dir_path, entry.clone());
                    let attr = self.entry_to_attr(&entry, inode);
                    reply.entry(&TTL, &attr, 0);
                }
                Err(errno) => reply.error(errno),
            }
            return;
        }

        // Leader: execute locally
        let mut inode_table = self.inode_table.write().unwrap();
        let mut file_index = self.file_index.write().unwrap();

        // Check if already exists
        if file_index.contains(&dir_path) {
            reply.error(libc::EEXIST);
            return;
        }

        // Create entry
        let now = SystemTime::now();
        let entry = FileEntry {
            size: 0,
            is_dir: true,
            permissions: mode,
            uid: req.uid(),
            gid: req.gid(),
            created: now,
            modified: now,
            accessed: now,
            chunks: Vec::new(),
            symlink_target: None,
        };

        // Allocate inode and add to tables
        let inode = self.allocate_inode();
        let dir_path_str = dir_path.to_string_lossy().to_string();
        inode_table.insert(inode, dir_path.clone());
        file_index.insert(dir_path, entry.clone());
        
        // Drop locks before broadcast
        drop(inode_table);
        drop(file_index);

        let attr = self.entry_to_attr(&entry, inode);
        
        // Broadcast mkdir to followers
        self.broadcast_index_update(IndexOperation::Mkdir {
            path: dir_path_str,
            permissions: mode,
        });
        
        reply.entry(&TTL, &attr, 0);
    }

    fn create(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        _flags: i32,
        reply: fuser::ReplyCreate,
    ) {
        let name_str = name.to_string_lossy();
        debug!("create: parent={}, name={}, mode={:o}", parent, name_str, mode);

        // Get parent path first (needed for forwarding)
        let parent_path = {
            let inode_table = self.inode_table.read().unwrap();
            if parent == ROOT_INODE {
                std::path::PathBuf::new()
            } else {
                match inode_table.get_path(parent) {
                    Some(p) => p.clone(),
                    None => {
                        reply.error(libc::ENOENT);
                        return;
                    }
                }
            }
        };

        let file_path = parent_path.join(name);

        // If not leader, forward to leader
        if !self.is_leader() {
            info!("Forwarding create to leader: {:?}", file_path);
            match self.forward_create_to_leader(&file_path.to_string_lossy(), mode, req.uid(), req.gid()) {
                Ok(()) => {
                    // Create local entry to reflect the change
                    let now = SystemTime::now();
                    let entry = FileEntry {
                        size: 0,
                        is_dir: false,
                        permissions: mode,
                        uid: req.uid(),
                        gid: req.gid(),
                        created: now,
                        modified: now,
                        accessed: now,
                        chunks: Vec::new(),
                        symlink_target: None,
                    };
                    let inode = self.allocate_inode();
                    self.inode_table.write().unwrap().insert(inode, file_path.clone());
                    self.file_index.write().unwrap().insert(file_path, entry.clone());
                    let fh = self.allocate_fh();
                    self.open_files.write().unwrap().insert(fh, inode);
                    let attr = self.entry_to_attr(&entry, inode);
                    reply.created(&TTL, &attr, 0, fh, 0);
                }
                Err(errno) => reply.error(errno),
            }
            return;
        }

        // Leader: execute locally
        let mut inode_table = self.inode_table.write().unwrap();
        let mut file_index = self.file_index.write().unwrap();

        // Check if already exists
        if file_index.contains(&file_path) {
            reply.error(libc::EEXIST);
            return;
        }

        // Create entry
        let now = SystemTime::now();
        let entry = FileEntry {
            size: 0,
            is_dir: false,
            permissions: mode,
            uid: req.uid(),
            gid: req.gid(),
            created: now,
            modified: now,
            accessed: now,
            chunks: Vec::new(),
            symlink_target: None,
        };

        // Allocate inode and add to tables
        let inode = self.allocate_inode();
        let file_path_str = file_path.to_string_lossy().to_string();
        inode_table.insert(inode, file_path.clone());
        file_index.insert(file_path, entry.clone());
        
        // Drop locks before broadcast
        drop(inode_table);
        drop(file_index);

        // Allocate file handle
        let fh = self.allocate_fh();
        self.open_files.write().unwrap().insert(fh, inode);

        let attr = self.entry_to_attr(&entry, inode);
        
        // Broadcast file creation to followers
        self.broadcast_index_update(IndexOperation::Upsert {
            path: file_path_str,
            size: 0,
            modified_ms: now.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
            permissions: mode,
            chunks: vec![],
        });
        
        reply.created(&TTL, &attr, 0, fh, 0);
    }

    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        let name_str = name.to_string_lossy();
        debug!("unlink: parent={}, name={}", parent, name_str);

        // Get parent path first (needed for forwarding)
        let parent_path = {
            let inode_table = self.inode_table.read().unwrap();
            if parent == ROOT_INODE {
                std::path::PathBuf::new()
            } else {
                match inode_table.get_path(parent) {
                    Some(p) => p.clone(),
                    None => {
                        reply.error(libc::ENOENT);
                        return;
                    }
                }
            }
        };

        let file_path = parent_path.join(name);

        // If not leader, forward to leader
        if !self.is_leader() {
            info!("Forwarding unlink to leader: {:?}", file_path);
            match self.forward_unlink_to_leader(&file_path.to_string_lossy()) {
                Ok(()) => {
                    // Remove local entry
                    let mut inode_table = self.inode_table.write().unwrap();
                    let mut file_index = self.file_index.write().unwrap();
                    if let Some(entry) = file_index.remove(&file_path) {
                        for chunk in &entry.chunks {
                            let _ = self.chunk_store.delete(&chunk.hash);
                        }
                    }
                    inode_table.remove_path(&file_path);
                    reply.ok();
                }
                Err(errno) => reply.error(errno),
            }
            return;
        }

        // Leader: execute locally
        let mut inode_table = self.inode_table.write().unwrap();
        let mut file_index = self.file_index.write().unwrap();

        // Check exists and is not a directory
        match file_index.get(&file_path) {
            Some(entry) if entry.is_dir => {
                reply.error(libc::EISDIR);
                return;
            }
            None => {
                reply.error(libc::ENOENT);
                return;
            }
            _ => {}
        }

        // Remove from index and inode table
        if let Some(entry) = file_index.remove(&file_path) {
            // Delete chunks
            for chunk in &entry.chunks {
                let _ = self.chunk_store.delete(&chunk.hash);
            }
        }
        inode_table.remove_path(&file_path);
        
        // Drop locks before broadcast
        drop(file_index);
        drop(inode_table);

        // Broadcast delete to followers
        self.broadcast_index_update(IndexOperation::Delete {
            path: file_path.to_string_lossy().to_string(),
        });

        reply.ok();
    }

    fn rmdir(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        let name_str = name.to_string_lossy();
        debug!("rmdir: parent={}, name={}", parent, name_str);

        // Get parent path first (needed for forwarding)
        let parent_path = {
            let inode_table = self.inode_table.read().unwrap();
            if parent == ROOT_INODE {
                std::path::PathBuf::new()
            } else {
                match inode_table.get_path(parent) {
                    Some(p) => p.clone(),
                    None => {
                        reply.error(libc::ENOENT);
                        return;
                    }
                }
            }
        };

        let dir_path = parent_path.join(name);

        // If not leader, forward to leader
        if !self.is_leader() {
            info!("Forwarding rmdir to leader: {:?}", dir_path);
            match self.forward_rmdir_to_leader(&dir_path.to_string_lossy()) {
                Ok(()) => {
                    // Remove local entry
                    let mut inode_table = self.inode_table.write().unwrap();
                    let mut file_index = self.file_index.write().unwrap();
                    file_index.remove(&dir_path);
                    inode_table.remove_path(&dir_path);
                    reply.ok();
                }
                Err(errno) => reply.error(errno),
            }
            return;
        }

        // Leader: execute locally
        let mut inode_table = self.inode_table.write().unwrap();
        let mut file_index = self.file_index.write().unwrap();

        // Check exists and is a directory
        match file_index.get(&dir_path) {
            Some(entry) if !entry.is_dir => {
                reply.error(libc::ENOTDIR);
                return;
            }
            None => {
                reply.error(libc::ENOENT);
                return;
            }
            _ => {}
        }

        // Check directory is empty
        for path in file_index.paths() {
            if let Some(parent) = path.parent() {
                if parent == dir_path {
                    reply.error(libc::ENOTEMPTY);
                    return;
                }
            }
        }

        // Remove from index and inode table
        file_index.remove(&dir_path);
        inode_table.remove_path(&dir_path);
        
        // Drop locks before broadcast
        drop(file_index);
        drop(inode_table);

        // Broadcast delete to followers
        self.broadcast_index_update(IndexOperation::Delete {
            path: dir_path.to_string_lossy().to_string(),
        });

        reply.ok();
    }

    fn rename(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        _flags: u32,
        reply: fuser::ReplyEmpty,
    ) {
        let name_str = name.to_string_lossy();
        let newname_str = newname.to_string_lossy();
        debug!("rename: parent={}, name={}, newparent={}, newname={}", parent, name_str, newparent, newname_str);

        // Get source and destination paths
        let (from_path, to_path) = {
            let inode_table = self.inode_table.read().unwrap();
            
            let parent_path = if parent == ROOT_INODE {
                std::path::PathBuf::new()
            } else {
                match inode_table.get_path(parent) {
                    Some(p) => p.clone(),
                    None => {
                        reply.error(libc::ENOENT);
                        return;
                    }
                }
            };
            
            let newparent_path = if newparent == ROOT_INODE {
                std::path::PathBuf::new()
            } else {
                match inode_table.get_path(newparent) {
                    Some(p) => p.clone(),
                    None => {
                        reply.error(libc::ENOENT);
                        return;
                    }
                }
            };
            
            (parent_path.join(name), newparent_path.join(newname))
        };

        // If not leader, forward to leader
        if !self.is_leader() {
            info!("Forwarding rename to leader: {:?} -> {:?}", from_path, to_path);
            match self.forward_rename_to_leader(&from_path.to_string_lossy(), &to_path.to_string_lossy()) {
                Ok(()) => {
                    // Update local entries
                    let mut inode_table = self.inode_table.write().unwrap();
                    let mut file_index = self.file_index.write().unwrap();
                    
                    if let Some(entry) = file_index.remove(&from_path) {
                        file_index.insert(to_path.clone(), entry);
                    }
                    
                    if let Some(ino) = inode_table.get_inode(&from_path) {
                        let ino = ino;
                        inode_table.remove_path(&from_path);
                        inode_table.insert(ino, to_path);
                    }
                    
                    reply.ok();
                }
                Err(errno) => reply.error(errno),
            }
            return;
        }

        // Leader: execute locally
        let mut inode_table = self.inode_table.write().unwrap();
        let mut file_index = self.file_index.write().unwrap();

        // Check source exists
        let entry = match file_index.get(&from_path) {
            Some(e) => e.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        // Check destination doesn't exist
        if file_index.contains(&to_path) {
            reply.error(libc::EEXIST);
            return;
        }

        // Move entry
        file_index.remove(&from_path);
        file_index.insert(to_path.clone(), entry);

        // Update inode table
        if let Some(ino) = inode_table.get_inode(&from_path) {
            let ino = ino;
            inode_table.remove_path(&from_path);
            inode_table.insert(ino, to_path.clone());
        }

        drop(file_index);
        drop(inode_table);

        // Broadcast rename to followers
        self.broadcast_index_update(IndexOperation::Rename {
            from_path: from_path.to_string_lossy().to_string(),
            to_path: to_path.to_string_lossy().to_string(),
        });

        reply.ok();
    }

    fn open(&mut self, _req: &Request, ino: u64, _flags: i32, reply: ReplyOpen) {
        debug!("open: ino={}", ino);

        // Verify file exists
        let inode_table = self.inode_table.read().unwrap();
        if inode_table.get_path(ino).is_none() && ino != ROOT_INODE {
            reply.error(libc::ENOENT);
            return;
        }

        let fh = self.allocate_fh();
        self.open_files.write().unwrap().insert(fh, ino);
        reply.opened(fh, 0);
    }

    fn release(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: fuser::ReplyEmpty,
    ) {
        debug!("release: fh={}", fh);
        self.open_files.write().unwrap().remove(&fh);
        
        // Save index to persist changes
        if let Ok(index) = self.file_index.read() {
            let _ = index.save(&self.config.index_dir());
        }
        
        reply.ok();
    }
}
