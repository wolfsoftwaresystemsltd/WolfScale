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
    /// Set the log level for the running service (requires sudo)
    LogLevel {
        /// Log level: debug, info, warn, error
        level: String,
    },
    /// Show live stats (updates every second, Ctrl+C to exit)
    Stats,
    /// Setup binlog replication by detecting current position from MariaDB
    BinlogSetup {
        /// MariaDB host to connect to (defaults to config database host)
        #[arg(long)]
        host: Option<String>,
        /// MariaDB port (defaults to 3306)
        #[arg(long)]
        port: Option<u16>,
        /// Database user (defaults to config database user)
        #[arg(long)]
        user: Option<String>,
        /// Database password (defaults to config database password)
        #[arg(long)]
        password: Option<String>,
    },
    /// Reset WAL and state on all nodes (DESTRUCTIVE - requires restart)
    Reset {
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
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

#[allow(dead_code)]
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

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct StatsApiResponse {
    #[serde(default)]
    node_id: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    current_lsn: u64,
    #[serde(default)]
    commit_lsn: u64,
    #[serde(default)]
    uptime_seconds: u64,
    #[serde(default)]
    cluster_size: usize,
    #[serde(default)]
    active_nodes: usize,
    #[serde(default)]
    followers: Vec<FollowerStatsApi>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct FollowerStatsApi {
    #[serde(default)]
    node_id: String,
    #[serde(default)]
    last_applied_lsn: u64,
    #[serde(default)]
    lag: u64,
    #[serde(default)]
    status: String,
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
        Commands::LogLevel { level } => set_log_level(level),
        Commands::Stats => show_stats(&endpoint).await,
        Commands::BinlogSetup { host, port, user, password } => {
            binlog_setup(&cli.config, host.clone(), *port, user.clone(), password.clone()).await
        }
        Commands::Reset { force } => reset_cluster(&endpoint, *force).await,
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
    println!("WolfScale Cluster Status (wolfctl v{})", env!("CARGO_PKG_VERSION"));
    println!("========================================");
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
    println!("{:<20} {:<25} {:<10} {:<10}",
        "NODE ID", "ADDRESS", "STATUS", "ROLE");
    println!("{}", "-".repeat(65));

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

        println!("{:<20} {:<25} {} {}",
            node.id,
            node.address,
            status_colored,
            role_colored
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

// ============ Binlog Setup ============

async fn binlog_setup(
    config_path: &PathBuf,
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    password: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!();
    println!("\x1b[1;36m╔══════════════════════════════════════════════════════════════╗\x1b[0m");
    println!("\x1b[1;36m║\x1b[0m            \x1b[1;37mWolfScale Binlog Replication Setup\x1b[0m              \x1b[1;36m║\x1b[0m");
    println!("\x1b[1;36m╚══════════════════════════════════════════════════════════════╝\x1b[0m");
    println!();

    // Load config to get database settings if not overridden
    let (db_host, db_port, db_user, db_pass) = if config_path.exists() {
        let content = std::fs::read_to_string(config_path)?;
        let config: FullConfig = toml::from_str(&content)?;
        let db = config.database.unwrap_or_default();
        (
            host.unwrap_or_else(|| db.host.unwrap_or_else(|| "localhost".to_string())),
            port.unwrap_or(db.port.unwrap_or(3306)),
            user.unwrap_or_else(|| db.user.unwrap_or_else(|| "root".to_string())),
            password.unwrap_or_else(|| db.password.unwrap_or_default()),
        )
    } else {
        (
            host.unwrap_or_else(|| "localhost".to_string()),
            port.unwrap_or(3306),
            user.unwrap_or_else(|| "root".to_string()),
            password.unwrap_or_default(),
        )
    };

    println!("Connecting to MariaDB at {}:{}...", db_host, db_port);

    // Use mysql command to get binlog position
    let output = std::process::Command::new("mysql")
        .arg("-h").arg(&db_host)
        .arg("-P").arg(db_port.to_string())
        .arg("-u").arg(&db_user)
        .arg(format!("-p{}", db_pass))
        .arg("-N")  // No column names
        .arg("-e").arg("SHOW MASTER STATUS")
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        
        // Check if binlog is not enabled
        if stderr.contains("Access denied") {
            return Err("Access denied - check database credentials".into());
        }
        
        return Err(format!("Failed to query MariaDB: {}", stderr).into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    
    if lines.is_empty() || stdout.trim().is_empty() {
        println!();
        println!("\x1b[1;31m✗ Binary logging is NOT enabled!\x1b[0m");
        println!();
        println!("To enable binlog, add to your MariaDB config (my.cnf):");
        println!();
        println!("  [mysqld]");
        println!("  log_bin = mysql-bin");
        println!("  binlog_format = MIXED");
        println!("  server_id = 1");
        println!();
        println!("Then restart MariaDB and run this command again.");
        return Ok(());
    }

    // Parse output: File  Position  Binlog_Do_DB  Binlog_Ignore_DB
    let parts: Vec<&str> = lines[0].split_whitespace().collect();
    if parts.len() < 2 {
        return Err("Unexpected output from SHOW MASTER STATUS".into());
    }

    let binlog_file = parts[0];
    let binlog_pos: u64 = parts[1].parse()?;

    println!("\x1b[1;32m✓\x1b[0m Binary logging is enabled");
    println!();
    println!("Current binlog position:");
    println!("  File:     {}", binlog_file);
    println!("  Position: {}", binlog_pos);
    println!();

    // Generate unique server ID (based on last octet of host IP + random)
    let server_id = 1001 + (std::process::id() % 1000);

    println!("\x1b[1;33mAdd this to your WolfScale config.toml:\x1b[0m");
    println!();
    println!("┌────────────────────────────────────────────────────────┐");
    println!("│ [replication]                                          │");
    println!("│ mode = \"binlog\"                                        │");
    println!("│                                                        │");
    println!("│ [binlog]                                               │");
    println!("│ server_id = {}                                       │", server_id);
    println!("│ start_file = \"{}\"                           │", binlog_file);
    println!("│ start_position = {}                                   │", binlog_pos);
    println!("└────────────────────────────────────────────────────────┘");
    println!();
    println!("\x1b[1;36mNote:\x1b[0m server_id must be unique across all MySQL replicas.");
    println!();

    Ok(())
}

// ============ Log Level ============

fn set_log_level(level: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Validate level
    let valid_levels = ["debug", "info", "warn", "error", "trace", "off"];
    let level_lower = level.to_lowercase();
    
    if !valid_levels.contains(&level_lower.as_str()) {
        return Err(format!(
            "Invalid log level '{}'. Valid levels: {}", 
            level, 
            valid_levels.join(", ")
        ).into());
    }

    // Create systemd drop-in directory
    let dropin_dir = "/etc/systemd/system/wolfscale.service.d";
    let dropin_file = format!("{}/logging.conf", dropin_dir);
    
    // Check if running as root
    if !nix::unistd::Uid::effective().is_root() {
        println!("\x1b[1;33mNote:\x1b[0m This command requires sudo to modify systemd configuration.");
        println!();
        println!("Run: sudo wolfctl log-level {}", level);
        return Ok(());
    }

    // Create directory
    std::fs::create_dir_all(dropin_dir)?;
    
    // Determine RUST_LOG value
    let rust_log = if level_lower == "off" {
        "".to_string()
    } else {
        format!("wolfscale={}", level_lower)
    };

    // Write drop-in file
    let content = format!(
        "[Service]\nEnvironment=\"RUST_LOG={}\"\n",
        rust_log
    );
    std::fs::write(&dropin_file, content)?;

    println!("\x1b[1;32m✓\x1b[0m Log level set to: {}", level_lower);
    
    // Reload systemd and restart service
    println!("Reloading systemd...");
    let reload = std::process::Command::new("systemctl")
        .args(["daemon-reload"])
        .status()?;
    
    if !reload.success() {
        return Err("Failed to reload systemd".into());
    }

    println!("Restarting wolfscale service...");
    let restart = std::process::Command::new("systemctl")
        .args(["restart", "wolfscale"])
        .status()?;
    
    if !restart.success() {
        return Err("Failed to restart wolfscale service".into());
    }

    println!("\x1b[1;32m✓\x1b[0m Service restarted with new log level");
    println!();
    println!("View logs: sudo journalctl -u wolfscale -f");
    
    Ok(())
}

// ============ Stats ============

async fn show_stats(endpoint: &str) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("{}/stats", endpoint);
    let client = reqwest::Client::new();
    
    // Track for throughput calculation  
    let mut last_lsn: Option<u64> = None;
    let mut last_time = std::time::Instant::now();
    let mut writes_per_sec: f64 = 0.0;
    
    // Throughput history for graph (last 40 samples)
    let mut throughput_history: Vec<f64> = vec![0.0; 40];
    let mut peak_throughput: f64 = 10.0;
    let mut total_writes: u64 = 0;
    let start_time = std::time::Instant::now();
    
    // Hide cursor
    print!("\x1b[?25l");
    
    // Set up Ctrl+C handler
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, std::sync::atomic::Ordering::SeqCst);
    })?;
    
    // Main loop
    while running.load(std::sync::atomic::Ordering::SeqCst) {
        // Clear screen and move cursor to top
        print!("\x1b[H\x1b[J");
        
        // Header
        println!();
        println!("  \x1b[1;36mWolfScale Live Statistics\x1b[0m");
        println!("  {}",  "=".repeat(50));
        println!();
        
        // Fetch stats
        match client.get(&url).send().await {
            Ok(response) if response.status().is_success() => {
                match response.json::<StatsApiResponse>().await {
                    Ok(stats) => {
                        // Calculate throughput
                        let now = std::time::Instant::now();
                        let elapsed = now.duration_since(last_time).as_secs_f64();
                        
                        if let Some(prev_lsn) = last_lsn {
                            if elapsed > 0.0 && stats.current_lsn > prev_lsn {
                                let delta = stats.current_lsn - prev_lsn;
                                writes_per_sec = delta as f64 / elapsed;
                                total_writes += delta;
                            } else if elapsed > 2.0 {
                                writes_per_sec *= 0.5;
                            }
                        }
                        last_lsn = Some(stats.current_lsn);
                        last_time = now;
                        
                        // Update throughput history
                        throughput_history.remove(0);
                        throughput_history.push(writes_per_sec);
                        
                        // Update peak for graph scale
                        if writes_per_sec > peak_throughput {
                            peak_throughput = writes_per_sec * 1.2;
                        }
                        
                        // Calculate average throughput
                        let uptime_secs = start_time.elapsed().as_secs_f64();
                        let avg_throughput = if uptime_secs > 0.0 { total_writes as f64 / uptime_secs } else { 0.0 };
                        
                        // Role with color
                        let role_str = if stats.role == "Leader" {
                            format!("\x1b[1;34m{}\x1b[0m", stats.role)
                        } else {
                            format!("\x1b[33m{}\x1b[0m", stats.role)
                        };
                        
                        // Node info
                        println!("  Node:     {}  [{}]", stats.node_id, role_str);
                        println!("  Cluster:  {}/{} nodes active", stats.active_nodes, stats.cluster_size);
                        println!("  LSN:      {}", stats.current_lsn);
                        println!();
                        
                        // Throughput section
                        println!("  \x1b[1mThroughput\x1b[0m");
                        println!("  {}", "-".repeat(50));
                        println!("  Current:  \x1b[1;32m{:>10.1}/s\x1b[0m", writes_per_sec);
                        println!("  Average:  \x1b[1;33m{:>10.1}/s\x1b[0m", avg_throughput);
                        println!("  Peak:     \x1b[1;35m{:>10.1}/s\x1b[0m", peak_throughput);
                        println!("  Total:    {:>10}", total_writes);
                        println!();
                        
                        // ASCII Graph
                        println!("  \x1b[1mHistory (last 40s)\x1b[0m");
                        draw_ascii_graph(&throughput_history, peak_throughput);
                        
                        // Follower replication status
                        if !stats.followers.is_empty() && stats.role == "Leader" {
                            println!();
                            println!("  \x1b[1mFollowers\x1b[0m");
                            println!("  {}", "-".repeat(50));
                            
                            for f in &stats.followers {
                                let status_char = match f.status.as_str() {
                                    "Active" => "\x1b[32m[OK]\x1b[0m",
                                    "Syncing" => "\x1b[33m[..]\x1b[0m",
                                    "Lagging" => "\x1b[33m[!!]\x1b[0m",
                                    "Dropped" => "\x1b[31m[XX]\x1b[0m",
                                    _ => "[??]",
                                };
                                
                                let lag_display = if f.lag == 0 {
                                    "\x1b[32m0\x1b[0m".to_string()
                                } else if f.lag < 100 {
                                    format!("\x1b[33m{}\x1b[0m", f.lag)
                                } else {
                                    format!("\x1b[31m{}\x1b[0m", f.lag)
                                };
                                
                                println!("  {} {:15} LSN: {:>10}  Lag: {:>6}", 
                                    status_char, f.node_id, f.last_applied_lsn, lag_display);
                            }
                        }
                        
                        // Footer
                        println!();
                        let uptime_fmt = format_duration(start_time.elapsed());
                        println!("  \x1b[2mSession: {} | Ctrl+C to exit\x1b[0m", uptime_fmt);
                    }
                    Err(e) => {
                        println!("  Error parsing stats: {}", e);
                    }
                }
            }
            Ok(response) => {
                println!("  \x1b[31mAPI Error: {}\x1b[0m", response.status());
                println!("  \x1b[2mCtrl+C to exit\x1b[0m");
            }
            Err(e) => {
                println!("  \x1b[31mConnection Error: {}\x1b[0m", e);
                println!("  Is WolfScale running?");
                println!("  \x1b[2mCtrl+C to exit\x1b[0m");
            }
        }
        
        // Wait 1 second before next update
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    
    // Show cursor again
    print!("\x1b[?25h");
    println!();
    println!("Stats monitoring stopped.");
    
    Ok(())
}

/// Reset WAL and state on all cluster nodes
async fn reset_cluster(endpoint: &str, force: bool) -> Result<(), Box<dyn std::error::Error>> {
    println!();
    println!("\x1b[1;31m WARNING: This will DESTROY all WAL and state data!\x1b[0m");
    println!();
    
    // Get all nodes
    let client = reqwest::Client::new();
    let cluster_url = format!("{}/cluster", endpoint);
    
    let response = client.get(&cluster_url).send().await?;
    if !response.status().is_success() {
        return Err(format!("Failed to get cluster info: {}", response.status()).into());
    }
    
    let cluster: ClusterInfoResponse = response.json().await?;
    
    println!("Nodes to reset:");
    for node in &cluster.nodes {
        println!("  - {} ({})", node.id, node.address);
    }
    println!();
    
    if !force {
        println!("Type 'RESET' to confirm: ");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if input.trim() != "RESET" {
            println!("Aborted.");
            return Ok(());
        }
    }
    
    println!();
    println!("Resetting nodes...");
    
    // Reset each node
    let mut success_count = 0;
    let mut error_count = 0;
    
    for node in &cluster.nodes {
        let reset_url = format!("http://{}/admin/reset", node.address);
        print!("  {} ... ", node.id);
        
        match client.post(&reset_url).send().await {
            Ok(response) if response.status().is_success() => {
                println!("\x1b[32mOK\x1b[0m");
                success_count += 1;
            }
            Ok(response) => {
                println!("\x1b[31mFAILED ({})\x1b[0m", response.status());
                error_count += 1;
            }
            Err(e) => {
                println!("\x1b[31mERROR: {}\x1b[0m", e);
                error_count += 1;
            }
        }
    }
    
    println!();
    println!("Reset complete: {} succeeded, {} failed", success_count, error_count);
    println!();
    println!("\x1b[1;33mIMPORTANT: You must restart all WolfScale services!\x1b[0m");
    println!("  sudo systemctl restart wolfscale");
    println!();
    
    Ok(())
}

/// Draw an ASCII graph of throughput history
fn draw_ascii_graph(history: &[f64], max_val: f64) {
    let graph_height = 6;
    let graph_width = history.len();
    
    // Draw from top to bottom
    for row in (0..graph_height).rev() {
        let threshold = (row as f64 / graph_height as f64) * max_val;
        
        // Y-axis label
        if row == graph_height - 1 {
            print!("  {:>6.0} |", max_val);
        } else if row == 0 {
            print!("       0 |");
        } else {
            print!("         |");
        }
        
        // Draw bars
        for &val in history {
            if val >= threshold {
                print!("\x1b[32m#\x1b[0m");
            } else {
                print!(" ");
            }
        }
        println!("|");
    }
    
    // X-axis
    print!("         +");
    print!("{}", "-".repeat(graph_width));
    println!("+");
    println!("          {:^width$}", "40s ago              now", width = graph_width);
}



/// Format duration as human-readable string
fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs >= 3600 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}
