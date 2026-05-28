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

    /// Bind address for the UDP socket (default: 0.0.0.0 = all interfaces)
    /// Set to a specific IP to restrict which interface WolfNet listens on
    #[serde(default = "default_bind_address")]
    pub bind_address: String,

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
fn default_bind_address() -> String { "0.0.0.0".into() }
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
    /// "Tunnel is alive" — at least one signed packet (handshake,
    /// keepalive, or data) has been observed in the last 120s. Kept
    /// for back-compat with older readers; prefer `data_flowing` when
    /// trying to answer "can I actually send a ping to this peer".
    pub connected: bool,
    /// Whether this peer is a gateway node
    #[serde(default)]
    pub is_gateway: bool,
    /// If learned via PEX, the IP of the peer that told us about this one
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_via: Option<String>,
    /// True iff we've decrypted a real DATA packet from this peer in
    /// the last 120s — distinct from `connected`, which is also true
    /// for handshake-only / keepalive-only peers. The asymmetric case
    /// where `connected=true && data_flowing=false` is the "wolfnet
    /// status lies" scenario from klasSponsor 2026-05-11 and the only
    /// reliable signal that data isn't actually getting through.
    #[serde(default)]
    pub data_flowing: bool,
    /// Seconds since we last decrypted a data packet. `u64::MAX` means
    /// "never since wolfnet started". Symmetric with `last_seen_secs`.
    #[serde(default = "default_max_u64")]
    pub last_data_rx_secs: u64,
}

fn default_max_u64() -> u64 { u64::MAX }

impl Config {
    /// Load configuration from a TOML file
    /// Includes auto-migration: fixes `ip =` → `allowed_ip =` and removes duplicate peers.
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        // Self-heal: if the primary file is missing or empty but a
        // sibling `.bak` exists from a previous successful save, restore
        // it before reading. klasSponsor 2026-05-28 reported that a
        // port edit on one node wiped config.toml entirely and wolfnet
        // then exited on every start. With atomic saves writing .bak
        // before each replace, the recovery path now lives here so a
        // crash mid-save doesn't brick the daemon — the next start
        // picks up the last good snapshot automatically and logs that
        // it did so.
        let primary_usable = std::fs::metadata(path)
            .map(|m| m.len() > 0)
            .unwrap_or(false);
        if !primary_usable {
            let bak = path.with_extension(
                match path.extension().and_then(|e| e.to_str()) {
                    Some(ext) => format!("{}.bak", ext),
                    None => "bak".to_string(),
                }
            );
            if let Ok(bak_meta) = std::fs::metadata(&bak) {
                if bak_meta.len() > 0 {
                    if let Err(e) = std::fs::copy(&bak, path) {
                        eprintln!(
                            "[wolfnet] Config recovery: primary {} is empty/missing but \
                             copying {} → {} failed: {}",
                            path.display(), bak.display(), path.display(), e
                        );
                    } else {
                        eprintln!(
                            "[wolfnet] Config recovery: primary {} was empty/missing — \
                             restored from {}",
                            path.display(), bak.display()
                        );
                    }
                }
            }
        }

        let content = std::fs::read_to_string(path)?;

        // --- Migration: replace `ip = "..."` with `allowed_ip = "..."` in [[peers]] ---
        // Only replace bare `ip = ` lines (not `allowed_ip =`, not `public_ip =` etc.)
        let mut migrated = false;
        let fixed: String = content.lines().map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("ip = ") && !trimmed.starts_with("ip_") {
                migrated = true;
                line.replace("ip = ", "allowed_ip = ")
            } else {
                line.to_string()
            }
        }).collect::<Vec<_>>().join("\n");

        let mut config: Config = toml::from_str(&fixed)?;

        // --- Dedup: remove peers with duplicate public_key or allowed_ip ---
        let before = config.peers.len();
        let mut seen_keys = std::collections::HashSet::new();
        let mut seen_ips = std::collections::HashSet::new();
        config.peers.retain(|p| {
            let key_new = seen_keys.insert(p.public_key.clone());
            let ip_new = seen_ips.insert(p.allowed_ip.clone());
            key_new && ip_new
        });
        let removed = before - config.peers.len();

        // Write back if anything changed
        if migrated || removed > 0 {
            if migrated {
                eprintln!("[wolfnet] Config migration: fixed 'ip' → 'allowed_ip' in {}", path.display());
            }
            if removed > 0 {
                eprintln!("[wolfnet] Config cleanup: removed {} duplicate peer(s) from {}", removed, path.display());
            }
            config.save(path).ok(); // best-effort write-back
        }

        Ok(config)
    }

    /// Save configuration to a TOML file.
    ///
    /// Atomic write with a `.bak` snapshot of the previous file. The
    /// non-atomic version (`fs::write` directly) could leave the live
    /// `config.toml` truncated or empty if the process was killed mid-
    /// write — klasSponsor 2026-05-28 hit exactly that after editing
    /// the listen port, and wolfnet then exited on every start because
    /// no usable config remained. The atomic rename + .bak combination
    /// means: a crash mid-write leaves the old file intact, and the
    /// load path can recover from .bak if anything still goes wrong.
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        if content.trim().is_empty()
            || !content.contains("[network]")
            || !content.contains("[security]")
        {
            return Err(format!(
                "Refusing to save WolfNet config to {}: serialized payload is \
                 empty or missing required sections. Existing file left untouched.",
                path.display()
            )
            .into());
        }

        // Stage in a sibling .tmp.
        let tmp = path.with_extension(
            match path.extension().and_then(|e| e.to_str()) {
                Some(ext) => format!("{}.tmp", ext),
                None => "tmp".to_string(),
            }
        );
        std::fs::write(&tmp, &content)?;

        // Snapshot the prior good config to .bak before replacing —
        // best-effort, an absent .bak isn't fatal.
        if path.exists() {
            let bak = path.with_extension(
                match path.extension().and_then(|e| e.to_str()) {
                    Some(ext) => format!("{}.bak", ext),
                    None => "bak".to_string(),
                }
            );
            let _ = std::fs::copy(path, &bak);
        }

        // Atomic rename — either the new file is fully visible or
        // it isn't, never half.
        std::fs::rename(&tmp, path)?;
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
                bind_address: default_bind_address(),
                gateway: false,
                discovery: true,
                mtu: default_mtu(),
            },
            security: SecurityConfig::default(),
            peers: Vec::new(),
        }
    }
}
