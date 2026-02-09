//! Network transport layer for WolfNet
//!
//! Handles UDP packet framing, handshake protocol, discovery broadcasts,
//! and peer exchange (PEX) for automatic mesh topology propagation.

use std::net::{UdpSocket, SocketAddr, Ipv4Addr};
use std::sync::Arc;
use tracing::{info, debug, warn};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::crypto::KeyPair;
use crate::peer::PeerManager;

/// Packet types
pub const PKT_HANDSHAKE: u8 = 0x01;
pub const PKT_DATA: u8 = 0x03;
pub const PKT_KEEPALIVE: u8 = 0x04;
pub const PKT_DISCOVERY: u8 = 0x05;
pub const PKT_PEER_EXCHANGE: u8 = 0x06;

/// Discovery port (UDP broadcast)
pub const DISCOVERY_PORT: u16 = 9601;
const DISCOVERY_PREFIX: &str = "WOLFNET";

/// Build a handshake packet:
/// [1: type] [32: public_key] [4: wolfnet_ip] [2: listen_port] [1: is_gateway] [N: hostname]
pub fn build_handshake(keypair: &KeyPair, wolfnet_ip: Ipv4Addr, listen_port: u16, hostname: &str, is_gateway: bool) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(40 + hostname.len());
    pkt.push(PKT_HANDSHAKE);
    pkt.extend_from_slice(keypair.public.as_bytes());
    pkt.extend_from_slice(&wolfnet_ip.octets());
    pkt.extend_from_slice(&listen_port.to_le_bytes());
    pkt.push(if is_gateway { 1 } else { 0 });
    pkt.extend_from_slice(hostname.as_bytes());
    pkt
}

/// Parse a handshake packet
pub fn parse_handshake(data: &[u8]) -> Option<(x25519_dalek::PublicKey, Ipv4Addr, u16, bool, String)> {
    if data.len() < 40 || data[0] != PKT_HANDSHAKE {
        return None;
    }
    let mut key_bytes = [0u8; 32];
    key_bytes.copy_from_slice(&data[1..33]);
    let public_key = x25519_dalek::PublicKey::from(key_bytes);

    let ip = Ipv4Addr::new(data[33], data[34], data[35], data[36]);
    let port = u16::from_le_bytes([data[37], data[38]]);
    let is_gateway = data[39] != 0;
    let hostname = String::from_utf8_lossy(&data[40..]).to_string();

    Some((public_key, ip, port, is_gateway, hostname))
}

/// Build a data packet:
/// [1: type] [4: peer_id] [8: nonce_counter] [N: encrypted_payload]
pub fn build_data_packet(peer_id: &[u8; 4], counter: u64, ciphertext: &[u8]) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(13 + ciphertext.len());
    pkt.push(PKT_DATA);
    pkt.extend_from_slice(peer_id);
    pkt.extend_from_slice(&counter.to_le_bytes());
    pkt.extend_from_slice(ciphertext);
    pkt
}

/// Parse a data packet header, returns (peer_id, counter, ciphertext)
pub fn parse_data_packet(data: &[u8]) -> Option<([u8; 4], u64, &[u8])> {
    if data.len() < 14 || data[0] != PKT_DATA {
        return None;
    }
    let mut peer_id = [0u8; 4];
    peer_id.copy_from_slice(&data[1..5]);
    let counter = u64::from_le_bytes(data[5..13].try_into().ok()?);
    Some((peer_id, counter, &data[13..]))
}

/// Build a keepalive packet
pub fn build_keepalive(peer_id: &[u8; 4]) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(5);
    pkt.push(PKT_KEEPALIVE);
    pkt.extend_from_slice(peer_id);
    pkt
}

/// Send handshakes to all peers that don't have active sessions
/// For PEX-learned peers, also sends via the relay peer so the handshake
/// gets forwarded through the mesh even when direct connectivity is impossible.
pub fn send_handshakes(
    socket: &UdpSocket,
    keypair: &KeyPair,
    peer_manager: &PeerManager,
    wolfnet_ip: Ipv4Addr,
    listen_port: u16,
    hostname: &str,
    is_gateway: bool,
) {
    let handshake = build_handshake(keypair, wolfnet_ip, listen_port, hostname, is_gateway);
    for ip in peer_manager.all_ips() {
        peer_manager.with_peer_by_ip(&ip, |peer| {
            if !peer.is_connected() {
                // Try direct handshake first
                if let Some(endpoint) = peer.endpoint {
                    debug!("Sending handshake to {} at {}", ip, endpoint);
                    if let Err(e) = socket.send_to(&handshake, endpoint) {
                        debug!("Handshake to {} at {} failed: {}", ip, endpoint, e);
                    }
                }
            }
        });
    }
}

/// Send keepalives to all connected peers
pub fn send_keepalives(socket: &UdpSocket, keypair: &KeyPair, peer_manager: &PeerManager) {
    let my_id = keypair.my_peer_id();
    let keepalive = build_keepalive(&my_id);
    for ip in peer_manager.all_ips() {
        peer_manager.with_peer_by_ip(&ip, |peer| {
            if peer.is_connected() {
                if let Some(endpoint) = peer.endpoint {
                    let _ = socket.send_to(&keepalive, endpoint);
                    // Note: do NOT set last_seen here — last_seen should only be updated
                    // when we actually RECEIVE a packet from this peer, not when we send.
                }
            }
        });
    }
}

/// Format a discovery broadcast message
pub fn format_discovery(
    node_id: &str,
    public_key: &x25519_dalek::PublicKey,
    wolfnet_ip: Ipv4Addr,
    listen_port: u16,
    hostname: &str,
    is_gateway: bool,
) -> String {
    format!(
        "{}|1|{}|{}|{}|{}|{}|{}",
        DISCOVERY_PREFIX, node_id,
        BASE64.encode(public_key.as_bytes()),
        wolfnet_ip, listen_port, hostname,
        if is_gateway { "G" } else { "N" },
    )
}

/// Parse a discovery broadcast message
pub fn parse_discovery(message: &str) -> Option<(String, x25519_dalek::PublicKey, Ipv4Addr, u16, String, bool)> {
    let parts: Vec<&str> = message.split('|').collect();
    if parts.len() < 8 || parts[0] != DISCOVERY_PREFIX {
        return None;
    }
    let node_id = parts[2].to_string();
    let public_key = crate::crypto::parse_public_key(parts[3]).ok()?;
    let wolfnet_ip: Ipv4Addr = parts[4].parse().ok()?;
    let listen_port: u16 = parts[5].parse().ok()?;
    let hostname = parts[6].to_string();
    let is_gateway = parts[7] == "G";
    Some((node_id, public_key, wolfnet_ip, listen_port, hostname, is_gateway))
}

/// A single peer entry in a peer exchange message
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct PexEntry {
    /// Base64 encoded public key
    pub public_key: String,
    /// WolfNet IP address
    pub wolfnet_ip: String,
    /// Real endpoint (ip:port) if known
    pub endpoint: Option<String>,
    /// Hostname
    pub hostname: String,
    /// Whether this peer is a gateway
    pub is_gateway: bool,
}

/// Build a peer exchange packet:
/// [1: type] [N: JSON array of PexEntry]
pub fn build_peer_exchange(my_ip: Ipv4Addr, peer_manager: &PeerManager) -> Vec<u8> {
    let entries = peer_manager.get_pex_entries(my_ip);
    let mut pkt = Vec::new();
    pkt.push(PKT_PEER_EXCHANGE);
    if let Ok(json) = serde_json::to_vec(&entries) {
        pkt.extend_from_slice(&json);
    }
    pkt
}

/// Parse a peer exchange packet
pub fn parse_peer_exchange(data: &[u8]) -> Option<Vec<PexEntry>> {
    if data.is_empty() || data[0] != PKT_PEER_EXCHANGE {
        return None;
    }
    serde_json::from_slice(&data[1..]).ok()
}

/// Send peer exchange to all connected peers
pub fn send_peer_exchange(
    socket: &UdpSocket,
    keypair: &KeyPair,
    peer_manager: &PeerManager,
    my_ip: Ipv4Addr,
) {
    let pex_packet = build_peer_exchange(my_ip, peer_manager);
    if pex_packet.len() <= 1 { return; } // No peers to share

    for ip in peer_manager.all_ips() {
        peer_manager.with_peer_by_ip(&ip, |peer| {
            if peer.is_connected() {
                if let Some(endpoint) = peer.endpoint {
                    match peer.encrypt(&pex_packet) {
                        Ok((counter, ciphertext)) => {
                            let pkt = build_data_packet(&keypair.my_peer_id(), counter, &ciphertext);
                            if let Err(e) = socket.send_to(&pkt, endpoint) {
                                debug!("PEX send error to {}: {}", ip, e);
                            } else {
                                debug!("Sent peer exchange to {} ({} bytes)", ip, pex_packet.len());
                            }
                        }
                        Err(e) => debug!("PEX encrypt error for {}: {}", ip, e),
                    }
                }
            }
        });
    }
}

/// Run discovery broadcaster in a loop (call from a thread)
pub fn run_discovery_broadcaster(
    wolfnet_ip: Ipv4Addr,
    public_key: x25519_dalek::PublicKey,
    listen_port: u16,
    hostname: String,
    is_gateway: bool,
    running: Arc<std::sync::atomic::AtomicBool>,
) {
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => { warn!("Discovery broadcaster bind failed: {}", e); return; }
    };
    socket.set_broadcast(true).ok();

    let node_id = hostname.clone();
    let broadcast_addr: SocketAddr = format!("255.255.255.255:{}", DISCOVERY_PORT).parse().unwrap();

    info!("Discovery broadcaster started");
    while running.load(std::sync::atomic::Ordering::Relaxed) {
        let msg = format_discovery(&node_id, &public_key, wolfnet_ip, listen_port, &hostname, is_gateway);
        let _ = socket.send_to(msg.as_bytes(), broadcast_addr);
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

/// Run discovery listener in a loop (call from a thread)
pub fn run_discovery_listener(
    my_node_id: String,
    keypair: Arc<KeyPair>,
    peer_manager: Arc<PeerManager>,
    running: Arc<std::sync::atomic::AtomicBool>,
) {
    let socket = match UdpSocket::bind(format!("0.0.0.0:{}", DISCOVERY_PORT)) {
        Ok(s) => s,
        Err(e) => { warn!("Discovery listener bind failed: {}", e); return; }
    };
    socket.set_read_timeout(Some(std::time::Duration::from_secs(1))).ok();

    info!("Discovery listener started on port {}", DISCOVERY_PORT);
    let mut buf = [0u8; 512];

    while running.load(std::sync::atomic::Ordering::Relaxed) {
        match socket.recv_from(&mut buf) {
            Ok((len, src)) => {
                if let Ok(msg) = std::str::from_utf8(&buf[..len]) {
                    if let Some((node_id, pub_key, wolfnet_ip, listen_port, hostname, is_gateway)) = parse_discovery(msg) {
                        if node_id == my_node_id { continue; }

                        // Use source IP with the advertised listen port
                        let endpoint = SocketAddr::new(src.ip(), listen_port);
                        peer_manager.update_from_discovery(&pub_key, endpoint, wolfnet_ip, &hostname, is_gateway);

                        // Always re-establish session on discovery — handles counter
                        // reset if the peer restarted
                        peer_manager.with_peer_by_ip(&wolfnet_ip, |peer| {
                            peer.establish_session(&keypair.secret, &keypair.public);
                        });
                    }
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => debug!("Discovery recv error: {}", e),
        }
    }
}
