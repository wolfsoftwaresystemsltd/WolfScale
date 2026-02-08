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
            
            // Create filesystem instance
            let fs = match WolfDiskFS::new(config) {
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
                std::process::exit(1);
            }
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
            info!("Cluster Status:");
            info!("  Node ID: {}", config.node.id);
            info!("  Data Dir: {:?}", config.node.data_dir);
            info!("  Mode: {:?}", config.replication.mode);
            // TODO: Add cluster connectivity check
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
