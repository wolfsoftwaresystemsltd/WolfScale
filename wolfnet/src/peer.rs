//! Peer management for WolfNet
//!
//! Tracks connected peers, their keys, endpoints, and session state.
//! Supports peer exchange (PEX) for automatic mesh topology propagation.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
#[allow(unused_imports)]
use std::sync::{Arc, RwLock};
use std::time::Instant;
use x25519_dalek::{PublicKey, StaticSecret};
use tracing::{info, debug};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::crypto::{SessionCipher, KeyPair};
use crate::transport::PexEntry;

/// Information about a known peer
pub struct Peer {
    /// Peer's public key
    pub public_key: PublicKey,
    /// Peer's 4-byte ID (hash of public key)
    pub peer_id: [u8; 4],
    /// Peer's IP on the WolfNet virtual network
    pub wolfnet_ip: Ipv4Addr,
    /// Peer's real endpoint (public IP:port)
    pub endpoint: Option<SocketAddr>,
    /// Peer's hostname
    pub hostname: String,
    /// Session cipher for encrypted comms
    pub cipher: Option<SessionCipher>,
    /// Whether this peer is a gateway
    pub is_gateway: bool,
    /// Last time we heard from this peer
    pub last_seen: Option<Instant>,
    /// Bytes received from this peer
    pub rx_bytes: u64,
    /// Bytes sent to this peer
    pub tx_bytes: u64,
    /// Last handshake time
    pub last_handshake: Option<Instant>,
    /// If we learned about this peer via PEX, which peer told us (relay via)
    /// This is the WolfNet IP of the peer that shared this entry with us
    pub relay_via: Option<Ipv4Addr>,
}

impl Peer {
    /// Create a new peer from config
    pub fn new(public_key: PublicKey, wolfnet_ip: Ipv4Addr) -> Self {
        let peer_id = KeyPair::peer_id(&public_key);
        Self {
            public_key,
            peer_id,
            wolfnet_ip,
            endpoint: None,
            hostname: String::new(),
            cipher: None,
            is_gateway: false,
            last_seen: None,
            rx_bytes: 0,
            tx_bytes: 0,
            last_handshake: None,
            relay_via: None,
        }
    }

    /// Establish a session with this peer using our secret key
    pub fn establish_session(&mut self, my_secret: &StaticSecret, my_public: &PublicKey) {
        let shared = my_secret.diffie_hellman(&self.public_key);
        self.cipher = Some(SessionCipher::new(shared.as_bytes(), my_public, &self.public_key));
        self.last_handshake = Some(Instant::now());
        info!("Session established with {} ({})", self.hostname, self.wolfnet_ip);
    }

    /// Check if this peer has an active session
    pub fn is_connected(&self) -> bool {
        self.cipher.is_some() && self.last_seen.map_or(false, |t| t.elapsed().as_secs() < 120)
    }

    /// Encrypt a packet for this peer
    pub fn encrypt(&mut self, data: &[u8]) -> Result<(u64, Vec<u8>), Box<dyn std::error::Error + Send + Sync>> {
        let cipher = self.cipher.as_mut().ok_or("No session established")?;
        let result = cipher.encrypt(data)?;
        self.tx_bytes += data.len() as u64;
        Ok(result)
    }

    /// Decrypt a packet from this peer
    pub fn decrypt(&mut self, counter: u64, data: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let cipher = self.cipher.as_mut().ok_or("No session established")?;
        let result = cipher.decrypt(counter, data)?;
        self.rx_bytes += result.len() as u64;
        self.last_seen = Some(Instant::now());
        Ok(result)
    }
}

/// Manages all known peers
pub struct PeerManager {
    /// Peers indexed by WolfNet IP
    peers_by_ip: Arc<RwLock<HashMap<Ipv4Addr, Peer>>>,
    /// Peer ID → WolfNet IP mapping for fast packet routing
    id_to_ip: Arc<RwLock<HashMap<[u8; 4], Ipv4Addr>>>,
    /// Endpoint → WolfNet IP mapping for incoming packet routing
    endpoint_to_ip: Arc<RwLock<HashMap<SocketAddr, Ipv4Addr>>>,
}

impl PeerManager {
    pub fn new() -> Self {
        Self {
            peers_by_ip: Arc::new(RwLock::new(HashMap::new())),
            id_to_ip: Arc::new(RwLock::new(HashMap::new())),
            endpoint_to_ip: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a peer
    pub fn add_peer(&self, peer: Peer) {
        let ip = peer.wolfnet_ip;
        let peer_id = peer.peer_id;
        info!("Added peer: {} ({}) id={}", ip, peer.hostname, hex::encode(peer_id));

        if let Some(endpoint) = peer.endpoint {
            self.endpoint_to_ip.write().unwrap().insert(endpoint, ip);
        }
        self.id_to_ip.write().unwrap().insert(peer_id, ip);
        self.peers_by_ip.write().unwrap().insert(ip, peer);
    }

    /// Get a mutable reference to a peer by WolfNet IP (via callback to avoid lock issues)
    pub fn with_peer_by_ip<F, R>(&self, ip: &Ipv4Addr, f: F) -> Option<R>
    where F: FnOnce(&mut Peer) -> R {
        let mut peers = self.peers_by_ip.write().unwrap();
        peers.get_mut(ip).map(f)
    }

    /// Find peer by incoming endpoint address
    pub fn find_ip_by_endpoint(&self, addr: &SocketAddr) -> Option<Ipv4Addr> {
        self.endpoint_to_ip.read().unwrap().get(addr).copied()
    }

    /// Find peer by peer ID
    pub fn find_ip_by_id(&self, id: &[u8; 4]) -> Option<Ipv4Addr> {
        self.id_to_ip.read().unwrap().get(id).copied()
    }

    /// Update a peer's endpoint (e.g. after receiving a packet from a new address)
    pub fn update_endpoint(&self, ip: &Ipv4Addr, new_endpoint: SocketAddr) {
        let mut peers = self.peers_by_ip.write().unwrap();
        if let Some(peer) = peers.get_mut(ip) {
            if let Some(old) = peer.endpoint {
                if old != new_endpoint {
                    self.endpoint_to_ip.write().unwrap().remove(&old);
                    debug!("Peer {} endpoint changed: {} -> {}", ip, old, new_endpoint);
                }
            }
            peer.endpoint = Some(new_endpoint);
            self.endpoint_to_ip.write().unwrap().insert(new_endpoint, *ip);
        }
    }

    /// Update a peer's endpoint and hostname from discovery
    pub fn update_from_discovery(&self, public_key: &PublicKey, endpoint: SocketAddr, wolfnet_ip: Ipv4Addr, hostname: &str, is_gateway: bool) {
        let mut peers = self.peers_by_ip.write().unwrap();
        
        // First check: does a peer with this exact IP exist?
        if let Some(peer) = peers.get_mut(&wolfnet_ip) {
            if peer.public_key == *public_key {
                // Same IP, same key — just update endpoint
                // Clean up old endpoint mapping first
                if let Some(old_ep) = peer.endpoint {
                    if old_ep != endpoint {
                        self.endpoint_to_ip.write().unwrap().remove(&old_ep);
                        debug!("Peer {} endpoint updated: {} -> {} (discovery)", wolfnet_ip, old_ep, endpoint);
                    }
                }
                peer.endpoint = Some(endpoint);
                peer.hostname = hostname.to_string();
                peer.is_gateway = is_gateway;
                // Direct discovery clears relay — we can reach them directly
                peer.relay_via = None;
                self.endpoint_to_ip.write().unwrap().insert(endpoint, wolfnet_ip);
                return;
            }
        }
        
        // Second check: does a peer with this public key exist under a DIFFERENT IP?
        // This handles the case where a peer's address changed (e.g., token join changed their IP)
        let existing_ip = peers.iter()
            .find(|(_, p)| p.public_key == *public_key)
            .map(|(ip, _)| *ip);
        
        if let Some(old_ip) = existing_ip {
            if old_ip != wolfnet_ip {
                // Peer changed their WolfNet IP — migrate to new IP
                info!("Peer {} ({}) changed IP: {} -> {}", hostname, hex::encode(KeyPair::peer_id(public_key)), old_ip, wolfnet_ip);
                let mut peer = peers.remove(&old_ip).unwrap();
                
                // Update old endpoint mapping
                if let Some(old_endpoint) = peer.endpoint {
                    self.endpoint_to_ip.write().unwrap().remove(&old_endpoint);
                }
                
                // Update peer with new info
                peer.wolfnet_ip = wolfnet_ip;
                peer.endpoint = Some(endpoint);
                peer.hostname = hostname.to_string();
                peer.is_gateway = is_gateway;
                peer.relay_via = None;
                let peer_id = peer.peer_id;
                
                // Update all mappings
                self.endpoint_to_ip.write().unwrap().insert(endpoint, wolfnet_ip);
                self.id_to_ip.write().unwrap().insert(peer_id, wolfnet_ip);
                peers.insert(wolfnet_ip, peer);
                return;
            }
        }
        
        // Entirely new peer discovered on LAN
        let mut peer = Peer::new(*public_key, wolfnet_ip);
        peer.endpoint = Some(endpoint);
        peer.hostname = hostname.to_string();
        peer.is_gateway = is_gateway;
        let peer_id = peer.peer_id;

        self.endpoint_to_ip.write().unwrap().insert(endpoint, wolfnet_ip);
        self.id_to_ip.write().unwrap().insert(peer_id, wolfnet_ip);
        peers.insert(wolfnet_ip, peer);
        info!("Discovered new peer: {} ({}) at {}", hostname, wolfnet_ip, endpoint);
    }

    /// Get all peer IPs
    pub fn all_ips(&self) -> Vec<Ipv4Addr> {
        self.peers_by_ip.read().unwrap().keys().copied().collect()
    }

    /// Find a connected gateway peer to route traffic through
    /// Returns the WolfNet IP of the first connected gateway peer
    pub fn find_gateway(&self) -> Option<Ipv4Addr> {
        let peers = self.peers_by_ip.read().unwrap();
        peers.iter()
            .find(|(_, p)| p.is_gateway && p.is_connected())
            .map(|(ip, _)| *ip)
    }

    /// Find the relay peer for a given destination IP
    /// If we learned about dest_ip via PEX from another peer, return that peer's IP
    pub fn find_relay_for(&self, dest_ip: &Ipv4Addr) -> Option<Ipv4Addr> {
        let peers = self.peers_by_ip.read().unwrap();
        if let Some(peer) = peers.get(dest_ip) {
            // If this peer has a relay_via and isn't directly connected, use the relay
            if !peer.is_connected() {
                return peer.relay_via;
            }
        }
        None
    }

    /// Get peer count
    pub fn count(&self) -> usize {
        self.peers_by_ip.read().unwrap().len()
    }

    /// Build PEX entries for all known peers (to share with others)
    /// Excludes the requesting peer's own IP and our own IP
    pub fn get_pex_entries(&self, my_ip: Ipv4Addr) -> Vec<PexEntry> {
        let peers = self.peers_by_ip.read().unwrap();
        peers.values()
            .filter(|p| p.wolfnet_ip != my_ip)
            .map(|p| {
                PexEntry {
                    public_key: BASE64.encode(p.public_key.as_bytes()),
                    wolfnet_ip: p.wolfnet_ip.to_string(),
                    endpoint: p.endpoint.map(|e| e.to_string()),
                    hostname: p.hostname.clone(),
                    is_gateway: p.is_gateway,
                }
            })
            .collect()
    }

    /// Process received PEX entries from a peer
    /// Adds new peers we haven't seen before, marking them as relay-via the sender
    pub fn add_from_pex(
        &self,
        entries: &[PexEntry],
        sender_ip: Ipv4Addr,
        my_ip: Ipv4Addr,
        keypair: &KeyPair,
    ) {
        let mut peers = self.peers_by_ip.write().unwrap();

        for entry in entries {
            // Skip ourselves
            let entry_ip: Ipv4Addr = match entry.wolfnet_ip.parse() {
                Ok(ip) => ip,
                Err(_) => continue,
            };
            if entry_ip == my_ip { continue; }

            // Skip if we already know this peer directly (LAN discovery or configured)
            if let Some(existing) = peers.get(&entry_ip) {
                if existing.is_connected() || existing.relay_via.is_none() {
                    // Already directly connected or manually configured — don't overwrite
                    continue;
                }
            }

            // Parse the public key
            let pub_key = match crate::crypto::parse_public_key(&entry.public_key) {
                Ok(k) => k,
                Err(_) => continue,
            };

            // Check if peer already exists by public key under a different IP
            let existing_by_key = peers.iter()
                .find(|(_, p)| p.public_key == pub_key)
                .map(|(ip, _)| *ip);
            if let Some(existing_ip) = existing_by_key {
                if existing_ip != entry_ip {
                    // Key exists under different IP — skip to avoid confusion
                    continue;
                }
            }

            if peers.contains_key(&entry_ip) {
                // Update relay_via if not directly connected
                if let Some(peer) = peers.get_mut(&entry_ip) {
                    if !peer.is_connected() {
                        peer.relay_via = Some(sender_ip);
                    }
                }
                continue;
            }

            // New peer from PEX — add it with relay routing through sender
            let mut peer = Peer::new(pub_key, entry_ip);
            peer.hostname = entry.hostname.clone();
            peer.is_gateway = entry.is_gateway;
            peer.relay_via = Some(sender_ip);

            // Parse endpoint if available
            if let Some(ref ep_str) = entry.endpoint {
                if let Ok(ep) = ep_str.parse::<SocketAddr>() {
                    peer.endpoint = Some(ep);
                    self.endpoint_to_ip.write().unwrap().insert(ep, entry_ip);
                }
            }

            // Establish crypto session so we can encrypt/decrypt
            peer.establish_session(&keypair.secret, &keypair.public);

            let peer_id = peer.peer_id;
            self.id_to_ip.write().unwrap().insert(peer_id, entry_ip);
            info!("PEX: learned about {} ({}) via {}", entry.hostname, entry_ip, sender_ip);
            peers.insert(entry_ip, peer);
        }
    }

    /// Collect status info for all peers
    pub fn status(&self) -> Vec<crate::config::PeerStatus> {
        let peers = self.peers_by_ip.read().unwrap();
        peers.values().map(|p| {
            crate::config::PeerStatus {
                hostname: p.hostname.clone(),
                address: p.wolfnet_ip.to_string(),
                endpoint: p.endpoint.map_or("-".into(), |e| e.to_string()),
                public_key: BASE64.encode(p.public_key.as_bytes()),
                last_seen_secs: p.last_seen.map_or(u64::MAX, |t| t.elapsed().as_secs()),
                rx_bytes: p.rx_bytes,
                tx_bytes: p.tx_bytes,
                connected: p.is_connected(),
                is_gateway: p.is_gateway,
                relay_via: p.relay_via.map(|ip| ip.to_string()),
            }
        }).collect()
    }
}
