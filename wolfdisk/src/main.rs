//! WolfDisk CLI
//!
//! Command-line interface for mounting and managing WolfDisk.

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::{info, error, debug};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use wolfdisk::{Config, fuse::WolfDiskFS, storage::{FileIndex, FileEntry, ChunkRef}};

#[derive(Parser)]
#[command(name = "wolfdisk")]
#[command(author = "Wolf Software Systems Ltd")]
#[command(version = "2.2.2")]
#[command(about = "Distributed file system with replicated and shared storage", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Path to configuration file
    #[arg(short, long, default_value = "/etc/wolfdisk/config.toml")]
    config: PathBuf,

    /// Enable debug logging
    #[arg(short, long)]
    debug: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Mount the WolfDisk filesystem
    Mount {
        /// Mount point path
        #[arg(short, long)]
        mountpoint: PathBuf,
    },

    /// Unmount the WolfDisk filesystem
    Unmount {
        /// Mount point path
        #[arg(short, long)]
        mountpoint: PathBuf,
    },

    /// Show cluster status
    Status,

    /// Live cluster statistics (refreshes every second)
    Stats,

    /// List all discovered servers in the cluster
    #[command(name = "list")]
    ListServers {
        /// What to list (servers)  
        #[arg(default_value = "servers")]
        what: String,
    },

    /// Initialize a new WolfDisk data directory
    Init {
        /// Data directory path
        #[arg(short, long, default_value = "/var/lib/wolfdisk")]
        data_dir: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    // Initialize logging
    let filter = if cli.debug { "debug" } else { "info" };
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_subscriber::EnvFilter::new(filter))
        .init();

    // Load config if it exists
    let config = if cli.config.exists() {
        match Config::load(&cli.config) {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to load config: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        info!("No config file found, using defaults");
        Config::default()
    };

    match cli.command {
        Commands::Mount { mountpoint } => {
            info!("Mounting WolfDisk at {:?}", mountpoint);
            info!("Node ID: {}, Role: {:?}", config.node.id, config.node.role);
            
            // Initialize cluster manager
            let mut cluster = wolfdisk::ClusterManager::new(config.clone());
            if let Err(e) = cluster.start() {
                error!("Failed to start cluster manager: {}", e);
                std::process::exit(1);
            }
            
            info!("Cluster state: {:?}", cluster.state());
            
            // Wrap cluster in Arc for sharing with filesystem
            let cluster = std::sync::Arc::new(cluster);
            
            // Create shared state for filesystem that can be accessed by message handler
            std::fs::create_dir_all(config.chunks_dir()).ok();
            std::fs::create_dir_all(config.index_dir()).ok();
            
            let file_index = std::sync::Arc::new(std::sync::RwLock::new(
                FileIndex::load_or_create(&config.index_dir())
                    .expect("Failed to load file index")
            ));
            let file_index_for_handler = file_index.clone();
            
            // Create chunk store for replication (shared with WolfDiskFS)
            let chunk_store = std::sync::Arc::new(
                wolfdisk::storage::ChunkStore::new(config.chunks_dir(), 4 * 1024 * 1024)
                    .expect("Failed to create chunk store")
            );
            let chunk_store_for_handler = chunk_store.clone();
            
            // Build inode table from index (shared with WolfDiskFS)
            let (inode_table_data, max_inode) = {
                let index = file_index.read().unwrap();
                wolfdisk::storage::InodeTable::from_index(&index)
            };
            let inode_table = std::sync::Arc::new(std::sync::RwLock::new(inode_table_data));
            let next_inode = std::sync::Arc::new(std::sync::RwLock::new(max_inode + 1));
            let inode_table_for_handler = inode_table.clone();
            let next_inode_for_handler = next_inode.clone();
            
            // Broadcast queue for message handler to queue FileSync broadcasts
            // (path, entry) tuples that need to be broadcast to followers
            let broadcast_queue: std::sync::Arc<std::sync::Mutex<Vec<(std::path::PathBuf, wolfdisk::storage::FileEntry)>>> = 
                std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let broadcast_queue_for_handler = broadcast_queue.clone();
            
            // Chunk streaming queue for streaming individual chunks to followers
            // (hash, data) tuples that are sent via StoreChunk messages
            let chunk_stream_queue: std::sync::Arc<std::sync::Mutex<Vec<([u8; 32], Vec<u8>)>>> =
                std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let chunk_stream_queue_for_handler = chunk_stream_queue.clone();
            
            // Metadata update queue for sending metadata-only FileSync to followers
            // during streaming replication (no chunk_data, just path + entry metadata)
            let metadata_update_queue: std::sync::Arc<std::sync::Mutex<Vec<(std::path::PathBuf, wolfdisk::storage::FileEntry)>>> =
                std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let metadata_update_queue_for_handler = metadata_update_queue.clone();
            
            // Track if this node is a client (clients don't store chunk data locally)
            let is_client_role = config.node.role == wolfdisk::config::NodeRole::Client;
            
            // Create peer manager for network communication
            let peer_manager = std::sync::Arc::new(
                wolfdisk::network::peer::PeerManager::new(
                    config.node.id.clone(),
                    config.node.bind.clone(),
                    move |peer_id, msg| {
                        use wolfdisk::network::protocol::*;
                        
                        match msg {
                            Message::StoreChunk(store_chunk) => {
                                // Handle streamed chunk from leader (streaming replication)
                                if !is_client_role {
                                    debug!("Received streamed chunk {} from leader", hex::encode(&store_chunk.hash));
                                    if let Err(e) = chunk_store_for_handler.store_with_hash(&store_chunk.hash, &store_chunk.data) {
                                        tracing::warn!("Failed to store streamed chunk: {}", e);
                                    }
                                }
                                None // No response needed
                            }
                            Message::IndexUpdate(update) => {
                                info!("Received IndexUpdate from {}: {:?}", peer_id, update.operation);
                                
                                // Apply the update to our local index
                                let mut index = file_index_for_handler.write().unwrap();
                                match update.operation {
                                    IndexOperation::Delete { path } => {
                                        info!("Replicating delete: {}", path);
                                        let del_path = std::path::PathBuf::from(&path);
                                        if let Some(entry) = index.remove(&del_path) {
                                            // Delete chunks from disk
                                            if !is_client_role {
                                                for chunk in &entry.chunks {
                                                    let _ = chunk_store_for_handler.delete(&chunk.hash);
                                                }
                                            }
                                            // Remove from inode table
                                            let mut inode_tbl = inode_table_for_handler.write().unwrap();
                                            inode_tbl.remove_path(&del_path);
                                            info!("Deleted file and {} chunks from follower: {}", entry.chunks.len(), path);
                                        }
                                    }
                                    IndexOperation::Upsert { path, size, modified_ms, permissions, chunks } => {
                                        info!("Replicating upsert: {} ({} bytes)", path, size);
                                        let chunk_refs: Vec<ChunkRef> = chunks.iter()
                                            .map(|c| ChunkRef {
                                                hash: c.hash,
                                                offset: c.offset,
                                                size: c.size,
                                            })
                                            .collect();
                                        let now = std::time::SystemTime::now();
                                        index.insert(std::path::PathBuf::from(&path), FileEntry {
                                            size,
                                            modified: std::time::UNIX_EPOCH + std::time::Duration::from_millis(modified_ms),
                                            permissions,
                                            is_dir: false,
                                            chunks: chunk_refs,
                                            uid: 0,
                                            gid: 0,
                                            created: now,
                                            accessed: now,
                                            symlink_target: None,
                                        });
                                    }
                                    IndexOperation::Mkdir { path, permissions } => {
                                        info!("Replicating mkdir: {}", path);
                                        let now = std::time::SystemTime::now();
                                        index.insert(std::path::PathBuf::from(&path), FileEntry {
                                            size: 0,
                                            modified: now,
                                            permissions,
                                            is_dir: true,
                                            chunks: vec![],
                                            uid: 0,
                                            gid: 0,
                                            created: now,
                                            accessed: now,
                                            symlink_target: None,
                                        });
                                    }
                                    IndexOperation::Rename { from_path, to_path } => {
                                        info!("Replicating rename: {} -> {}", from_path, to_path);
                                        if let Some(entry) = index.remove(&std::path::PathBuf::from(&from_path)) {
                                            index.insert(std::path::PathBuf::from(&to_path), entry);
                                        }
                                    }
                                }
                                
                                None // No response needed for replication
                            }
                            Message::FileSync(sync) => {
                                // Check if this is a deletion signal (size == u64::MAX)
                                if sync.size == u64::MAX {
                                    info!("Received FileDelete from {}: {}", peer_id, sync.path);
                                    
                                    let path = std::path::PathBuf::from(&sync.path);
                                    let mut index = file_index_for_handler.write().unwrap();
                                    
                                    if let Some(entry) = index.remove(&path) {
                                        // Delete chunks
                                        for chunk in &entry.chunks {
                                            let _ = chunk_store_for_handler.delete(&chunk.hash);
                                        }
                                        
                                        // Remove from inode table
                                        let mut inode_tbl = inode_table_for_handler.write().unwrap();
                                        inode_tbl.remove_path(&path);
                                        
                                        info!("Deleted file from follower: {}", sync.path);
                                    }
                                    return None;
                                }
                                
                                info!("Received FileSync from {}: {} ({} bytes, {} chunk_refs, {} chunk_data)", 
                                    peer_id, sync.path, sync.size, sync.chunks.len(), sync.chunk_data.len());
                                
                                // Store chunks locally (skip for client nodes - they read from leader)
                                if !is_client_role {
                                    for chunk_with_data in &sync.chunk_data {
                                        if let Err(e) = chunk_store_for_handler.store_with_hash(&chunk_with_data.hash, &chunk_with_data.data) {
                                            tracing::warn!("Failed to store chunk: {}", e);
                                        }
                                    }
                                }
                                
                                // Update index
                                let mut index = file_index_for_handler.write().unwrap();
                                let path = std::path::PathBuf::from(&sync.path);
                                
                                // If the incoming message has chunk_refs, use them (authoritative metadata).
                                // If chunk_refs is empty but we have chunk_data, this is a subsequent batch
                                // of a multi-batch transfer — only store the chunks, keep existing entry.
                                if !sync.chunks.is_empty() {
                                    // Authoritative update: replace the full file entry
                                    let chunk_refs: Vec<ChunkRef> = sync.chunks.iter()
                                        .map(|c| ChunkRef {
                                            hash: c.hash,
                                            offset: c.offset,
                                            size: c.size,
                                        })
                                        .collect();
                                    
                                    index.insert(path.clone(), FileEntry {
                                        size: sync.size,
                                        is_dir: sync.is_dir,
                                        permissions: sync.permissions,
                                        uid: sync.uid,
                                        gid: sync.gid,
                                        modified: std::time::UNIX_EPOCH + std::time::Duration::from_millis(sync.modified_ms),
                                        created: std::time::UNIX_EPOCH + std::time::Duration::from_millis(sync.modified_ms),
                                        accessed: std::time::SystemTime::now(),
                                        chunks: chunk_refs,
                                        symlink_target: None,
                                    });
                                } else if !sync.chunk_data.is_empty() {
                                    // Subsequent batch: only storing chunk data, keep existing index entry.
                                    // Update size if it has grown.
                                    if let Some(entry) = index.get_mut(&path) {
                                        if sync.size > entry.size {
                                            entry.size = sync.size;
                                        }
                                        entry.modified = std::time::UNIX_EPOCH + std::time::Duration::from_millis(sync.modified_ms);
                                    }
                                    info!("Stored {} additional chunks for {} (batch continuation)", sync.chunk_data.len(), sync.path);
                                } else {
                                    // Metadata-only update (streaming replication final sync)
                                    // The chunks were already streamed via StoreChunk messages.
                                    // Update the index entry with final metadata + chunk refs.
                                    let chunk_refs: Vec<ChunkRef> = Vec::new();
                                    // If we already have an entry, update it; otherwise create new.
                                    if let Some(entry) = index.get_mut(&path) {
                                        entry.size = sync.size;
                                        entry.permissions = sync.permissions;
                                        entry.uid = sync.uid;
                                        entry.gid = sync.gid;
                                        entry.modified = std::time::UNIX_EPOCH + std::time::Duration::from_millis(sync.modified_ms);
                                    } else {
                                        index.insert(path.clone(), FileEntry {
                                            size: sync.size,
                                            is_dir: sync.is_dir,
                                            permissions: sync.permissions,
                                            uid: sync.uid,
                                            gid: sync.gid,
                                            modified: std::time::UNIX_EPOCH + std::time::Duration::from_millis(sync.modified_ms),
                                            created: std::time::UNIX_EPOCH + std::time::Duration::from_millis(sync.modified_ms),
                                            accessed: std::time::SystemTime::now(),
                                            chunks: chunk_refs,
                                            symlink_target: None,
                                        });
                                    }
                                }
                                
                                // Also update inode table so the file can be looked up
                                let mut inode_tbl = inode_table_for_handler.write().unwrap();
                                let mut next_ino = next_inode_for_handler.write().unwrap();
                                if inode_tbl.get_inode(&path).is_none() {
                                    let ino = *next_ino;
                                    *next_ino += 1;
                                    inode_tbl.insert(ino, path.clone());
                                    info!("Added inode {} for replicated path: {}", ino, sync.path);
                                }
                                
                                info!("FileSync complete for {}", sync.path);
                                None
                            }
                            Message::WriteRequest(write_req) => {
                                // Handle write request from follower (we're the leader)
                                info!("Received WriteRequest from {}: {} (offset: {}, size: {})", 
                                    peer_id, write_req.path, write_req.offset, write_req.data.len());
                                
                                // Write data to chunk store
                                let path = std::path::PathBuf::from(&write_req.path);
                                let mut index = file_index_for_handler.write().unwrap();
                                
                                if let Some(entry) = index.get_mut(&path) {
                                    // Track chunks before write to detect new ones
                                    let chunks_before = entry.chunks.len();
                                    
                                    match chunk_store_for_handler.write(&mut entry.chunks, write_req.offset, &write_req.data) {
                                        Ok(written) => {
                                            let new_end = write_req.offset + written as u64;
                                            if new_end > entry.size {
                                                entry.size = new_end;
                                            }
                                            entry.modified = std::time::SystemTime::now();
                                            
                                            // Stream any new chunks to followers immediately
                                            let new_chunks: Vec<_> = entry.chunks[chunks_before..].to_vec();
                                            let entry_clone = entry.clone();
                                            let path_clone = path.clone();
                                            drop(index); // Release lock before streaming
                                            
                                            // Queue new chunks for streaming to followers
                                            if !new_chunks.is_empty() {
                                                let mut stream_queue = chunk_stream_queue_for_handler.lock().unwrap();
                                                for chunk_ref in &new_chunks {
                                                    if let Ok(chunk_data) = chunk_store_for_handler.get(&chunk_ref.hash) {
                                                        stream_queue.push((chunk_ref.hash, chunk_data));
                                                    }
                                                }
                                            }
                                            
                                            // Queue metadata-only update (chunks already streamed above)
                                            metadata_update_queue_for_handler.lock().unwrap().push((path_clone, entry_clone));
                                            
                                            info!("Leader wrote {} bytes to {} ({} new chunks streamed)", written, write_req.path, new_chunks.len());
                                            
                                            Some(Message::ClientResponse(ClientResponseMsg {
                                                success: true,
                                                data: None,
                                                error: None,
                                            }))
                                        }
                                        Err(e) => {
                                            tracing::warn!("Leader write error: {}", e);
                                            Some(Message::ClientResponse(ClientResponseMsg {
                                                success: false,
                                                data: None,
                                                error: Some(format!("Write failed: {}", e)),
                                            }))
                                        }
                                    }
                                } else {
                                    // Log all paths in index for debugging
                                    let paths: Vec<String> = index.iter()
                                        .map(|(p, _)| format!("{:?}", p))
                                        .collect();
                                    tracing::warn!("File not found on leader: {:?} (path string: {})", path, write_req.path);
                                    tracing::warn!("Index contains {} entries: {:?}", paths.len(), paths);
                                    Some(Message::ClientResponse(ClientResponseMsg {
                                        success: false,
                                        data: None,
                                        error: Some("File not found".to_string()),
                                    }))
                                }
                            }
                            Message::CreateFile(create_req) => {
                                // Handle create file request from follower (we're the leader)
                                info!("Received CreateFile from {}: {} (mode: {:o})", 
                                    peer_id, create_req.path, create_req.mode);
                                
                                let path = std::path::PathBuf::from(&create_req.path);
                                let mut index = file_index_for_handler.write().unwrap();
                                
                                if index.get(&path).is_some() {
                                    // File already exists
                                    Some(Message::FileOpResponse(FileOpResponseMsg {
                                        success: false,
                                        error: Some("File already exists".to_string()),
                                    }))
                                } else {
                                    // Create new empty file entry
                                    let entry = FileEntry {
                                        size: 0,
                                        is_dir: false,
                                        permissions: create_req.mode,
                                        uid: create_req.uid,
                                        gid: create_req.gid,
                                        modified: std::time::SystemTime::now(),
                                        created: std::time::SystemTime::now(),
                                        accessed: std::time::SystemTime::now(),
                                        chunks: Vec::new(),
                                        symlink_target: None,
                                    };
                                    
                                    index.insert(path.clone(), entry);
                                    
                                    // Also update inode table
                                    let mut inode_tbl = inode_table_for_handler.write().unwrap();
                                    let mut next_ino = next_inode_for_handler.write().unwrap();
                                    let ino = *next_ino;
                                    *next_ino += 1;
                                    inode_tbl.insert(ino, path.clone());
                                    
                                    info!("Leader created file: {} with inode {}", create_req.path, ino);
                                    
                                    // Queue broadcast to followers
                                    let entry_for_broadcast = index.get(&path).unwrap().clone();
                                    drop(index);
                                    drop(inode_tbl);
                                    drop(next_ino);
                                    broadcast_queue_for_handler.lock().unwrap().push((path, entry_for_broadcast));
                                    
                                    Some(Message::FileOpResponse(FileOpResponseMsg {
                                        success: true,
                                        error: None,
                                    }))
                                }
                            }
                            Message::DeleteFile(del) => {
                                // Handle incoming delete request (if we're leader)
                                info!("Received DeleteFile: {}", del.path);
                                
                                let path = std::path::PathBuf::from(&del.path);
                                let mut index = file_index_for_handler.write().unwrap();
                                
                                if let Some(entry) = index.remove(&path) {
                                    // Delete chunks
                                    for chunk in &entry.chunks {
                                        let _ = chunk_store_for_handler.delete(&chunk.hash);
                                    }
                                    
                                    // Remove from inode table
                                    let mut inode_tbl = inode_table_for_handler.write().unwrap();
                                    inode_tbl.remove_path(&path);
                                    
                                    info!("Leader deleted file: {}", del.path);
                                    
                                    // Queue delete broadcast to followers
                                    // We use a special entry with size u64::MAX to signal deletion
                                    let delete_marker = wolfdisk::storage::FileEntry {
                                        size: u64::MAX, // Signals deletion
                                        is_dir: false,
                                        permissions: 0,
                                        uid: 0,
                                        gid: 0,
                                        modified: std::time::SystemTime::now(),
                                        created: std::time::SystemTime::now(),
                                        accessed: std::time::SystemTime::now(),
                                        chunks: Vec::new(),
                                        symlink_target: None,
                                    };
                                    drop(index);
                                    drop(inode_tbl);
                                    broadcast_queue_for_handler.lock().unwrap().push((path, delete_marker));
                                    
                                    Some(Message::FileOpResponse(FileOpResponseMsg {
                                        success: true,
                                        error: None,
                                    }))
                                } else {
                                    Some(Message::FileOpResponse(FileOpResponseMsg {
                                        success: false,
                                        error: Some("File not found".to_string()),
                                    }))
                                }
                            }
                            Message::CreateDir(dir_req) => {
                                // Handle incoming mkdir request (if we're leader)
                                info!("Received CreateDir: {}", dir_req.path);
                                
                                let path = std::path::PathBuf::from(&dir_req.path);
                                let mut index = file_index_for_handler.write().unwrap();
                                
                                if index.get(&path).is_some() {
                                    // Dir already exists
                                    Some(Message::FileOpResponse(FileOpResponseMsg {
                                        success: false,
                                        error: Some("Directory already exists".to_string()),
                                    }))
                                } else {
                                    // Create new directory entry
                                    let entry = FileEntry {
                                        size: 0,
                                        is_dir: true,
                                        permissions: dir_req.mode,
                                        uid: dir_req.uid,
                                        gid: dir_req.gid,
                                        modified: std::time::SystemTime::now(),
                                        created: std::time::SystemTime::now(),
                                        accessed: std::time::SystemTime::now(),
                                        chunks: Vec::new(),
                                        symlink_target: None,
                                    };
                                    
                                    index.insert(path.clone(), entry.clone());
                                    
                                    // Also update inode table
                                    let mut inode_tbl = inode_table_for_handler.write().unwrap();
                                    let mut next_ino = next_inode_for_handler.write().unwrap();
                                    let ino = *next_ino;
                                    *next_ino += 1;
                                    inode_tbl.insert(ino, path.clone());
                                    
                                    info!("Leader created directory: {} with inode {}", dir_req.path, ino);
                                    
                                    // Queue broadcast to followers
                                    drop(index);
                                    drop(inode_tbl);
                                    drop(next_ino);
                                    broadcast_queue_for_handler.lock().unwrap().push((path, entry));
                                    
                                    Some(Message::FileOpResponse(FileOpResponseMsg {
                                        success: true,
                                        error: None,
                                    }))
                                }
                            }
                            Message::DeleteDir(del) => {
                                // Handle incoming rmdir request (if we're leader)
                                info!("Received DeleteDir: {}", del.path);
                                
                                let path = std::path::PathBuf::from(&del.path);
                                let mut index = file_index_for_handler.write().unwrap();
                                
                                match index.get(&path) {
                                    Some(entry) if !entry.is_dir => {
                                        Some(Message::FileOpResponse(FileOpResponseMsg {
                                            success: false,
                                            error: Some("Not a directory".to_string()),
                                        }))
                                    }
                                    None => {
                                        Some(Message::FileOpResponse(FileOpResponseMsg {
                                            success: false,
                                            error: Some("Directory not found".to_string()),
                                        }))
                                    }
                                    _ => {
                                        // Check directory is empty
                                        let has_children = index.paths().any(|p| {
                                            if let Some(parent) = p.parent() {
                                                parent == path
                                            } else {
                                                false
                                            }
                                        });
                                        
                                        if has_children {
                                            return Some(Message::FileOpResponse(FileOpResponseMsg {
                                                success: false,
                                                error: Some("Directory not empty".to_string()),
                                            }));
                                        }
                                        
                                        index.remove(&path);
                                        
                                        // Remove from inode table
                                        let mut inode_tbl = inode_table_for_handler.write().unwrap();
                                        inode_tbl.remove_path(&path);
                                        
                                        info!("Leader deleted directory: {}", del.path);
                                        
                                        // Queue delete broadcast to followers
                                        let delete_marker = wolfdisk::storage::FileEntry {
                                            size: u64::MAX, // Signals deletion
                                            is_dir: true,
                                            permissions: 0,
                                            uid: 0,
                                            gid: 0,
                                            modified: std::time::SystemTime::now(),
                                            created: std::time::SystemTime::now(),
                                            accessed: std::time::SystemTime::now(),
                                            chunks: Vec::new(),
                                            symlink_target: None,
                                        };
                                        drop(index);
                                        drop(inode_tbl);
                                        broadcast_queue_for_handler.lock().unwrap().push((path, delete_marker));
                                        
                                        Some(Message::FileOpResponse(FileOpResponseMsg {
                                            success: true,
                                            error: None,
                                        }))
                                    }
                                }
                            }
                            Message::RenameFile(rename_req) => {
                                // Handle incoming rename request (if we're leader)
                                info!("Received RenameFile: {} -> {}", rename_req.from_path, rename_req.to_path);
                                
                                let from_path = std::path::PathBuf::from(&rename_req.from_path);
                                let to_path = std::path::PathBuf::from(&rename_req.to_path);
                                let mut index = file_index_for_handler.write().unwrap();
                                
                                // Check source exists
                                let entry = match index.get(&from_path) {
                                    Some(e) => e.clone(),
                                    None => {
                                        return Some(Message::FileOpResponse(FileOpResponseMsg {
                                            success: false,
                                            error: Some("Source not found".to_string()),
                                        }));
                                    }
                                };
                                
                                // Check dest doesn't exist
                                if index.contains(&to_path) {
                                    return Some(Message::FileOpResponse(FileOpResponseMsg {
                                        success: false,
                                        error: Some("Destination already exists".to_string()),
                                    }));
                                }
                                
                                // Move entry in index
                                index.remove(&from_path);
                                index.insert(to_path.clone(), entry.clone());
                                
                                // Update inode table
                                let mut inode_tbl = inode_table_for_handler.write().unwrap();
                                if let Some(ino) = inode_tbl.get_inode(&from_path) {
                                    let ino = ino;
                                    inode_tbl.remove_path(&from_path);
                                    inode_tbl.insert(ino, to_path.clone());
                                }
                                
                                info!("Leader renamed: {} -> {}", rename_req.from_path, rename_req.to_path);
                                
                                // Queue broadcast: delete old path, sync new path
                                let delete_marker = wolfdisk::storage::FileEntry {
                                    size: u64::MAX, // Delete marker
                                    is_dir: entry.is_dir,
                                    permissions: 0,
                                    uid: 0,
                                    gid: 0,
                                    modified: std::time::SystemTime::now(),
                                    created: std::time::SystemTime::now(),
                                    accessed: std::time::SystemTime::now(),
                                    chunks: Vec::new(),
                                    symlink_target: None,
                                };
                                drop(index);
                                drop(inode_tbl);
                                
                                let mut queue = broadcast_queue_for_handler.lock().unwrap();
                                queue.push((from_path, delete_marker));
                                queue.push((to_path, entry));
                                drop(queue);
                                
                                Some(Message::FileOpResponse(FileOpResponseMsg {
                                    success: true,
                                    error: None,
                                }))
                            }
                            Message::CreateSymlink(symlink_req) => {
                                // Handle incoming symlink request (if we're leader)
                                info!("Received CreateSymlink: {} -> {}", symlink_req.link_path, symlink_req.target);
                                
                                let link_path = std::path::PathBuf::from(&symlink_req.link_path);
                                let mut index = file_index_for_handler.write().unwrap();
                                
                                // Check link doesn't exist
                                if index.contains(&link_path) {
                                    return Some(Message::FileOpResponse(FileOpResponseMsg {
                                        success: false,
                                        error: Some("Path already exists".to_string()),
                                    }));
                                }
                                
                                // Create symlink entry
                                let entry = wolfdisk::storage::FileEntry {
                                    size: symlink_req.target.len() as u64,
                                    is_dir: false,
                                    permissions: 0o777,
                                    uid: 0,
                                    gid: 0,
                                    modified: std::time::SystemTime::now(),
                                    created: std::time::SystemTime::now(),
                                    accessed: std::time::SystemTime::now(),
                                    chunks: Vec::new(),
                                    symlink_target: Some(symlink_req.target.clone()),
                                };
                                
                                // Insert into index
                                index.insert(link_path.clone(), entry.clone());
                                
                                // Add to inode table
                                let mut inode_tbl = inode_table_for_handler.write().unwrap();
                                let inode = *next_inode_for_handler.read().unwrap();
                                *next_inode_for_handler.write().unwrap() += 1;
                                inode_tbl.insert(inode, link_path.clone());
                                
                                info!("Leader created symlink: {} -> {}", symlink_req.link_path, symlink_req.target);
                                
                                // Queue broadcast
                                drop(index);
                                drop(inode_tbl);
                                broadcast_queue_for_handler.lock().unwrap().push((link_path, entry));
                                
                                Some(Message::FileOpResponse(FileOpResponseMsg {
                                    success: true,
                                    error: None,
                                }))
                            }
                            Message::SetAttr(setattr_req) => {
                                // Handle setattr request (truncation, chmod, chown, etc.)
                                info!("Received SetAttr from {}: {} (size={:?})", 
                                    peer_id, setattr_req.path, setattr_req.size);
                                
                                let path = std::path::PathBuf::from(&setattr_req.path);
                                let mut index = file_index_for_handler.write().unwrap();
                                
                                if let Some(entry) = index.get_mut(&path) {
                                    // Handle truncation
                                    if let Some(new_size) = setattr_req.size {
                                        if new_size == 0 {
                                            // Full truncation: delete all chunks
                                            for chunk in &entry.chunks {
                                                let _ = chunk_store_for_handler.delete(&chunk.hash);
                                            }
                                            entry.chunks.clear();
                                            entry.size = 0;
                                        } else if new_size < entry.size {
                                            // Partial truncation
                                            entry.chunks.retain(|chunk| chunk.offset < new_size);
                                            entry.size = new_size;
                                        } else {
                                            entry.size = new_size;
                                        }
                                    }
                                    
                                    if let Some(perms) = setattr_req.permissions {
                                        entry.permissions = perms;
                                    }
                                    if let Some(uid) = setattr_req.uid {
                                        entry.uid = uid;
                                    }
                                    if let Some(gid) = setattr_req.gid {
                                        entry.gid = gid;
                                    }
                                    if let Some(mtime_ms) = setattr_req.modified_ms {
                                        entry.modified = std::time::UNIX_EPOCH + std::time::Duration::from_millis(mtime_ms);
                                    }
                                    
                                    info!("Leader applied setattr to {}", setattr_req.path);
                                    
                                    // Queue broadcast to followers
                                    let entry_clone = entry.clone();
                                    drop(index);
                                    broadcast_queue_for_handler.lock().unwrap().push((path, entry_clone));
                                    
                                    Some(Message::FileOpResponse(FileOpResponseMsg {
                                        success: true,
                                        error: None,
                                    }))
                                } else {
                                    Some(Message::FileOpResponse(FileOpResponseMsg {
                                        success: false,
                                        error: Some("File not found".to_string()),
                                    }))
                                }
                            }
                            Message::ReadRequest(read_req) => {
                                // Handle read request from client (reads from local chunks)
                                debug!("Received ReadRequest: {} offset={} size={}", read_req.path, read_req.offset, read_req.size);
                                
                                let file_path = std::path::PathBuf::from(&read_req.path);
                                let index = file_index_for_handler.read().unwrap();
                                
                                match index.get(&file_path) {
                                    Some(entry) => {
                                        let chunks = entry.chunks.clone();
                                        drop(index);
                                        
                                        match chunk_store_for_handler.read(&chunks, read_req.offset, read_req.size as usize) {
                                            Ok(data) => {
                                                Some(Message::ClientResponse(ClientResponseMsg {
                                                    success: true,
                                                    data: Some(data),
                                                    error: None,
                                                }))
                                            }
                                            Err(e) => {
                                                Some(Message::ClientResponse(ClientResponseMsg {
                                                    success: false,
                                                    data: None,
                                                    error: Some(format!("Read error: {}", e)),
                                                }))
                                            }
                                        }
                                    }
                                    None => {
                                        Some(Message::ClientResponse(ClientResponseMsg {
                                            success: false,
                                            data: None,
                                            error: Some("File not found".to_string()),
                                        }))
                                    }
                                }
                            }
                            Message::ReadDir(readdir_req) => {
                                // Handle readdir request from client
                                debug!("Received ReadDir: {}", readdir_req.path);
                                
                                let dir_path = std::path::PathBuf::from(&readdir_req.path);
                                let index = file_index_for_handler.read().unwrap();
                                
                                let mut entries = Vec::new();
                                for (path, entry) in index.iter() {
                                    if let Some(parent) = path.parent() {
                                        if parent == dir_path {
                                            if let Some(name) = path.file_name() {
                                                entries.push(DirEntryMsg {
                                                    name: name.to_string_lossy().to_string(),
                                                    is_dir: entry.is_dir,
                                                });
                                            }
                                        }
                                    } else if dir_path == std::path::PathBuf::new() {
                                        // Root directory entries have no parent
                                        if path.components().count() == 1 {
                                            if let Some(name) = path.file_name() {
                                                entries.push(DirEntryMsg {
                                                    name: name.to_string_lossy().to_string(),
                                                    is_dir: entry.is_dir,
                                                });
                                            }
                                        }
                                    }
                                }
                                
                                Some(Message::ReadDirResponse(ReadDirResponseMsg {
                                    success: true,
                                    entries,
                                    error: None,
                                }))
                            }
                            Message::GetAttr(getattr_req) => {
                                // Handle getattr request from client
                                debug!("Received GetAttr: {}", getattr_req.path);
                                
                                let file_path = std::path::PathBuf::from(&getattr_req.path);
                                let index = file_index_for_handler.read().unwrap();
                                
                                match index.get(&file_path) {
                                    Some(entry) => {
                                        let modified_ms = entry.modified
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_millis() as u64;
                                        let created_ms = entry.created
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_millis() as u64;
                                        let accessed_ms = entry.accessed
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_millis() as u64;
                                        
                                        Some(Message::GetAttrResponse(GetAttrResponseMsg {
                                            exists: true,
                                            is_dir: entry.is_dir,
                                            size: entry.size,
                                            permissions: entry.permissions,
                                            uid: entry.uid,
                                            gid: entry.gid,
                                            modified_ms,
                                            created_ms,
                                            accessed_ms,
                                        }))
                                    }
                                    None => {
                                        Some(Message::GetAttrResponse(GetAttrResponseMsg {
                                            exists: false,
                                            is_dir: false,
                                            size: 0,
                                            permissions: 0,
                                            uid: 0,
                                            gid: 0,
                                            modified_ms: 0,
                                            created_ms: 0,
                                            accessed_ms: 0,
                                        }))
                                    }
                                }
                            }
                            Message::SyncRequest(_sync_req) => {
                                // Handle full index sync request (from new follower/client)
                                info!("Received SyncRequest from {} - sending full index", peer_id);
                                
                                let index = file_index_for_handler.read().unwrap();
                                let mut entries = Vec::new();
                                
                                for (path, entry) in index.iter() {
                                    let modified_ms = entry.modified
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis() as u64;
                                    
                                    let chunks: Vec<ChunkRefMsg> = entry.chunks.iter()
                                        .map(|c| ChunkRefMsg {
                                            hash: c.hash,
                                            offset: c.offset,
                                            size: c.size,
                                        })
                                        .collect();
                                    
                                    entries.push(IndexEntryMsg {
                                        path: path.to_string_lossy().to_string(),
                                        is_dir: entry.is_dir,
                                        size: entry.size,
                                        modified_ms,
                                        permissions: entry.permissions,
                                        chunks,
                                    });
                                }
                                
                                info!("Sending SyncResponse with {} entries", entries.len());
                                
                                Some(Message::SyncResponse(SyncResponseMsg {
                                    current_version: 0,
                                    entries,
                                }))
                            }
                            Message::GetChunk(get_chunk) => {
                                // Handle chunk fetch request from follower
                                debug!("Received GetChunk request from {} for {}", peer_id, hex::encode(&get_chunk.hash));
                                match chunk_store_for_handler.get(&get_chunk.hash) {
                                    Ok(data) => {
                                        Some(Message::ChunkData(ChunkDataMsg {
                                            hash: get_chunk.hash,
                                            data: Some(data),
                                            error: None,
                                        }))
                                    }
                                    Err(e) => {
                                        tracing::warn!("Chunk {} not found on leader: {}", hex::encode(&get_chunk.hash), e);
                                        Some(Message::ChunkData(ChunkDataMsg {
                                            hash: get_chunk.hash,
                                            data: None,
                                            error: Some(format!("Chunk not found: {}", e)),
                                        }))
                                    }
                                }
                            }
                            _ => {
                                debug!("Unhandled message from {}: {:?}", peer_id, msg);
                                None
                            }
                        }
                    },
                )
            );
            
            // Start peer manager to listen for connections
            if let Err(e) = peer_manager.start() {
                error!("Failed to start peer manager: {}", e);
                std::process::exit(1);
            }
            info!("Peer manager started on {}", config.node.bind);
            
            // Spawn broadcast processing thread
            let peer_manager_for_broadcast = peer_manager.clone();
            let chunk_store_for_broadcast = chunk_store.clone();
            let broadcast_queue_for_thread = broadcast_queue.clone();
            let chunk_stream_queue_for_thread = chunk_stream_queue.clone();
            let metadata_update_queue_for_thread = metadata_update_queue.clone();
            let cluster_for_broadcast = cluster.clone();
            std::thread::spawn(move || {
                use wolfdisk::network::protocol::{Message, FileSyncMsg, ChunkWithData, StoreChunkMsg, ChunkRefMsg};
                loop {
                    // Check queues every 50ms
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    
                    // Ensure we have outbound connections to all known peers
                    // (clients need metadata updates; followers need chunks + metadata)
                    let peers = cluster_for_broadcast.peers();
                    for peer in &peers {
                        if peer_manager_for_broadcast.get(&peer.node_id).is_none() {
                            info!("Broadcast thread: connecting to peer {} at {}", peer.node_id, peer.address);
                            if let Err(e) = peer_manager_for_broadcast.connect(&peer.node_id, &peer.address) {
                                tracing::warn!("Broadcast thread: failed to connect to {} at {}: {}", peer.node_id, peer.address, e);
                            } else {
                                info!("Broadcast thread: connected to peer {} at {} (total connections: {})", 
                                    peer.node_id, peer.address, peer_manager_for_broadcast.connection_count());
                            }
                        }
                    }
                    
                    // First, drain and broadcast any streamed chunks (high priority)
                    let pending_chunks: Vec<_> = {
                        let mut queue = chunk_stream_queue_for_thread.lock().unwrap();
                        queue.drain(..).collect()
                    };
                    
                    for (hash, data) in &pending_chunks {
                        let msg = Message::StoreChunk(StoreChunkMsg {
                            hash: *hash,
                            data: data.clone(),
                        });
                        peer_manager_for_broadcast.broadcast(&msg);
                    }
                    if !pending_chunks.is_empty() {
                        let num_conns = peer_manager_for_broadcast.connection_count();
                        info!("Streamed {} chunks to {} followers (peers known: {})", 
                            pending_chunks.len(), num_conns, peers.len());
                    }
                    
                    // Second, send metadata-only updates (file growing during writes)
                    let pending_metadata: Vec<_> = {
                        let mut queue = metadata_update_queue_for_thread.lock().unwrap();
                        queue.drain(..).collect()
                    };
                    
                    for (path, entry) in pending_metadata {
                        let modified_ms = entry.modified
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0);
                        
                        let chunk_refs: Vec<ChunkRefMsg> = entry.chunks.iter().map(|c| ChunkRefMsg {
                            hash: c.hash,
                            offset: c.offset,
                            size: c.size,
                        }).collect();
                        
                        let msg = Message::FileSync(FileSyncMsg {
                            path: path.to_string_lossy().to_string(),
                            size: entry.size,
                            is_dir: entry.is_dir,
                            permissions: entry.permissions,
                            uid: entry.uid,
                            gid: entry.gid,
                            modified_ms,
                            chunks: chunk_refs,
                            chunk_data: Vec::new(), // Metadata only — chunks already streamed
                        });
                        
                        info!("Broadcasting metadata update for {} ({} bytes, {} chunks) to {} connections", 
                            path.display(), entry.size, entry.chunks.len(),
                            peer_manager_for_broadcast.connection_count());
                        peer_manager_for_broadcast.broadcast(&msg);
                    }
                    
                    // Third, drain full broadcasts (creates, deletes, directory syncs)
                    let pending: Vec<_> = {
                        let mut queue = broadcast_queue_for_thread.lock().unwrap();
                        queue.drain(..).collect()
                    };
                    
                    for (path, entry) in pending {
                        // Check if this is a deletion (size == u64::MAX)
                        if entry.size == u64::MAX {
                            // Broadcast deletion
                            let msg = Message::FileSync(FileSyncMsg {
                                path: path.to_string_lossy().to_string(),
                                size: u64::MAX, // Signals deletion
                                is_dir: false,
                                permissions: 0,
                                uid: 0,
                                gid: 0,
                                modified_ms: 0,
                                chunks: Vec::new(),
                                chunk_data: Vec::new(),
                            });
                            
                            info!("Broadcasting FileDelete for {}", path.display());
                            peer_manager_for_broadcast.broadcast(&msg);
                            continue;
                        }
                        
                        // Normal file sync - send chunks in batches to avoid loading
                        // entire large files into memory at once
                        let modified_ms = entry.modified
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0);

                        let chunk_refs: Vec<wolfdisk::network::protocol::ChunkRefMsg> = entry.chunks.iter().map(|c| wolfdisk::network::protocol::ChunkRefMsg {
                            hash: c.hash.clone(),
                            offset: c.offset,
                            size: c.size,
                        }).collect();

                        let total_chunks = entry.chunks.len();
                        const BATCH_SIZE: usize = 4; // ~16MB per batch at 4MB chunks

                        if total_chunks == 0 {
                            // Directory or empty file - send metadata only
                            let msg = Message::FileSync(FileSyncMsg {
                                path: path.to_string_lossy().to_string(),
                                size: entry.size,
                                is_dir: entry.is_dir,
                                permissions: entry.permissions,
                                uid: entry.uid,
                                gid: entry.gid,
                                modified_ms,
                                chunks: chunk_refs,
                                chunk_data: Vec::new(),
                            });
                            info!("Broadcasting FileSync for {} (metadata only)", path.display());
                            peer_manager_for_broadcast.broadcast(&msg);
                        } else {
                            // Send chunks in batches to bound memory usage
                            for (batch_idx, chunk_batch) in entry.chunks.chunks(BATCH_SIZE).enumerate() {
                                let mut chunks_with_data = Vec::with_capacity(chunk_batch.len());
                                for chunk_ref in chunk_batch {
                                    if let Ok(data) = chunk_store_for_broadcast.get(&chunk_ref.hash) {
                                        chunks_with_data.push(ChunkWithData {
                                            hash: chunk_ref.hash.clone(),
                                            data,
                                        });
                                    }
                                }

                                // First batch includes full metadata + chunk refs;
                                // subsequent batches just carry chunk data
                                let msg = Message::FileSync(FileSyncMsg {
                                    path: path.to_string_lossy().to_string(),
                                    size: entry.size,
                                    is_dir: entry.is_dir,
                                    permissions: entry.permissions,
                                    uid: entry.uid,
                                    gid: entry.gid,
                                    modified_ms,
                                    chunks: if batch_idx == 0 { chunk_refs.clone() } else { Vec::new() },
                                    chunk_data: chunks_with_data,
                                });

                                if batch_idx == 0 {
                                    info!("Broadcasting FileSync for {} ({} bytes, {} chunks in {} batches)", 
                                        path.display(), entry.size, total_chunks,
                                        (total_chunks + BATCH_SIZE - 1) / BATCH_SIZE);
                                }
                                peer_manager_for_broadcast.broadcast(&msg);
                            }
                        }
                    }
                }
            });
            
            // Spawn initial sync thread for non-leader nodes
            // Use configured role, NOT runtime state, since all nodes start as Discovering
            // before the election happens (5 second delay)
            let should_sync = config.node.role == wolfdisk::config::NodeRole::Client
                || config.node.role == wolfdisk::config::NodeRole::Follower;
            // For Auto role, we need to sync in the thread after election decides
            let is_auto_role = config.node.role == wolfdisk::config::NodeRole::Auto;
            
            if should_sync || is_auto_role {
                let sync_cluster = cluster.clone();
                let sync_peer_manager = peer_manager.clone();
                let sync_file_index = file_index.clone();
                let sync_inode_table = inode_table.clone();
                let sync_next_inode = next_inode.clone();
                let sync_is_client = config.node.role == wolfdisk::config::NodeRole::Client;
                let _sync_chunk_store = chunk_store.clone();
                let sync_node_id = config.node.id.clone();
                
                std::thread::spawn(move || {
                    use wolfdisk::network::protocol::*;
                    use wolfdisk::storage::{ChunkRef, FileEntry};
                    use tracing::{info, warn, debug};
                    
                    info!("Initial sync thread started - waiting for leader discovery...");
                    
                    // Wait for leader to be discovered (up to 30 seconds)
                    let mut leader_found = false;
                    for _ in 0..60 {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        
                        // If we became leader ourselves, abort sync (we ARE the source of truth)
                        if sync_cluster.is_leader() {
                            info!("We became leader - skipping initial sync (we are the source of truth)");
                            return;
                        }
                        
                        if sync_cluster.leader_id().is_some() {
                            leader_found = true;
                            break;
                        }
                    }
                    
                    if !leader_found {
                        warn!("No leader found after 30s - skipping initial sync");
                        return;
                    }
                    
                    let leader_id = sync_cluster.leader_id().unwrap();
                    
                    // Don't sync from ourselves!
                    if leader_id == sync_node_id {
                        info!("We are the leader - skipping initial sync");
                        return;
                    }
                    
                    let leader_addr = match sync_cluster.leader_address() {
                        Some(addr) => addr,
                        None => {
                            warn!("Leader found but no address available");
                            return;
                        }
                    };
                    
                    info!("Leader discovered: {} at {} - requesting initial sync", leader_id, leader_addr);
                    
                    // Connect to leader and request sync
                    match sync_peer_manager.get_or_connect_leader(&leader_id, &leader_addr) {
                        Ok(conn) => {
                            let msg = Message::SyncRequest(SyncRequestMsg { from_version: 0 });
                            match conn.request(&msg) {
                                Ok(Message::SyncResponse(response)) => {
                                    info!("Received SyncResponse with {} entries", response.entries.len());
                                    
                                    let mut index = sync_file_index.write().unwrap();
                                    let mut inode_tbl = sync_inode_table.write().unwrap();
                                    let mut next_ino = sync_next_inode.write().unwrap();
                                    
                                    // Clear existing index and inode table to remove stale entries
                                    // from previous sessions (especially important for clients)
                                    if sync_is_client {
                                        info!("Client mode: clearing local index before sync ({} stale entries)", index.len());
                                        // Replace index with a fresh one
                                        *index = wolfdisk::storage::FileIndex::new();
                                        *inode_tbl = wolfdisk::storage::InodeTable::new();
                                        *next_ino = 2; // Reset inodes (1 = root)
                                    }
                                    
                                    for entry_msg in &response.entries {
                                        let path = std::path::PathBuf::from(&entry_msg.path);
                                        
                                        let chunk_refs: Vec<ChunkRef> = entry_msg.chunks.iter()
                                            .map(|c| ChunkRef {
                                                hash: c.hash,
                                                offset: c.offset,
                                                size: c.size,
                                            })
                                            .collect();
                                        
                                        let entry = FileEntry {
                                            size: entry_msg.size,
                                            is_dir: entry_msg.is_dir,
                                            permissions: entry_msg.permissions,
                                            uid: 0,
                                            gid: 0,
                                            modified: std::time::UNIX_EPOCH + std::time::Duration::from_millis(entry_msg.modified_ms),
                                            created: std::time::SystemTime::now(),
                                            accessed: std::time::SystemTime::now(),
                                            chunks: chunk_refs,
                                            symlink_target: None,
                                        };
                                        
                                        index.insert(path.clone(), entry);
                                        
                                        // Add to inode table
                                        let ino = *next_ino;
                                        *next_ino += 1;
                                        inode_tbl.insert(ino, path.clone());
                                        
                                        debug!("Synced: {} ({} bytes)", entry_msg.path, entry_msg.size);
                                    }
                                    
                                    if sync_is_client {
                                        info!("Initial index sync complete - {} files (client mode, no chunks stored)", response.entries.len());
                                    } else {
                                        info!("Initial index sync complete - {} files (follower will receive chunks via FileSync)", response.entries.len());
                                    }
                                }
                                Ok(other) => {
                                    warn!("Unexpected response to SyncRequest: {:?}", other);
                                }
                                Err(e) => {
                                    warn!("Failed to get SyncResponse: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Failed to connect to leader for initial sync: {}", e);
                        }
                    }
                });
            }
            
            // Create filesystem instance with cluster support (using shared state)
            let fs = match WolfDiskFS::with_cluster(
                config.clone(),
                Some(cluster.clone()),
                Some(peer_manager.clone()),
                file_index.clone(),
                chunk_store.clone(),
                inode_table.clone(),
                next_inode.clone(),
            ) {
                Ok(fs) => fs,
                Err(e) => {
                    error!("Failed to create filesystem: {}", e);
                    std::process::exit(1);
                }
            };

            // Mount options
            let options = vec![
                fuser::MountOption::FSName("wolfdisk".to_string()),
                fuser::MountOption::AutoUnmount,
                fuser::MountOption::AllowOther,
            ];

            // Start status file writer thread for wolfdiskctl
            let status_cluster = cluster.clone();
            std::thread::spawn(move || {
                while std::sync::Arc::strong_count(&status_cluster) > 1 {
                    status_cluster.write_status_file();
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            });

            // Mount the filesystem (this blocks)
            if let Err(e) = fuser::mount2(fs, &mountpoint, &options) {
                error!("Mount failed: {}", e);
                cluster.stop();
                std::process::exit(1);
            }
            
            cluster.stop();
        }

        Commands::Unmount { mountpoint } => {
            info!("Unmounting WolfDisk at {:?}", mountpoint);
            // Use fusermount to unmount
            let status = std::process::Command::new("fusermount")
                .arg("-u")
                .arg(&mountpoint)
                .status();

            match status {
                Ok(s) if s.success() => info!("Unmounted successfully"),
                Ok(s) => {
                    error!("Unmount failed with status: {}", s);
                    std::process::exit(1);
                }
                Err(e) => {
                    error!("Failed to run fusermount: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Commands::Status => {
            println!("╔══════════════════════════════════════════════════════════════╗");
            println!("║                     WolfDisk Status                          ║");
            println!("╚══════════════════════════════════════════════════════════════╝");
            println!();
            println!("Node Configuration:");
            println!("  Node ID:      {}", config.node.id);
            println!("  Role:         {:?}", config.node.role);
            println!("  Bind:         {}", config.node.bind);
            println!("  Data Dir:     {:?}", config.node.data_dir);
            println!();
            println!("Replication:");
            println!("  Mode:         {:?}", config.replication.mode);
            println!("  Factor:       {}", config.replication.factor);
            println!("  Chunk Size:   {} bytes", config.replication.chunk_size);
            println!();
            
            // Check if the mount path exists and is mounted
            let mountpoint = &config.mount.path;
            let is_mounted = std::path::Path::new(&mountpoint).exists() 
                && std::fs::read_dir(&mountpoint).is_ok();
            
            println!("Mount Status:");
            println!("  Mount Path:   {}", mountpoint.display());
            println!("  Mounted:      {}", if is_mounted { "Yes" } else { "No" });
            println!();
            
            // Show discovery/peer config (without starting discovery)
            if let Some(ref discovery) = config.cluster.discovery {
                println!("Discovery:      {}", discovery);
            }
            if !config.cluster.peers.is_empty() {
                println!("Static Peers:   {:?}", config.cluster.peers);
            }
            
            println!();
            println!("Note: Run 'wolfdisk stats' for live cluster statistics");
        }

        Commands::Stats => {
            println!("╔══════════════════════════════════════════════════════════════╗");
            println!("║                   WolfDisk Live Stats                        ║");
            println!("║                   Press Ctrl+C to exit                       ║");
            println!("╚══════════════════════════════════════════════════════════════╝");
            println!();

            // Start cluster manager to get live stats
            let mut cluster = wolfdisk::ClusterManager::new(config.clone());
            if let Err(e) = cluster.start() {
                error!("Failed to start cluster manager: {}", e);
                std::process::exit(1);
            }

            // Set up Ctrl+C handler
            let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
            let r = running.clone();
            ctrlc::set_handler(move || {
                r.store(false, std::sync::atomic::Ordering::SeqCst);
            }).expect("Failed to set Ctrl+C handler");

            // Give discovery time to find peers initially
            std::thread::sleep(std::time::Duration::from_secs(2));

            while running.load(std::sync::atomic::Ordering::SeqCst) {
                // Clear screen and move cursor to top
                print!("\x1B[2J\x1B[1;1H");
                
                let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                println!("WolfDisk Cluster Stats - {} (Ctrl+C to exit)", now);
                println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                println!();
                
                println!("This Node:    {} ({:?})", config.node.id, cluster.state());
                if let Some(leader) = cluster.leader_id() {
                    let is_me = leader == config.node.id;
                    println!("Leader:       {}{}", leader, if is_me { " (this node)" } else { "" });
                } else {
                    println!("Leader:       (discovering...)");
                }
                println!("Term:         {}", cluster.term());
                println!("Index Ver:    {}", cluster.index_version());
                println!();
                
                let peers = cluster.peers();
                println!("Cluster Nodes ({}):", peers.len() + 1);
                println!("  ● {} (self) - {:?}", config.node.id, cluster.state());
                for peer in &peers {
                    let status = if peer.last_seen.elapsed().as_secs() < 4 { "●" } else { "○" };
                    let role = if peer.is_leader { "leader" } else if peer.is_client { "client" } else { "follower" };
                    let ago = peer.last_seen.elapsed().as_secs();
                    println!("  {} {} - {} (seen {}s ago)", status, peer.node_id, role, ago);
                }
                
                std::thread::sleep(std::time::Duration::from_secs(1));
            }

            cluster.stop();
            println!("\nStopped.");
        }

        Commands::ListServers { what } => {
            if what != "servers" {
                error!("Unknown list type '{}'. Use: wolfdisk list servers", what);
                std::process::exit(1);
            }
            
            println!("Discovering servers...\n");

            // Start cluster manager to discover peers
            let mut cluster = wolfdisk::ClusterManager::new(config.clone());
            if let Err(e) = cluster.start() {
                error!("Failed to start cluster manager: {}", e);
                std::process::exit(1);
            }

            // Give discovery time to find peers
            std::thread::sleep(std::time::Duration::from_secs(3));

            let peers = cluster.peers();
            
            println!("╔════════════════════════════════════════════════════════════════╗");
            println!("║                    WolfDisk Servers                            ║");
            println!("╠════════════════════════════════════════════════════════════════╣");
            
            // Print this node first
            let my_role = if cluster.is_leader() { "LEADER" } else if cluster.state() == wolfdisk::cluster::ClusterState::Client { "CLIENT" } else { "FOLLOWER" };
            println!("║ ● {:15} {:22} {:8} ║", config.node.id, config.node.bind, my_role);
            
            for peer in &peers {
                let status = if peer.last_seen.elapsed().as_secs() < 4 { "●" } else { "○" };
                let role = if peer.is_leader { "LEADER" } else if peer.is_client { "CLIENT" } else { "FOLLOWER" };
                println!("║ {} {:15} {:22} {:8} ║", status, peer.node_id, peer.address, role);
            }
            
            println!("╚════════════════════════════════════════════════════════════════╝");
            println!();
            println!("Total: {} server(s)", peers.len() + 1);
            
            cluster.stop();
        }

        Commands::Init { data_dir } => {
            info!("Initializing WolfDisk data directory at {:?}", data_dir);
            
            // Create directory structure
            let chunks_dir = data_dir.join("chunks");
            let index_dir = data_dir.join("index");
            let wal_dir = data_dir.join("wal");

            for dir in [&chunks_dir, &index_dir, &wal_dir] {
                if let Err(e) = std::fs::create_dir_all(dir) {
                    error!("Failed to create {:?}: {}", dir, e);
                    std::process::exit(1);
                }
            }

            info!("Created directory structure:");
            info!("  {}/chunks/  - chunk storage", data_dir.display());
            info!("  {}/index/   - file index", data_dir.display());
            info!("  {}/wal/     - write-ahead log", data_dir.display());
            info!("Initialization complete!");
        }
    }
}
