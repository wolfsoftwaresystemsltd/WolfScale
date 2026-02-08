//! UDP Broadcast Discovery for WolfDisk nodes
//!
//! Uses broadcast (255.255.255.255) for node discovery, same as WolfScale.

use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::thread;
use std::net::UdpSocket;

use tracing::{debug, info, warn};

use crate::config::NodeRole;

/// Discovery port
pub const DISCOVERY_PORT: u16 = 9501;

/// Discovery message prefix
const DISCOVERY_PREFIX: &str = "WOLFDISK";

/// Discovery message version
const DISCOVERY_VERSION: u8 = 1;

/// Role in discovery packet
#[derive(Debug, Clone, Copy)]
pub enum DiscoveryRole {
    Server,  // Leader or Follower - participates in replication
    Client,  // Client-only - mount access only
}

impl From<NodeRole> for DiscoveryRole {
    fn from(role: NodeRole) -> Self {
        match role {
            NodeRole::Client => DiscoveryRole::Client,
            _ => DiscoveryRole::Server,
        }
    }
}

/// Discovered peer information
#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    pub node_id: String,
    pub address: String,
    pub role: DiscoveryRole,
    pub is_leader: bool,
    pub last_seen: Instant,
}

/// Discovery service for finding cluster peers via UDP broadcast
pub struct Discovery {
    node_id: String,
    bind_address: String,
    role: DiscoveryRole,
    peers: Arc<RwLock<HashMap<String, DiscoveredPeer>>>,
    is_leader: Arc<RwLock<bool>>,
    running: Arc<RwLock<bool>>,
}

impl Discovery {
    /// Create a new discovery service
    pub fn new(node_id: String, bind_address: String, role: NodeRole) -> Self {
        Self {
            node_id,
            bind_address,
            role: role.into(),
            peers: Arc::new(RwLock::new(HashMap::new())),
            is_leader: Arc::new(RwLock::new(false)),
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Set leader status
    pub fn set_leader(&self, is_leader: bool) {
        *self.is_leader.write().unwrap() = is_leader;
    }

    /// Get list of discovered peers
    pub fn peers(&self) -> Vec<DiscoveredPeer> {
        self.peers.read().unwrap().values().cloned().collect()
    }

    /// Get current leader (if known)
    pub fn leader(&self) -> Option<DiscoveredPeer> {
        self.peers.read().unwrap()
            .values()
            .find(|p| p.is_leader)
            .cloned()
    }

    /// Start discovery in background threads
    pub fn start(&self) -> std::io::Result<()> {
        *self.running.write().unwrap() = true;

        // Start broadcaster thread
        let node_id = self.node_id.clone();
        let bind_address = self.bind_address.clone();
        let role = self.role;
        let is_leader = Arc::clone(&self.is_leader);
        let running = Arc::clone(&self.running);

        thread::spawn(move || {
            if let Err(e) = run_broadcaster(node_id, bind_address, role, is_leader, running) {
                warn!("Discovery broadcaster error: {}", e);
            }
        });

        // Start listener thread
        let node_id = self.node_id.clone();
        let peers = Arc::clone(&self.peers);
        let running = Arc::clone(&self.running);

        thread::spawn(move || {
            if let Err(e) = run_listener(node_id, peers, running) {
                warn!("Discovery listener error: {}", e);
            }
        });

        info!("Discovery started on port {} (UDP broadcast)", DISCOVERY_PORT);
        Ok(())
    }

    /// Stop discovery
    pub fn stop(&self) {
        *self.running.write().unwrap() = false;
    }
}

/// Format a discovery broadcast message
fn format_message(node_id: &str, address: &str, is_server: bool, is_leader: bool) -> String {
    let role = if is_server { "S" } else { "C" };
    let leader = if is_leader { "L" } else { "F" };
    format!(
        "{}|{}|{}|{}|{}|{}",
        DISCOVERY_PREFIX,
        DISCOVERY_VERSION,
        node_id,
        address,
        role,
        leader
    )
}

/// Parse a discovery broadcast message
fn parse_message(message: &str) -> Option<(String, String, bool, bool)> {
    let parts: Vec<&str> = message.split('|').collect();
    
    if parts.len() < 6 {
        return None;
    }

    // Validate prefix
    if parts[0] != DISCOVERY_PREFIX {
        return None;
    }

    // Validate version
    let version: u8 = parts[1].parse().ok()?;
    if version != DISCOVERY_VERSION {
        return None;
    }

    let node_id = parts[2].to_string();
    let address = parts[3].to_string();
    let is_server = parts[4] == "S";
    let is_leader = parts[5] == "L";

    Some((node_id, address, is_server, is_leader))
}

/// Run the discovery broadcaster
fn run_broadcaster(
    node_id: String,
    bind_address: String,
    role: DiscoveryRole,
    is_leader: Arc<RwLock<bool>>,
    running: Arc<RwLock<bool>>,
) -> std::io::Result<()> {
    // Create UDP socket for broadcasting
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_broadcast(true)?;
    
    let broadcast_addr: SocketAddr = format!("255.255.255.255:{}", DISCOVERY_PORT)
        .parse()
        .unwrap();

    info!("Discovery broadcaster started for node {}", node_id);

    while *running.read().unwrap() {
        let is_server = matches!(role, DiscoveryRole::Server);
        let leader = *is_leader.read().unwrap();
        
        let message = format_message(&node_id, &bind_address, is_server, leader);

        match socket.send_to(message.as_bytes(), broadcast_addr) {
            Ok(_) => debug!("Discovery broadcast sent: {}", message),
            Err(e) => debug!("Broadcast send failed: {}", e),
        }

        thread::sleep(Duration::from_secs(2));
    }

    Ok(())
}

/// Run the discovery listener
fn run_listener(
    node_id: String,
    peers: Arc<RwLock<HashMap<String, DiscoveredPeer>>>,
    running: Arc<RwLock<bool>>,
) -> std::io::Result<()> {
    // Bind to discovery port
    let socket = match UdpSocket::bind(format!("0.0.0.0:{}", DISCOVERY_PORT)) {
        Ok(s) => {
            info!("Discovery listener bound to port {}", DISCOVERY_PORT);
            s
        }
        Err(e) => {
            warn!("Failed to bind discovery listener on port {}: {}", DISCOVERY_PORT, e);
            return Err(e);
        }
    };
    
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;

    let mut buf = [0u8; 512];
    let stale_threshold = Duration::from_secs(10);

    while *running.read().unwrap() {
        match socket.recv_from(&mut buf) {
            Ok((len, src)) => {
                if let Ok(message) = std::str::from_utf8(&buf[..len]) {
                    if let Some((msg_node_id, msg_address, is_server, is_leader)) = parse_message(message) {
                        // Skip our own broadcasts
                        if msg_node_id == node_id {
                            continue;
                        }

                        info!("Discovered peer: {} at {} (from {})", msg_node_id, msg_address, src);

                        let role = if is_server { DiscoveryRole::Server } else { DiscoveryRole::Client };
                        
                        peers.write().unwrap().insert(
                            msg_node_id.clone(),
                            DiscoveredPeer {
                                node_id: msg_node_id,
                                address: msg_address,
                                role,
                                is_leader,
                                last_seen: Instant::now(),
                            },
                        );
                    }
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // Timeout - clean up stale peers
                peers.write().unwrap().retain(|_, peer| {
                    peer.last_seen.elapsed() < stale_threshold
                });
            }
            Err(e) => {
                debug!("Discovery recv error: {}", e);
            }
        }
    }

    Ok(())
}
