//! wolfnetctl — CLI utility for WolfNet status and management

use std::path::PathBuf;
use clap::{Parser, Subcommand};

/// Status file location (written by wolfnet daemon)
const STATUS_FILE: &str = "/var/run/wolfnet/status.json";

#[derive(Parser)]
#[command(name = "wolfnetctl", version, about = "WolfNet control utility")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show this node's status
    Status,
    /// List all peers on the network with hostnames
    Peers,
    /// Show network summary
    Info,
}

#[derive(serde::Deserialize)]
struct NodeStatus {
    hostname: String,
    address: String,
    public_key: String,
    listen_port: u16,
    gateway: bool,
    interface: String,
    uptime_secs: u64,
    peers: Vec<PeerStatus>,
}

#[derive(serde::Deserialize)]
#[allow(dead_code)]
struct PeerStatus {
    hostname: String,
    address: String,
    endpoint: String,
    public_key: String,
    last_seen_secs: u64,
    rx_bytes: u64,
    tx_bytes: u64,
    connected: bool,
}

fn main() {
    let cli = Cli::parse();
    let status = load_status();

    match cli.command {
        Commands::Status => cmd_status(&status),
        Commands::Peers => cmd_peers(&status),
        Commands::Info => cmd_info(&status),
    }
}

fn load_status() -> NodeStatus {
    let path = PathBuf::from(STATUS_FILE);
    if !path.exists() {
        eprintln!("Error: WolfNet daemon is not running (no status file at {})", STATUS_FILE);
        eprintln!("Start the daemon with: sudo wolfnet");
        std::process::exit(1);
    }
    let content = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("Error reading status file: {}", e);
        std::process::exit(1);
    });
    serde_json::from_str(&content).unwrap_or_else(|e| {
        eprintln!("Error parsing status: {}", e);
        std::process::exit(1);
    })
}

fn cmd_status(status: &NodeStatus) {
    println!("╔══════════════════════════════════════════════════════╗");
    println!("║              🐺  WolfNet Status                     ║");
    println!("╚══════════════════════════════════════════════════════╝");
    println!();
    println!("  Hostname:    {}", status.hostname);
    println!("  WolfNet IP:  {}", status.address);
    println!("  Interface:   {}", status.interface);
    println!("  Listen Port: {}", status.listen_port);
    println!("  Gateway:     {}", if status.gateway { "Yes" } else { "No" });
    println!("  Public Key:  {}...{}", &status.public_key[..8], &status.public_key[status.public_key.len()-4..]);
    println!("  Uptime:      {}", format_duration(status.uptime_secs));
    println!("  Peers:       {} ({} connected)",
        status.peers.len(),
        status.peers.iter().filter(|p| p.connected).count(),
    );
    println!();
}

fn cmd_peers(status: &NodeStatus) {
    if status.peers.is_empty() {
        println!("No peers configured.");
        println!("Add peers via /etc/wolfnet/config.toml or enable LAN discovery.");
        return;
    }

    println!("╔══════════════════════════════════════════════════════════════════════════╗");
    println!("║  🐺  WolfNet Peers                                                      ║");
    println!("╠══════════════════════════════════════════════════════════════════════════╣");
    println!("║  {:<15} {:<15} {:<22} {:<8} {:<10} ║", "HOSTNAME", "WOLFNET IP", "ENDPOINT", "STATUS", "LAST SEEN");
    println!("╠══════════════════════════════════════════════════════════════════════════╣");

    for peer in &status.peers {
        let status_str = if peer.connected { "🟢 online" } else { "🔴 offline" };
        let last_seen = if peer.last_seen_secs == u64::MAX {
            "never".to_string()
        } else {
            format_duration(peer.last_seen_secs)
        };
        let host = if peer.hostname.is_empty() { "-" } else { &peer.hostname };
        println!("║  {:<15} {:<15} {:<22} {:<8} {:<10} ║",
            host, peer.address, peer.endpoint, status_str, last_seen);
    }
    println!("╚══════════════════════════════════════════════════════════════════════════╝");

    // Traffic summary
    let total_rx: u64 = status.peers.iter().map(|p| p.rx_bytes).sum();
    let total_tx: u64 = status.peers.iter().map(|p| p.tx_bytes).sum();
    println!();
    println!("  Traffic: ↓ {} received  ↑ {} sent", format_bytes(total_rx), format_bytes(total_tx));
    println!();
}

fn cmd_info(status: &NodeStatus) {
    cmd_status(status);
    if !status.peers.is_empty() {
        cmd_peers(status);
    }
}

fn format_duration(secs: u64) -> String {
    if secs < 60 { return format!("{}s", secs); }
    if secs < 3600 { return format!("{}m {}s", secs / 60, secs % 60); }
    if secs < 86400 { return format!("{}h {}m", secs / 3600, (secs % 3600) / 60); }
    format!("{}d {}h", secs / 86400, (secs % 86400) / 3600)
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 { return format!("{} B", bytes); }
    if bytes < 1024 * 1024 { return format!("{:.1} KB", bytes as f64 / 1024.0); }
    if bytes < 1024 * 1024 * 1024 { return format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)); }
    format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}
