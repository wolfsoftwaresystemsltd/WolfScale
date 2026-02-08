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
#[command(version = "0.1.0")]
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
            
            // Create peer manager for network communication
            let peer_manager = std::sync::Arc::new(
                wolfdisk::network::peer::PeerManager::new(
                    config.node.id.clone(),
                    config.node.bind.clone(),
                    move |peer_id, msg| {
                        use wolfdisk::network::protocol::*;
                        
                        match msg {
                            Message::IndexUpdate(update) => {
                                info!("Received IndexUpdate from {}: {:?}", peer_id, update.operation);
                                
                                // Apply the update to our local index
                                let mut index = file_index_for_handler.write().unwrap();
                                match update.operation {
                                    IndexOperation::Delete { path } => {
                                        info!("Replicating delete: {}", path);
                                        index.remove(&std::path::PathBuf::from(&path));
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
                                        });
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
                                
                                info!("Received FileSync from {}: {} ({} bytes, {} chunks)", 
                                    peer_id, sync.path, sync.size, sync.chunk_data.len());
                                
                                // Store all chunks first
                                for chunk_with_data in &sync.chunk_data {
                                    if let Err(e) = chunk_store_for_handler.store_with_hash(&chunk_with_data.hash, &chunk_with_data.data) {
                                        tracing::warn!("Failed to store chunk: {}", e);
                                    }
                                }
                                
                                // Update index
                                let mut index = file_index_for_handler.write().unwrap();
                                let chunk_refs: Vec<ChunkRef> = sync.chunks.iter()
                                    .map(|c| ChunkRef {
                                        hash: c.hash,
                                        offset: c.offset,
                                        size: c.size,
                                    })
                                    .collect();
                                
                                let path = std::path::PathBuf::from(&sync.path);
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
                                });
                                
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
                                    match chunk_store_for_handler.write(&mut entry.chunks, write_req.offset, &write_req.data) {
                                        Ok(written) => {
                                            let new_end = write_req.offset + written as u64;
                                            if new_end > entry.size {
                                                entry.size = new_end;
                                            }
                                            entry.modified = std::time::SystemTime::now();
                                            info!("Leader wrote {} bytes to {}", written, write_req.path);
                                            
                                            // Queue broadcast to all followers
                                            let entry_clone = entry.clone();
                                            let path_clone = path.clone();
                                            drop(index); // Release lock before queueing
                                            broadcast_queue_for_handler.lock().unwrap().push((path_clone, entry_clone));
                                            
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
            std::thread::spawn(move || {
                use wolfdisk::network::protocol::{Message, FileSyncMsg, ChunkWithData};
                loop {
                    // Check queue every 50ms
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    
                    // Drain pending broadcasts
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
                        
                        // Normal file sync - read chunk data
                        let mut chunks_with_data = Vec::new();
                        for chunk_ref in &entry.chunks {
                            if let Ok(data) = chunk_store_for_broadcast.get(&chunk_ref.hash) {
                                chunks_with_data.push(ChunkWithData {
                                    hash: chunk_ref.hash.clone(),
                                    data,
                                });
                            }
                        }
                        
                        // Convert SystemTime to ms since epoch
                        let modified_ms = entry.modified
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0);
                        
                        let msg = Message::FileSync(FileSyncMsg {
                            path: path.to_string_lossy().to_string(),
                            size: entry.size,
                            is_dir: entry.is_dir,
                            permissions: entry.permissions,
                            uid: entry.uid,
                            gid: entry.gid,
                            modified_ms,
                            chunks: entry.chunks.iter().map(|c| wolfdisk::network::protocol::ChunkRefMsg {
                                hash: c.hash.clone(),
                                offset: c.offset,
                                size: c.size,
                            }).collect(),
                            chunk_data: chunks_with_data,
                        });
                        
                        info!("Broadcasting FileSync for {} ({} bytes, {} chunks)", 
                            path.display(), entry.size, entry.chunks.len());
                        peer_manager_for_broadcast.broadcast(&msg);
                    }
                }
            });
            
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
                    let role = if peer.is_leader { "leader" } else { "follower" };
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
            let my_role = if cluster.is_leader() { "LEADER" } else { "FOLLOWER" };
            println!("║ ● {:15} {:22} {:8} ║", config.node.id, config.node.bind, my_role);
            
            for peer in &peers {
                let status = if peer.last_seen.elapsed().as_secs() < 4 { "●" } else { "○" };
                let role = if peer.is_leader { "LEADER" } else { "FOLLOWER" };
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
