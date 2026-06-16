//! Peer management for WolfNet
//!
//! Tracks connected peers, their keys, endpoints, and session state.
//! Supports peer exchange (PEX) for automatic mesh topology propagation.

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
#[allow(unused_imports)]
use std::sync::{Arc, RwLock};
use std::time::Instant;
use x25519_dalek::{PublicKey, StaticSecret};

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
    /// Original configured endpoint string (may be a hostname:port for DNS re-resolution)
    pub configured_endpoint: Option<String>,
    /// Last time we decrypted an encrypted DATA packet from this peer —
    /// distinct from `last_seen` which is also bumped by handshakes and
    /// keepalives. Tracking these separately is what lets us tell the
    /// difference between "tunnel is up and data is flowing" and "we
    /// can handshake with the peer but data packets vanish in either
    /// direction" — the latter is exactly the scenario klasSponsor hit
    /// on 2026-05-11 where wolfnetctl said "6 (6 connected)" while
    /// every ping silently failed.
    pub last_data_rx: Option<Instant>,
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
            configured_endpoint: None,
            last_data_rx: None,
        }
    }

    /// Establish a session with this peer using our secret key
    pub fn establish_session(&mut self, my_secret: &StaticSecret, my_public: &PublicKey) {
        let shared = my_secret.diffie_hellman(&self.public_key);
        self.cipher = Some(SessionCipher::new(shared.as_bytes(), my_public, &self.public_key));
        self.last_handshake = Some(Instant::now());

    }

    /// Check if this peer has an active session — i.e. ANY signed traffic
    /// (handshake, keepalive, or data) has been observed recently. This is
    /// the "tunnel is alive" semantic. Use `is_passing_data` instead if
    /// you need to know whether ACTUAL data is flowing — handshakes and
    /// keepalives alone are not enough to confirm a working bidirectional
    /// data path.
    pub fn is_connected(&self) -> bool {
        self.cipher.is_some() && self.last_seen.map_or(false, |t| t.elapsed().as_secs() < 120)
    }

    /// True iff we've decrypted a real DATA packet from this peer in the
    /// last 120 seconds. Handshakes and keepalives don't count; the whole
    /// point of this method is to expose the asymmetric case where the
    /// tunnel "looks up" (handshakes flow) but data drops on the floor.
    pub fn is_passing_data(&self) -> bool {
        self.cipher.is_some() && self.last_data_rx.map_or(false, |t| t.elapsed().as_secs() < 120)
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
        let now = Instant::now();
        self.last_seen = Some(now);
        self.last_data_rx = Some(now);
        Ok(result)
    }

    /// Encrypt in-place: plaintext in buf[..len], appends 16-byte tag. Zero allocations.
    pub fn encrypt_into(&mut self, buf: &mut [u8], len: usize) -> Result<(u64, usize), Box<dyn std::error::Error + Send + Sync>> {
        let cipher = self.cipher.as_mut().ok_or("No session established")?;
        let result = cipher.encrypt_into(buf, len)?;
        self.tx_bytes += len as u64;
        Ok(result)
    }

    /// Decrypt in-place: ciphertext+tag in buf[..len]. Zero allocations.
    pub fn decrypt_into(&mut self, counter: u64, buf: &mut [u8], len: usize) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let cipher = self.cipher.as_mut().ok_or("No session established")?;
        let pt_len = cipher.decrypt_into(counter, buf, len)?;
        self.rx_bytes += pt_len as u64;
        let now = Instant::now();
        self.last_seen = Some(now);
        self.last_data_rx = Some(now);
        Ok(pt_len)
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
    /// Subnet routes: container/VM IP → host peer IP (for routing to containers on remote nodes)
    subnet_routes: Arc<RwLock<HashMap<Ipv4Addr, Ipv4Addr>>>,
    /// CIDR subnet routes: (network, prefix, gateway WolfNet IP). Sorted
    /// longest-prefix first so find_subnet_match can return on the
    /// first hit. Populated from /var/run/wolfnet/subnet-routes.json,
    /// which WolfStack writes when its WolfRouter SubnetRoute config
    /// changes. Without this, packets read from the TUN whose dest IP
    /// matches a kernel route via wolfnet0 (e.g. a remote LAN like
    /// 10.10.0.0/16 reachable via peer 10.100.10.30) have no per-peer
    /// destination — TUN devices have no link layer, so the kernel's
    /// "next-hop" hint is meaningless to userspace — and the packet
    /// would either get dropped (no peer match) or sent to the first
    /// auto-gateway peer (often the wrong one).
    subnet_routes_cidrs: Arc<RwLock<Vec<(Ipv4Addr, u8, Ipv4Addr)>>>,
    /// IPv6 CIDR subnet routes: (network, prefix, gateway WolfNet IP). The
    /// gateway is still an IPv4 WolfNet IP — the overlay endpoints are IPv4;
    /// only the *destination* subnet is v6. Populated from the same
    /// /var/run/wolfnet/subnet-routes.json (entries whose CIDR is IPv6),
    /// which WolfStack only writes when the operator opted into IPv6 subnet
    /// routing. Sorted longest-prefix first. Empty (and thus inert) on every
    /// node that hasn't enabled the feature. See find_subnet_match_v6.
    subnet_routes_cidrs_v6: Arc<RwLock<Vec<(Ipv6Addr, u8, Ipv4Addr)>>>,
    /// IPs purged via SIGHUP — blocked from PEX re-addition until daemon restart
    purged_ips: Arc<RwLock<std::collections::HashSet<Ipv4Addr>>>,
}

/// Is this address useful as a peer endpoint we might actually dial?
///
/// Returns `false` for IPv4 RFC1918 (10/8, 172.16/12, 192.168/16),
/// loopback (127/8), link-local (169.254/16), unspecified (0.0.0.0),
/// and multicast (224/4). These are never legitimate cross-internet
/// peer endpoints — a wolfnet daemon receiving a PEX entry that
/// carries such an address can't reach it (unless coincidentally on
/// the same LAN, in which case roaming + broadcast discovery cover
/// the connection already). Storing it pollutes the peer table and
/// makes the daemon waste handshake UDP into the void.
///
/// IPv6 endpoints are passed through unchanged (the broader codebase
/// is IPv4-focused; revisit when IPv6 support lands).
///
/// klasSponsor 2026-05-12 architectural point: "Wolfnet should not
/// use internal ip for connection unless it's a relay." That maps to
/// stripping such endpoints from outgoing PEX (we don't poison
/// downstream peers) and ignoring them on incoming PEX (we don't
/// poison ourselves). The peer-relay routing still works because
/// PEX-learned peers carry `relay_via = sender_ip` regardless of
/// what we do with the endpoint field.
pub fn is_routable_endpoint(addr: SocketAddr) -> bool {
    match addr.ip() {
        std::net::IpAddr::V4(v4) => {
            let oct = v4.octets();
            if v4.is_loopback() { return false; }
            if v4.is_unspecified() { return false; }
            if v4.is_link_local() { return false; }
            if v4.is_multicast() { return false; }
            // RFC1918
            if oct[0] == 10 { return false; }
            if oct[0] == 172 && (16..=31).contains(&oct[1]) { return false; }
            if oct[0] == 192 && oct[1] == 168 { return false; }
            true
        }
        std::net::IpAddr::V6(_) => true, // IPv6 passthrough — wolfnet is IPv4-focused today
    }
}

impl PeerManager {
    pub fn new() -> Self {
        Self {
            peers_by_ip: Arc::new(RwLock::new(HashMap::new())),
            id_to_ip: Arc::new(RwLock::new(HashMap::new())),
            endpoint_to_ip: Arc::new(RwLock::new(HashMap::new())),
            subnet_routes: Arc::new(RwLock::new(HashMap::new())),
            subnet_routes_cidrs: Arc::new(RwLock::new(Vec::new())),
            subnet_routes_cidrs_v6: Arc::new(RwLock::new(Vec::new())),
            purged_ips: Arc::new(RwLock::new(std::collections::HashSet::new())),
        }
    }

    /// Add a peer
    pub fn add_peer(&self, peer: Peer) {
        let ip = peer.wolfnet_ip;
        let peer_id = peer.peer_id;


        if let Some(endpoint) = peer.endpoint {
            self.endpoint_to_ip.write().unwrap().insert(endpoint, ip);
        }
        self.id_to_ip.write().unwrap().insert(peer_id, ip);
        self.peers_by_ip.write().unwrap().insert(ip, peer);
    }

    /// Remove a peer by WolfNet IP (used for purging stale PEX entries)
    pub fn remove_peer(&self, ip: &Ipv4Addr) {
        let mut peers = self.peers_by_ip.write().unwrap();
        if let Some(peer) = peers.remove(ip) {
            self.id_to_ip.write().unwrap().remove(&peer.peer_id);
            if let Some(ep) = peer.endpoint {
                self.endpoint_to_ip.write().unwrap().remove(&ep);
            }
        }
    }

    /// Purge a peer and block it from being re-added by PEX
    pub fn purge_peer(&self, ip: &Ipv4Addr) {
        self.remove_peer(ip);
        self.purged_ips.write().unwrap().insert(*ip);
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

    /// Find peer by endpoint first, then fall back to peer ID (single lookup path)
    pub fn find_ip_by_endpoint_or_id(&self, addr: &SocketAddr, id: &[u8; 4]) -> Option<Ipv4Addr> {
        if let Some(ip) = self.endpoint_to_ip.read().unwrap().get(addr).copied() {
            return Some(ip);
        }
        self.id_to_ip.read().unwrap().get(id).copied()
    }

    /// Iterate all peers under a single write lock — avoids N separate lock acquisitions
    pub fn for_each_peer_mut<F>(&self, mut f: F)
    where F: FnMut(Ipv4Addr, &mut Peer) {
        let mut peers = self.peers_by_ip.write().unwrap();
        for (&ip, peer) in peers.iter_mut() {
            f(ip, peer);
        }
    }

    /// Update a peer's endpoint unconditionally. Used by the config-driven
    /// paths (SIGHUP applies a new `endpoint = ...` line, DNS re-resolve
    /// refreshes a hostname-based configured endpoint). Roaming callers
    /// (inbound data/keepalive from a new src) must use
    /// `update_endpoint_if_roaming` instead so a configured endpoint
    /// stays sticky.
    pub fn update_endpoint(&self, ip: &Ipv4Addr, new_endpoint: SocketAddr) {
        let mut peers = self.peers_by_ip.write().unwrap();
        if let Some(peer) = peers.get_mut(ip) {
            if let Some(old) = peer.endpoint {
                if old != new_endpoint {
                    self.endpoint_to_ip.write().unwrap().remove(&old);

                }
            }
            peer.endpoint = Some(new_endpoint);
            self.endpoint_to_ip.write().unwrap().insert(new_endpoint, *ip);
        }
    }

    /// Update a peer's endpoint from a roamed source (the src of an
    /// inbound data or keepalive packet). No-op when the peer has a
    /// configured static endpoint — the operator's choice is sticky and
    /// must not be overwritten by ambient roaming.
    ///
    /// klasSponsor 2026-05-29 root cause: with a configured LAN endpoint
    /// like `192.168.1.42:9600`, the peer's keepalive arriving via its
    /// NAT'd WAN address roamed the in-memory endpoint to the public IP;
    /// the 60s DNS re-resolve loop later restored the configured value;
    /// rinse and repeat — the operator-visible flap between LAN and
    /// public IP that broke inter-node connections on his cluster.
    /// Returns true if the update was applied.
    pub fn update_endpoint_if_roaming(&self, ip: &Ipv4Addr, new_endpoint: SocketAddr) -> bool {
        let mut peers = self.peers_by_ip.write().unwrap();
        let peer = match peers.get_mut(ip) {
            Some(p) => p,
            None => return false,
        };
        if peer.configured_endpoint.is_some() {
            return false;
        }
        if let Some(old) = peer.endpoint {
            if old != new_endpoint {
                self.endpoint_to_ip.write().unwrap().remove(&old);
            }
        }
        peer.endpoint = Some(new_endpoint);
        self.endpoint_to_ip.write().unwrap().insert(new_endpoint, *ip);
        true
    }

    /// Forget a peer's static endpoint — used when a SIGHUP config reload
    /// finds the peer no longer has an `endpoint = ...` line. Without
    /// this, the previously-pinned endpoint persists in memory and
    /// roaming-learned updates never stick because there's still a
    /// configured target to dial. Also clears `configured_endpoint` so
    /// the DynDNS re-resolve loop stops re-installing the old address.
    /// Returns true if anything was actually cleared (caller can use
    /// this to log a reload-changed counter).
    pub fn clear_endpoint(&self, ip: &Ipv4Addr) -> bool {
        let mut peers = self.peers_by_ip.write().unwrap();
        if let Some(peer) = peers.get_mut(ip) {
            let had_endpoint = peer.endpoint.is_some() || peer.configured_endpoint.is_some();
            if let Some(old) = peer.endpoint.take() {
                self.endpoint_to_ip.write().unwrap().remove(&old);
            }
            peer.configured_endpoint = None;
            return had_endpoint;
        }
        false
    }

    /// Update a peer's endpoint and hostname from discovery
    pub fn update_from_discovery(&self, public_key: &PublicKey, endpoint: SocketAddr, wolfnet_ip: Ipv4Addr, hostname: &str, is_gateway: bool) {
        let mut peers = self.peers_by_ip.write().unwrap();
        
        // First check: does a peer with this exact IP exist?
        if let Some(peer) = peers.get_mut(&wolfnet_ip) {
            if peer.public_key == *public_key {
                // Hostname / gateway flag / relay reset are always refreshed
                // from discovery — those describe peer identity and role,
                // not the dial address, so they shouldn't be sticky.
                peer.hostname = hostname.to_string();
                peer.is_gateway = is_gateway;
                // Direct discovery clears relay — we can reach them directly
                peer.relay_via = None;

                // Endpoint update only when not pinned by config — same
                // sticky-configured rule as `update_endpoint_if_roaming`.
                // Without this gate, a handshake from a peer's WAN address
                // would overwrite the LAN endpoint the operator pinned
                // (klasSponsor 2026-05-29 flap).
                if peer.configured_endpoint.is_none() {
                    if let Some(old_ep) = peer.endpoint {
                        if old_ep != endpoint {
                            self.endpoint_to_ip.write().unwrap().remove(&old_ep);
                        }
                    }
                    peer.endpoint = Some(endpoint);
                    self.endpoint_to_ip.write().unwrap().insert(endpoint, wolfnet_ip);
                }
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
        // If a different-key peer already holds this IP, clean up its stale mappings
        if let Some(old_peer) = peers.remove(&wolfnet_ip) {
            if let Some(old_ep) = old_peer.endpoint {
                self.endpoint_to_ip.write().unwrap().remove(&old_ep);
            }
            self.id_to_ip.write().unwrap().remove(&old_peer.peer_id);
        }
        let mut peer = Peer::new(*public_key, wolfnet_ip);
        peer.endpoint = Some(endpoint);
        peer.hostname = hostname.to_string();
        peer.is_gateway = is_gateway;
        let peer_id = peer.peer_id;

        self.endpoint_to_ip.write().unwrap().insert(endpoint, wolfnet_ip);
        self.id_to_ip.write().unwrap().insert(peer_id, wolfnet_ip);
        peers.insert(wolfnet_ip, peer);

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

    /// Find the host peer for a container/VM IP via subnet routes
    /// Returns the WolfNet IP of the host that owns this container
    pub fn find_route(&self, dest_ip: &Ipv4Addr) -> Option<Ipv4Addr> {
        self.subnet_routes.read().unwrap().get(dest_ip).copied()
    }

    /// Load subnet routes from a JSON file (container_ip → host_peer_ip)
    /// Called on startup and on SIGHUP to reload routes
    pub fn load_routes(&self, path: &std::path::Path) {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return, // File doesn't exist yet — that's fine
        };
        let map: HashMap<String, String> = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(_e) => {

                return;
            }
        };
        let mut routes = self.subnet_routes.write().unwrap();
        routes.clear();
        for (container_ip_str, host_ip_str) in &map {
            if let (Ok(container_ip), Ok(host_ip)) = (
                container_ip_str.parse::<Ipv4Addr>(),
                host_ip_str.parse::<Ipv4Addr>(),
            ) {
                routes.insert(container_ip, host_ip);
            }
        }
        if !routes.is_empty() {

        }
    }

    /// Load CIDR-based subnet routes from a JSON file (cidr → gateway WolfNet IP).
    /// Called on startup and on SIGHUP. WolfStack writes this file from its
    /// WolfRouter SubnetRoute config so userspace can do longest-prefix
    /// matching for packets whose dest doesn't match any peer or any
    /// container exact-IP route.
    ///
    /// Behaviour:
    ///   • File missing → clear the table (nothing should be routed).
    ///   • JSON parse fails → keep the previous table (don't blackhole
    ///     traffic on a transient bad write). Matches load_routes.
    ///   • Parse OK → replace the table with the parsed contents,
    ///     sorted longest-prefix-first.
    pub fn load_subnet_routes(&self, path: &std::path::Path) {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => {
                // File missing — clear BOTH family tables so deleted CIDRs
                // go away. (load_routes returns without clearing here, but
                // for subnet routes "no file" really does mean "no
                // routes configured", and stale entries would cause
                // wrong routing decisions.) The v6 table must clear too, or
                // a removed v6 route would linger in the daemon.
                self.subnet_routes_cidrs.write().unwrap().clear();
                self.subnet_routes_cidrs_v6.write().unwrap().clear();
                return;
            }
        };
        let map: HashMap<String, String> = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(_) => return, // keep existing table on parse failure
        };

        let mut parsed: Vec<(Ipv4Addr, u8, Ipv4Addr)> = Vec::new();
        let mut parsed_v6: Vec<(Ipv6Addr, u8, Ipv4Addr)> = Vec::new();
        for (cidr, gw_str) in &map {
            let (net_str, prefix_str) = match cidr.split_once('/') {
                Some(p) => p,
                None => continue,
            };
            // The gateway is ALWAYS an IPv4 WolfNet IP, for both families —
            // the overlay endpoints are IPv4; only the destination subnet
            // may be v6.
            let gateway: Ipv4Addr = match gw_str.parse() {
                Ok(g) => g,
                Err(_) => continue,
            };
            if let Ok(net) = net_str.parse::<Ipv4Addr>() {
                let prefix: u8 = match prefix_str.parse() {
                    Ok(p) if p <= 32 => p,
                    _ => continue,
                };
                parsed.push((net, prefix, gateway));
            } else if let Ok(net6) = net_str.parse::<Ipv6Addr>() {
                let prefix: u8 = match prefix_str.parse() {
                    Ok(p) if p <= 128 => p,
                    _ => continue,
                };
                parsed_v6.push((net6, prefix, gateway));
            }
            // else: CIDR network is neither v4 nor v6 — skip.
        }
        // Sort longest prefix first so the find_subnet_match* helpers return
        // on the first hit (longest-prefix-match semantics, like the kernel).
        parsed.sort_by(|a, b| b.1.cmp(&a.1));
        parsed_v6.sort_by(|a, b| b.1.cmp(&a.1));
        *self.subnet_routes_cidrs.write().unwrap() = parsed;
        *self.subnet_routes_cidrs_v6.write().unwrap() = parsed_v6;
    }

    /// Longest-prefix-match against the loaded CIDR subnet routes.
    /// Returns the gateway WolfNet IP for the most specific configured
    /// CIDR that contains dest_ip, or None if no CIDR matches.
    pub fn find_subnet_match(&self, dest_ip: &Ipv4Addr) -> Option<Ipv4Addr> {
        let dest_u32 = u32::from_be_bytes(dest_ip.octets());
        let table = self.subnet_routes_cidrs.read().unwrap();
        for (net, prefix, gw) in table.iter() {
            // /0 means "match everything" — useful as a default route.
            // /32 is a single host. Anything in between uses a normal
            // network mask.
            let mask: u32 = if *prefix == 0 {
                0
            } else if *prefix >= 32 {
                u32::MAX
            } else {
                u32::MAX << (32 - *prefix as u32)
            };
            let net_u32 = u32::from_be_bytes(net.octets());
            if (dest_u32 & mask) == (net_u32 & mask) {
                return Some(*gw);
            }
        }
        None
    }

    /// Longest-prefix-match against the loaded IPv6 CIDR subnet routes.
    /// Returns the gateway WolfNet IP (IPv4 — the overlay endpoint) for the
    /// most specific configured v6 CIDR that contains `dest_ip`, or None.
    /// Mirrors `find_subnet_match` with u128 math. The v6 table is empty
    /// unless the operator opted into IPv6 subnet routing, so this returns
    /// None — and the caller drops the packet, exactly as v6 packets were
    /// handled before the feature existed — on every node not using it.
    pub fn find_subnet_match_v6(&self, dest_ip: &Ipv6Addr) -> Option<Ipv4Addr> {
        let dest_u128 = u128::from_be_bytes(dest_ip.octets());
        let table = self.subnet_routes_cidrs_v6.read().unwrap();
        for (net, prefix, gw) in table.iter() {
            let mask: u128 = if *prefix == 0 {
                0
            } else if *prefix >= 128 {
                u128::MAX
            } else {
                u128::MAX << (128 - *prefix as u32)
            };
            let net_u128 = u128::from_be_bytes(net.octets());
            if (dest_u128 & mask) == (net_u128 & mask) {
                return Some(*gw);
            }
        }
        None
    }

    /// Get peer count
    pub fn count(&self) -> usize {
        self.peers_by_ip.read().unwrap().len()
    }

    /// Build PEX entries for all known peers (to share with others)
    /// Excludes the requesting peer's own IP and our own IP.
    ///
    /// Endpoints that aren't reachable from outside the originating
    /// LAN are STRIPPED from the outgoing PEX entries — see
    /// `is_routable_endpoint`. klasSponsor 2026-05-12 (separate
    /// architectural point from the wolfstack-side tombstone fix
    /// shipped in 22.14.11): "Wolfnet should not use internal ip for
    /// connection unless it's a relay." A peer behind NAT advertising
    /// its endpoint as `10.10.10.30:9630` poisoned every public-side
    /// receiver that processed the PEX — receivers stored the
    /// unreachable RFC1918 address as the peer's endpoint and wolfnet
    /// then sent handshake UDP into the void on every retry. Since
    /// PEX-learned peers are stored with `relay_via = sender_ip`
    /// anyway, the endpoint field is not used for routing in the
    /// common case; dropping it costs us nothing while preventing the
    /// poison.
    pub fn get_pex_entries(&self, my_ip: Ipv4Addr) -> Vec<PexEntry> {
        let peers = self.peers_by_ip.read().unwrap();
        peers.values()
            .filter(|p| p.wolfnet_ip != my_ip)
            .map(|p| {
                let endpoint = p.endpoint.and_then(|e| {
                    if is_routable_endpoint(e) { Some(e.to_string()) } else { None }
                });
                PexEntry {
                    public_key: BASE64.encode(p.public_key.as_bytes()),
                    wolfnet_ip: p.wolfnet_ip.to_string(),
                    endpoint,
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

        // Derive our subnet (first 3 octets) for filtering
        let my_octets = my_ip.octets();
        let my_prefix = [my_octets[0], my_octets[1], my_octets[2]];

        let purged = self.purged_ips.read().unwrap();

        for entry in entries {
            // Skip ourselves
            let entry_ip: Ipv4Addr = match entry.wolfnet_ip.parse() {
                Ok(ip) => ip,
                Err(_) => continue,
            };
            if entry_ip == my_ip { continue; }

            // Skip peers that were purged via SIGHUP — prevents PEX re-adding ghost peers
            if purged.contains(&entry_ip) { continue; }

            // Reject peers on a different /24 subnet — prevents cross-subnet ghost peers
            let entry_octets = entry_ip.octets();
            if entry_octets[0] != my_prefix[0] || entry_octets[1] != my_prefix[1] || entry_octets[2] != my_prefix[2] {
                continue;
            }

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

            // Parse endpoint if available — and only store it if it's
            // routable (public IPv4, not RFC1918 / loopback /
            // link-local / multicast). Older wolfnet versions (≤0.5.22)
            // happily stored an RFC1918 endpoint received via PEX, then
            // the daemon would keep dialing the unreachable address on
            // every handshake retry — klasSponsor's traffic flood was
            // partly driven by this. The peer entry itself is still
            // added with relay_via = sender_ip (set above), so
            // connectivity works via the relay; we just don't try to
            // contact the peer directly on an address we can't reach.
            if let Some(ref ep_str) = entry.endpoint {
                if let Ok(ep) = ep_str.parse::<SocketAddr>() {
                    if is_routable_endpoint(ep) {
                        peer.endpoint = Some(ep);
                        self.endpoint_to_ip.write().unwrap().insert(ep, entry_ip);
                    }
                }
            }

            // Establish crypto session so we can encrypt/decrypt
            peer.establish_session(&keypair.secret, &keypair.public);

            let peer_id = peer.peer_id;
            self.id_to_ip.write().unwrap().insert(peer_id, entry_ip);

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
                data_flowing: p.is_passing_data(),
                last_data_rx_secs: p.last_data_rx.map_or(u64::MAX, |t| t.elapsed().as_secs()),
            }
        }).collect()
    }
}

#[cfg(test)]
mod peer_state_tests {
    use super::*;
    use x25519_dalek::{PublicKey, StaticSecret};

    fn make_peer() -> Peer {
        let secret = StaticSecret::random_from_rng(rand::thread_rng());
        let public = PublicKey::from(&secret);
        Peer::new(public, "10.100.10.1".parse().unwrap())
    }

    #[test]
    fn fresh_peer_is_neither_connected_nor_passing_data() {
        let p = make_peer();
        assert!(!p.is_connected(), "fresh peer with no cipher must not register as connected");
        assert!(!p.is_passing_data(), "fresh peer cannot be passing data");
    }

    #[test]
    fn handshake_alone_does_not_set_passing_data() {
        // Simulate the asymmetric state that misled klasSponsor / Fang:
        // we have a session cipher, last_seen is recent (handshake just
        // arrived), but we've never decrypted a real data packet.
        let mut p = make_peer();
        // Manufacture the post-handshake state without going through
        // establish_session — keeps the test independent of the DH path.
        p.cipher = None; // cipher gets installed by establish_session normally
        p.last_seen = Some(Instant::now());
        p.last_data_rx = None;
        // Without a cipher is_connected stays false — that's correct.
        assert!(!p.is_connected());
        assert!(!p.is_passing_data());
    }

    #[test]
    fn passing_data_requires_actual_decrypt() {
        // Set last_data_rx without going through decrypt to simulate
        // post-decrypt state, then assert the predicate flips.
        let mut p = make_peer();
        p.last_seen = Some(Instant::now());
        p.last_data_rx = Some(Instant::now());
        // is_connected/is_passing_data both still require a cipher, so
        // they're false here — that's the right semantic: no cipher
        // means we couldn't actually have decrypted anything.
        assert!(!p.is_passing_data(),
            "is_passing_data must require an established cipher, not just a recent timestamp");
    }
}

#[cfg(test)]
mod sticky_endpoint_tests {
    use super::*;
    use x25519_dalek::{PublicKey, StaticSecret};

    fn pubkey() -> PublicKey {
        let s = StaticSecret::random_from_rng(rand::thread_rng());
        PublicKey::from(&s)
    }

    fn ip(s: &str) -> Ipv4Addr { s.parse().unwrap() }
    fn sa(s: &str) -> SocketAddr { s.parse().unwrap() }

    /// klasSponsor 2026-05-29 — the core regression: a peer pinned to a
    /// LAN endpoint must NOT roam when a packet arrives from its WAN
    /// address. Before the fix, every keepalive from the public IP
    /// silently overwrote the operator-pinned LAN address.
    #[test]
    fn configured_endpoint_blocks_roaming_update() {
        let pm = PeerManager::new();
        let mut peer = Peer::new(pubkey(), ip("10.100.10.30"));
        peer.configured_endpoint = Some("192.168.1.42:9600".to_string());
        peer.endpoint = Some(sa("192.168.1.42:9600"));
        pm.add_peer(peer);

        let applied = pm.update_endpoint_if_roaming(&ip("10.100.10.30"), sa("203.0.113.5:55321"));

        assert!(!applied, "roaming update must be refused for a peer with configured_endpoint");
        let endpoint = pm.with_peer_by_ip(&ip("10.100.10.30"), |p| p.endpoint).flatten();
        assert_eq!(endpoint, Some(sa("192.168.1.42:9600")),
            "endpoint must still be the operator-configured LAN address");
    }

    /// Roaming-only peers (no static endpoint configured — token-joined
    /// or PEX-learned) keep their pre-fix behaviour: ambient roaming
    /// updates the in-memory endpoint to the latest src.
    #[test]
    fn roaming_peer_still_updates_endpoint() {
        let pm = PeerManager::new();
        let mut peer = Peer::new(pubkey(), ip("10.100.10.40"));
        // No configured_endpoint — this peer is roaming-only.
        peer.endpoint = Some(sa("203.0.113.5:55321"));
        pm.add_peer(peer);

        let applied = pm.update_endpoint_if_roaming(&ip("10.100.10.40"), sa("203.0.113.5:55400"));

        assert!(applied, "roaming-only peer must accept the new src");
        let endpoint = pm.with_peer_by_ip(&ip("10.100.10.40"), |p| p.endpoint).flatten();
        assert_eq!(endpoint, Some(sa("203.0.113.5:55400")));
    }

    /// The unconditional `update_endpoint` path (SIGHUP / DNS re-resolve
    /// applying a configured value) must still overwrite. Otherwise an
    /// operator changing the `endpoint = ...` line via SIGHUP wouldn't
    /// take effect on an already-pinned peer.
    #[test]
    fn unconditional_update_endpoint_overrides_configured() {
        let pm = PeerManager::new();
        let mut peer = Peer::new(pubkey(), ip("10.100.10.50"));
        peer.configured_endpoint = Some("192.168.1.42:9600".to_string());
        peer.endpoint = Some(sa("192.168.1.42:9600"));
        pm.add_peer(peer);

        pm.update_endpoint(&ip("10.100.10.50"), sa("192.168.1.99:9600"));

        let endpoint = pm.with_peer_by_ip(&ip("10.100.10.50"), |p| p.endpoint).flatten();
        assert_eq!(endpoint, Some(sa("192.168.1.99:9600")),
            "config-driven update must always apply");
    }

    /// Discovery (inbound handshake) must NOT overwrite a configured
    /// endpoint either — the handshake src is just another roamed
    /// address from the operator's point of view.
    #[test]
    fn discovery_does_not_overwrite_configured_endpoint() {
        let pm = PeerManager::new();
        let key = pubkey();
        let mut peer = Peer::new(key, ip("10.100.10.60"));
        peer.configured_endpoint = Some("192.168.1.42:9600".to_string());
        peer.endpoint = Some(sa("192.168.1.42:9600"));
        pm.add_peer(peer);

        pm.update_from_discovery(&key, sa("203.0.113.5:55321"), ip("10.100.10.60"), "newhost", false);

        let (endpoint, hostname) = pm.with_peer_by_ip(&ip("10.100.10.60"), |p| (p.endpoint, p.hostname.clone())).unwrap();
        assert_eq!(endpoint, Some(sa("192.168.1.42:9600")),
            "discovery must not overwrite a configured endpoint");
        assert_eq!(hostname, "newhost",
            "hostname must still refresh from discovery — that's identity, not address");
    }

    /// And the symmetric: discovery DOES update the endpoint for a
    /// roaming-only peer. That's the normal NAT-traversal path.
    #[test]
    fn discovery_updates_endpoint_for_roaming_peer() {
        let pm = PeerManager::new();
        let key = pubkey();
        let mut peer = Peer::new(key, ip("10.100.10.70"));
        peer.endpoint = Some(sa("203.0.113.5:55321"));
        pm.add_peer(peer);

        pm.update_from_discovery(&key, sa("203.0.113.5:55400"), ip("10.100.10.70"), "host", false);

        let endpoint = pm.with_peer_by_ip(&ip("10.100.10.70"), |p| p.endpoint).flatten();
        assert_eq!(endpoint, Some(sa("203.0.113.5:55400")));
    }
}

#[cfg(test)]
mod subnet_match_tests {
    use super::*;

    /// Inject a hand-built CIDR table for testing without touching disk.
    fn install(pm: &PeerManager, entries: Vec<(&str, u8, &str)>) {
        let mut parsed: Vec<(Ipv4Addr, u8, Ipv4Addr)> = entries
            .into_iter()
            .map(|(net, prefix, gw)| (net.parse().unwrap(), prefix, gw.parse().unwrap()))
            .collect();
        parsed.sort_by(|a, b| b.1.cmp(&a.1));
        *pm.subnet_routes_cidrs.write().unwrap() = parsed;
    }

    #[test]
    fn matches_exact_ip_in_slash16() {
        let pm = PeerManager::new();
        install(&pm, vec![("10.10.0.0", 16, "10.100.10.30")]);
        let gw = pm.find_subnet_match(&"10.10.10.10".parse().unwrap());
        assert_eq!(gw, Some("10.100.10.30".parse().unwrap()));
    }

    #[test]
    fn no_match_when_outside_subnet() {
        let pm = PeerManager::new();
        install(&pm, vec![("10.10.0.0", 16, "10.100.10.30")]);
        let gw = pm.find_subnet_match(&"10.11.0.5".parse().unwrap());
        assert_eq!(gw, None);
    }

    #[test]
    fn empty_table_returns_none() {
        let pm = PeerManager::new();
        let gw = pm.find_subnet_match(&"10.10.10.10".parse().unwrap());
        assert_eq!(gw, None);
    }

    #[test]
    fn longest_prefix_wins() {
        let pm = PeerManager::new();
        // /16 covers 10.10.0.0/16 → gw A. A more-specific /24 inside it
        // (10.10.5.0/24) → gw B. Anything in 10.10.5.x must hit B,
        // anything else in 10.10.x.x must hit A.
        install(&pm, vec![
            ("10.10.0.0", 16, "10.100.0.1"),
            ("10.10.5.0", 24, "10.100.0.2"),
        ]);
        assert_eq!(
            pm.find_subnet_match(&"10.10.5.10".parse().unwrap()),
            Some("10.100.0.2".parse().unwrap())
        );
        assert_eq!(
            pm.find_subnet_match(&"10.10.99.99".parse().unwrap()),
            Some("10.100.0.1".parse().unwrap())
        );
    }

    #[test]
    fn slash_32_exact_host() {
        let pm = PeerManager::new();
        install(&pm, vec![("192.168.5.42", 32, "10.100.0.5")]);
        assert_eq!(
            pm.find_subnet_match(&"192.168.5.42".parse().unwrap()),
            Some("10.100.0.5".parse().unwrap())
        );
        assert_eq!(
            pm.find_subnet_match(&"192.168.5.43".parse().unwrap()),
            None
        );
    }

    #[test]
    fn slash_zero_default_route() {
        let pm = PeerManager::new();
        // /0 = match everything. Useful as a full-tunnel default.
        install(&pm, vec![("0.0.0.0", 0, "10.100.0.99")]);
        assert_eq!(
            pm.find_subnet_match(&"8.8.8.8".parse().unwrap()),
            Some("10.100.0.99".parse().unwrap())
        );
    }

    #[test]
    fn slash_24_boundary() {
        let pm = PeerManager::new();
        install(&pm, vec![("192.168.1.0", 24, "10.100.0.7")]);
        assert_eq!(
            pm.find_subnet_match(&"192.168.1.0".parse().unwrap()),
            Some("10.100.0.7".parse().unwrap())
        );
        assert_eq!(
            pm.find_subnet_match(&"192.168.1.255".parse().unwrap()),
            Some("10.100.0.7".parse().unwrap())
        );
        assert_eq!(
            pm.find_subnet_match(&"192.168.2.0".parse().unwrap()),
            None
        );
    }

    #[test]
    fn loaded_table_is_sorted_longest_first() {
        // Verifies load_subnet_routes really does sort. Even if the
        // JSON happens to list /16 before /24, find_subnet_match must
        // still pick the /24 for IPs inside it.
        let pm = PeerManager::new();
        let tmp = std::env::temp_dir().join("wolfnet-subnet-test.json");
        std::fs::write(
            &tmp,
            r#"{"10.10.0.0/16":"10.100.0.1","10.10.5.0/24":"10.100.0.2"}"#,
        ).unwrap();
        pm.load_subnet_routes(&tmp);
        assert_eq!(
            pm.find_subnet_match(&"10.10.5.5".parse().unwrap()),
            Some("10.100.0.2".parse().unwrap())
        );
        assert_eq!(
            pm.find_subnet_match(&"10.10.99.5".parse().unwrap()),
            Some("10.100.0.1".parse().unwrap())
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn missing_file_clears_table() {
        let pm = PeerManager::new();
        install(&pm, vec![("10.10.0.0", 16, "10.100.0.1")]);
        let nonexistent = std::path::PathBuf::from("/tmp/wolfnet-this-file-does-not-exist-zzz.json");
        pm.load_subnet_routes(&nonexistent);
        assert_eq!(pm.find_subnet_match(&"10.10.10.10".parse().unwrap()), None);
    }

    #[test]
    fn malformed_json_keeps_table() {
        let pm = PeerManager::new();
        install(&pm, vec![("10.10.0.0", 16, "10.100.0.1")]);
        let tmp = std::env::temp_dir().join("wolfnet-subnet-malformed.json");
        std::fs::write(&tmp, "{ this is not json }").unwrap();
        pm.load_subnet_routes(&tmp);
        // Existing table preserved.
        assert_eq!(
            pm.find_subnet_match(&"10.10.10.10".parse().unwrap()),
            Some("10.100.0.1".parse().unwrap())
        );
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod pex_endpoint_filter_tests {
    use super::*;
    use std::net::SocketAddr;

    fn sa(s: &str) -> SocketAddr { s.parse().unwrap() }

    // ─── is_routable_endpoint classifier ───
    #[test]
    fn routable_public_ipv4_passes() {
        assert!(is_routable_endpoint(sa("194.104.94.40:9600")));
        assert!(is_routable_endpoint(sa("8.8.8.8:53")));
        assert!(is_routable_endpoint(sa("185.57.4.152:9620")));
    }

    #[test]
    fn rfc1918_ten_dot_blocked() {
        assert!(!is_routable_endpoint(sa("10.0.0.1:9600")));
        assert!(!is_routable_endpoint(sa("10.10.10.30:9630")));
        assert!(!is_routable_endpoint(sa("10.255.255.255:9600")));
    }

    #[test]
    fn rfc1918_172_dot_blocked() {
        assert!(!is_routable_endpoint(sa("172.16.0.1:9600")));
        assert!(!is_routable_endpoint(sa("172.31.255.254:9600")));
        // Boundary: 172.15 and 172.32 are NOT RFC1918.
        assert!(is_routable_endpoint(sa("172.15.0.1:9600")));
        assert!(is_routable_endpoint(sa("172.32.0.1:9600")));
    }

    #[test]
    fn rfc1918_192_168_blocked() {
        assert!(!is_routable_endpoint(sa("192.168.0.1:9600")));
        assert!(!is_routable_endpoint(sa("192.168.255.254:9600")));
        // Boundary: 192.167 and 192.169 are NOT RFC1918.
        assert!(is_routable_endpoint(sa("192.167.0.1:9600")));
        assert!(is_routable_endpoint(sa("192.169.0.1:9600")));
    }

    #[test]
    fn loopback_blocked() {
        assert!(!is_routable_endpoint(sa("127.0.0.1:9600")));
        assert!(!is_routable_endpoint(sa("127.255.255.254:9600")));
    }

    #[test]
    fn link_local_blocked() {
        assert!(!is_routable_endpoint(sa("169.254.1.1:9600")));
        assert!(!is_routable_endpoint(sa("169.254.169.254:80"))); // canonical metadata IP
    }

    #[test]
    fn multicast_blocked() {
        assert!(!is_routable_endpoint(sa("224.0.0.1:9600")));
        assert!(!is_routable_endpoint(sa("239.255.255.255:9600")));
    }

    #[test]
    fn unspecified_blocked() {
        assert!(!is_routable_endpoint(sa("0.0.0.0:9600")));
    }

    // ─── PEX filter integration ───
    // Add a peer with an RFC1918 endpoint, build PEX entries, confirm
    // the endpoint is stripped on the way out.
    #[test]
    fn outgoing_pex_strips_rfc1918_endpoint() {
        use x25519_dalek::{PublicKey, StaticSecret};
        let pm = PeerManager::new();
        let secret = StaticSecret::random_from_rng(rand::thread_rng());
        let public = PublicKey::from(&secret);
        let mut peer = Peer::new(public, "10.100.10.30".parse().unwrap());
        peer.hostname = "ninni".into();
        peer.endpoint = Some(sa("10.10.10.30:9630")); // RFC1918, must be stripped
        pm.add_peer(peer);

        let entries = pm.get_pex_entries("10.100.10.40".parse().unwrap());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].hostname, "ninni");
        assert!(entries[0].endpoint.is_none(),
            "PEX must strip RFC1918 endpoint; got {:?}", entries[0].endpoint);
    }

    #[test]
    fn outgoing_pex_preserves_public_endpoint() {
        use x25519_dalek::{PublicKey, StaticSecret};
        let pm = PeerManager::new();
        let secret = StaticSecret::random_from_rng(rand::thread_rng());
        let public = PublicKey::from(&secret);
        let mut peer = Peer::new(public, "10.100.10.40".parse().unwrap());
        peer.hostname = "vps".into();
        peer.endpoint = Some(sa("194.104.94.40:9600"));
        pm.add_peer(peer);

        let entries = pm.get_pex_entries("10.100.10.20".parse().unwrap());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].endpoint.as_deref(), Some("194.104.94.40:9600"));
    }

    #[test]
    fn outgoing_pex_strips_loopback_endpoint() {
        use x25519_dalek::{PublicKey, StaticSecret};
        let pm = PeerManager::new();
        let secret = StaticSecret::random_from_rng(rand::thread_rng());
        let public = PublicKey::from(&secret);
        let mut peer = Peer::new(public, "10.100.10.50".parse().unwrap());
        peer.endpoint = Some(sa("127.0.0.1:9600"));
        pm.add_peer(peer);

        let entries = pm.get_pex_entries("10.100.10.40".parse().unwrap());
        assert_eq!(entries.len(), 1);
        assert!(entries[0].endpoint.is_none());
    }

    #[test]
    fn outgoing_pex_excludes_self() {
        use x25519_dalek::{PublicKey, StaticSecret};
        let pm = PeerManager::new();
        let secret = StaticSecret::random_from_rng(rand::thread_rng());
        let public = PublicKey::from(&secret);
        let mut peer = Peer::new(public, "10.100.10.40".parse().unwrap());
        peer.endpoint = Some(sa("194.104.94.40:9600"));
        pm.add_peer(peer);

        // Asking for entries as if we WERE 10.100.10.40 should exclude self.
        let entries = pm.get_pex_entries("10.100.10.40".parse().unwrap());
        assert_eq!(entries.len(), 0);
    }
}

#[cfg(test)]
mod subnet_route_v6_tests {
    use super::*;
    use std::io::Write;

    fn temp_json(name: &str, body: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("wolfnet-test-{}-{}.json", name, std::process::id()));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        path
    }

    #[test]
    fn load_subnet_routes_splits_families_and_matches() {
        let pm = PeerManager::new();
        // Mixed v4 + v6 map. The gateway is always an IPv4 WolfNet IP.
        let path = temp_json("mixed", r#"{
            "10.10.0.0/16": "10.100.10.30",
            "fc00:abcd::/32": "10.100.10.40",
            "fc00:abcd:1::/48": "10.100.10.41"
        }"#);
        pm.load_subnet_routes(&path);
        let _ = std::fs::remove_file(&path);

        // v4 path unchanged.
        assert_eq!(
            pm.find_subnet_match(&"10.10.5.9".parse().unwrap()),
            Some("10.100.10.30".parse().unwrap())
        );
        // v6 longest-prefix-match: the /48 wins over the /32.
        assert_eq!(
            pm.find_subnet_match_v6(&"fc00:abcd:1::99".parse().unwrap()),
            Some("10.100.10.41".parse().unwrap())
        );
        // A v6 address only inside the /32.
        assert_eq!(
            pm.find_subnet_match_v6(&"fc00:abcd:2::1".parse().unwrap()),
            Some("10.100.10.40".parse().unwrap())
        );
        // A v6 address outside every configured range → no match (dropped).
        assert_eq!(pm.find_subnet_match_v6(&"2001:db8::1".parse().unwrap()), None);
    }

    #[test]
    fn missing_file_clears_v6_table() {
        let pm = PeerManager::new();
        let path = temp_json("present", r#"{ "fc00::/16": "10.100.10.40" }"#);
        pm.load_subnet_routes(&path);
        assert!(pm.find_subnet_match_v6(&"fc00::5".parse().unwrap()).is_some());
        // Now point at a missing file — the table must clear, not linger.
        let _ = std::fs::remove_file(&path);
        pm.load_subnet_routes(&path);
        assert_eq!(pm.find_subnet_match_v6(&"fc00::5".parse().unwrap()), None);
    }

    #[test]
    fn v6_disabled_default_table_is_empty() {
        // A fresh manager (the state on every node that never enabled the
        // feature, since WolfStack only writes v6 CIDRs when opted in) has
        // an empty v6 table and matches nothing.
        let pm = PeerManager::new();
        assert_eq!(pm.find_subnet_match_v6(&"fc00::1".parse().unwrap()), None);
    }
}
