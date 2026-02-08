//! UDP multicast discovery for WolfDisk nodes

use std::net::{SocketAddr, UdpSocket, Ipv4Addr};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::thread;

use tracing::{debug, info, warn};
use serde::{Deserialize, Serialize};

use crate::config::NodeRole;

/// Default multicast group for discovery
pub const DEFAULT_MULTICAST_ADDR: &str = "239.255.0.1";
pub const DEFAULT_MULTICAST_PORT: u16 = 9501;

/// Discovery announcement packet
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryPacket {
    pub node_id: String,
    pub address: String,
    pub role: DiscoveryRole,
    pub is_leader: bool,
}

/// Role in discovery packet
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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

/// Discovery service for finding cluster peers
pub struct Discovery {
    node_id: String,
    bind_address: String,
    role: DiscoveryRole,
    multicast_addr: Ipv4Addr,
    multicast_port: u16,
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
            multicast_addr: DEFAULT_MULTICAST_ADDR.parse().unwrap(),
            multicast_port: DEFAULT_MULTICAST_PORT,
            peers: Arc::new(RwLock::new(HashMap::new())),
            is_leader: Arc::new(RwLock::new(false)),
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Set custom multicast address and port
    pub fn with_multicast(mut self, addr: Ipv4Addr, port: u16) -> Self {
        self.multicast_addr = addr;
        self.multicast_port = port;
        self
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

    /// Start discovery in background thread
    pub fn start(&self) -> std::io::Result<()> {
        *self.running.write().unwrap() = true;
        
        let node_id = self.node_id.clone();
        let bind_address = self.bind_address.clone();
        let role = self.role;
        let multicast_addr = self.multicast_addr;
        let multicast_port = self.multicast_port;
        let peers = Arc::clone(&self.peers);
        let is_leader = Arc::clone(&self.is_leader);
        let running = Arc::clone(&self.running);

        // Receiver thread
        let recv_peers = Arc::clone(&peers);
        let recv_running = Arc::clone(&running);
        let recv_node_id = node_id.clone();
        
        thread::spawn(move || {
            if let Err(e) = run_receiver(
                recv_node_id,
                multicast_addr,
                multicast_port,
                recv_peers,
                recv_running,
            ) {
                warn!("Discovery receiver error: {}", e);
            }
        });

        // Sender thread
        thread::spawn(move || {
            if let Err(e) = run_sender(
                node_id,
                bind_address,
                role,
                multicast_addr,
                multicast_port,
                is_leader,
                running,
            ) {
                warn!("Discovery sender error: {}", e);
            }
        });

        info!("Discovery started on {}:{}", multicast_addr, multicast_port);
        Ok(())
    }

    /// Stop discovery
    pub fn stop(&self) {
        *self.running.write().unwrap() = false;
    }
}

/// Run the discovery receiver
fn run_receiver(
    node_id: String,
    multicast_addr: Ipv4Addr,
    port: u16,
    peers: Arc<RwLock<HashMap<String, DiscoveredPeer>>>,
    running: Arc<RwLock<bool>>,
) -> std::io::Result<()> {
    let socket = UdpSocket::bind(("0.0.0.0", port))?;
    socket.join_multicast_v4(&multicast_addr, &Ipv4Addr::UNSPECIFIED)?;
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;

    let mut buf = [0u8; 1024];

    while *running.read().unwrap() {
        match socket.recv_from(&mut buf) {
            Ok((len, _src)) => {
                if let Ok(packet) = bincode::deserialize::<DiscoveryPacket>(&buf[..len]) {
                    // Don't add ourselves
                    if packet.node_id != node_id {
                        debug!("Discovered peer: {} at {}", packet.node_id, packet.address);
                        
                        peers.write().unwrap().insert(
                            packet.node_id.clone(),
                            DiscoveredPeer {
                                node_id: packet.node_id,
                                address: packet.address,
                                role: packet.role,
                                is_leader: packet.is_leader,
                                last_seen: Instant::now(),
                            },
                        );
                    }
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // Timeout, check for stale peers
                let stale_threshold = Duration::from_secs(10);
                peers.write().unwrap().retain(|_, peer| {
                    peer.last_seen.elapsed() < stale_threshold
                });
            }
            Err(e) => {
                warn!("Discovery receive error: {}", e);
            }
        }
    }

    Ok(())
}

/// Run the discovery sender
fn run_sender(
    node_id: String,
    bind_address: String,
    role: DiscoveryRole,
    multicast_addr: Ipv4Addr,
    port: u16,
    is_leader: Arc<RwLock<bool>>,
    running: Arc<RwLock<bool>>,
) -> std::io::Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    let dest: SocketAddr = (multicast_addr, port).into();

    while *running.read().unwrap() {
        let packet = DiscoveryPacket {
            node_id: node_id.clone(),
            address: bind_address.clone(),
            role,
            is_leader: *is_leader.read().unwrap(),
        };

        if let Ok(data) = bincode::serialize(&packet) {
            let _ = socket.send_to(&data, dest);
        }

        thread::sleep(Duration::from_secs(2));
    }

    Ok(())
}
