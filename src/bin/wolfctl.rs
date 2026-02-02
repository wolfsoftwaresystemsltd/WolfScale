//! WolfCtl - Command line tool for managing WolfScale clusters
//!
//! Usage:
//!   wolfctl list servers     - Show cluster node status
//!   wolfctl status           - Show local node status
//!   wolfctl promote          - Promote this node to leader
//!   wolfctl demote           - Demote this node from leader

use clap::{Parser, Subcommand};
use serde::Deserialize;
use std::path::PathBuf;

/// WolfScale Cluster Control Tool
#[derive(Parser)]
#[command(name = "wolfctl")]
#[command(about = "Control and monitor WolfScale clusters", long_about = None)]
struct Cli {
    /// Path to config file
    #[arg(short, long, default_value = "/etc/wolfscale/config.toml")]
    config: PathBuf,

    /// API endpoint to connect to (overrides config)
    #[arg(short, long)]
    endpoint: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List cluster servers and their status
    List {
        #[command(subcommand)]
        what: ListSubcommand,
    },
    /// Show status of local node
    Status,
    /// Promote this node to leader
    Promote,
    /// Demote this node from leader
    Demote,
}

#[derive(Subcommand)]
enum ListSubcommand {
    /// List all servers in the cluster
    Servers,
}

// ============ API Response Types ============

#[derive(Debug, Deserialize)]
struct ClusterInfoResponse {
    summary: ClusterSummary,
    nodes: Vec<NodeState>,
}

#[derive(Debug, Deserialize)]
struct ClusterSummary {
    total_nodes: usize,
    active_nodes: usize,
    #[serde(default)]
    leader_id: Option<String>,
    has_quorum: bool,
}

#[derive(Debug, Deserialize)]
struct NodeState {
    id: String,
    address: String,
    status: String,
    role: String,
    last_applied_lsn: u64,
    replication_lag: u64,
}

#[derive(Debug, Deserialize)]
struct StatusResponse {
    node_id: String,
    is_leader: bool,
    leader_id: Option<String>,
    cluster_size: usize,
    has_quorum: bool,
}

#[derive(Debug, Deserialize)]
struct WriteResponse {
    success: bool,
    message: Option<String>,
}

// ============ Config ============

#[derive(Debug, Deserialize)]
struct Config {
    #[serde(default)]
    api: ApiConfig,
}

#[derive(Debug, Deserialize, Default)]
struct ApiConfig {
    #[serde(default = "default_api_bind")]
    bind_address: String,
}

fn default_api_bind() -> String {
    "0.0.0.0:8080".to_string()
}

// ============ Main ============

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Determine API endpoint
    let endpoint = match &cli.endpoint {
        Some(e) => e.clone(),
        None => {
            // Try to read from config file
            if cli.config.exists() {
                match std::fs::read_to_string(&cli.config) {
                    Ok(content) => {
                        match toml::from_str::<Config>(&content) {
                            Ok(config) => {
                                // Convert bind address to localhost if it's 0.0.0.0
                                let addr = config.api.bind_address;
                                if addr.starts_with("0.0.0.0") {
                                    format!("http://127.0.0.1:{}", addr.split(':').nth(1).unwrap_or("8080"))
                                } else {
                                    format!("http://{}", addr)
                                }
                            }
                            Err(_) => "http://127.0.0.1:8080".to_string(),
                        }
                    }
                    Err(_) => "http://127.0.0.1:8080".to_string(),
                }
            } else {
                "http://127.0.0.1:8080".to_string()
            }
        }
    };

    let result = match &cli.command {
        Commands::List { what } => match what {
            ListSubcommand::Servers => list_servers(&endpoint).await,
        },
        Commands::Status => show_status(&endpoint).await,
        Commands::Promote => promote(&endpoint).await,
        Commands::Demote => demote(&endpoint).await,
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

// ============ Commands ============

async fn list_servers(endpoint: &str) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("{}/cluster", endpoint);
    let client = reqwest::Client::new();
    
    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        return Err(format!("API error: {}", response.status()).into());
    }

    let info: ClusterInfoResponse = response.json().await?;

    // Print header
    println!();
    println!("WolfScale Cluster Status");
    println!("========================");
    println!();
    println!("Total: {} nodes  |  Active: {}  |  Quorum: {}", 
        info.summary.total_nodes,
        info.summary.active_nodes,
        if info.summary.has_quorum { "Yes" } else { "NO" }
    );
    
    if let Some(leader) = &info.summary.leader_id {
        println!("Leader: {}", leader);
    } else {
        println!("Leader: NONE");
    }
    println!();

    // Print table header
    println!("{:<20} {:<25} {:<10} {:<10} {:<12} {:<8}",
        "NODE ID", "ADDRESS", "STATUS", "ROLE", "LSN", "LAG");
    println!("{}", "-".repeat(85));

    // Print nodes
    for node in &info.nodes {
        let status_colored = match node.status.as_str() {
            "Active" => format!("\x1b[32m{}\x1b[0m", node.status),  // Green
            "Joining" => format!("\x1b[33m{}\x1b[0m", node.status), // Yellow
            "Lagging" => format!("\x1b[33m{}\x1b[0m", node.status), // Yellow
            "Offline" => format!("\x1b[31m{}\x1b[0m", node.status), // Red
            _ => node.status.clone(),
        };

        let role_colored = match node.role.as_str() {
            "Leader" => format!("\x1b[1;34m{}\x1b[0m", node.role),  // Bold Blue
            _ => node.role.clone(),
        };

        let lag_display = if node.replication_lag == 0 {
            "-".to_string()
        } else {
            format!("{}", node.replication_lag)
        };

        println!("{:<20} {:<25} {:<20} {:<20} {:<12} {:<8}",
            node.id,
            node.address,
            status_colored,
            role_colored,
            node.last_applied_lsn,
            lag_display
        );
    }
    println!();

    Ok(())
}

async fn show_status(endpoint: &str) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("{}/status", endpoint);
    let client = reqwest::Client::new();
    
    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        return Err(format!("API error: {}", response.status()).into());
    }

    let status: StatusResponse = response.json().await?;

    println!();
    println!("Node Status");
    println!("===========");
    println!();
    println!("Node ID:      {}", status.node_id);
    println!("Role:         {}", if status.is_leader { "LEADER" } else { "Follower" });
    if let Some(leader) = &status.leader_id {
        println!("Leader:       {}", leader);
    }
    println!("Cluster Size: {}", status.cluster_size);
    println!("Has Quorum:   {}", if status.has_quorum { "Yes" } else { "No" });
    println!();

    Ok(())
}

async fn promote(endpoint: &str) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("{}/admin/promote", endpoint);
    let client = reqwest::Client::new();
    
    let response = client.post(&url).send().await?;

    if !response.status().is_success() {
        return Err(format!("API error: {}", response.status()).into());
    }

    let result: WriteResponse = response.json().await?;

    if result.success {
        println!("Promotion requested successfully");
        if let Some(msg) = result.message {
            println!("{}", msg);
        }
    } else {
        println!("Promotion failed");
        if let Some(msg) = result.message {
            println!("{}", msg);
        }
    }

    Ok(())
}

async fn demote(endpoint: &str) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("{}/admin/demote", endpoint);
    let client = reqwest::Client::new();
    
    let response = client.post(&url).send().await?;

    if !response.status().is_success() {
        return Err(format!("API error: {}", response.status()).into());
    }

    let result: WriteResponse = response.json().await?;

    if result.success {
        println!("Demotion requested successfully");
        if let Some(msg) = result.message {
            println!("{}", msg);
        }
    } else {
        println!("Demotion failed");
        if let Some(msg) = result.message {
            println!("{}", msg);
        }
    }

    Ok(())
}
