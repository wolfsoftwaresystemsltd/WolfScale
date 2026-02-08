//! WolfDiskCtl - Control utility for WolfDisk
//!
//! Usage:
//!   wolfdiskctl status          - Show node status from running service
//!   wolfdiskctl list servers    - List all discovered servers
//!   wolfdiskctl stats           - Live cluster statistics

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// WolfDisk Cluster Control Tool
#[derive(Parser)]
#[command(name = "wolfdiskctl")]
#[command(about = "Control and monitor WolfDisk clusters", long_about = None)]
struct Cli {
    /// Path to status file (written by running wolfdisk service)
    #[arg(short, long, default_value = "/var/lib/wolfdisk/cluster_status.json")]
    status_file: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show status of local node
    Status,
    /// List cluster servers and their status
    List {
        #[command(subcommand)]
        what: ListSubcommand,
    },
    /// Show live stats (updates every second)
    Stats,
}

#[derive(Subcommand)]
enum ListSubcommand {
    /// List all servers in the cluster
    Servers,
}

// ============ Status File Types ============

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterStatus {
    pub node_id: String,
    pub role: String,
    pub state: String,
    pub bind_address: String,
    pub leader_id: Option<String>,
    pub index_version: u64,
    pub peers: Vec<PeerStatus>,
    pub updated_at: u64, // Unix timestamp
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerStatus {
    pub node_id: String,
    pub address: String,
    #[serde(default)]
    pub role: Option<String>,
    pub is_leader: bool,
    #[serde(default)]
    pub is_client: bool,
    pub last_seen_secs_ago: u64,
}

fn main() {
    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Status => show_status(&cli.status_file),
        Commands::List { what } => match what {
            ListSubcommand::Servers => list_servers(&cli.status_file),
        },
        Commands::Stats => show_stats(&cli.status_file),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn read_status(path: &PathBuf) -> Result<ClusterStatus, Box<dyn std::error::Error>> {
    if !path.exists() {
        return Err(format!(
            "Status file not found: {}\n\nIs the wolfdisk service running?\nStart it with: sudo systemctl start wolfdisk",
            path.display()
        ).into());
    }

    let content = std::fs::read_to_string(path)?;
    let status: ClusterStatus = serde_json::from_str(&content)?;

    // Check if status is stale (more than 10 seconds old)
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    if now - status.updated_at > 10 {
        return Err("Status file is stale. Is the wolfdisk service running?".into());
    }

    Ok(status)
}

fn show_status(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let status = read_status(path)?;

    println!();
    println!("WolfDisk Status");
    println!("===============");
    println!();
    println!("Node ID:       {}", status.node_id);
    println!("Role:          {}", status.role);
    println!("State:         {}", status.state);
    println!("Bind Address:  {}", status.bind_address);
    if let Some(ref leader) = status.leader_id {
        println!("Leader:        {}", leader);
    }
    println!("Index Version: {}", status.index_version);
    println!("Peers:         {}", status.peers.len());
    println!();

    Ok(())
}

fn list_servers(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let status = read_status(path)?;

    // Find leader
    let leader = if status.role == "leader" {
        status.node_id.clone()
    } else {
        status.peers.iter()
            .find(|p| p.is_leader)
            .map(|p| p.node_id.clone())
            .unwrap_or_else(|| "unknown".to_string())
    };

    // Count active nodes
    let active_peers = status.peers.iter()
        .filter(|p| p.last_seen_secs_ago < 10)
        .count();
    let total_nodes = status.peers.len() + 1;
    let active_nodes = active_peers + 1; // Include self

    println!();
    println!("WolfDisk Cluster Status (wolfdiskctl v2.1.1)");
    println!("============================================");
    println!();
    println!("Total: {} nodes  |  Active: {}", total_nodes, active_nodes);
    println!("Leader: {}", leader);
    println!();
    println!("{:20} {:25} {:10} {:10}", "NODE ID", "ADDRESS", "STATUS", "ROLE");
    println!("{}", "-".repeat(65));

    // Print this node first
    let my_role = if status.role == "leader" { "Leader" } else if status.role == "client" { "Client" } else { "Follower" };
    println!("{:20} {:25} {:10} {:10}", status.node_id, status.bind_address, "Active", my_role);

    // Print peers
    for peer in &status.peers {
        let node_status = if peer.last_seen_secs_ago < 10 { "Active" } else { "Stale" };
        let role = if peer.is_leader { "Leader" } else if peer.is_client { "Client" } else { "Follower" };
        println!("{:20} {:25} {:10} {:10}", peer.node_id, peer.address, node_status, role);
    }

    println!();

    Ok(())
}

fn show_stats(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    println!("WolfDisk Live Stats (Ctrl+C to exit)");
    println!();
    
    loop {
        // Clear screen
        print!("\x1B[2J\x1B[1;1H");

        match read_status(path) {
            Ok(status) => {
                println!("╔════════════════════════════════════════════════════════════════╗");
                println!("║                    WolfDisk Cluster Stats                      ║");
                println!("╠════════════════════════════════════════════════════════════════╣");
                println!("║ Node:    {:54} ║", status.node_id);
                println!("║ Role:    {:54} ║", status.role.to_uppercase());
                println!("║ State:   {:54} ║", status.state);
                println!("║ Version: {:54} ║", status.index_version);
                println!("╠════════════════════════════════════════════════════════════════╣");
                
                println!("║ Cluster Nodes ({})                                              ║", status.peers.len() + 1);
                println!("║   ● {} (self) - {}                          ║", status.node_id, status.state);
                
                for peer in &status.peers {
                    let indicator = if peer.last_seen_secs_ago < 4 { "●" } else { "○" };
                    let role = if peer.is_leader { "leader" } else if peer.is_client { "client" } else { "follower" };
                    println!("║   {} {} - {} (seen {}s ago)                    ║", 
                        indicator, peer.node_id, role, peer.last_seen_secs_ago);
                }
                
                println!("╚════════════════════════════════════════════════════════════════╝");
            }
            Err(e) => {
                println!("Error reading status: {}", e);
            }
        }

        std::thread::sleep(Duration::from_secs(1));
    }
}
