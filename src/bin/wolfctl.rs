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
    /// Migrate database from another node (for adding new nodes to existing clusters)
    Migrate {
        /// Source node address (e.g., 10.0.10.111:8080 or http://wolftest1:8080)
        #[arg(long)]
        from: String,
    },
    /// Check configuration file for errors
    CheckConfig {
        /// Path to config file to check (defaults to --config path)
        #[arg(short, long)]
        file: Option<PathBuf>,
    },
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

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ClusterSummary {
    #[serde(default)]
    total_nodes: usize,
    #[serde(default)]
    active_nodes: usize,
    #[serde(default)]
    leader_id: Option<String>,
    #[serde(default)]
    has_quorum: bool,
}

#[derive(Debug, Deserialize)]
struct NodeState {
    #[serde(default)]
    id: String,
    #[serde(default)]
    address: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    last_applied_lsn: u64,
    #[serde(default)]
    replication_lag: u64,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct StatusResponse {
    #[serde(default)]
    node_id: String,
    #[serde(default)]
    is_leader: bool,
    #[serde(default)]
    leader_id: Option<String>,
    #[serde(default)]
    cluster_size: usize,
    #[serde(default)]
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
        Commands::Migrate { from } => migrate(from, &cli.config).await,
        Commands::CheckConfig { file } => {
            let config_path = file.clone().unwrap_or_else(|| cli.config.clone());
            check_config(&config_path)
        }
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
    println!("Total: {} nodes  |  Active: {}", 
        info.summary.total_nodes,
        info.summary.active_nodes
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
        // Pad status to fixed width BEFORE adding color codes
        let status_padded = format!("{:<10}", node.status);
        let status_colored = match node.status.as_str() {
            "Active" => format!("\x1b[32m{}\x1b[0m", status_padded),  // Green
            "Joining" => format!("\x1b[33m{}\x1b[0m", status_padded), // Yellow
            "Lagging" => format!("\x1b[33m{}\x1b[0m", status_padded), // Yellow
            "Offline" => format!("\x1b[31m{}\x1b[0m", status_padded), // Red
            _ => status_padded,
        };

        // Pad role to fixed width BEFORE adding color codes
        let role_padded = format!("{:<10}", node.role);
        let role_colored = match node.role.as_str() {
            "Leader" => format!("\x1b[1;34m{}\x1b[0m", role_padded),  // Bold Blue
            _ => role_padded,
        };

        let lag_display = if node.replication_lag == 0 {
            "-".to_string()
        } else {
            format!("{}", node.replication_lag)
        };

        println!("{:<20} {:<25} {} {} {:<12} {:<8}",
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

// ============ Config Check ============

/// Full configuration structure for validation
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct FullConfig {
    node: Option<NodeConfig>,
    database: Option<DatabaseConfig>,
    cluster: Option<ClusterConfig>,
    api: Option<ApiConfig>,
    proxy: Option<ProxyConfig>,
    wal: Option<WalConfig>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct NodeConfig {
    id: Option<String>,
    bind_address: Option<String>,
    advertise_address: Option<String>,
    data_dir: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Default)]
struct DatabaseConfig {
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    password: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ClusterConfig {
    bootstrap: Option<bool>,
    peers: Option<Vec<String>>,
    heartbeat_interval_ms: Option<u64>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ProxyConfig {
    enabled: Option<bool>,
    bind_address: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct WalConfig {
    batch_size: Option<usize>,
    segment_size_mb: Option<usize>,
}

fn check_config(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    println!();
    println!("\x1b[1;36m╔══════════════════════════════════════════════════════════════╗\x1b[0m");
    println!("\x1b[1;36m║\x1b[0m            \x1b[1;37mWolfScale Configuration Check\x1b[0m                     \x1b[1;36m║\x1b[0m");
    println!("\x1b[1;36m╚══════════════════════════════════════════════════════════════╝\x1b[0m");
    println!();

    // Check if file exists
    if !path.exists() {
        println!("\x1b[1;31m✗ ERROR:\x1b[0m Config file not found: {}", path.display());
        return Ok(());
    }
    println!("\x1b[1;32m✓\x1b[0m Config file: {}", path.display());

    // Read file content
    let content = std::fs::read_to_string(path)?;
    
    // Check for common typos in raw content
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    
    // Check for common typos
    if content.contains("dvertise_address") && !content.contains("advertise_address") {
        errors.push("Typo detected: 'dvertise_address' should be 'advertise_address'".to_string());
    }
    if content.contains("ertise_address") && !content.contains("advertise_address") {
        errors.push("Typo detected: 'ertise_address' should be 'advertise_address'".to_string());
    }
    if content.contains("adverrtise_address") {
        errors.push("Typo detected: 'adverrtise_address' should be 'advertise_address'".to_string());
    }
    
    // Try to parse as TOML
    let config: FullConfig = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            println!("\x1b[1;31m✗ ERROR:\x1b[0m Failed to parse config: {}", e);
            return Ok(());
        }
    };
    println!("\x1b[1;32m✓\x1b[0m Config file is valid TOML");

    // Validate [node] section
    if let Some(ref node) = config.node {
        if node.id.is_none() {
            errors.push("[node] id is required".to_string());
        } else {
            println!("\x1b[1;32m✓\x1b[0m Node ID: {}", node.id.as_ref().unwrap());
        }
        
        if node.advertise_address.is_none() {
            errors.push("[node] advertise_address is MISSING - this will cause cluster issues!".to_string());
        } else {
            let addr = node.advertise_address.as_ref().unwrap();
            if addr.starts_with("0.0.0.0") || addr.starts_with("127.0") {
                warnings.push(format!("[node] advertise_address '{}' should be your external IP, not localhost/0.0.0.0", addr));
            } else {
                println!("\x1b[1;32m✓\x1b[0m Advertise address: {}", addr);
            }
        }
        
        if node.bind_address.is_none() {
            warnings.push("[node] bind_address not set, will use default".to_string());
        }
    } else {
        errors.push("[node] section is missing".to_string());
    }

    // Validate [cluster] section
    if let Some(ref cluster) = config.cluster {
        if let Some(ref peers) = cluster.peers {
            // Check if node lists itself in peers
            if let Some(ref node) = config.node {
                if let Some(ref advertise) = node.advertise_address {
                    for peer in peers {
                        if peer.contains(&advertise.replace(":7654", "").replace(":","")) {
                            warnings.push(format!("Peer list contains this node's own address '{}' - remove self from peers", peer));
                        }
                    }
                }
            }
            println!("\x1b[1;32m✓\x1b[0m Peers configured: {} nodes", peers.len());
            for peer in peers {
                println!("    - {}", peer);
            }
        } else {
            warnings.push("[cluster] peers not configured - single node mode".to_string());
        }
        
        if let Some(bootstrap) = cluster.bootstrap {
            if bootstrap {
                println!("\x1b[1;33m!\x1b[0m Bootstrap mode: \x1b[1mENABLED\x1b[0m (only one node should have this)");
            }
        }
    }

    // Validate [database] section
    if let Some(ref db) = config.database {
        if db.host.is_none() || db.user.is_none() {
            errors.push("[database] host and user are required".to_string());
        } else {
            println!("\x1b[1;32m✓\x1b[0m Database: {}@{}:{}", 
                db.user.as_ref().unwrap(),
                db.host.as_ref().unwrap(),
                db.port.unwrap_or(3306));
        }
    } else {
        warnings.push("[database] section not configured - database sync disabled".to_string());
    }

    // Print warnings
    println!();
    if !warnings.is_empty() {
        println!("\x1b[1;33mWarnings ({}):\x1b[0m", warnings.len());
        for w in &warnings {
            println!("  \x1b[33m⚠\x1b[0m  {}", w);
        }
        println!();
    }

    // Print errors
    if !errors.is_empty() {
        println!("\x1b[1;31mErrors ({}):\x1b[0m", errors.len());
        for e in &errors {
            println!("  \x1b[31m✗\x1b[0m  {}", e);
        }
        println!();
        println!("\x1b[1;31mConfiguration has errors that must be fixed!\x1b[0m");
    } else {
        println!("\x1b[1;32m✓ Configuration looks good!\x1b[0m");
    }
    println!();

    Ok(())
}

// ============ Migrate ============

/// Response from dump endpoint
#[derive(Debug, Deserialize)]
struct DumpInfo {
    lsn: u64,
    database: String,
}

async fn migrate(source: &str, config_path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    println!("\x1b[1;36mWolfScale Database Migration\x1b[0m");
    println!("============================\n");

    // Normalize source address
    let source_endpoint = if source.starts_with("http://") || source.starts_with("https://") {
        source.to_string()
    } else {
        format!("http://{}", source)
    };

    // Load local config to get database credentials
    println!("Loading local configuration...");
    let config_content = std::fs::read_to_string(config_path)?;
    let config: FullConfig = toml::from_str(&config_content)?;

    let db_config = config.database.unwrap_or_default();
    let db_host = db_config.host.as_deref().unwrap_or("localhost");
    let db_port = db_config.port.unwrap_or(3306);
    let db_user = db_config.user.as_deref().unwrap_or("root");
    let db_pass = db_config.password.as_deref().unwrap_or("");

    println!("  Local database: {}:{}", db_host, db_port);
    println!("  Source node: {}", source_endpoint);
    println!();

    // Get dump info from source
    println!("Requesting database dump from source...");
    let client = reqwest::Client::new();
    let info_url = format!("{}/dump/info", source_endpoint);
    
    let info_response = client.get(&info_url).send().await?;
    if !info_response.status().is_success() {
        return Err(format!("Source node returned error: {}", info_response.status()).into());
    }
    
    let dump_info: DumpInfo = info_response.json().await?;
    println!("  Source LSN: {}", dump_info.lsn);
    println!("  Database: {}", dump_info.database);
    println!();

    // Stream the dump
    println!("Streaming database dump...");
    let dump_url = format!("{}/dump", source_endpoint);
    let dump_response = client.get(&dump_url).send().await?;
    
    if !dump_response.status().is_success() {
        return Err(format!("Failed to get dump: {}", dump_response.status()).into());
    }

    // Save dump to temp file
    let temp_path = std::env::temp_dir().join("wolfscale_migration.sql");
    let dump_bytes = dump_response.bytes().await?;
    std::fs::write(&temp_path, &dump_bytes)?;
    println!("  Downloaded {} bytes", dump_bytes.len());

    // Apply dump using mysql client
    println!("\nApplying dump to local database...");
    let mysql_cmd = std::process::Command::new("mysql")
        .arg("-h").arg(db_host)
        .arg("-P").arg(db_port.to_string())
        .arg("-u").arg(db_user)
        .arg(format!("-p{}", db_pass))
        .stdin(std::fs::File::open(&temp_path)?)
        .output()?;

    if !mysql_cmd.status.success() {
        let stderr = String::from_utf8_lossy(&mysql_cmd.stderr);
        return Err(format!("Failed to apply dump: {}", stderr).into());
    }

    // Clean up temp file
    let _ = std::fs::remove_file(&temp_path);

    println!("\x1b[32m✓ Migration complete!\x1b[0m");
    println!();
    println!("The database has been migrated from the source node.");
    println!("You can now start WolfScale - it will sync from LSN {}.", dump_info.lsn);
    println!();

    Ok(())
}
