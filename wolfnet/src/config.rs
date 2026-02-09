//! Configuration for WolfNet

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::net::Ipv4Addr;

/// Main configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Network settings
    pub network: NetworkConfig,

    /// Security settings
    #[serde(default)]
    pub security: SecurityConfig,

    /// Configured peers
    #[serde(default)]
    pub peers: Vec<PeerConfig>,
}

/// Network configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// TUN interface name
    #[serde(default = "default_interface")]
    pub interface: String,

    /// This node's IP address on the virtual network (e.g. "10.0.10.1")
    pub address: String,

    /// Subnet mask in CIDR notation
    #[serde(default = "default_subnet")]
    pub subnet: u8,

    /// UDP listen port for tunnel traffic
    #[serde(default = "default_port")]
    pub listen_port: u16,

    /// Act as a gateway (NAT internet traffic for other nodes)
    #[serde(default)]
    pub gateway: bool,

    /// Enable LAN auto-discovery
    #[serde(default = "default_true")]
    pub discovery: bool,

    /// MTU for the TUN interface
    #[serde(default = "default_mtu")]
    pub mtu: u16,
}

/// Security configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// Path to the private key file
    #[serde(default = "default_key_path")]
    pub private_key_file: PathBuf,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            private_key_file: default_key_path(),
        }
    }
}

/// Configured peer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerConfig {
    /// Peer's public key (base64 encoded)
    pub public_key: String,

    /// Peer's endpoint (ip:port) — optional for LAN-discovered peers
    pub endpoint: Option<String>,

    /// Peer's WolfNet IP address
    pub allowed_ip: String,

    /// Optional friendly name
    pub name: Option<String>,
}

fn default_interface() -> String { "wolfnet0".into() }
fn default_subnet() -> u8 { 24 }
fn default_port() -> u16 { 9600 }
fn default_true() -> bool { true }
fn default_mtu() -> u16 { 1400 }
fn default_key_path() -> PathBuf { PathBuf::from("/etc/wolfnet/private.key") }

/// Status information written by daemon, read by wolfnetctl
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStatus {
    pub hostname: String,
    pub address: String,
    pub public_key: String,
    pub listen_port: u16,
    pub gateway: bool,
    pub interface: String,
    pub uptime_secs: u64,
    pub peers: Vec<PeerStatus>,
}

/// Status of a single peer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerStatus {
    pub hostname: String,
    pub address: String,
    pub endpoint: String,
    pub public_key: String,
    pub last_seen_secs: u64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub connected: bool,
    /// If learned via PEX, the IP of the peer that told us about this one
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_via: Option<String>,
}

impl Config {
    /// Load configuration from a TOML file
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Save configuration to a TOML file
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Parse this node's IP address
    pub fn ip_addr(&self) -> Result<Ipv4Addr, Box<dyn std::error::Error>> {
        Ok(self.network.address.parse()?)
    }

    /// Get the subnet as "address/mask" string
    pub fn cidr(&self) -> String {
        format!("{}/{}", self.network.address, self.network.subnet)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            network: NetworkConfig {
                interface: default_interface(),
                address: "10.0.10.1".into(),
                subnet: default_subnet(),
                listen_port: default_port(),
                gateway: false,
                discovery: true,
                mtu: default_mtu(),
            },
            security: SecurityConfig::default(),
            peers: Vec::new(),
        }
    }
}
