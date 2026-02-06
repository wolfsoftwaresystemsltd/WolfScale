//! WolfScale Configuration
//!
//! This module provides configuration structures for the WolfScale
//! distributed synchronization manager.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// Main WolfScale configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WolfScaleConfig {
    /// Node-specific configuration
    pub node: NodeConfig,

    /// Database connection configuration
    pub database: DatabaseConfig,

    /// Write-Ahead Log configuration
    pub wal: WalConfig,

    /// Cluster configuration
    pub cluster: ClusterConfig,

    /// API configuration
    #[serde(default)]
    pub api: ApiConfig,

    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingConfig,

    /// MySQL proxy configuration
    #[serde(default)]
    pub proxy: ProxyConfig,
    
    /// Replication mode configuration
    #[serde(default)]
    pub replication: ReplicationModeConfig,
    
    /// Binlog configuration (when replication.mode = "binlog")
    #[serde(default)]
    pub binlog: BinlogConfig,
    
    /// Performance auto-tuning configuration
    #[serde(default)]
    pub performance: PerformanceConfig,
}

/// Node-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Unique node identifier
    pub id: String,

    /// Address to bind for cluster communication
    pub bind_address: String,

    /// Data directory for WAL and state storage
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,

    /// Advertised address for other nodes to connect
    #[serde(default)]
    pub advertise_address: Option<String>,
}

/// Database connection configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DatabaseConfig {
    /// MariaDB host
    pub host: String,

    /// MariaDB port
    #[serde(default = "default_db_port")]
    pub port: u16,

    /// Database user
    pub user: String,

    /// Database password
    pub password: String,

    /// Database name (optional - leave empty for server-wide replication)
    #[serde(default)]
    pub database: Option<String>,

    /// Connection pool size
    #[serde(default = "default_pool_size")]
    pub pool_size: u32,

    /// Connection timeout in seconds
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u64,
}

/// Write-Ahead Log configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalConfig {
    /// Number of entries to batch before flushing
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,

    /// Flush interval in milliseconds
    #[serde(default = "default_flush_interval_ms")]
    pub flush_interval_ms: u64,

    /// Enable LZ4 compression for WAL entries
    #[serde(default = "default_compression")]
    pub compression: bool,

    /// Maximum segment size in megabytes
    #[serde(default = "default_segment_size_mb")]
    pub segment_size_mb: u64,

    /// Retention period in hours (0 = infinite)
    #[serde(default)]
    pub retention_hours: u64,

    /// Use fsync for durability (slower but safer)
    #[serde(default = "default_fsync")]
    pub fsync: bool,
}

/// Cluster configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterConfig {
    /// Bootstrap this node as the initial leader (first node in cluster)
    #[serde(default)]
    pub bootstrap: bool,

    /// List of peer node addresses
    #[serde(default)]
    pub peers: Vec<String>,

    /// Heartbeat interval in milliseconds
    #[serde(default = "default_heartbeat_interval_ms")]
    pub heartbeat_interval_ms: u64,

    /// Election timeout in milliseconds (legacy, use election_timeout_min/max)
    #[serde(default = "default_election_timeout_ms")]
    pub election_timeout_ms: u64,

    /// Minimum election timeout in milliseconds
    /// Default: 1000ms (1 second). For WAN/unreliable networks, increase to 3000-5000ms
    #[serde(default = "default_election_timeout_min_ms")]
    pub election_timeout_min_ms: u64,

    /// Maximum election timeout in milliseconds  
    /// Default: 1500ms (1.5 seconds). For WAN/unreliable networks, increase to 5000-10000ms
    #[serde(default = "default_election_timeout_max_ms")]
    pub election_timeout_max_ms: u64,
    
    /// If true, this node will never become leader (read-only replica)
    /// Use for disaster recovery, reporting, or geo-distributed read replicas
    #[serde(default)]
    pub never_leader: bool,

    /// Minimum number of nodes for quorum (0 = auto-calculate)
    #[serde(default)]
    pub min_quorum: usize,

    /// Maximum entries per replication batch
    #[serde(default = "default_max_batch_entries")]
    pub max_batch_entries: usize,

    /// Disable automatic leader election (require manual promotion)
    #[serde(default)]
    pub disable_auto_election: bool,
}

/// API configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// Enable HTTP API
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// HTTP API bind address
    #[serde(default = "default_api_address")]
    pub bind_address: String,

    /// Enable CORS
    #[serde(default)]
    pub cors_enabled: bool,
}

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Log level (trace, debug, info, warn, error)
    #[serde(default = "default_log_level")]
    pub level: String,

    /// Log format (pretty, json)
    #[serde(default = "default_log_format")]
    pub format: String,

    /// Log to file path (optional)
    pub file: Option<PathBuf>,
}

/// MySQL proxy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// Enable built-in MySQL proxy
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// MySQL proxy bind address
    #[serde(default = "default_proxy_address")]
    pub bind_address: String,
    
    /// Enable SSL/TLS for MySQL proxy connections
    #[serde(default)]
    pub ssl_enabled: bool,
    
    /// Path to SSL certificate file (PEM format)
    #[serde(default)]
    pub ssl_cert: Option<PathBuf>,
    
    /// Path to SSL private key file (PEM format)
    #[serde(default)]
    pub ssl_key: Option<PathBuf>,
    
    /// Require SSL from clients (reject non-SSL connections)
    #[serde(default)]
    pub ssl_required: bool,
}

/// Replication mode configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationModeConfig {
    /// Replication mode: "proxy" (default) or "binlog"
    /// - proxy: Captures writes through WolfScale's MySQL proxy
    /// - binlog: Reads from MariaDB's binary log (for external Galera clusters)
    #[serde(default = "default_replication_mode")]
    pub mode: String,
}

/// Binlog configuration (when replication.mode = "binlog")
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinlogConfig {
    /// Unique server ID for binlog replication
    /// Must be unique across all MySQL replicas (including Galera nodes)
    #[serde(default = "default_binlog_server_id")]
    pub server_id: u32,
    
    /// Starting binlog file (optional - auto-detect from SHOW MASTER STATUS if not set)
    #[serde(default)]
    pub start_file: Option<String>,
    
    /// Starting binlog position (optional - uses current position if not set)
    #[serde(default)]
    pub start_position: Option<u64>,
}

/// Performance auto-tuning configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceConfig {
    /// Enable automatic hardware detection and tuning
    /// When true, WolfScale will detect CPU cores and RAM to optimize settings
    /// while reserving resources for MariaDB (default: true)
    #[serde(default = "default_true")]
    pub auto_tune: bool,
    
    /// Override: manually set worker threads (0 = auto-detect)
    #[serde(default)]
    pub worker_threads: usize,
    
    /// Override: manually set channel buffer size (0 = auto-detect)
    #[serde(default)]
    pub channel_buffer: usize,
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            auto_tune: true,
            worker_threads: 0,
            channel_buffer: 0,
        }
    }
}

fn default_db_port() -> u16 {
    3306
}

fn default_pool_size() -> u32 {
    10
}

fn default_connect_timeout() -> u64 {
    30
}

fn default_batch_size() -> usize {
    1000
}

fn default_flush_interval_ms() -> u64 {
    10  // 10ms = up to 100 flushes/sec for high throughput
}

fn default_compression() -> bool {
    true
}

fn default_segment_size_mb() -> u64 {
    256  // Large enough for massive INSERT statements from dump imports
}

fn default_fsync() -> bool {
    true
}

fn default_heartbeat_interval_ms() -> u64 {
    200
}

fn default_election_timeout_ms() -> u64 {
    2000
}

fn default_election_timeout_min_ms() -> u64 {
    1000 // 1 second for fast failover
}

fn default_election_timeout_max_ms() -> u64 {
    1500 // 1.5 seconds max
}

fn default_max_batch_entries() -> usize {
    5000
}

fn default_true() -> bool {
    true
}

fn default_api_address() -> String {
    "0.0.0.0:8080".to_string()
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_log_format() -> String {
    "pretty".to_string()
}

fn default_proxy_address() -> String {
    "0.0.0.0:8007".to_string()
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("/var/lib/wolfscale")
}

fn default_replication_mode() -> String {
    "proxy".to_string()
}

fn default_binlog_server_id() -> u32 {
    1001
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind_address: default_api_address(),
            cors_enabled: false,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
            file: None,
        }
    }
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind_address: default_proxy_address(),
            ssl_enabled: false,
            ssl_cert: None,
            ssl_key: None,
            ssl_required: false,
        }
    }
}

impl Default for ReplicationModeConfig {
    fn default() -> Self {
        Self {
            mode: default_replication_mode(),
        }
    }
}

impl Default for BinlogConfig {
    fn default() -> Self {
        Self {
            server_id: default_binlog_server_id(),
            start_file: None,
            start_position: None,
        }
    }
}

impl WolfScaleConfig {
    /// Load configuration from a TOML file
    pub fn from_file(path: &std::path::Path) -> crate::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: WolfScaleConfig = toml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    /// Load configuration from a TOML string
    pub fn from_str(content: &str) -> crate::Result<Self> {
        let config: WolfScaleConfig = toml::from_str(content)?;
        config.validate()?;
        Ok(config)
    }

    /// Validate the configuration
    pub fn validate(&self) -> crate::Result<()> {
        if self.node.id.is_empty() {
            return Err(crate::Error::Config("node.id cannot be empty".into()));
        }

        if self.node.bind_address.is_empty() {
            return Err(crate::Error::Config("node.bind_address cannot be empty".into()));
        }

        if self.database.host.is_empty() {
            return Err(crate::Error::Config("database.host cannot be empty".into()));
        }

        Ok(())
    }

    /// Get the advertised address (or bind address if not set)
    pub fn advertise_address(&self) -> &str {
        self.node
            .advertise_address
            .as_deref()
            .unwrap_or(&self.node.bind_address)
    }

    /// Get the data directory path
    pub fn data_dir(&self) -> &PathBuf {
        &self.node.data_dir
    }

    /// Get the WAL directory path
    pub fn wal_dir(&self) -> PathBuf {
        self.node.data_dir.join("wal")
    }

    /// Get the state directory path
    pub fn state_dir(&self) -> PathBuf {
        self.node.data_dir.join("state")
    }

    /// Get heartbeat interval as Duration
    pub fn heartbeat_interval(&self) -> Duration {
        Duration::from_millis(self.cluster.heartbeat_interval_ms)
    }

    /// Get election timeout as Duration
    pub fn election_timeout(&self) -> Duration {
        Duration::from_millis(self.cluster.election_timeout_ms)
    }

    /// Get flush interval as Duration
    pub fn flush_interval(&self) -> Duration {
        Duration::from_millis(self.wal.flush_interval_ms)
    }

    /// Calculate quorum size
    pub fn quorum_size(&self) -> usize {
        if self.cluster.min_quorum > 0 {
            self.cluster.min_quorum
        } else {
            let total_nodes = self.cluster.peers.len() + 1;
            (total_nodes / 2) + 1
        }
    }

    /// Get database connection URL
    pub fn database_url(&self) -> String {
        match &self.database.database {
            Some(db) => format!(
                "mysql://{}:{}@{}:{}/{}",
                self.database.user,
                self.database.password,
                self.database.host,
                self.database.port,
                db
            ),
            None => format!(
                "mysql://{}:{}@{}:{}",
                self.database.user,
                self.database.password,
                self.database.host,
                self.database.port
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let toml = r#"
[node]
id = "node-1"
bind_address = "0.0.0.0:7654"
data_dir = "/var/lib/wolfscale"

[database]
host = "localhost"
port = 3306
user = "wolfscale"
password = "secret"
database = "myapp"

[wal]
batch_size = 1000
flush_interval_ms = 100
compression = true

[cluster]
peers = ["node-2:7654", "node-3:7654"]
"#;

        let config = WolfScaleConfig::from_str(toml).unwrap();
        assert_eq!(config.node.id, "node-1");
        assert_eq!(config.cluster.peers.len(), 2);
        assert_eq!(config.quorum_size(), 2); // 3 nodes, quorum = 2
    }
}
