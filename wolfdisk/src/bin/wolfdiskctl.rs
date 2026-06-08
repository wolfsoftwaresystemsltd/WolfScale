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
    /// Path to status file. When omitted, it is derived from `data_dir` in
    /// /etc/wolfdisk/config.toml, so wolfdiskctl follows a custom data
    /// directory without the operator passing -s on every command.
    #[arg(short, long)]
    status_file: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

/// Resolve the status-file path: an explicit `-s/--status-file` always wins;
/// otherwise read `data_dir` from /etc/wolfdisk/config.toml and use
/// `<data_dir>/cluster_status.json` — the exact path the daemon writes
/// (cluster/state.rs). Falls back to the historical /var/lib/wolfdisk default
/// only when the config is absent/unreadable. WolfDisk install report B5
/// (2026-06-08): wolfdiskctl ignored a custom data_dir and every command
/// required `-s` with the full path.
fn resolve_status_file(explicit: &Option<PathBuf>) -> PathBuf {
    if let Some(p) = explicit {
        return p.clone();
    }
    const FALLBACK: &str = "/var/lib/wolfdisk/cluster_status.json";
    std::fs::read_to_string("/etc/wolfdisk/config.toml")
        .ok()
        .and_then(|c| status_file_from_config(&c))
        .unwrap_or_else(|| PathBuf::from(FALLBACK))
}

/// Derive `<data_dir>/cluster_status.json` from raw config.toml content — the
/// exact path the daemon writes (cluster/state.rs). Returns None when the
/// `[node].data_dir` key is missing/empty or the TOML doesn't parse, so the
/// caller falls back to the historical default.
fn status_file_from_config(config: &str) -> Option<PathBuf> {
    let value: toml::Value = config.parse().ok()?;
    let dir = value.get("node")?.get("data_dir")?.as_str()?;
    if dir.is_empty() {
        return None;
    }
    Some(PathBuf::from(dir).join("cluster_status.json"))
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
    #[serde(default)]
    pub file_count: usize,
    #[serde(default)]
    pub total_size: u64,
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
    let status_file = resolve_status_file(&cli.status_file);

    let result = match &cli.command {
        Commands::Status => show_status(&status_file),
        Commands::List { what } => match what {
            ListSubcommand::Servers => list_servers(&status_file),
        },
        Commands::Stats => show_stats(&status_file),
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

fn format_size(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

fn show_status(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let status = read_status(path)?;

    println!();
    println!("  WolfDisk Status");
    println!("  {}", "─".repeat(40));
    println!();
    println!("  Node ID       {}", status.node_id);
    println!("  Role          {}", status.role.to_uppercase());
    println!("  State         {}", status.state);
    println!("  Bind Address  {}", status.bind_address);
    if let Some(ref leader) = status.leader_id {
        println!("  Leader        {}", leader);
    }
    println!("  Index Version {}", status.index_version);
    println!("  Files         {}", status.file_count);
    println!("  Total Size    {}", format_size(status.total_size));
    println!("  Peers         {}", status.peers.len());
    println!();

    Ok(())
}

fn list_servers(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let status = read_status(path)?;

    // Find leader
    let leader = if status.role == "leader" {
        status.node_id.clone()
    } else {
        status
            .peers
            .iter()
            .find(|p| p.is_leader)
            .map(|p| p.node_id.clone())
            .unwrap_or_else(|| "unknown".to_string())
    };

    // Count active nodes
    let active_peers = status
        .peers
        .iter()
        .filter(|p| p.last_seen_secs_ago < 10)
        .count();
    let total_nodes = status.peers.len() + 1;
    let active_nodes = active_peers + 1;

    println!();
    println!("  WolfDisk Cluster");
    println!("  {}", "─".repeat(50));
    println!();
    println!(
        "  Nodes {} active / {} total    Leader: {}",
        active_nodes, total_nodes, leader
    );
    println!();
    println!("  {:20} {:25} {:10}", "NODE", "ADDRESS", "ROLE");
    println!(
        "  {:20} {:25} {:10}",
        "─".repeat(18),
        "─".repeat(23),
        "─".repeat(8)
    );

    // Print this node first
    let my_role = if status.role == "leader" {
        "Leader"
    } else if status.role == "client" {
        "Client"
    } else {
        "Follower"
    };
    println!(
        "  {:20} {:25} {:10}",
        format!("● {} (self)", status.node_id),
        status.bind_address,
        my_role
    );

    // Print peers
    for peer in &status.peers {
        let indicator = if peer.last_seen_secs_ago < 10 {
            "●"
        } else {
            "○"
        };
        let role = if peer.is_leader {
            "Leader"
        } else if peer.is_client {
            "Client"
        } else {
            "Follower"
        };
        let name = format!("{} {}", indicator, peer.node_id);
        println!("  {:20} {:25} {:10}", name, peer.address, role);
    }

    println!();

    Ok(())
}

fn show_stats(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        // Clear screen
        print!("\x1B[2J\x1B[1;1H");

        match read_status(path) {
            Ok(status) => {
                println!();
                println!("  WolfDisk Cluster Stats");
                println!("  {}", "─".repeat(50));
                println!();
                println!("  Node       {}", status.node_id);
                println!("  Role       {}", status.role.to_uppercase());
                println!("  State      {}", status.state);
                println!("  Version    {}", status.index_version);
                println!(
                    "  Files      {}    Size  {}",
                    status.file_count,
                    format_size(status.total_size)
                );
                println!();
                println!("  Cluster Nodes ({})", status.peers.len() + 1);
                println!("  {}", "─".repeat(50));
                println!("    ● {} (self) - {}", status.node_id, status.state);

                for peer in &status.peers {
                    let indicator = if peer.last_seen_secs_ago < 4 {
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
                    println!(
                        "    {} {} - {} (seen {}s ago)",
                        indicator, peer.node_id, role, peer.last_seen_secs_ago
                    );
                }

                println!();
                println!("  Ctrl+C to exit");
            }
            Err(e) => {
                println!("Error reading status: {}", e);
            }
        }

        std::thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_status_path_from_custom_data_dir() {
        // B5: a custom data_dir must drive the status-file path so wolfdiskctl
        // works without -s (WolfDisk install report 2026-06-08).
        let cfg = "\
[node]\n\
id = \"pve01\"\n\
role = \"leader\"\n\
bind = \"10.10.1.21:8650\"\n\
data_dir = \"/appdata/wolfdisk\"\n";
        assert_eq!(
            status_file_from_config(cfg),
            Some(PathBuf::from("/appdata/wolfdisk/cluster_status.json"))
        );
    }

    #[test]
    fn falls_back_when_data_dir_absent_or_invalid() {
        assert_eq!(status_file_from_config("[node]\nid = \"x\"\n"), None);
        assert_eq!(status_file_from_config("data_dir = \"\"\n"), None); // not under [node]
        assert_eq!(status_file_from_config("this is not valid toml ["), None);
    }
}
