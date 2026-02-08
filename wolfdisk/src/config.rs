//! Configuration types for WolfDisk

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::Result;

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Node-specific configuration
    pub node: NodeConfig,

    /// Cluster configuration
    pub cluster: ClusterConfig,

    /// Replication settings
    pub replication: ReplicationConfig,

    /// Mount options
    pub mount: MountConfig,
}

/// Node configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Unique node identifier
    pub id: String,

    /// Node role in the cluster
    #[serde(default = "default_role")]
    pub role: NodeRole,

    /// Bind address for cluster communication
    #[serde(default = "default_bind")]
    pub bind: String,

    /// Data directory for chunks and index
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
}

fn default_role() -> NodeRole {
    NodeRole::Auto
}

fn default_bind() -> String {
    "0.0.0.0:9500".to_string()
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("/var/lib/wolfdisk")
}

/// Cluster configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterConfig {
    /// List of peer addresses
    #[serde(default)]
    pub peers: Vec<String>,

    /// Discovery address (UDP multicast or DNS)
    pub discovery: Option<String>,
}

/// Replication mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReplicationMode {
    /// Data replicated to N nodes
    Replicated,
    /// Single leader, others are read-only clients
    Shared,
}

/// Node role in the cluster
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeRole {
    /// Leader node - accepts writes, replicates to followers
    Leader,
    /// Follower node - replicates data from leader
    Follower,
    /// Client node - mount-only, no replication, accesses shared drive via leader
    Client,
    /// Auto-detect role (will become leader if no others, otherwise follower)
    Auto,
}

/// Replication configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationConfig {
    /// Replication mode
    #[serde(default = "default_mode")]
    pub mode: ReplicationMode,

    /// Replication factor (for replicated mode)
    #[serde(default = "default_factor")]
    pub factor: usize,

    /// Chunk size in bytes (default 4MB)
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
}

fn default_mode() -> ReplicationMode {
    ReplicationMode::Shared
}

fn default_factor() -> usize {
    3
}

fn default_chunk_size() -> usize {
    4 * 1024 * 1024 // 4MB
}

/// Mount configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountConfig {
    /// Default mount path
    #[serde(default = "default_mount_path")]
    pub path: PathBuf,

    /// Allow other users to access the mount
    #[serde(default = "default_allow_other")]
    pub allow_other: bool,
}

fn default_mount_path() -> PathBuf {
    PathBuf::from("/mnt/wolfdisk")
}

fn default_allow_other() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            node: NodeConfig {
                id: hostname::get()
                    .map(|h| h.to_string_lossy().to_string())
                    .unwrap_or_else(|_| "node1".to_string()),
                role: default_role(),
                bind: default_bind(),
                data_dir: default_data_dir(),
            },
            cluster: ClusterConfig {
                peers: Vec::new(),
                discovery: None,
            },
            replication: ReplicationConfig {
                mode: default_mode(),
                factor: default_factor(),
                chunk_size: default_chunk_size(),
            },
            mount: MountConfig {
                path: default_mount_path(),
                allow_other: default_allow_other(),
            },
        }
    }
}

impl Config {
    /// Load configuration from a TOML file
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Save configuration to a TOML file
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Get the chunks directory path
    pub fn chunks_dir(&self) -> PathBuf {
        self.node.data_dir.join("chunks")
    }

    /// Get the index directory path
    pub fn index_dir(&self) -> PathBuf {
        self.node.data_dir.join("index")
    }

    /// Get the WAL directory path
    pub fn wal_dir(&self) -> PathBuf {
        self.node.data_dir.join("wal")
    }
}
