//! WolfScale - Distributed MariaDB Synchronization Manager
//!
//! A high-performance Rust application that keeps multiple MariaDB
//! databases in sync using a Write-Ahead Log (WAL).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use tokio::sync::RwLock;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use wolfscale::config::WolfScaleConfig;
use wolfscale::wal::{WalWriter, WalReader};
use wolfscale::state::{StateTracker, ClusterMembership, ElectionConfig};
use wolfscale::executor::MariaDbExecutor;
use wolfscale::api::HttpServer;
use wolfscale::network::{NetworkServer, NetworkClient};
use wolfscale::replication::{LeaderNode, FollowerNode, ReplicationConfig};
use wolfscale::proxy::{ProxyServer, ProxyConfig};
use wolfscale::error::Result;

/// WolfScale - Distributed MariaDB Synchronization Manager
#[derive(Parser)]
#[command(name = "wolfscale")]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, default_value = "wolfscale.toml")]
    config: PathBuf,

    /// Log level (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info")]
    log_level: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the WolfScale node
    Start {
        /// Force start as leader (for initial cluster bootstrap)
        #[arg(long)]
        bootstrap: bool,
    },
    
    /// Join an existing cluster
    Join {
        /// Leader address to join (host:port)
        leader: String,
    },
    
    /// Check cluster status
    Status {
        /// Node address to query (defaults to localhost)
        #[arg(short, long, default_value = "localhost:8080")]
        address: String,
    },
    
    /// Force synchronization check
    Sync {
        /// Target node address
        #[arg(short, long, default_value = "localhost:8080")]
        address: String,
    },
    
    /// Initialize a new configuration file
    Init {
        /// Output path for configuration file
        #[arg(short, long, default_value = "wolfscale.toml")]
        output: PathBuf,
        
        /// Node ID
        #[arg(long, default_value = "node-1")]
        node_id: String,
    },
    
    /// Validate configuration file
    Validate,
    
    /// Show node information
    Info,
    
    /// Start MySQL protocol proxy
    Proxy {
        /// Address to listen on
        #[arg(short, long, default_value = "0.0.0.0:8007")]
        listen: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    init_logging(&cli.log_level);

    match cli.command {
        Commands::Start { bootstrap } => {
            run_start(cli.config, bootstrap).await
        }
        Commands::Join { leader } => {
            run_join(cli.config, leader).await
        }
        Commands::Status { address } => {
            run_status(address).await
        }
        Commands::Sync { address } => {
            run_sync(address).await
        }
        Commands::Init { output, node_id } => {
            run_init(output, node_id)
        }
        Commands::Validate => {
            run_validate(cli.config)
        }
        Commands::Info => {
            run_info(cli.config)
        }
        Commands::Proxy { listen } => {
            run_proxy(cli.config, listen).await
        }
    }
}

/// Initialize logging
fn init_logging(level: &str) {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| level.into());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .init();
}

/// Start the WolfScale node
async fn run_start(config_path: PathBuf, bootstrap: bool) -> Result<()> {
    // Print ASCII art banner
    println!(r#"
 █     █░ ▒█████   ██▓      █████▒ ██████  ▄████▄   ▄▄▄       ██▓    ▓█████ 
▓█░ █ ░█░▒██▒  ██▒▓██▒    ▓██   ▒▒██    ▒ ▒██▀ ▀█  ▒████▄    ▓██▒    ▓█   ▀ 
▒█░ █ ░█ ▒██░  ██▒▒██░    ▒████ ░░ ▓██▄   ▒ ▓███▄░▒██  ▀█▄  ▒██░    ▒███   
░█░ █ ░█ ▒██   ██░▒██░    ░▓█▒  ░  ▒   ██▒░▓█  ▀█▓░██▄▄▄▄██ ▒██░    ▒▓█  ▄ 
░░██▒██▓ ░ ████▓▒░░██████▒░▒█░   ▒██████▒▒░▒▓███▀▒ ▓█   ▓██▒░██████▒░▒████▒
░ ▓░▒ ▒  ░ ▒░▒░▒░ ░ ▒░▓  ░ ▒ ░   ▒ ▒▓▒ ▒ ░ ░▒   ▒  ▒▒   ▓▒█░░ ▒░▓  ░░░ ▒░ ░
  ▒ ░ ░    ░ ▒ ▒░ ░ ░ ▒  ░ ░     ░ ░▒  ░ ░  ░   ░   ▒   ▒▒ ░░ ░ ▒  ░ ░ ░  ░
  ░   ░  ░ ░ ░ ▒    ░ ░    ░ ░   ░  ░  ░  ░ ░   ░   ░   ▒     ░ ░      ░   
    ░        ░ ░      ░  ░             ░        ░       ░  ░    ░  ░   ░  ░
                                                                           
        (C) Wolf Software Systems Ltd -- https://wolf.uk.com
"#);

    tracing::info!("Starting WolfScale node...");

    // Load configuration
    let config = match WolfScaleConfig::from_file(&config_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to load configuration from {:?}: {}", config_path, e);
            tracing::error!("Please check that the config file exists and is valid TOML");
            return Err(e);
        }
    };
    tracing::info!("Loaded configuration for node: {}", config.node.id);

    // Ensure directories exist
    if let Err(e) = std::fs::create_dir_all(config.data_dir()) {
        tracing::error!("Failed to create data directory {:?}: {}", config.data_dir(), e);
        return Err(e.into());
    }
    if let Err(e) = std::fs::create_dir_all(config.wal_dir()) {
        tracing::error!("Failed to create WAL directory {:?}: {}", config.wal_dir(), e);
        return Err(e.into());
    }
    if let Err(e) = std::fs::create_dir_all(config.state_dir()) {
        tracing::error!("Failed to create state directory {:?}: {}", config.state_dir(), e);
        return Err(e.into());
    }

    // Initialize WAL
    let wal_writer = match WalWriter::new(
        config.data_dir().clone(),
        config.wal.clone(),
        config.node.id.clone(),
    ).await {
        Ok(w) => w,
        Err(e) => {
            tracing::error!("Failed to initialize WAL: {}", e);
            return Err(e);
        }
    };
    tracing::info!("WAL initialized, current LSN: {}", wal_writer.current_lsn().await);

    let wal_reader = match WalReader::new(
        config.data_dir().clone(),
        config.wal.segment_size_mb,
        config.wal.compression,
    ) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to initialize WAL reader: {}", e);
            return Err(e);
        }
    };
    // Create a second WAL reader for sync requests (shared across async tasks)
    let sync_wal_reader = match WalReader::new(
        config.data_dir().clone(),
        config.wal.segment_size_mb,
        config.wal.compression,
    ) {
        Ok(r) => Arc::new(tokio::sync::RwLock::new(r)),
        Err(e) => {
            tracing::error!("Failed to initialize sync WAL reader: {}", e);
            return Err(e);
        }
    };
    let _shared_wal_reader = Arc::clone(&sync_wal_reader);

    // Initialize state tracker
    let state_tracker = match StateTracker::new(
        config.state_dir(),
        config.node.id.clone(),
    ) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::error!("Failed to initialize state tracker: {}", e);
            return Err(e);
        }
    };
    tracing::info!("State tracker initialized, last applied LSN: {}", 
        state_tracker.last_applied_lsn().await?);

    // Initialize cluster membership
    // heartbeat_timeout = 15x heartbeat_interval to allow for slow DDL operations
    let cluster = Arc::new(ClusterMembership::new(
        config.node.id.clone(),
        config.advertise_address().to_string(),
        config.heartbeat_interval() * 15,  // Timeout is 15x heartbeat interval (3s at 200ms interval)
        config.election_timeout(),
    ));

    // Add configured peers (automatically filter out our own address)
    let own_address = config.advertise_address();
    for peer in &config.cluster.peers {
        // Skip if this peer is ourselves
        if peer == own_address {
            tracing::debug!("Skipping peer {} (that's us)", peer);
            continue;
        }
        let peer_id = format!("peer-{}", peer.replace(':', "-"));
        cluster.add_peer(peer_id, peer.clone()).await?;
    }
    tracing::info!("Cluster initialized with {} nodes", cluster.size().await);

    // Initialize database executor
    tracing::info!("Connecting to MariaDB at {}:{}...", config.database.host, config.database.port);
    let executor = match MariaDbExecutor::new(&config.database).await {
        Ok(e) => Arc::new(e),
        Err(e) => {
            tracing::error!("Failed to connect to MariaDB: {}", e);
            tracing::error!("  Host: {}:{}", config.database.host, config.database.port);
            tracing::error!("  User: {}", config.database.user);
            tracing::error!("Please check that MariaDB is running and credentials are correct");
            return Err(e);
        }
    };
    match executor.health_check().await {
        Ok(true) => tracing::info!("Database connection established"),
        Ok(false) => tracing::warn!("Database health check returned false"),
        Err(e) => {
            tracing::error!("Database health check failed: {}", e);
            return Err(e);
        }
    }

    // Initialize network - separate channels for incoming and outgoing messages
    // Outgoing: used by leader to queue messages to send to peers
    let (outgoing_tx, mut outgoing_rx) = tokio::sync::mpsc::channel::<(String, wolfscale::replication::Message)>(10000);
    // Incoming: used by network server to receive messages from peers
    let (incoming_tx, mut incoming_rx) = tokio::sync::mpsc::channel(10000);
    // Shared heartbeat timestamp: updated by incoming loop, read by follower to reset election timer
    let shared_heartbeat_time = Arc::new(std::sync::atomic::AtomicU64::new(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    ));
    
    let network_server = NetworkServer::new(
        config.node.bind_address.clone(),
        incoming_tx,
    );

    // Create network client for outbound messages
    let network_client = Arc::new(NetworkClient::new(
        Duration::from_secs(5),   // connect timeout
        Duration::from_secs(10),  // request timeout
    ));

    // Start OUTGOING message delivery loop - sends queued messages to peers
    let delivery_client = Arc::clone(&network_client);
    tokio::spawn(async move {
        while let Some((target_address, message)) = outgoing_rx.recv().await {
            tracing::trace!("SENDING {} to {}", message.type_name(), target_address);
            
            match delivery_client.send_async(&target_address, message).await {
                Ok(()) => {}
                Err(e) => tracing::error!("FAILED to deliver to {}: {}", target_address, e),
            }
        }
    });

    // Shared node instances for delegating message handling
    let shared_follower = Arc::new(RwLock::new(None::<Arc<FollowerNode>>));
    let shared_leader = Arc::new(RwLock::new(None::<Arc<LeaderNode>>));

    // Channel for forwarding entries from message loop to FollowerNode
    // (We can't call FollowerNode methods directly due to Send trait constraints on rusqlite)
    // Uses ReplicationBatch so FollowerNode can send ACK after processing
    let (entry_tx, entry_rx) = tokio::sync::mpsc::channel::<wolfscale::replication::ReplicationBatch>(10);
    let shared_entry_rx = Arc::new(tokio::sync::Mutex::new(Some(entry_rx)));

    // Start INCOMING message processing loop - handles messages from peers
    // NOTE: Cannot call FollowerNode/LeaderNode methods here as they contain non-Send types (rusqlite)
    let incoming_cluster = Arc::clone(&cluster);
    tracing::info!("Message loop cluster Arc ptr: {:p}", Arc::as_ptr(&incoming_cluster));
    let response_tx = outgoing_tx.clone();  
    let our_node_id = config.node.id.clone();
    let incoming_heartbeat_time = Arc::clone(&shared_heartbeat_time);
    let incoming_entry_tx = entry_tx;

    tokio::spawn(async move {
        while let Some((peer_addr, message)) = incoming_rx.recv().await {
            tracing::trace!("RECEIVED {} from {}", message.type_name(), peer_addr);
            
            match message {
                wolfscale::replication::Message::Heartbeat { leader_id, commit_lsn, term, members } => {
                    // Sync cluster membership from leader - this is how followers learn about each other
                    for (member_id, member_addr) in members {
                        if member_id == our_node_id {
                            continue;  // Skip self
                        }
                        
                        if incoming_cluster.get_node(&member_id).await.is_none() {
                            // Remove any synthetic peer with this address
                            let synthetic_id = format!("peer-{}", member_addr.replace(':', "-"));
                            let _ = incoming_cluster.remove_peer(&synthetic_id).await;
                            let _ = incoming_cluster.add_peer(member_id.clone(), member_addr.clone()).await;
                        }
                        // Mark this node as active (leader says they're in the cluster)
                        let _ = incoming_cluster.record_heartbeat(&member_id, 0).await;
                    }
                    
                    // Update cluster: mark sender as leader
                    if let Err(e) = incoming_cluster.set_leader(&leader_id).await {
                        tracing::warn!("Failed to set leader from heartbeat: {}", e);
                    }
                    // Record the heartbeat
                    let _ = incoming_cluster.record_heartbeat(&leader_id, commit_lsn).await;
                    
                    // Update shared heartbeat timestamp for election timer reset
                    incoming_heartbeat_time.store(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64,
                        std::sync::atomic::Ordering::Relaxed
                    );
                    
                    // Send HeartbeatResponse back to the leader with our actual applied LSN
                    let last_applied = incoming_cluster.get_self().await.last_applied_lsn;
                    let response = wolfscale::replication::Message::HeartbeatResponse {
                        node_id: our_node_id.clone(),
                        term,
                        last_applied_lsn: last_applied,
                        success: true,
                    };
                    // Use registered leader address from cluster, not the ephemeral source port
                    let leader_addr = incoming_cluster.get_node(&leader_id).await
                        .map(|n| n.address.clone())
                        .unwrap_or_else(|| {
                            // Fallback: replace ephemeral port with cluster port
                            if let Some(colon_idx) = peer_addr.rfind(':') {
                                format!("{}:7654", &peer_addr[..colon_idx])
                            } else {
                                peer_addr.clone()
                            }
                        });
                    let _ = response_tx.send((leader_addr, response)).await;
                }
                wolfscale::replication::Message::AppendEntries { term, leader_id, prev_lsn: _, prev_term: _, entries, leader_commit_lsn: _ } => {
                    tracing::debug!("RECEIVED {} entries from leader {}", entries.len(), leader_id);
                    let _ = incoming_cluster.record_heartbeat(&leader_id, 0).await;
                    
                    // Update heartbeat time
                    incoming_heartbeat_time.store(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64,
                        std::sync::atomic::Ordering::Relaxed
                    );
                    
                    // Get leader address for ACK response
                    let leader_addr = incoming_cluster.get_node(&leader_id).await
                        .map(|n| n.address.clone())
                        .unwrap_or_else(|| {
                            // Fallback: replace ephemeral port with cluster port
                            if let Some(colon_idx) = peer_addr.rfind(':') {
                                format!("{}:7654", &peer_addr[..colon_idx])
                            } else {
                                peer_addr.clone()
                            }
                        });
                    
                    // Forward entries to FollowerNode for processing
                    // FollowerNode will send ACK after entries are applied
                    if !entries.is_empty() {
                        let batch = wolfscale::replication::ReplicationBatch {
                            entries,
                            term,
                            leader_id,
                            leader_address: leader_addr,
                        };
                        // Use try_send to avoid blocking message loop (which handles heartbeats)
                        // If channel is full, entries will be retried by leader on next replication cycle
                        match incoming_entry_tx.try_send(batch) {
                            Ok(()) => {}
                            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                                tracing::warn!("Entry processing queue full - applying backpressure");
                            }
                            Err(e) => {
                                tracing::error!("Failed to forward entries to follower: {}", e);
                            }
                        }
                    }
                    // NOTE: No ACK here - FollowerNode sends ACK after processing
                }
                wolfscale::replication::Message::HeartbeatResponse { node_id, success, last_applied_lsn, .. } => {
                    // Leader receives response from follower - mark follower as active
                    if success {
                        // Register the follower if we don't know it yet
                        let follower_addr = if let Some(colon_idx) = peer_addr.rfind(':') {
                            format!("{}:7654", &peer_addr[..colon_idx])
                        } else {
                            peer_addr.clone()
                        };
                        
                        if incoming_cluster.get_node(&node_id).await.is_none() {
                            // Remove any existing synthetic peer with this address
                            let synthetic_id = format!("peer-{}", follower_addr.replace(':', "-"));
                            let _ = incoming_cluster.remove_peer(&synthetic_id).await;
                            let _ = incoming_cluster.add_peer(node_id.clone(), follower_addr).await;
                        }
                        
                        let _ = incoming_cluster.record_heartbeat(&node_id, last_applied_lsn).await;
                    }
                }
                wolfscale::replication::Message::AppendEntriesResponse { node_id, success, match_lsn, .. } => {
                    // Leader receives ACK from follower - update their progress
                    if success {
                        // Register the follower if we don't know it yet (same as HeartbeatResponse)
                        let node_existed = incoming_cluster.get_node(&node_id).await.is_some();
                        if !node_existed {
                            let follower_addr = if let Some(colon_idx) = peer_addr.rfind(':') {
                                format!("{}:7654", &peer_addr[..colon_idx])
                            } else {
                                peer_addr.clone()
                            };
                            tracing::debug!("Registering new follower {} at {}", node_id, follower_addr);
                            let _ = incoming_cluster.add_peer(node_id.clone(), follower_addr).await;
                        }
                        let _ = incoming_cluster.record_heartbeat(&node_id, match_lsn).await;
                        // Verify the update was recorded
                        if let Some(updated_node) = incoming_cluster.get_node(&node_id).await {
                            tracing::debug!("Follower {} acknowledged up to LSN {}, cluster now shows lsn={}", 
                                node_id, match_lsn, updated_node.last_applied_lsn);
                        } else {
                            tracing::warn!("Follower {} ACK received but node not found in cluster!", node_id);
                        }
                    }
                }
                wolfscale::replication::Message::RequestVote { candidate_id, .. } => {
                    tracing::info!("Vote request from {}", candidate_id);
                }
                wolfscale::replication::Message::PeerHeartbeat { node_id, members, .. } => {
                    // Peer heartbeat - record that this peer is alive
                    tracing::trace!("Peer heartbeat from {}", node_id);
                    
                    // Register peer if not known
                    if incoming_cluster.get_node(&node_id).await.is_none() {
                        // Find this node's address from the members list
                        if let Some((_, addr)) = members.iter().find(|(id, _)| id == &node_id) {
                            let _ = incoming_cluster.add_peer(node_id.clone(), addr.clone()).await;
                        }
                    }
                    
                    // Record heartbeat from this peer
                    let _ = incoming_cluster.record_heartbeat(&node_id, 0).await;
                }
                _ => {
                    tracing::trace!("Ignoring message type {} from {}", message.type_name(), peer_addr);
                }
            }
        }
        tracing::info!("Incoming message processing loop stopped");
    });

    // Use outgoing_tx for replication (pass this to LeaderNode/FollowerNode)
    let msg_tx = outgoing_tx;

    // Initialize HTTP API
    let http_server = HttpServer::new(
        config.api.clone(),
        config.node.id.clone(),
        Arc::clone(&cluster),
        config.data_dir().clone(),
    );

    // Determine role BEFORE starting proxy - leader if bootstrap CLI flag, config bootstrap, or no peers
    let is_leader = bootstrap || config.cluster.bootstrap || config.cluster.peers.is_empty();
    
    if is_leader {
        tracing::info!("This node will start as LEADER");
        cluster.set_leader(&config.node.id).await?;
    }

    // Start built-in MySQL proxy if enabled
    if config.proxy.enabled {
        let proxy_config = ProxyConfig {
            listen_address: config.proxy.bind_address.clone(),
            backend_host: config.database.host.clone(),
            backend_port: config.database.port,
            backend_user: config.database.user.clone(),
            backend_password: config.database.password.clone(),
        };
        let proxy_cluster = Arc::clone(&cluster);
        let proxy_wal = wal_writer.clone();
        let proxy = ProxyServer::with_wal(proxy_config, proxy_cluster, proxy_wal);
        tracing::info!("MySQL proxy listening on {} (WAL-enabled)", config.proxy.bind_address);
        tokio::spawn(async move {
            if let Err(e) = proxy.start().await {
                tracing::error!("Proxy error: {}", e);
            }
        });
    }

    // Start periodic LSN tracker update for stats (100ms interval)
    let stats_lsn_tracker = http_server.get_lsn_tracker();
    let stats_wal_writer = wal_writer.clone();
    tokio::spawn(async move {
        loop {
            let current = stats_wal_writer.current_lsn().await;
            stats_lsn_tracker.store(current, std::sync::atomic::Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    if is_leader {
        tracing::info!("Starting LEADER components");
        tracing::info!("LeaderNode cluster Arc ptr: {:p}", Arc::as_ptr(&cluster));

        let leader = Arc::new(LeaderNode::new(
            config.node.id.clone(),
            wal_writer,
            wal_reader,
            Arc::clone(&state_tracker),
            Arc::clone(&cluster),
            ReplicationConfig {
                max_batch_entries: config.cluster.max_batch_entries,
                heartbeat_interval_ms: config.cluster.heartbeat_interval_ms,
                replication_timeout_ms: config.cluster.election_timeout_ms,
            },
            msg_tx,
            Some(Arc::clone(&executor)),
        ));

        // Store in shared state for message delegation
        *shared_leader.write().await = Some(Arc::clone(&leader));

        // Start all components
        tokio::select! {
            result = leader.start() => {
                if let Err(e) = result {
                    tracing::error!("Leader error: {}", e);
                }
            }
            result = http_server.start() => {
                if let Err(e) = result {
                    tracing::error!("HTTP server error: {}", e);
                }
            }
            result = network_server.start() => {
                if let Err(e) = result {
                    tracing::error!("Network server error: {}", e);
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Received shutdown signal");
            }
        }
    } else {
        tracing::info!("Starting as FOLLOWER");

        // Create shared state for role transitions
        let follower = Arc::new(FollowerNode::new(
            config.node.id.clone(),
            wal_writer,
            Arc::clone(&state_tracker),
            Arc::clone(&cluster),
            Arc::clone(&executor),
            ReplicationConfig {
                max_batch_entries: config.cluster.max_batch_entries,
                heartbeat_interval_ms: config.cluster.heartbeat_interval_ms,
                replication_timeout_ms: config.cluster.election_timeout_ms,
            },
            msg_tx.clone(),
            ElectionConfig {
                timeout_min_ms: config.cluster.election_timeout_min_ms,
                timeout_max_ms: config.cluster.election_timeout_max_ms,
            },
            config.cluster.disable_auto_election,
        ));

        // Connect the entry channel to the follower
        // This allows the message loop to forward entries to the follower for processing
        if let Some(rx) = shared_entry_rx.lock().await.take() {
            follower.set_entry_receiver(rx).await;
        }

        // Store in shared state for message delegation
        *shared_follower.write().await = Some(Arc::clone(&follower));

        let follower_clone = Arc::clone(&follower);
        let http_server_handle = tokio::spawn(async move {
            if let Err(e) = http_server.start().await {
                tracing::error!("HTTP server error: {}", e);
            }
        });

        let network_server_handle = tokio::spawn(async move {
            if let Err(e) = network_server.start().await {
                tracing::error!("Network server error: {}", e);
            }
        });

        // Entry processing is now handled internally by FollowerNode during handle_append_entries
        // This avoids the redundant loop in main.rs and ensures proper ACK signaling.

        // Peer heartbeat loop - followers broadcast heartbeats to all peers
        // This enables proper health detection even when leader is down
        let peer_cluster = Arc::clone(&cluster);
        let peer_msg_tx = msg_tx.clone();
        let peer_node_id = config.node.id.clone();
        let peer_heartbeat_interval = Duration::from_millis(config.cluster.heartbeat_interval_ms);
        tokio::spawn(async move {
            tracing::info!("Peer heartbeat loop started");
            let mut interval = tokio::time::interval(peer_heartbeat_interval);
            loop {
                interval.tick().await;
                
                // Get all known real peers (exclude synthetic peers from membership lists)
                let peers = peer_cluster.real_peers().await;
                let self_node = peer_cluster.get_self().await;
                
                // Build membership list
                let mut members: Vec<(String, String)> = vec![
                    (peer_node_id.clone(), self_node.address.clone())
                ];
                for peer in &peers {
                    members.push((peer.id.clone(), peer.address.clone()));
                }
                
                // Send peer heartbeat to all known peers
                for peer in &peers {
                    let msg = wolfscale::replication::Message::PeerHeartbeat {
                        node_id: peer_node_id.clone(),
                        term: 0,  // Followers don't track term
                        members: members.clone(),
                    };
                    if let Err(e) = peer_msg_tx.send((peer.address.clone(), msg)).await {
                        tracing::debug!("Failed to send peer heartbeat to {}: {}", peer.id, e);
                    }
                }
            }
        });

        // Role transition loop - monitors for election wins and checks heartbeat
        let role_check_interval = Duration::from_millis(100);
        let mut role_ticker = tokio::time::interval(role_check_interval);
        let follower_heartbeat_time = Arc::clone(&shared_heartbeat_time);
        let mut last_checked_heartbeat: u64 = 0;

        // Start follower processing (runs in same task since rusqlite isn't Send)
        let follower_start = follower_clone.start();
        tokio::pin!(follower_start);

        loop {
            tokio::select! {
                result = &mut follower_start => {
                    if let Err(e) = result {
                        tracing::error!("Follower error: {}", e);
                    }
                    break;
                }
                _ = role_ticker.tick() => {
                    // Check for new heartbeat and reset election timer
                    let current_heartbeat = follower_heartbeat_time.load(std::sync::atomic::Ordering::Relaxed);
                    if current_heartbeat > last_checked_heartbeat {
                        last_checked_heartbeat = current_heartbeat;
                        follower_clone.reset_election_timer().await;
                    }
                    
                    // Check for timed-out nodes (including leader)
                    let timed_out = cluster.check_timeouts().await;
                    for node_id in &timed_out {
                        tracing::warn!("Node {} timed out", node_id);
                    }
                    
                    // Check if we won an election
                    if follower_clone.is_leader().await {
                        tracing::info!("Election won! Transitioning from FOLLOWER to LEADER");
                        
                        // Stop follower mode
                        if let Err(e) = follower_clone.stop().await {
                            tracing::error!("Error stopping follower: {}", e);
                        }

                        // Create new WAL writer and reader for leader mode
                        let wal_writer = WalWriter::new(
                            config.data_dir().clone(),
                            config.wal.clone(),
                            config.node.id.clone(),
                        ).await?;

                        let wal_reader = WalReader::new(
                            config.data_dir().clone(),
                            config.wal.segment_size_mb,
                            config.wal.compression,
                        )?;

                        // Start as leader
                        let leader = LeaderNode::new(
                            config.node.id.clone(),
                            wal_writer,
                            wal_reader,
                            Arc::clone(&state_tracker),
                            Arc::clone(&cluster),
                            ReplicationConfig {
                                max_batch_entries: config.cluster.max_batch_entries,
                                heartbeat_interval_ms: config.cluster.heartbeat_interval_ms,
                                replication_timeout_ms: config.cluster.election_timeout_ms,
                            },
                            msg_tx.clone(),
                            Some(executor.clone()),
                        );

                        tracing::info!("Now running as LEADER");

                        tokio::select! {
                            result = leader.start() => {
                                if let Err(e) = result {
                                    tracing::error!("Leader error: {}", e);
                                }
                            }
                            _ = tokio::signal::ctrl_c() => {
                                tracing::info!("Received shutdown signal");
                            }
                        }
                        break;
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("Received shutdown signal");
                    break;
                }
            }
        }

        // Cleanup
        http_server_handle.abort();
        network_server_handle.abort();
    }

    tracing::info!("WolfScale shutdown complete");
    Ok(())
}

/// Join an existing cluster
async fn run_join(config_path: PathBuf, leader: String) -> Result<()> {
    tracing::info!("Joining cluster via leader: {}", leader);

    let config = WolfScaleConfig::from_file(&config_path)?;
    
    // Connect to leader
    let client = NetworkClient::new(
        Duration::from_secs(10),
        Duration::from_secs(30),
    );

    let join_msg = wolfscale::replication::Message::JoinRequest {
        node_id: config.node.id.clone(),
        address: config.advertise_address().to_string(),
    };

    match client.send(&leader, join_msg).await {
        Ok(response) => {
            match response {
                wolfscale::replication::Message::JoinResponse { success, message, .. } => {
                    if success {
                        tracing::info!("Successfully joined cluster");
                        // Now start normally
                        run_start(config_path, false).await
                    } else {
                        tracing::error!("Join failed: {:?}", message);
                        Err(wolfscale::error::Error::Replication(
                            message.unwrap_or_else(|| "Join failed".to_string())
                        ))
                    }
                }
                _ => {
                    tracing::error!("Unexpected response from leader");
                    Err(wolfscale::error::Error::Replication("Unexpected response".into()))
                }
            }
        }
        Err(e) => {
            tracing::error!("Failed to connect to leader: {}", e);
            Err(e)
        }
    }
}

/// Check cluster status
async fn run_status(address: String) -> Result<()> {
    let url = format!("http://{}/status", address);
    
    match reqwest::get(&url).await {
        Ok(response) => {
            let status: serde_json::Value = response.json().await
                .map_err(|e| wolfscale::error::Error::Network(e.to_string()))?;
            println!("{}", serde_json::to_string_pretty(&status).unwrap());
            Ok(())
        }
        Err(e) => {
            eprintln!("Failed to get status: {}", e);
            Err(wolfscale::error::Error::Network(e.to_string()))
        }
    }
}

/// Force synchronization
async fn run_sync(address: String) -> Result<()> {
    let url = format!("http://{}/cluster", address);
    
    match reqwest::get(&url).await {
        Ok(response) => {
            let cluster: serde_json::Value = response.json().await
                .map_err(|e| wolfscale::error::Error::Network(e.to_string()))?;
            println!("Cluster Info:");
            println!("{}", serde_json::to_string_pretty(&cluster).unwrap());
            Ok(())
        }
        Err(e) => {
            eprintln!("Failed to get cluster info: {}", e);
            Err(wolfscale::error::Error::Network(e.to_string()))
        }
    }
}

/// Initialize configuration file
fn run_init(output: PathBuf, node_id: String) -> Result<()> {
    let config_content = format!(r#"# WolfScale Configuration
# Generated configuration file

[node]
id = "{node_id}"
bind_address = "0.0.0.0:7654"
data_dir = "/var/lib/wolfscale/{node_id}"
# advertise_address = "my-public-ip:7654"

[database]
host = "localhost"
port = 3306
user = "wolfscale"
password = "changeme"
database = "myapp"
pool_size = 10
connect_timeout_secs = 30

[wal]
batch_size = 1000
flush_interval_ms = 100
compression = true
segment_size_mb = 64
retention_hours = 168
fsync = true

[cluster]
peers = []
# peers = ["node-2.example.com:7654", "node-3.example.com:7654"]
heartbeat_interval_ms = 500
election_timeout_ms = 2000
max_batch_entries = 1000

[api]
enabled = true
bind_address = "0.0.0.0:8080"
cors_enabled = false

[logging]
level = "info"
format = "pretty"
# file = "/var/log/wolfscale/wolfscale.log"

[proxy]
enabled = true
bind_address = "0.0.0.0:8007"
"#);

    std::fs::write(&output, config_content)?;
    println!("Configuration file created: {}", output.display());
    println!("\nEdit the file to configure your database and cluster settings.");
    println!("Then start with: wolfscale start --config {}", output.display());
    
    Ok(())
}

/// Validate configuration
fn run_validate(config_path: PathBuf) -> Result<()> {
    match WolfScaleConfig::from_file(&config_path) {
        Ok(config) => {
            println!("✓ Configuration is valid");
            println!("  Node ID: {}", config.node.id);
            println!("  Bind Address: {}", config.node.bind_address);
            println!("  Database: {}@{}:{}/{}", 
                config.database.user,
                config.database.host,
                config.database.port,
                config.database.database.as_deref().unwrap_or("(all)"));
            println!("  Peers: {}", config.cluster.peers.len());
            println!("  Quorum Size: {}", config.quorum_size());
            Ok(())
        }
        Err(e) => {
            eprintln!("✗ Configuration error: {}", e);
            Err(e)
        }
    }
}

/// Show node information
fn run_info(config_path: PathBuf) -> Result<()> {
    let config = WolfScaleConfig::from_file(&config_path)?;
    
    println!("WolfScale Node Information");
    println!("==========================");
    println!();
    println!("Node ID:          {}", config.node.id);
    println!("Bind Address:     {}", config.node.bind_address);
    println!("Advertise:        {}", config.advertise_address());
    println!("Data Directory:   {}", config.data_dir().display());
    println!();
    println!("Database Configuration:");
    println!("  Host:           {}:{}", config.database.host, config.database.port);
    println!("  Database:       {}", config.database.database.as_deref().unwrap_or("(all - server-wide)"));
    println!("  Pool Size:      {}", config.database.pool_size);
    println!();
    println!("WAL Configuration:");
    println!("  Batch Size:     {}", config.wal.batch_size);
    println!("  Compression:    {}", config.wal.compression);
    println!("  Segment Size:   {} MB", config.wal.segment_size_mb);
    println!("  Fsync:          {}", config.wal.fsync);
    println!();
    println!("Cluster Configuration:");
    println!("  Peers:          {:?}", config.cluster.peers);
    println!("  Quorum Size:    {}", config.quorum_size());
    println!("  Heartbeat:      {} ms", config.cluster.heartbeat_interval_ms);
    println!("  Election:       {} ms", config.cluster.election_timeout_ms);
    
    Ok(())
}

/// Run the MySQL protocol proxy
async fn run_proxy(config_path: PathBuf, listen_address: String) -> Result<()> {
    let config = WolfScaleConfig::from_file(&config_path)?;
    
    tracing::info!("Starting WolfScale MySQL Proxy");
    tracing::info!("Node ID: {}", config.node.id);
    
    // Create cluster membership (we need it to find the leader)
    // In proxy mode, we'll use the first peer as the default backend
    let cluster = Arc::new(ClusterMembership::new(
        config.node.id.clone(),
        config.advertise_address().to_string(),
        Duration::from_secs(1),
        Duration::from_secs(5),
    ));
    
    // Create proxy configuration
    let proxy_config = ProxyConfig {
        listen_address,
        backend_host: config.database.host.clone(),
        backend_port: config.database.port,
        backend_user: config.database.user.clone(),
        backend_password: config.database.password.clone(),
    };
    
    let proxy = ProxyServer::new(proxy_config, cluster);
    
    println!("WolfScale MySQL Proxy");
    println!("====================");
    println!();
    println!("MySQL clients can connect to this proxy as if it were a MariaDB server.");
    println!("Writes will be automatically routed to the cluster leader.");
    println!();
    
    tokio::select! {
        result = proxy.start() => {
            if let Err(e) = result {
                tracing::error!("Proxy error: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Received shutdown signal");
        }
    }
    
    Ok(())
}

