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

        // --- Dedup: collapse peers that share a public_key ---
        // A peer's public_key IS its cryptographic identity, so two [[peers]]
        // entries with the same key are the same peer listed twice — safe to
        // collapse. We deliberately DO NOT dedup by allowed_ip: two entries
        // that merely share a WolfNet IP but have *different* keys are distinct
        // peers (an operator misconfiguration to surface, never to silently
        // resolve by picking a victim).
        //
        // The previous logic removed a peer if its key OR its allowed_ip
        // collided and then overwrote config.toml. A single duplicated IP — or
        // a config WolfStack had just rewritten — could therefore delete
        // unique-key peers wholesale and, because of the write-back, make them
        // unrecoverable. Gary KO4BSR 2026-06-07: 7 unique-key blade peers were
        // wiped as "duplicates" on a SIGHUP reload, leaving only the single
        // peer WolfStack already knew. When a key genuinely is duplicated we
        // keep the entry that carries an endpoint, so a reload can never drop a
        // working endpoint in favour of a bare twin.
        let before = config.peers.len();
        let mut kept: Vec<PeerConfig> = Vec::with_capacity(before);
        let mut idx_by_key: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for peer in config.peers.drain(..) {
            match idx_by_key.get(&peer.public_key).copied() {
                Some(i) => {
                    // Same identity listed twice — prefer the one with an endpoint.
                    if kept[i].endpoint.is_none() && peer.endpoint.is_some() {
                        kept[i] = peer;
                    }
                }
                None => {
                    idx_by_key.insert(peer.public_key.clone(), kept.len());
                    kept.push(peer);
                }
            }
        }
        config.peers = kept;
        let removed = before - config.peers.len();

        // Surface — but never act on — IP conflicts between distinct peers.
        // Deleting one would be the exact data loss removed above; the operator
        // decides which entry is wrong.
        let mut first_name_by_ip: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for peer in &config.peers {
            let pname = peer.name.clone().unwrap_or_else(|| "(unnamed)".to_string());
            if let Some(first) = first_name_by_ip.get(&peer.allowed_ip) {
                eprintln!(
                    "[wolfnet] Config warning: peers '{}' and '{}' both claim \
                     allowed_ip {} with different keys — both kept; resolve the \
                     conflict in {}",
                    first, pname, peer.allowed_ip, path.display()
                );
            } else {
                first_name_by_ip.insert(peer.allowed_ip.clone(), pname);
            }
        }

        // Write back if anything changed
        if migrated || removed > 0 {
            if migrated {
                eprintln!("[wolfnet] Config migration: fixed 'ip' → 'allowed_ip' in {}", path.display());
            }
            if removed > 0 {
                eprintln!("[wolfnet] Config cleanup: collapsed {} duplicate-key peer(s) in {}", removed, path.display());
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

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "\
[network]
address = \"10.10.10.30\"
listen_port = 9600
subnet = 24

[security]
private_key_file = \"/etc/wolfnet/private.key\"
";

    /// Write `content` to a per-test temp file so parallel test threads (and
    /// the daemon's own .bak/.tmp siblings) never collide.
    fn temp_config(tag: &str, content: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir()
            .join(format!("wolfnet_cfgtest_{}_{}.toml", std::process::id(), tag));
        std::fs::write(&path, content).expect("write temp config");
        path
    }

    fn cleanup(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("toml.bak"));
        let _ = std::fs::remove_file(path.with_extension("toml.tmp"));
    }

    fn names(c: &Config) -> Vec<String> {
        c.peers.iter().map(|p| p.name.clone().unwrap_or_default()).collect()
    }

    /// The core regression (WolfNet Bug Report Bug 2 / test-plan T3): peers
    /// with UNIQUE public keys must never be removed as "duplicates", even
    /// when two of them share an allowed_ip. The pre-fix dedup removed by key
    /// OR ip and overwrote config.toml, wiping unique-key peers (Gary KO4BSR:
    /// 7 blade peers deleted on a SIGHUP reload). All distinct identities must
    /// survive a load AND the write-back it can trigger AND a second reload.
    #[test]
    fn keeps_unique_key_peers_even_on_shared_ip() {
        let content = format!("{}
[[peers]]
name = \"kaha\"
public_key = \"key-kaha\"
allowed_ip = \"10.10.10.31\"
endpoint = \"10.10.1.31:9600\"

[[peers]]
name = \"pve02\"
public_key = \"key-pve02\"
allowed_ip = \"10.10.10.22\"
endpoint = \"10.10.1.22:9600\"

[[peers]]
name = \"pve03\"
public_key = \"key-pve03\"
allowed_ip = \"10.10.10.22\"
endpoint = \"10.10.1.23:9600\"
", HEADER);
        let path = temp_config("shared_ip", &content);
        let cfg = Config::load(&path).expect("load");
        assert_eq!(cfg.peers.len(), 3, "all 3 unique-key peers must survive; got {:?}", names(&cfg));
        assert!(names(&cfg).contains(&"pve03".to_string()),
            "pve03 (shares an IP but has a unique key) must NOT be deleted");
        // A second load simulates a SIGHUP reload — nothing must erode.
        let cfg2 = Config::load(&path).expect("reload");
        assert_eq!(cfg2.peers.len(), 3, "reload must not erode peers");
        cleanup(&path);
    }

    /// A genuinely duplicated public_key (the same identity listed twice) IS
    /// collapsed, and the entry carrying an endpoint wins so a reload can't
    /// drop a working endpoint for a bare twin.
    #[test]
    fn collapses_true_key_duplicate_preferring_endpoint() {
        // Bare twin first, endpoint-bearing twin second — the endpoint must win.
        let content = format!("{}
[[peers]]
name = \"twin-bare\"
public_key = \"key-dup\"
allowed_ip = \"10.10.10.40\"

[[peers]]
name = \"twin-endpoint\"
public_key = \"key-dup\"
allowed_ip = \"10.10.10.40\"
endpoint = \"10.10.1.40:9600\"
", HEADER);
        let path = temp_config("key_dup", &content);
        let cfg = Config::load(&path).expect("load");
        assert_eq!(cfg.peers.len(), 1, "same-key entries collapse to one");
        assert_eq!(cfg.peers[0].endpoint.as_deref(), Some("10.10.1.40:9600"),
            "the endpoint-bearing twin must win the collapse");
        cleanup(&path);
    }

    /// A load -> save -> reload round-trip preserves every peer and its
    /// endpoint (the daemon-side half of "endpoints not persisted").
    #[test]
    fn round_trip_preserves_all_endpoints() {
        let content = format!("{}
[[peers]]
name = \"a\"
public_key = \"key-a\"
allowed_ip = \"10.10.10.41\"
endpoint = \"10.10.1.41:9600\"

[[peers]]
name = \"b\"
public_key = \"key-b\"
allowed_ip = \"10.10.10.42\"
", HEADER);
        let path = temp_config("roundtrip", &content);
        let cfg = Config::load(&path).expect("load");
        cfg.save(&path).expect("save");
        let cfg2 = Config::load(&path).expect("reload");
        assert_eq!(cfg2.peers.len(), 2);
        let a = cfg2.peers.iter().find(|p| p.name.as_deref() == Some("a")).unwrap();
        assert_eq!(a.endpoint.as_deref(), Some("10.10.1.41:9600"), "pinned endpoint preserved");
        let b = cfg2.peers.iter().find(|p| p.name.as_deref() == Some("b")).unwrap();
        assert_eq!(b.endpoint, None, "auto-discovery peer stays endpoint-less");
        cleanup(&path);
    }
}
