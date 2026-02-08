//! WolfDisk CLI
//!
//! Command-line interface for mounting and managing WolfDisk.

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::{info, error};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use wolfdisk::{Config, fuse::WolfDiskFS};

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
            
            // Create peer manager for network communication
            let peer_manager = std::sync::Arc::new(
                wolfdisk::network::peer::PeerManager::new(
                    config.node.id.clone(),
                    config.node.bind.clone(),
                    |_peer_id, _msg| None, // TODO: Handle incoming messages from leader
                )
            );
            
            // Create filesystem instance with cluster support
            let fs = match WolfDiskFS::with_cluster(
                config,
                Some(cluster.clone()),
                Some(peer_manager.clone()),
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
            println!("Note: Run 'journalctl -u wolfdisk -f' to see live cluster status");
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
