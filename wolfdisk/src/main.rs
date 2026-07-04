//! WolfDisk CLI
//!
//! Command-line interface for mounting and managing WolfDisk.

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::{debug, error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use wolfdisk::{
    fuse::WolfDiskFS,
    storage::{ChunkRef, FileEntry, FileIndex},
    Config,
};

#[derive(Parser)]
#[command(name = "wolfdisk")]
#[command(author = "Wolf Software Systems Ltd")]
#[command(version = env!("CARGO_PKG_VERSION"))]
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

/// Paths the follower must delete on a full (authoritative) sync: every local
/// path the leader's complete index no longer contains. This is the
/// data-deleting decision of the self-heal, so it's a pure function with a
/// direct regression test (see tests at the bottom of this file).
fn full_sync_stale_paths<'a>(
    leader_paths: &std::collections::HashSet<std::path::PathBuf>,
    local_paths: impl Iterator<Item = &'a std::path::PathBuf>,
) -> Vec<std::path::PathBuf> {
    local_paths
        .filter(|p| !leader_paths.contains(*p))
        .cloned()
        .collect()
}

/// May a full-sync response authoritatively DELETE this follower's local
/// entries (those the leader didn't send)? This is the one place a follower can
/// lose data, so it is deliberately conservative — two independent conditions
/// must BOTH hold:
///
///   1. `leader_entry_count > 0` — never wipe to match a leader whose index
///      failed to load (it reads as zero entries; honouring it nukes the data).
///   2. `leader_version >= our_version` — never delete our (more-advanced) data
///      to match a leader that has REGRESSED below what we already knew. A
///      leader at a lower or zero version has most likely just restarted before
///      its persisted version reloaded (or lost persistence entirely); its
///      smaller index may be stale/half-loaded. Honouring it let a transient
///      post-restart blip drive a cluster-wide delete — observed in wabil's
///      logs as a leader briefly at v0/226 entries (2026-06-29).
///
/// When this returns false we still apply the leader's ADDITIONS (we never
/// reject data) and simply keep our local extras; the next cycle, once the
/// leader's version is credible again, the reconcile proceeds normally. So the
/// cost of a false negative is at most a slightly delayed convergence — never
/// lost data. Paired with the monotonic version rule in the resync loop (we
/// never lower our own version to match a regressed leader), a follower cannot
/// be tricked into deleting data it correctly held.
fn full_sync_may_delete(leader_version: u64, our_version: u64, leader_entry_count: usize) -> bool {
    leader_entry_count > 0 && leader_version >= our_version
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

            // If metadata was just moved onto a cache disk, copy an existing
            // index/wal across BEFORE we read from the (otherwise empty) new path,
            // so enabling the cache never orphans an existing namespace.
            config.migrate_metadata_to_cache();

            // Create shared state for filesystem that can be accessed by message handler
            std::fs::create_dir_all(config.chunks_dir()).ok();
            std::fs::create_dir_all(config.index_dir()).ok();

            let file_index = std::sync::Arc::new(std::sync::RwLock::new(
                FileIndex::load_or_create(&config.index_dir()).expect("Failed to load file index"),
            ));
            let file_index_for_handler = file_index.clone();

            // Restore the replication version + changelog from disk so a restart
            // doesn't reset us to v0 — which made every node falsely agree "v0, in
            // sync" while their file sets diverged, and forced a heavy full
            // reconcile on every restart (wabil 2026-06-29).
            cluster.load_replication_state(&config.index_dir());

            // Create chunk store for replication (shared with WolfDiskFS). When an SSD
            // cache tier is configured it sits in front of the bulk data_dir.
            let chunk_store = std::sync::Arc::new(
                wolfdisk::storage::ChunkStore::with_cache(
                    config.chunks_dir(),
                    4 * 1024 * 1024,
                    config.cache_chunks_dir(),
                    config.cache_mode(),
                    config.cache.max_bytes,
                ).expect("Failed to create chunk store"),
            );
            // Crash recovery: re-queue any chunk that was written to the SSD cache but
            // not yet flushed to the bulk store before an unclean shutdown.
            let recovered = chunk_store.recover_dirty();
            if recovered > 0 {
                tracing::warn!("Chunk cache: recovered {} un-flushed chunk(s) from the SSD tier", recovered);
            }
            if chunk_store.has_cache() {
                tracing::info!(
                    "Chunk cache active: {:?} mode, SSD tier at {:?}",
                    chunk_store.cache_mode(), config.cache_chunks_dir()
                );
            }
            // Write-back: background flusher copies dirty SSD chunks down to the bulk
            // store. Exits when the store Arc has no other owners (process shutdown).
            if chunk_store.cache_mode() == wolfdisk::storage::chunks::CacheMode::WriteBack {
                let flusher = chunk_store.clone();
                std::thread::spawn(move || {
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(2));
                        // Flush BEFORE the exit check so the final iteration (once the
                        // other owners have dropped on shutdown) still drains dirty
                        // chunks to the bulk store. recover_dirty re-flushes anything
                        // an unclean kill leaves behind, so no acked chunk is lost.
                        flusher.flush_dirty();
                        if std::sync::Arc::strong_count(&flusher) <= 1 {
                            break;
                        }
                    }
                });
            }
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
            let broadcast_queue: std::sync::Arc<
                std::sync::Mutex<Vec<(std::path::PathBuf, wolfdisk::storage::FileEntry)>>,
            > = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let broadcast_queue_for_handler = broadcast_queue.clone();

            // Chunk streaming queue for streaming individual chunks to followers
            // (hash, data) tuples that are sent via StoreChunk messages
            #[allow(clippy::type_complexity)]
            let chunk_stream_queue: std::sync::Arc<std::sync::Mutex<Vec<([u8; 32], Vec<u8>)>>> =
                std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let chunk_stream_queue_for_handler = chunk_stream_queue.clone();

            // Metadata update queue for sending metadata-only FileSync to followers
            // during streaming replication (no chunk_data, just path + entry metadata)
            let metadata_update_queue: std::sync::Arc<
                std::sync::Mutex<Vec<(std::path::PathBuf, wolfdisk::storage::FileEntry)>>,
            > = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let metadata_update_queue_for_handler = metadata_update_queue.clone();

            // Track if this node is a client (clients don't store chunk data locally)
            let is_client_role = config.node.role == wolfdisk::config::NodeRole::Client;
            let cluster_for_handler = cluster.clone();

            // Create peer manager for network communication
            let peer_manager = std::sync::Arc::new(wolfdisk::network::peer::PeerManager::new(
                config.node.id.clone(),
                config.node.bind.clone(),
                move |peer_id, msg| {
                    use wolfdisk::network::protocol::*;

                    match msg {
                        Message::StoreChunk(store_chunk) => {
                            // Handle streamed chunk from leader (streaming replication)
                            if !is_client_role {
                                debug!(
                                    "Received streamed chunk {} from leader",
                                    hex::encode(store_chunk.hash)
                                );
                                if let Err(e) = chunk_store_for_handler
                                    .store_with_hash(&store_chunk.hash, &store_chunk.data)
                                {
                                    tracing::warn!("Failed to store streamed chunk: {}", e);
                                }
                            }
                            None // No response needed
                        }
                        Message::IndexUpdate(update) => {
                            debug!(
                                "Received IndexUpdate from {}: {:?}",
                                peer_id, update.operation
                            );

                            // Acquire locks in correct order (Inode -> Index) to match readdir and avoid deadlocks/races
                            let mut inode_tbl = inode_table_for_handler.write().unwrap();
                            let mut index = file_index_for_handler.write().unwrap();

                            // Track chunks to delete after dropping locks
                            let mut chunks_to_delete = Vec::new();

                            match update.operation {
                                IndexOperation::Delete { path } => {
                                    debug!("Replicating delete: {}", path);
                                    let del_path = std::path::PathBuf::from(&path);

                                    // Update index
                                    if let Some(entry) = index.remove(&del_path) {
                                        if !is_client_role {
                                            chunks_to_delete = entry.chunks;
                                        }
                                        debug!("Deleted file from follower: {}", path);
                                    }

                                    // Update inode table (atomic with index update)
                                    inode_tbl.remove_path(&del_path);
                                }
                                IndexOperation::Upsert {
                                    path,
                                    size,
                                    modified_ms,
                                    permissions,
                                    chunks,
                                    uid,
                                    gid,
                                    symlink_target,
                                } => {
                                    debug!("Replicating upsert: {} ({} bytes)", path, size);
                                    let chunk_refs: Vec<ChunkRef> = chunks
                                        .iter()
                                        .map(|c| ChunkRef {
                                            hash: c.hash,
                                            offset: c.offset,
                                            size: c.size,
                                        })
                                        .collect();
                                    let now = std::time::SystemTime::now();
                                    let file_path = std::path::PathBuf::from(&path);

                                    // Update index. is_dir/symlink derived from the
                                    // op: a symlink_target marks a symlink.
                                    let old_entry = index.insert(
                                        file_path.clone(),
                                        FileEntry {
                                            size,
                                            modified: std::time::UNIX_EPOCH
                                                + std::time::Duration::from_millis(modified_ms),
                                            permissions,
                                            is_dir: false,
                                            chunks: chunk_refs,
                                            uid,
                                            gid,
                                            created: now,
                                            accessed: now,
                                            symlink_target,
                                        },
                                    );

                                    // If we overwrote an existing file, clean up its chunks
                                    if let Some(entry) = old_entry {
                                        if !entry.is_dir && !is_client_role {
                                            // We can't delete immediately if we are holding locks?
                                            // Actually we can, chunk_store doesn't use these locks.
                                            // But for safety/consistency we'll collect them to delete later.
                                            chunks_to_delete.extend(entry.chunks);
                                        }
                                    }

                                    // Update inode table if needed (atomic with index update)
                                    if inode_tbl.get_inode(&file_path).is_none() {
                                        let mut next_ino = next_inode_for_handler.write().unwrap();
                                        let ino = *next_ino;
                                        *next_ino += 1;
                                        inode_tbl.insert(ino, file_path);
                                    }
                                }
                                IndexOperation::Mkdir { path, permissions } => {
                                    debug!("Replicating mkdir: {}", path);
                                    let now = std::time::SystemTime::now();
                                    let dir_path = std::path::PathBuf::from(&path);

                                    // Update index
                                    index.insert(
                                        dir_path.clone(),
                                        FileEntry {
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
                                        },
                                    );

                                    // Update inode table if needed
                                    if inode_tbl.get_inode(&dir_path).is_none() {
                                        let mut next_ino = next_inode_for_handler.write().unwrap();
                                        let ino = *next_ino;
                                        *next_ino += 1;
                                        inode_tbl.insert(ino, dir_path);
                                    }
                                }
                                IndexOperation::Rename { from_path, to_path } => {
                                    debug!("Replicating rename: {} -> {}", from_path, to_path);
                                    let from = std::path::PathBuf::from(&from_path);
                                    let to = std::path::PathBuf::from(&to_path);

                                    // Handle overwrite at destination
                                    if let Some(target_entry) = index.remove(&to) {
                                        if !target_entry.is_dir && !is_client_role {
                                            chunks_to_delete.extend(target_entry.chunks);
                                        }
                                        // Also remove from inode table
                                        // Note: inode_table.remove_path deletes by path, effectively removing the target inode mapping.
                                        inode_tbl.remove_path(&to);
                                    }

                                    // Move entry
                                    if let Some(entry) = index.remove(&from) {
                                        index.insert(to.clone(), entry);

                                        // Update inode table
                                        inode_tbl.remove_path(&from);
                                        let mut next_ino = next_inode_for_handler.write().unwrap();
                                        let ino = *next_ino;
                                        *next_ino += 1;
                                        inode_tbl.insert(ino, to);
                                    }
                                }
                            }

                            // Drop locks before doing IO (deleting chunks)
                            drop(index);
                            drop(inode_tbl);

                            // Delete chunks if any
                            for chunk in chunks_to_delete {
                                let _ = chunk_store_for_handler.delete(&chunk.hash);
                            }

                            None // No response needed for replication
                        }
                        Message::FileSync(sync) => {
                            // Check if this is a deletion signal (size == u64::MAX)
                            if sync.size == u64::MAX {
                                debug!("Received FileDelete from {}: {}", peer_id, sync.path);

                                let path = std::path::PathBuf::from(&sync.path);

                                // Lock ordering: Inode -> Index
                                let mut inode_tbl = inode_table_for_handler.write().unwrap();
                                let mut index = file_index_for_handler.write().unwrap();

                                let chunks_to_delete = if let Some(entry) = index.remove(&path) {
                                    debug!("Deleted file from follower: {}", sync.path);
                                    inode_tbl.remove_path(&path);
                                    entry.chunks
                                } else {
                                    Vec::new()
                                };

                                drop(index);
                                drop(inode_tbl);

                                // Delete chunks
                                for chunk in chunks_to_delete {
                                    let _ = chunk_store_for_handler.delete(&chunk.hash);
                                }
                                return None;
                            }

                            debug!("Received FileSync from {}: {} ({} bytes, {} chunk_refs, {} chunk_data)", 
                                    peer_id, sync.path, sync.size, sync.chunks.len(), sync.chunk_data.len());

                            // Store chunks locally (skip for client nodes - they read from leader)
                            if !is_client_role {
                                for chunk_with_data in &sync.chunk_data {
                                    if let Err(e) = chunk_store_for_handler.store_with_hash(
                                        &chunk_with_data.hash,
                                        &chunk_with_data.data,
                                    ) {
                                        tracing::warn!("Failed to store chunk: {}", e);
                                    }
                                }
                            }

                            // Lock ordering: Inode -> Index
                            let mut inode_tbl = inode_table_for_handler.write().unwrap();
                            let mut index = file_index_for_handler.write().unwrap();

                            let path = std::path::PathBuf::from(&sync.path);

                            // If the incoming message has chunk_refs, use them (authoritative metadata).
                            // If chunk_refs is empty but we have chunk_data, this is a subsequent batch
                            // of a multi-batch transfer — only store the chunks, keep existing entry.
                            if !sync.chunks.is_empty() {
                                // Authoritative update: replace the full file entry
                                let chunk_refs: Vec<ChunkRef> = sync
                                    .chunks
                                    .iter()
                                    .map(|c| ChunkRef {
                                        hash: c.hash,
                                        offset: c.offset,
                                        size: c.size,
                                    })
                                    .collect();

                                index.insert(
                                    path.clone(),
                                    FileEntry {
                                        size: sync.size,
                                        is_dir: sync.is_dir,
                                        permissions: sync.permissions,
                                        uid: sync.uid,
                                        gid: sync.gid,
                                        modified: std::time::UNIX_EPOCH
                                            + std::time::Duration::from_millis(sync.modified_ms),
                                        created: std::time::UNIX_EPOCH
                                            + std::time::Duration::from_millis(sync.modified_ms),
                                        accessed: std::time::SystemTime::now(),
                                        chunks: chunk_refs,
                                        symlink_target: sync.symlink_target.clone(),
                                    },
                                );
                            } else if !sync.chunk_data.is_empty() {
                                // Subsequent batch: only storing chunk data, keep existing index entry.
                                // Update size if it has grown.
                                if let Some(entry) = index.get_mut(&path) {
                                    if sync.size > entry.size {
                                        entry.size = sync.size;
                                    }
                                    entry.modified = std::time::UNIX_EPOCH
                                        + std::time::Duration::from_millis(sync.modified_ms);
                                }
                                debug!(
                                    "Stored {} additional chunks for {} (batch continuation)",
                                    sync.chunk_data.len(),
                                    sync.path
                                );
                            } else {
                                // Metadata-only update (streaming replication final sync)
                                // The chunks were already streamed via StoreChunk messages.
                                // Update the index entry with final metadata + chunk refs.
                                let chunk_refs: Vec<ChunkRef> = Vec::new();
                                // If we already have an entry, update it; otherwise create new.
                                if let Some(entry) = index.get_mut(&path) {
                                    // Only increase size, never decrease — prevents out-of-order
                                    // CreateFile broadcasts (size=0) from overwriting WriteRequest
                                    // metadata (correct size) that arrived first.
                                    if sync.size > entry.size {
                                        entry.size = sync.size;
                                    }
                                    entry.permissions = sync.permissions;
                                    entry.uid = sync.uid;
                                    entry.gid = sync.gid;
                                    entry.symlink_target = sync.symlink_target.clone();
                                    entry.modified = std::time::UNIX_EPOCH
                                        + std::time::Duration::from_millis(sync.modified_ms);
                                } else {
                                    index.insert(
                                        path.clone(),
                                        FileEntry {
                                            size: sync.size,
                                            is_dir: sync.is_dir,
                                            permissions: sync.permissions,
                                            uid: sync.uid,
                                            gid: sync.gid,
                                            modified: std::time::UNIX_EPOCH
                                                + std::time::Duration::from_millis(
                                                    sync.modified_ms,
                                                ),
                                            created: std::time::UNIX_EPOCH
                                                + std::time::Duration::from_millis(
                                                    sync.modified_ms,
                                                ),
                                            accessed: std::time::SystemTime::now(),
                                            chunks: chunk_refs,
                                            symlink_target: sync.symlink_target.clone(),
                                        },
                                    );
                                }
                            }

                            // Update inode table
                            if inode_tbl.get_inode(&path).is_none() {
                                let mut next_ino = next_inode_for_handler.write().unwrap();
                                let ino = *next_ino;
                                *next_ino += 1;
                                inode_tbl.insert(ino, path.clone());
                                debug!("Added inode {} for replicated path: {}", ino, sync.path);
                            }

                            drop(index);
                            drop(inode_tbl);

                            debug!("FileSync complete for {}", sync.path);
                            None
                        }
                        Message::WriteRequest(write_req) => {
                            // Handle write request from follower (we're the leader)
                            debug!(
                                "Received WriteRequest from {}: {} (offset: {}, size: {})",
                                peer_id,
                                write_req.path,
                                write_req.offset,
                                write_req.data.len()
                            );

                            // Write data to chunk store
                            let path = std::path::PathBuf::from(&write_req.path);
                            let mut index = file_index_for_handler.write().unwrap();

                            if let Some(entry) = index.get_mut(&path) {
                                // Track chunks before write to detect new ones
                                let chunks_before = entry.chunks.len();

                                match chunk_store_for_handler.write(
                                    &mut entry.chunks,
                                    write_req.offset,
                                    &write_req.data,
                                ) {
                                    Ok(written) => {
                                        let new_end = write_req.offset + written as u64;
                                        if new_end > entry.size {
                                            entry.size = new_end;
                                        }
                                        entry.modified = std::time::SystemTime::now();

                                        // Stream any new chunks to followers immediately
                                        let new_chunks: Vec<_> =
                                            entry.chunks[chunks_before..].to_vec();
                                        let entry_clone = entry.clone();
                                        let path_clone = path.clone();
                                        drop(index); // Release lock before streaming

                                        // Queue new chunks for streaming to followers
                                        if !new_chunks.is_empty() {
                                            let mut stream_queue =
                                                chunk_stream_queue_for_handler.lock().unwrap();
                                            for chunk_ref in &new_chunks {
                                                if let Ok(chunk_data) =
                                                    chunk_store_for_handler.get(&chunk_ref.hash)
                                                {
                                                    stream_queue.push((chunk_ref.hash, chunk_data));
                                                }
                                            }
                                        }

                                        // Queue metadata-only update (chunks already streamed above)
                                        cluster_for_handler
                                            .increment_index_version(path_clone.clone());
                                        metadata_update_queue_for_handler
                                            .lock()
                                            .unwrap()
                                            .push((path_clone, entry_clone));

                                        debug!(
                                            "Leader wrote {} bytes to {} ({} new chunks streamed)",
                                            written,
                                            write_req.path,
                                            new_chunks.len()
                                        );

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
                                let paths: Vec<String> =
                                    index.iter().map(|(p, _)| format!("{:?}", p)).collect();
                                tracing::warn!(
                                    "File not found on leader: {:?} (path string: {})",
                                    path,
                                    write_req.path
                                );
                                tracing::warn!(
                                    "Index contains {} entries: {:?}",
                                    paths.len(),
                                    paths
                                );
                                Some(Message::ClientResponse(ClientResponseMsg {
                                    success: false,
                                    data: None,
                                    error: Some("File not found".to_string()),
                                }))
                            }
                        }
                        Message::CreateFile(create_req) => {
                            // Handle create file request from follower (we're the leader)
                            debug!(
                                "Received CreateFile from {}: {} (mode: {:o})",
                                peer_id, create_req.path, create_req.mode
                            );

                            let path = std::path::PathBuf::from(&create_req.path);

                            // Lock ordering: Inode -> Index
                            let mut inode_tbl = inode_table_for_handler.write().unwrap();
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

                                // Update index
                                index.insert(path.clone(), entry);

                                // Update inode table
                                let mut next_ino = next_inode_for_handler.write().unwrap();
                                let ino = *next_ino;
                                *next_ino += 1;
                                inode_tbl.insert(ino, path.clone());

                                debug!(
                                    "Leader created file: {} with inode {}",
                                    create_req.path, ino
                                );

                                // Queue broadcast to followers
                                cluster_for_handler.increment_index_version(path.clone());
                                let entry_for_broadcast = index.get(&path).unwrap().clone();

                                broadcast_queue_for_handler
                                    .lock()
                                    .unwrap()
                                    .push((path, entry_for_broadcast));

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

                            // Lock ordering: Inode -> Index
                            let mut inode_tbl = inode_table_for_handler.write().unwrap();
                            let mut index = file_index_for_handler.write().unwrap();

                            if let Some(entry) = index.remove(&path) {
                                // Delete chunks (can do this after dropping locks, or here?)
                                // For now collect them to delete later or just delete (fast enough usually)
                                // Or better: drop locks then delete. But we need index lock to remove entry.
                                let chunks_to_delete = entry.chunks;

                                // Remove from inode table
                                inode_tbl.remove_path(&path);

                                info!("Leader deleted file: {}", del.path);

                                // Queue delete broadcast to followers
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

                                // Drop locks before IO
                                drop(index);
                                drop(inode_tbl);

                                // Delete chunks
                                for chunk in chunks_to_delete {
                                    let _ = chunk_store_for_handler.delete(&chunk.hash);
                                }

                                cluster_for_handler.record_deletion(path.clone());
                                broadcast_queue_for_handler
                                    .lock()
                                    .unwrap()
                                    .push((path, delete_marker));

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

                            // Lock ordering: Inode -> Index
                            let mut inode_tbl = inode_table_for_handler.write().unwrap();
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

                                // Update index
                                index.insert(path.clone(), entry.clone());

                                // Update inode table
                                let mut next_ino = next_inode_for_handler.write().unwrap();
                                let ino = *next_ino;
                                *next_ino += 1;
                                inode_tbl.insert(ino, path.clone());

                                debug!(
                                    "Leader created directory: {} with inode {}",
                                    dir_req.path, ino
                                );

                                // Queue broadcast to followers
                                drop(index);
                                drop(inode_tbl);
                                drop(next_ino);
                                broadcast_queue_for_handler
                                    .lock()
                                    .unwrap()
                                    .push((path, entry));

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

                            // Lock ordering: Inode -> Index
                            let mut inode_tbl = inode_table_for_handler.write().unwrap();
                            let mut index = file_index_for_handler.write().unwrap();

                            match index.get(&path) {
                                Some(entry) if !entry.is_dir => {
                                    Some(Message::FileOpResponse(FileOpResponseMsg {
                                        success: false,
                                        error: Some("Not a directory".to_string()),
                                    }))
                                }
                                None => Some(Message::FileOpResponse(FileOpResponseMsg {
                                    success: false,
                                    error: Some("Directory not found".to_string()),
                                })),
                                _ => {
                                    // Check directory is empty (requires checking all paths in index)
                                    // This is a bit slow while holding write lock, but necessary for consistency
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

                                    cluster_for_handler.record_deletion(path.clone());
                                    broadcast_queue_for_handler
                                        .lock()
                                        .unwrap()
                                        .push((path, delete_marker));

                                    Some(Message::FileOpResponse(FileOpResponseMsg {
                                        success: true,
                                        error: None,
                                    }))
                                }
                            }
                        }
                        Message::RenameFile(rename_req) => {
                            // Handle incoming rename request (if we're leader)
                            info!(
                                "Received RenameFile: {} -> {}",
                                rename_req.from_path, rename_req.to_path
                            );

                            let from_path = std::path::PathBuf::from(&rename_req.from_path);
                            let to_path = std::path::PathBuf::from(&rename_req.to_path);

                            // Lock ordering: Inode -> Index
                            let mut inode_tbl = inode_table_for_handler.write().unwrap();
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

                            // Handle overwrite
                            if let Some(target_entry) = index.get(&to_path) {
                                // Check if directory is empty
                                if target_entry.is_dir {
                                    let has_children =
                                        index.paths().any(|p| p.parent() == Some(&to_path));
                                    if has_children {
                                        return Some(Message::FileOpResponse(FileOpResponseMsg {
                                            success: false,
                                            error: Some(
                                                "Destination directory not empty".to_string(),
                                            ),
                                        }));
                                    }
                                }

                                // Delete chunks of target
                                for chunk in &target_entry.chunks {
                                    let _ = chunk_store_for_handler.delete(&chunk.hash);
                                }

                                // Remove target from index/inode
                                index.remove(&to_path);
                                inode_tbl.remove_path(&to_path);
                            }

                            // Move entry in index
                            index.remove(&from_path);
                            index.insert(to_path.clone(), entry.clone());

                            // Update inode table
                            // Here we can try to preserve inode if we want, or just re-map
                            if let Some(ino) = inode_tbl.get_inode(&from_path) {
                                inode_tbl.remove_path(&from_path);
                                inode_tbl.insert(ino, to_path.clone());
                            }

                            info!(
                                "Leader renamed: {} -> {}",
                                rename_req.from_path, rename_req.to_path
                            );

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
                            info!(
                                "Received CreateSymlink: {} -> {}",
                                symlink_req.link_path, symlink_req.target
                            );

                            let link_path = std::path::PathBuf::from(&symlink_req.link_path);

                            // Lock ordering: Inode -> Index
                            let mut inode_tbl = inode_table_for_handler.write().unwrap();
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
                                uid: symlink_req.uid,
                                gid: symlink_req.gid,
                                modified: std::time::SystemTime::now(),
                                created: std::time::SystemTime::now(),
                                accessed: std::time::SystemTime::now(),
                                chunks: Vec::new(),
                                symlink_target: Some(symlink_req.target.clone()),
                            };

                            // Insert into index
                            index.insert(link_path.clone(), entry.clone());

                            // Add to inode table
                            let mut next_ino = next_inode_for_handler.write().unwrap();
                            let inode = *next_ino;
                            *next_ino += 1;
                            inode_tbl.insert(inode, link_path.clone());

                            info!(
                                "Leader created symlink: {} -> {}",
                                symlink_req.link_path, symlink_req.target
                            );

                            // Record in the changelog so the delta re-sync carries
                            // the symlink too (not just the live broadcast), then
                            // queue the broadcast.
                            drop(index);
                            drop(inode_tbl);
                            drop(next_ino);
                            cluster_for_handler.increment_index_version(link_path.clone());
                            broadcast_queue_for_handler
                                .lock()
                                .unwrap()
                                .push((link_path, entry));

                            Some(Message::FileOpResponse(FileOpResponseMsg {
                                success: true,
                                error: None,
                            }))
                        }
                        Message::SetAttr(setattr_req) => {
                            // Handle setattr request (truncation, chmod, chown, etc.)
                            info!(
                                "Received SetAttr from {}: {} (size={:?})",
                                peer_id, setattr_req.path, setattr_req.size
                            );

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
                                    entry.modified = std::time::UNIX_EPOCH
                                        + std::time::Duration::from_millis(mtime_ms);
                                }

                                info!("Leader applied setattr to {}", setattr_req.path);

                                // Record in the changelog (so delta re-sync carries
                                // this chmod/chown) then queue the broadcast.
                                let entry_clone = entry.clone();
                                drop(index);
                                cluster_for_handler.increment_index_version(path.clone());
                                broadcast_queue_for_handler
                                    .lock()
                                    .unwrap()
                                    .push((path, entry_clone));

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
                            debug!(
                                "Received ReadRequest: {} offset={} size={}",
                                read_req.path, read_req.offset, read_req.size
                            );

                            let file_path = std::path::PathBuf::from(&read_req.path);
                            let index = file_index_for_handler.read().unwrap();

                            match index.get(&file_path) {
                                Some(entry) => {
                                    let chunks = entry.chunks.clone();
                                    drop(index);

                                    match chunk_store_for_handler.read(
                                        &chunks,
                                        read_req.offset,
                                        read_req.size as usize,
                                    ) {
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
                                None => Some(Message::ClientResponse(ClientResponseMsg {
                                    success: false,
                                    data: None,
                                    error: Some("File not found".to_string()),
                                })),
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
                                    let modified_ms = entry
                                        .modified
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis()
                                        as u64;
                                    let created_ms = entry
                                        .created
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis()
                                        as u64;
                                    let accessed_ms = entry
                                        .accessed
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis()
                                        as u64;

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
                                None => Some(Message::GetAttrResponse(GetAttrResponseMsg {
                                    exists: false,
                                    is_dir: false,
                                    size: 0,
                                    permissions: 0,
                                    uid: 0,
                                    gid: 0,
                                    modified_ms: 0,
                                    created_ms: 0,
                                    accessed_ms: 0,
                                })),
                            }
                        }
                        Message::SyncRequest(sync_req) => {
                            // Handle index sync request (from follower/client)

                            // Register the peer for push replication if they sent their identity.
                            // This is critical: without this, the leader can only push to peers
                            // discovered via UDP broadcast. If UDP doesn't work (firewall, different
                            // subnet, LXC container), the leader has no way to push updates.
                            // By registering here, the broadcast/replication threads can establish
                            // outbound connections to the peer.
                            if let (Some(ref nid), Some(ref addr)) =
                                (&sync_req.node_id, &sync_req.bind_address)
                            {
                                // Fix 0.0.0.0 bind addresses — use the actual source IP
                                let actual_addr = if addr.starts_with("0.0.0.0:") {
                                    let port = addr.strip_prefix("0.0.0.0:").unwrap_or("8550");
                                    // peer_id is "ip:port" from the incoming TCP socket
                                    if let Some(ip) = peer_id.rsplit_once(':').map(|(h, _)| h) {
                                        format!("{}:{}", ip, port)
                                    } else {
                                        addr.clone()
                                    }
                                } else {
                                    addr.clone()
                                };

                                info!("SyncRequest from {} — registering peer {} at {} (is_client: {})",
                                        peer_id, nid, actual_addr, sync_req.is_client);
                                cluster_for_handler.update_peer(
                                    wolfdisk::network::discovery::DiscoveredPeer {
                                        node_id: nid.clone(),
                                        address: actual_addr,
                                        role: if sync_req.is_client {
                                            wolfdisk::network::discovery::DiscoveryRole::Client
                                        } else {
                                            wolfdisk::network::discovery::DiscoveryRole::Server
                                        },
                                        is_leader: false,
                                        last_seen: std::time::Instant::now(),
                                    },
                                );
                            }

                            let current_version = cluster_for_handler.index_version();

                            // If follower is already at current version, skip transfer
                            if sync_req.from_version > 0 && sync_req.from_version == current_version
                            {
                                debug!("SyncRequest from {}: already at version {} — no transfer needed", peer_id, current_version);
                                Some(Message::SyncResponse(SyncResponseMsg {
                                    current_version,
                                    entries: Vec::new(),
                                    deleted_paths: Vec::new(),
                                    is_full: false,
                                }))
                            } else if sync_req.from_version > 0 {
                                // Try delta sync — only send files that changed since their version
                                match cluster_for_handler.changes_since(sync_req.from_version) {
                                    Some((changed_paths, deleted_paths_buf)) => {
                                        // Delta sync: send changed entries + deleted paths
                                        let index = file_index_for_handler.read().unwrap();
                                        let mut entries = Vec::new();

                                        for changed_path in &changed_paths {
                                            if let Some(entry) = index.get(changed_path) {
                                                let modified_ms = entry
                                                    .modified
                                                    .duration_since(std::time::UNIX_EPOCH)
                                                    .unwrap_or_default()
                                                    .as_millis()
                                                    as u64;

                                                let chunks: Vec<ChunkRefMsg> = entry
                                                    .chunks
                                                    .iter()
                                                    .map(|c| ChunkRefMsg {
                                                        hash: c.hash,
                                                        offset: c.offset,
                                                        size: c.size,
                                                    })
                                                    .collect();

                                                entries.push(IndexEntryMsg {
                                                    path: changed_path
                                                        .to_string_lossy()
                                                        .to_string(),
                                                    is_dir: entry.is_dir,
                                                    size: entry.size,
                                                    modified_ms,
                                                    permissions: entry.permissions,
                                                    chunks,
                                                    uid: entry.uid,
                                                    gid: entry.gid,
                                                    symlink_target: entry.symlink_target.clone(),
                                                });
                                            }
                                        }

                                        // Convert deleted paths to strings for the response
                                        let deleted_strs: Vec<String> = deleted_paths_buf
                                            .iter()
                                            .map(|p| p.to_string_lossy().to_string())
                                            .collect();

                                        info!("Delta sync to {} — {} changed, {} deleted (version {} → {})", 
                                                peer_id, entries.len(), deleted_strs.len(), sync_req.from_version, current_version);

                                        Some(Message::SyncResponse(SyncResponseMsg {
                                            current_version,
                                            entries,
                                            deleted_paths: deleted_strs,
                                            is_full: false,
                                        }))
                                    }
                                    None => {
                                        // Changelog truncated, fall back to full sync
                                        info!("Changelog too old for {} (version {}), sending full index", 
                                                peer_id, sync_req.from_version);

                                        let index = file_index_for_handler.read().unwrap();
                                        let mut entries = Vec::new();

                                        for (path, entry) in index.iter() {
                                            let modified_ms = entry
                                                .modified
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .unwrap_or_default()
                                                .as_millis()
                                                as u64;

                                            let chunks: Vec<ChunkRefMsg> = entry
                                                .chunks
                                                .iter()
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
                                                uid: entry.uid,
                                                gid: entry.gid,
                                                symlink_target: entry.symlink_target.clone(),
                                            });
                                        }

                                        info!("Sending full SyncResponse with {} entries (version {})", entries.len(), current_version);

                                        Some(Message::SyncResponse(SyncResponseMsg {
                                            current_version,
                                            entries,
                                            deleted_paths: Vec::new(),
                                            // Full index → follower reconciles authoritatively.
                                            is_full: true,
                                        }))
                                    }
                                }
                            } else {
                                // from_version == 0: initial sync, send everything
                                info!(
                                    "Initial SyncRequest from {} - sending full index ({} version)",
                                    peer_id, current_version
                                );

                                let index = file_index_for_handler.read().unwrap();
                                let mut entries = Vec::new();

                                for (path, entry) in index.iter() {
                                    let modified_ms = entry
                                        .modified
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis()
                                        as u64;

                                    let chunks: Vec<ChunkRefMsg> = entry
                                        .chunks
                                        .iter()
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
                                        uid: entry.uid,
                                        gid: entry.gid,
                                        symlink_target: entry.symlink_target.clone(),
                                    });
                                }

                                info!(
                                    "Sending full SyncResponse with {} entries (version {})",
                                    entries.len(),
                                    current_version
                                );

                                Some(Message::SyncResponse(SyncResponseMsg {
                                    current_version,
                                    entries,
                                    deleted_paths: Vec::new(),
                                    // Full index (initial / from_version 0) → authoritative.
                                    is_full: true,
                                }))
                            }
                        }
                        Message::GetChunk(get_chunk) => {
                            // Handle chunk fetch request from follower
                            debug!(
                                "Received GetChunk request from {} for {}",
                                peer_id,
                                hex::encode(get_chunk.hash)
                            );
                            match chunk_store_for_handler.get(&get_chunk.hash) {
                                Ok(data) => Some(Message::ChunkData(ChunkDataMsg {
                                    hash: get_chunk.hash,
                                    data: Some(data),
                                    error: None,
                                })),
                                Err(e) => {
                                    tracing::warn!(
                                        "Chunk {} not found on leader: {}",
                                        hex::encode(get_chunk.hash),
                                        e
                                    );
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
            ));

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
            let file_index_for_broadcast = file_index.clone();
            std::thread::spawn(move || {
                use wolfdisk::network::protocol::{
                    ChunkRefMsg, ChunkWithData, FileSyncMsg, Message, StoreChunkMsg,
                };
                loop {
                    // Check queues every 50ms
                    std::thread::sleep(std::time::Duration::from_millis(50));

                    // Safety valve: Clear queues if they get too large (e.g. after a massive cancelled copy)
                    // This prevents the broadcast thread from stalling for minutes/hours trying to drain million-item queues
                    {
                        let mut q = chunk_stream_queue_for_thread.lock().unwrap();
                        if q.len() > 5000 {
                            tracing::error!("Chunk stream queue overflow ({} items) - clearing to prevent stall", q.len());
                            q.clear();
                        }
                    }
                    {
                        let mut q = broadcast_queue_for_thread.lock().unwrap();
                        if q.len() > 5000 {
                            tracing::error!(
                                "Broadcast queue overflow ({} items) - clearing to prevent stall",
                                q.len()
                            );
                            q.clear();
                        }
                    }

                    // Ensure we have outbound connections to all known peers
                    // (clients need metadata updates; followers need chunks + metadata)
                    let peers = cluster_for_broadcast.peers();
                    for peer in &peers {
                        if peer_manager_for_broadcast.get(&peer.node_id).is_none() {
                            info!(
                                "Broadcast thread: connecting to peer {} at {} (role: {})",
                                peer.node_id,
                                peer.address,
                                if peer.is_client { "client" } else { "follower" }
                            );
                            if let Err(e) =
                                peer_manager_for_broadcast.connect(&peer.node_id, &peer.address)
                            {
                                tracing::warn!(
                                    "Broadcast thread: failed to connect to {} at {}: {}",
                                    peer.node_id,
                                    peer.address,
                                    e
                                );
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

                    if !pending_chunks.is_empty() {
                        let total_bytes: usize = pending_chunks.iter().map(|(_, d)| d.len()).sum();
                        debug!(
                            "Broadcasting {} chunks ({} bytes)",
                            pending_chunks.len(),
                            total_bytes
                        );

                        for (hash, data) in &pending_chunks {
                            let msg = Message::StoreChunk(StoreChunkMsg {
                                hash: *hash,
                                data: data.clone(),
                            });

                            // Optimization: Only send chunks to followers (storage nodes).
                            // Clients don't store chunks locally, so sending them data is a waste of bandwidth.
                            for peer in &peers {
                                if peer.is_client {
                                    continue;
                                }
                                let _ = peer_manager_for_broadcast.send_to(&peer.node_id, &msg);
                            }
                        }
                    }

                    // Second, send metadata-only updates (file growing during writes)
                    let pending_metadata: Vec<_> = {
                        let mut queue = metadata_update_queue_for_thread.lock().unwrap();
                        queue.drain(..).collect()
                    };

                    for (path, entry) in pending_metadata {
                        let modified_ms = entry
                            .modified
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0);

                        let chunk_refs: Vec<ChunkRefMsg> = entry
                            .chunks
                            .iter()
                            .map(|c| ChunkRefMsg {
                                hash: c.hash,
                                offset: c.offset,
                                size: c.size,
                            })
                            .collect();

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
                            symlink_target: entry.symlink_target.clone(),
                        });

                        // Metadata updates go to everyone (Clients need size/mtime updates)
                        peer_manager_for_broadcast.broadcast(&msg);
                    }

                    // Third, drain full broadcasts (creates, deletes, directory syncs)
                    let pending: Vec<_> = {
                        let mut queue = broadcast_queue_for_thread.lock().unwrap();
                        queue.drain(..).collect()
                    };

                    for (path, mut entry) in pending {
                        // Re-read the entry from the current index to get the latest state.
                        {
                            let current_index = file_index_for_broadcast.read().unwrap();
                            if let Some(current_entry) = current_index.get(&path) {
                                entry = current_entry.clone();
                            }
                        }
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
                                symlink_target: None,
                            });

                            info!("Broadcasting FileDelete for {}", path.display());
                            peer_manager_for_broadcast.broadcast(&msg);
                            continue;
                        }

                        // Normal file sync
                        let modified_ms = entry
                            .modified
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0);

                        let chunk_refs: Vec<wolfdisk::network::protocol::ChunkRefMsg> = entry
                            .chunks
                            .iter()
                            .map(|c| wolfdisk::network::protocol::ChunkRefMsg {
                                hash: c.hash,
                                offset: c.offset,
                                size: c.size,
                            })
                            .collect();

                        let total_chunks = entry.chunks.len();
                        const BATCH_SIZE: usize = 4; // ~16MB per batch at 4MB chunks

                        if total_chunks == 0 {
                            // Directory or empty file - send metadata only to everyone
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
                                symlink_target: entry.symlink_target.clone(),
                            });
                            peer_manager_for_broadcast.broadcast(&msg);
                        } else {
                            // Send chunks in batches
                            for (batch_idx, chunk_batch) in
                                entry.chunks.chunks(BATCH_SIZE).enumerate()
                            {
                                let mut chunks_with_data = Vec::with_capacity(chunk_batch.len());
                                for chunk_ref in chunk_batch {
                                    if let Ok(data) = chunk_store_for_broadcast.get(&chunk_ref.hash)
                                    {
                                        chunks_with_data.push(ChunkWithData {
                                            hash: chunk_ref.hash,
                                            data,
                                        });
                                    }
                                }

                                // Create two messages: one with data (for followers), one without (for clients)
                                let msg_full = Message::FileSync(FileSyncMsg {
                                    path: path.to_string_lossy().to_string(),
                                    size: entry.size,
                                    is_dir: entry.is_dir,
                                    permissions: entry.permissions,
                                    uid: entry.uid,
                                    gid: entry.gid,
                                    modified_ms,
                                    chunks: if batch_idx == 0 {
                                        chunk_refs.clone()
                                    } else {
                                        Vec::new()
                                    },
                                    chunk_data: chunks_with_data, // Has Data
                                    symlink_target: entry.symlink_target.clone(),
                                });

                                let msg_meta = Message::FileSync(FileSyncMsg {
                                    path: path.to_string_lossy().to_string(),
                                    size: entry.size,
                                    is_dir: entry.is_dir,
                                    permissions: entry.permissions,
                                    uid: entry.uid,
                                    gid: entry.gid,
                                    modified_ms,
                                    chunks: if batch_idx == 0 {
                                        chunk_refs.clone()
                                    } else {
                                        Vec::new()
                                    },
                                    chunk_data: Vec::new(), // No Data
                                    symlink_target: entry.symlink_target.clone(),
                                });

                                if batch_idx == 0 {
                                    info!(
                                        "Broadcasting FileSync for {} ({} bytes)",
                                        path.display(),
                                        entry.size
                                    );
                                }

                                // Splitted broadcast
                                for peer in &peers {
                                    if peer.is_client {
                                        // Clients get lightweight metadata msg (avoids flooding them with data they don't store)
                                        let _ = peer_manager_for_broadcast
                                            .send_to(&peer.node_id, &msg_meta);
                                    } else {
                                        // Followers get full data
                                        let _ = peer_manager_for_broadcast
                                            .send_to(&peer.node_id, &msg_full);
                                    }
                                }
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
                let sync_bind_address = config.node.bind.clone();

                std::thread::spawn(move || {
                    use tracing::{info, warn};
                    use wolfdisk::network::protocol::*;
                    use wolfdisk::storage::{ChunkRef, FileEntry};

                    info!("Initial sync thread started - waiting for leader discovery...");

                    // Check if we already have data loaded from disk
                    let have_local_data = {
                        let index = sync_file_index.read().unwrap();
                        !index.is_empty()
                    };

                    // How long to wait for a leader depends on our role:
                    // - Client: wait FOREVER (a client is useless without a leader)
                    // - Auto with existing data: wait 10s (we're likely the leader restarting)
                    // - Follower/Auto with no data: wait 30s (we need a leader to get data)
                    let max_wait_iterations = if sync_is_client {
                        u64::MAX // Wait forever
                    } else if have_local_data {
                        20 // 10 seconds
                    } else {
                        60 // 30 seconds
                    };

                    let mut leader_found = false;
                    let mut wait_count = 0u64;
                    let sync_peers = sync_cluster.config().cluster.peers.clone();
                    loop {
                        if wait_count >= max_wait_iterations {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        wait_count += 1;

                        if let Some(lid) = sync_cluster.leader_id() {
                            // Don't try to sync from ourselves
                            if lid != sync_node_id {
                                leader_found = true;
                                break;
                            }
                        }

                        // After 15s with no leader from UDP discovery, try direct TCP probe to configured peers
                        if wait_count == 30 && !sync_peers.is_empty() {
                            info!("UDP discovery hasn't found leader — trying direct TCP probe to {} configured peers", sync_peers.len());
                            for peer_addr in &sync_peers {
                                info!("Probing peer at {} via TCP...", peer_addr);
                                match wolfdisk::network::peer::PeerConnection::connect(
                                    "probe".to_string(),
                                    peer_addr,
                                ) {
                                    Ok(conn) => {
                                        // Try a SyncRequest — if this peer is the leader, it'll respond
                                        let msg = Message::SyncRequest(SyncRequestMsg {
                                            from_version: 0,
                                            node_id: Some(sync_node_id.clone()),
                                            bind_address: Some(sync_bind_address.clone()),
                                            is_client: sync_is_client,
                                        });
                                        match conn.request(&msg) {
                                            Ok(Message::SyncResponse(response)) => {
                                                info!("Direct TCP probe to {} succeeded! Got {} entries — this peer is the leader", peer_addr, response.entries.len());
                                                // Apply the sync response directly
                                                let mut added = 0usize;
                                                {
                                                    let mut index = sync_file_index.write().unwrap();
                                                    let mut inode_tbl = sync_inode_table.write().unwrap();
                                                    let mut next_ino = sync_next_inode.write().unwrap();
                                                    for entry_msg in &response.entries {
                                                        let path = std::path::PathBuf::from(&entry_msg.path);
                                                        let chunk_refs: Vec<ChunkRef> = entry_msg.chunks.iter()
                                                            .map(|c| ChunkRef { hash: c.hash, offset: c.offset, size: c.size })
                                                            .collect();
                                                        let entry = FileEntry {
                                                            size: entry_msg.size,
                                                            is_dir: entry_msg.is_dir,
                                                            permissions: entry_msg.permissions,
                                                            uid: entry_msg.uid, gid: entry_msg.gid,
                                                            modified: std::time::UNIX_EPOCH + std::time::Duration::from_millis(entry_msg.modified_ms),
                                                            created: std::time::SystemTime::now(),
                                                            accessed: std::time::SystemTime::now(),
                                                            chunks: chunk_refs,
                                                            symlink_target: entry_msg.symlink_target.clone(),
                                                        };
                                                        if !index.contains(&path) {
                                                            if inode_tbl.get_inode(&path).is_none() {
                                                                let ino = *next_ino;
                                                                *next_ino += 1;
                                                                inode_tbl.insert(ino, path.clone());
                                                            }
                                                            added += 1;
                                                        }
                                                        index.insert(path, entry);
                                                    }
                                                }
                                                info!("Direct TCP sync complete: {} entries added from {}", added, peer_addr);
                                                // Publish the synced version so reporting is correct
                                                // and the re-sync thread seeds from it (avoids a
                                                // redundant full sync). (adversarial review W4)
                                                sync_cluster.set_index_version(response.current_version);
                                                // Store the peer address as leader so future ops can find it
                                                sync_cluster.add_peer_as_leader(peer_addr);
                                                leader_found = true;
                                                break;
                                            }
                                            Ok(_) => info!("Peer {} responded but is not leader", peer_addr),
                                            Err(e) => info!("Peer {} SyncRequest failed: {} (may not be leader)", peer_addr, e),
                                        }
                                    }
                                    Err(e) => info!("TCP connect to {} failed: {}", peer_addr, e),
                                }
                            }
                            if leader_found {
                                break;
                            }
                        }

                        // Log progress for clients waiting a long time
                        if sync_is_client && wait_count.is_multiple_of(20) {
                            info!("Client waiting for leader... ({}s elapsed)", wait_count / 2);
                        }
                    }

                    if !leader_found {
                        if have_local_data {
                            info!("No leader found but we have {} local files - marking sync complete (will become leader)", 
                                sync_file_index.read().unwrap().len());
                        } else {
                            info!(
                                "No leader found after waiting - marking sync complete (sole node)"
                            );
                        }
                        sync_cluster.set_sync_complete();
                        return;
                    }

                    let leader_id = sync_cluster.leader_id().unwrap();

                    // Don't sync from ourselves
                    if leader_id == sync_node_id {
                        info!("We are the leader - marking sync complete");
                        sync_cluster.set_sync_complete();
                        return;
                    }

                    let leader_addr = match sync_cluster.leader_address() {
                        Some(addr) => addr,
                        None => {
                            warn!("Leader found but no address available");
                            sync_cluster.set_sync_complete();
                            return;
                        }
                    };

                    info!(
                        "Leader discovered: {} at {} - requesting initial sync",
                        leader_id, leader_addr
                    );

                    // Connect to leader and request sync
                    match sync_peer_manager.get_or_connect_leader(&leader_id, &leader_addr) {
                        Ok(conn) => {
                            let msg = Message::SyncRequest(SyncRequestMsg {
                                from_version: 0,
                                node_id: Some(sync_node_id.clone()),
                                bind_address: Some(sync_bind_address.clone()),
                                is_client: sync_is_client,
                            });
                            match conn.request(&msg) {
                                Ok(Message::SyncResponse(response)) => {
                                    info!(
                                        "Received SyncResponse with {} entries from leader",
                                        response.entries.len()
                                    );

                                    // ALL ROLES: Merge semantics (add/update only, never delete).
                                    // - Workers store actual chunk data locally
                                    // - Clients keep the same index for FUSE lookups but pull
                                    //   actual file data from the leader on read
                                    // Both maintain the same index structure. Deletions are
                                    // handled by real-time DeleteFile/DeleteDir broadcasts.
                                    let mut added = 0usize;
                                    let mut updated = 0usize;
                                    let mut unchanged = 0usize;

                                    // Apply in batches of 50 to avoid starving FUSE operations
                                    let batch_size = 50;
                                    for batch in response.entries.chunks(batch_size) {
                                        let mut index = sync_file_index.write().unwrap();
                                        let mut inode_tbl = sync_inode_table.write().unwrap();
                                        let mut next_ino = sync_next_inode.write().unwrap();

                                        for entry_msg in batch {
                                            let path = std::path::PathBuf::from(&entry_msg.path);

                                            let chunk_refs: Vec<ChunkRef> = entry_msg
                                                .chunks
                                                .iter()
                                                .map(|c| ChunkRef {
                                                    hash: c.hash,
                                                    offset: c.offset,
                                                    size: c.size,
                                                })
                                                .collect();

                                            // Only add/update, never delete
                                            let should_update = match index.get(&path) {
                                                None => true,
                                                Some(existing) => {
                                                    // Also detect metadata-only changes (chmod,
                                                    // chown, symlink target) — not just size/chunks
                                                    // — or chmod/chown never replicated on re-sync
                                                    // (wabil 2026-06-28).
                                                    existing.size != entry_msg.size
                                                        || existing.chunks.len()
                                                            != entry_msg.chunks.len()
                                                        || existing.permissions
                                                            != entry_msg.permissions
                                                        || existing.uid != entry_msg.uid
                                                        || existing.gid != entry_msg.gid
                                                        || existing.symlink_target
                                                            != entry_msg.symlink_target
                                                }
                                            };

                                            if should_update {
                                                let is_new = !index.contains(&path);

                                                let entry = FileEntry {
                                                    size: entry_msg.size,
                                                    is_dir: entry_msg.is_dir,
                                                    permissions: entry_msg.permissions,
                                                    uid: entry_msg.uid,
                                                    gid: entry_msg.gid,
                                                    modified: std::time::UNIX_EPOCH
                                                        + std::time::Duration::from_millis(
                                                            entry_msg.modified_ms,
                                                        ),
                                                    created: std::time::SystemTime::now(),
                                                    accessed: std::time::SystemTime::now(),
                                                    chunks: chunk_refs,
                                                    symlink_target: entry_msg.symlink_target.clone(),
                                                };

                                                index.insert(path.clone(), entry);

                                                if is_new {
                                                    if inode_tbl.get_inode(&path).is_none() {
                                                        let ino = *next_ino;
                                                        *next_ino += 1;
                                                        inode_tbl.insert(ino, path);
                                                    }
                                                    added += 1;
                                                } else {
                                                    updated += 1;
                                                }
                                            } else {
                                                unchanged += 1;
                                            }
                                        }
                                        // Locks dropped here — FUSE ops can proceed between batches
                                    }

                                    info!("Initial sync complete: {} added, {} updated, {} unchanged (total from leader: {})",
                                        added, updated, unchanged, response.entries.len());

                                    // Apply deletions from the leader's changelog
                                    if !response.deleted_paths.is_empty() {
                                        let mut removed = 0usize;
                                        for del_batch in response.deleted_paths.chunks(50) {
                                            let mut index = sync_file_index.write().unwrap();
                                            let mut inode_tbl = sync_inode_table.write().unwrap();
                                            for del_path_str in del_batch {
                                                let del_path =
                                                    std::path::PathBuf::from(del_path_str);
                                                if index.remove(&del_path).is_some() {
                                                    inode_tbl.remove_path(&del_path);
                                                    removed += 1;
                                                }
                                            }
                                            // Locks dropped between batches
                                        }
                                        if removed > 0 {
                                            info!("Initial sync: removed {} deleted entries from leader changelog", removed);
                                        }
                                    }

                                    // Publish the synced version only AFTER entries AND
                                    // deletions are applied — otherwise the re-sync thread
                                    // (which seeds from index_version) could skip these
                                    // deletions in its first delta. (adversarial review C2)
                                    sync_cluster.set_index_version(response.current_version);
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

                    // Mark sync complete regardless of outcome so election can proceed
                    sync_cluster.set_sync_complete();
                });
            }

            // Periodic re-sync thread: workers pull from leader every 30s
            // This catches any missed broadcasts (e.g., worker was offline)
            if should_sync || is_auto_role {
                let resync_cluster = cluster.clone();
                let resync_peer_manager = peer_manager.clone();
                let resync_file_index = file_index.clone();
                let resync_inode_table = inode_table.clone();
                let resync_next_inode = next_inode.clone();
                let resync_node_id = config.node.id.clone();
                let resync_bind_address = config.node.bind.clone();
                let resync_is_client = config.node.role == wolfdisk::config::NodeRole::Client;

                std::thread::spawn(move || {
                    use tracing::{debug, info, warn};
                    use wolfdisk::network::protocol::*;
                    use wolfdisk::storage::{ChunkRef, FileEntry};

                    // Wait for initial sync to complete first
                    std::thread::sleep(std::time::Duration::from_secs(15));

                    // Track last synced version — if leader version matches, no transfer
                    // needed. Seed from the cluster's current version (set by the initial
                    // sync) so a restart doesn't force a redundant full sync.
                    let mut last_synced_version: u64 = resync_cluster.index_version();
                    // Whether we're currently deferring an authoritative reconcile because the
                    // leader regressed below us. Used to WARN only on the transition INTO that
                    // state (a leader restart can take minutes to reload — we must not WARN-spam
                    // every 10s; "log state changes, not heartbeats").
                    let mut deferral_active = false;
                    // Last leader-connect error, so a PERSISTENT failure is a
                    // visible WARN (once, on change) instead of debug-only —
                    // a node that can't reach its leader was completely silent
                    // at default log level while stuck forever (wabil
                    // 2026-07-04). Recovery logs INFO and re-arms.
                    let mut last_connect_err: Option<String> = None;

                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(10));

                        // Skip if we're the leader
                        if resync_cluster.is_leader() {
                            continue;
                        }

                        let leader_id = match resync_cluster.leader_id() {
                            Some(id) if id != resync_node_id => id,
                            _ => continue,
                        };

                        let leader_addr = match resync_cluster.leader_address() {
                            Some(addr) => addr,
                            None => continue,
                        };

                        debug!(
                            "Periodic re-sync: checking leader {} (our version: {})",
                            leader_id, last_synced_version
                        );

                        let conn = match resync_peer_manager
                            .get_or_connect_leader(&leader_id, &leader_addr)
                        {
                            Ok(c) => {
                                if last_connect_err.take().is_some() {
                                    info!("Re-sync: connection to leader {} recovered", leader_id);
                                }
                                c
                            }
                            Err(e) => {
                                let msg = e.to_string();
                                if last_connect_err.as_deref() != Some(msg.as_str()) {
                                    warn!(
                                        "Re-sync: cannot reach leader {} at {}: {} — will keep retrying every 10s (check connectivity and that all nodes run the same wolfdisk version)",
                                        leader_id, leader_addr, msg
                                    );
                                    last_connect_err = Some(msg);
                                }
                                continue;
                            }
                        };

                        let msg = Message::SyncRequest(SyncRequestMsg {
                            from_version: last_synced_version,
                            node_id: Some(resync_node_id.clone()),
                            bind_address: Some(resync_bind_address.clone()),
                            is_client: resync_is_client,
                        });
                        match conn.request(&msg) {
                            Ok(Message::SyncResponse(response)) => {
                                // If versions match and no entries, we're up to date
                                if response.entries.is_empty() {
                                    debug!(
                                        "Periodic re-sync: already at version {}, no changes",
                                        response.current_version
                                    );
                                    // MONOTONIC (see the post-reconcile note): never lower our version
                                    // to match a leader reporting a version below ours. Otherwise a
                                    // regressed leader's "you're caught up" reply would drag us down and
                                    // the next full sync would delete our data.
                                    if response.current_version >= last_synced_version {
                                        last_synced_version = response.current_version;
                                        resync_cluster.set_index_version(last_synced_version);
                                    }
                                    continue;
                                }

                                let mut added = 0;
                                let mut updated = 0;

                                // Apply in batches of 50 to avoid starving FUSE operations.
                                // readdir/lookup/getattr need read locks on file_index & inode_table;
                                // holding write locks for thousands of entries causes hangs.
                                let batch_size = 50;
                                for chunk in response.entries.chunks(batch_size) {
                                    let mut index = resync_file_index.write().unwrap();
                                    let mut inode_tbl = resync_inode_table.write().unwrap();
                                    let mut next_ino = resync_next_inode.write().unwrap();

                                    for entry_msg in chunk {
                                        let path = std::path::PathBuf::from(&entry_msg.path);

                                        let chunk_refs: Vec<ChunkRef> = entry_msg
                                            .chunks
                                            .iter()
                                            .map(|c| ChunkRef {
                                                hash: c.hash,
                                                offset: c.offset,
                                                size: c.size,
                                            })
                                            .collect();

                                        let new_entry = FileEntry {
                                            size: entry_msg.size,
                                            is_dir: entry_msg.is_dir,
                                            permissions: entry_msg.permissions,
                                            uid: entry_msg.uid,
                                            gid: entry_msg.gid,
                                            modified: std::time::UNIX_EPOCH
                                                + std::time::Duration::from_millis(
                                                    entry_msg.modified_ms,
                                                ),
                                            created: std::time::SystemTime::now(),
                                            accessed: std::time::SystemTime::now(),
                                            chunks: chunk_refs,
                                            symlink_target: entry_msg.symlink_target.clone(),
                                        };

                                        // Only update if missing or if leader has newer/different data
                                        match index.get(&path) {
                                            None => {
                                                index.insert(path.clone(), new_entry);
                                                let ino = *next_ino;
                                                *next_ino += 1;
                                                inode_tbl.insert(ino, path.clone());
                                                added += 1;
                                            }
                                            Some(existing)
                                                if existing.size != entry_msg.size
                                                    || existing.chunks.len()
                                                        != entry_msg.chunks.len()
                                                    || existing.permissions
                                                        != entry_msg.permissions
                                                    || existing.uid != entry_msg.uid
                                                    || existing.gid != entry_msg.gid
                                                    || existing.symlink_target
                                                        != entry_msg.symlink_target =>
                                            {
                                                index.insert(path.clone(), new_entry);
                                                // Ensure inode exists
                                                if inode_tbl.get_inode(&path).is_none() {
                                                    let ino = *next_ino;
                                                    *next_ino += 1;
                                                    inode_tbl.insert(ino, path.clone());
                                                }
                                                updated += 1;
                                            }
                                            _ => {} // Already in sync
                                        }
                                    }
                                    // Locks automatically dropped here, allowing FUSE ops between batches
                                }

                                // Apply deletions from the leader's changelog
                                let mut removed = 0usize;
                                if !response.deleted_paths.is_empty() {
                                    for del_batch in response.deleted_paths.chunks(50) {
                                        let mut index = resync_file_index.write().unwrap();
                                        let mut inode_tbl = resync_inode_table.write().unwrap();
                                        for del_path_str in del_batch {
                                            let del_path = std::path::PathBuf::from(del_path_str);
                                            if index.remove(&del_path).is_some() {
                                                inode_tbl.remove_path(&del_path);
                                                removed += 1;
                                            }
                                        }
                                        // Locks dropped between batches
                                    }
                                }

                                // Authoritative full sync: the leader sent its COMPLETE index, so
                                // any local entry it did NOT send is stale and must go. This is what
                                // makes a follower actually CONVERGE after it falls out of changelog
                                // range or misses a bulk delete — the old code only ever added/updated
                                // and applied changelog deletions, so a follower holding 250k stale
                                // files re-received the leader's small index every cycle and kept the
                                // extras forever (wabil 2026-06-29). Snapshot local paths under a
                                // brief read lock, diff against the leader's set off-lock, then delete
                                // in small batches so FUSE ops aren't starved on a huge index.
                                // Authoritative deletion is the one place a follower can LOSE data,
                                // so full_sync_may_delete() gates it on TWO things: a non-empty leader
                                // index (never wipe to match a failed-to-load leader), AND the leader
                                // not having regressed below our version (never delete our more-advanced
                                // data to match a leader that just restarted before its persisted
                                // version reloaded — the v0/226-entry blip in wabil's logs, 2026-06-29).
                                // Additions are already applied above either way; deferring only delays
                                // convergence a cycle, it never loses data.
                                if response.is_full
                                    && full_sync_may_delete(response.current_version, last_synced_version, response.entries.len())
                                {
                                    let keep: std::collections::HashSet<std::path::PathBuf> = response
                                        .entries
                                        .iter()
                                        .map(|e| std::path::PathBuf::from(&e.path))
                                        .collect();
                                    let stale: Vec<std::path::PathBuf> = {
                                        let index = resync_file_index.read().unwrap();
                                        full_sync_stale_paths(&keep, index.paths())
                                    };
                                    for del_batch in stale.chunks(50) {
                                        let mut index = resync_file_index.write().unwrap();
                                        let mut inode_tbl = resync_inode_table.write().unwrap();
                                        for del_path in del_batch {
                                            if index.remove(del_path).is_some() {
                                                inode_tbl.remove_path(del_path);
                                                removed += 1;
                                            }
                                        }
                                    }
                                    deferral_active = false; // reconciled normally — clear any prior deferral
                                } else if response.is_full
                                    && !response.entries.is_empty()
                                    && response.current_version < last_synced_version
                                {
                                    // Leader regressed below us — keep our data, don't reconcile-delete.
                                    // WARN only on the transition into deferral; a leader restart can sit
                                    // here for minutes while it reloads, so subsequent ticks log at debug.
                                    let local_len = resync_file_index.read().unwrap().len();
                                    if !deferral_active {
                                        warn!(
                                            "Deferring authoritative reconcile: leader version {} is below ours {} \
                                             (leader sent {} entries, we hold {}). Leader may be recovering — \
                                             keeping local entries to avoid data loss.",
                                            response.current_version, last_synced_version,
                                            response.entries.len(), local_len
                                        );
                                        deferral_active = true;
                                    } else {
                                        debug!(
                                            "Still deferring reconcile: leader version {} below ours {} ({} entries)",
                                            response.current_version, last_synced_version, response.entries.len()
                                        );
                                    }
                                } else {
                                    deferral_active = false; // any normal full/delta apply clears deferral
                                }

                                // Update our version to the leader's version, and publish it
                                // to the cluster so the health UI reflects real progress.
                                // Previously this stayed in this thread's local variable, so a
                                // follower that had applied all the data still reported v0 and
                                // showed as "N behind" forever (wabil 2026-06-28).
                                // MONOTONIC: never LOWER our version to match a leader that has
                                // regressed below us. Adopting a regressed leader's version would let
                                // the next cycle's full sync pass full_sync_may_delete() and delete our
                                // more-advanced data — the second half of the regressed-leader trap.
                                if response.current_version >= last_synced_version {
                                    last_synced_version = response.current_version;
                                    resync_cluster.set_index_version(last_synced_version);
                                }

                                if added > 0 || updated > 0 || removed > 0 {
                                    info!("Periodic re-sync: added {}, updated {}, removed {} (now at version {})", added, updated, removed, last_synced_version);
                                } else {
                                    debug!(
                                        "Periodic re-sync: index verified ({} entries, version {})",
                                        response.entries.len(),
                                        last_synced_version
                                    );
                                }
                            }
                            Ok(_) => {}
                            Err(e) => {
                                info!(
                                    "Re-sync failed: {} — dropping stale connection to leader",
                                    e
                                );
                                resync_peer_manager.disconnect_leader(&leader_id);
                            }
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
            let status_file_index = file_index.clone();
            std::thread::spawn(move || {
                // file_count is O(1) (len); total_size is an O(n) sum over the
                // ENTIRE index. Walking 250k+ entries under the read lock every
                // second blocked EVERY writer for the walk's duration (and the
                // writers starved this thread back) — on a bulk small-files copy
                // that stalled replication/applies and made the node report
                // "stale" while alive (wabil 2026-06-28). Two changes: (1)
                // try_read so this thread NEVER blocks the apply path — if a
                // writer holds the lock we reuse the last values and still write
                // a FRESH status (the node keeps reporting); (2) recompute the
                // expensive size only every ~15s, not every second.
                let mut cached_count: usize = 0;
                let mut cached_size: u64 = 0;
                let mut tick: u64 = 0;
                while std::sync::Arc::strong_count(&status_cluster) > 1 {
                    if let Ok(index) = status_file_index.try_read() {
                        cached_count = index.len();
                        if tick.is_multiple_of(15) {
                            cached_size = index.iter().map(|(_, e)| e.size).sum();
                        }
                    }
                    status_cluster.write_status_file_with_stats(cached_count, cached_size);
                    tick = tick.wrapping_add(1);
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            });

            // Start periodic index persistence thread
            // This ensures ALL changes (including replication-received updates) are saved to disk.
            // The FUSE filesystem has its own save logic, but the message handler modifies
            // the index directly without setting the FUSE dirty flag.
            let persist_file_index = file_index.clone();
            let persist_index_dir = config.index_dir().to_path_buf();
            let persist_cluster = cluster.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(5));
                if std::sync::Arc::strong_count(&persist_file_index) <= 1 {
                    break;
                }
                // Persist the replication version + changelog so a restart resumes
                // at the right version instead of resetting to v0 (wabil 2026-06-29).
                if let Err(e) = persist_cluster.save_replication_state(&persist_index_dir) {
                    tracing::warn!("Failed to persist replication state: {}", e);
                }
                // Serialize under the read lock (CPU only), then write to disk
                // AFTER releasing it. The old code held the read lock across the
                // whole disk write of a 250k-entry index every 5s, blocking every
                // writer (the replication apply path) for the entire save — a
                // periodic freeze that, on a bulk small-files copy, starved
                // followers into "stale" and tanked throughput (wabil 2026-06-28).
                let serialized = {
                    let index = persist_file_index.read().unwrap();
                    let n = index.len();
                    (index.serialize(), n)
                };
                match serialized {
                    (Ok(bytes), n) => {
                        if let Err(e) = wolfdisk::storage::FileIndex::write_serialized(
                            &persist_index_dir,
                            &bytes,
                        ) {
                            tracing::warn!("Failed to persist index: {}", e);
                        } else {
                            tracing::debug!("Periodic index save: {} entries", n);
                        }
                    }
                    (Err(e), _) => tracing::warn!("Failed to serialize index: {}", e),
                }
            });

            // Start S3-compatible API server if enabled
            if config.s3.enabled {
                let s3_file_index = file_index.clone();
                let s3_chunk_store = chunk_store.clone();
                let s3_inode_table = inode_table.clone();
                let s3_next_inode = next_inode.clone();
                let s3_bind = config.s3.bind.clone();
                let s3_credentials = config.s3.credentials();
                let s3_region = config.s3.region.clone();
                let s3_meta_path = wolfdisk::s3::meta::meta_path(&config.index_dir());
                let s3_buckets = config.s3.buckets.clone();

                std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(2)
                        .enable_all()
                        .build()
                        .expect("Failed to create S3 tokio runtime");

                    rt.block_on(async {
                        let server = wolfdisk::s3::S3Server::new(
                            s3_bind,
                            s3_file_index,
                            s3_chunk_store,
                            s3_inode_table,
                            s3_next_inode,
                            s3_credentials,
                            s3_region,
                            s3_meta_path,
                            s3_buckets,
                        );

                        if let Err(e) = server.run().await {
                            error!("S3 server failed: {}", e);
                        }
                    });
                });

                info!("S3-compatible API enabled on {}", config.s3.bind);
            }

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
            println!();
            println!("  WolfDisk Status");
            println!("  {}", "─".repeat(50));
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
                && std::fs::read_dir(mountpoint).is_ok();

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
            println!();
            println!("  WolfDisk Live Stats");
            println!("  {}", "─".repeat(50));
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
            })
            .expect("Failed to set Ctrl+C handler");

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
                    println!(
                        "Leader:       {}{}",
                        leader,
                        if is_me { " (this node)" } else { "" }
                    );
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
                    let status = if peer.last_seen.elapsed().as_secs() < 4 {
                        "●"
                    } else {
                        "○"
                    };
                    let role = if peer.is_leader {
                        "leader"
                    } else if peer.is_client {
                        "client"
                    } else {
                        "follower"
                    };
                    let ago = peer.last_seen.elapsed().as_secs();
                    println!(
                        "  {} {} - {} (seen {}s ago)",
                        status, peer.node_id, role, ago
                    );
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

            println!();
            println!("  WolfDisk Servers");
            println!("  {}", "─".repeat(50));
            println!();

            // Print this node first
            let my_role = if cluster.is_leader() {
                "LEADER"
            } else if cluster.state() == wolfdisk::cluster::ClusterState::Client {
                "CLIENT"
            } else {
                "FOLLOWER"
            };
            println!(
                "  ● {:15} {:22} {}",
                config.node.id, config.node.bind, my_role
            );

            for peer in &peers {
                let status = if peer.last_seen.elapsed().as_secs() < 4 {
                    "●"
                } else {
                    "○"
                };
                let role = if peer.is_leader {
                    "LEADER"
                } else if peer.is_client {
                    "CLIENT"
                } else {
                    "FOLLOWER"
                };
                println!(
                    "  {} {:15} {:22} {}",
                    status, peer.node_id, peer.address, role
                );
            }

            println!();
            println!("  Total: {} server(s)", peers.len() + 1);

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

#[cfg(test)]
mod full_sync_tests {
    use super::{full_sync_may_delete, full_sync_stale_paths};
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn p(s: &str) -> PathBuf { PathBuf::from(s) }

    #[test]
    fn may_delete_only_when_leader_is_credible() {
        // Normal: leader at/ahead of us with real entries → reconcile (delete) ok.
        assert!(full_sync_may_delete(1000, 1000, 226));   // equal version
        assert!(full_sync_may_delete(2000, 1000, 50));    // leader ahead
        assert!(full_sync_may_delete(1, 0, 5));           // fresh-ish, leader ahead
        // v0/v0 base case: a fresh/fully-reset cluster MUST still reconcile (the
        // original self-heal). Pinned so a future `>=`→`>` "tightening" can't
        // silently break it.
        assert!(full_sync_may_delete(0, 0, 226));

        // Failed-to-load leader (0 entries) → never wipe to match it.
        assert!(!full_sync_may_delete(5000, 1000, 0));

        // The wabil data-loss case: leader regressed to v0 with a small index
        // while we were caught up at a high version → MUST NOT delete our data.
        assert!(!full_sync_may_delete(0, 1_050_909, 226));
        // Any leader regression below us is refused, regardless of entry count.
        assert!(!full_sync_may_delete(900, 1000, 100_000));
    }

    #[test]
    fn deletes_only_paths_absent_from_leader() {
        // Leader's authoritative set (post bulk-delete): two files.
        let leader: HashSet<PathBuf> = [p("/keep/a"), p("/keep/b")].into_iter().collect();
        // Follower still holds the leader's two PLUS three stale extras.
        let local = vec![p("/keep/a"), p("/keep/b"), p("/old/1"), p("/old/2"), p("/old/3")];
        let mut stale = full_sync_stale_paths(&leader, local.iter());
        stale.sort();
        assert_eq!(stale, vec![p("/old/1"), p("/old/2"), p("/old/3")]);
    }

    #[test]
    fn keeps_everything_when_follower_matches_leader() {
        let leader: HashSet<PathBuf> = [p("/a"), p("/b")].into_iter().collect();
        let local = vec![p("/a"), p("/b")];
        assert!(full_sync_stale_paths(&leader, local.iter()).is_empty());
    }
}
