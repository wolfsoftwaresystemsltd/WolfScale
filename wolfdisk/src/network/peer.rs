//! Peer connection management for WolfDisk nodes

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::Duration;

use tracing::{debug, info, warn};

use crate::network::protocol::{decode_message, encode_message, Message};

/// Wire handshake, exchanged FIRST on every TCP connection (both directions)
/// since v2.11.8. The transport is positional bincode with no per-message
/// versioning, so mixed-version clusters didn't fail — they silently decoded
/// garbage or looped on errors forever (wabil 2026-07-04: a freshly
/// reinstalled node sat at index v0 for good because v2.11.6 appended a field
/// to SyncResponseMsg that its older leader never sent). The handshake turns
/// that into a one-line, named error on the NEW side:
///
///   magic "WDHS" (4 bytes) + wire_version u16 LE + ver_len u16 LE + version string
///
/// Deliberately NOT bincode (no chicken-and-egg) and deliberately magic-first:
/// an old peer's first 4 bytes are a frame-length prefix, which can never
/// equal the magic, so a missing handshake is detected deterministically.
/// An old node RECEIVING our magic reads it as a ~1.4GB length prefix and
/// rejects it via its existing 100MB frame cap — closing the socket, which
/// the new side reports as "peer speaks no handshake".
/// Bump WIRE_VERSION whenever protocol.rs changes any message layout.
const WIRE_MAGIC: [u8; 4] = *b"WDHS";
const WIRE_VERSION: u16 = 2;

fn write_handshake(stream: &mut TcpStream) -> std::io::Result<()> {
    let ver = env!("CARGO_PKG_VERSION").as_bytes();
    let mut buf = Vec::with_capacity(8 + ver.len());
    buf.extend_from_slice(&WIRE_MAGIC);
    buf.extend_from_slice(&WIRE_VERSION.to_le_bytes());
    buf.extend_from_slice(&(ver.len() as u16).to_le_bytes());
    buf.extend_from_slice(ver);
    stream.write_all(&buf)?;
    stream.flush()
}

/// Read and validate the peer's handshake. Returns the peer's software
/// version string. Every failure names the likely cause — this is the
/// diagnosis surface for mixed-version clusters.
fn read_handshake(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut magic = [0u8; 4];
    stream.read_exact(&mut magic).map_err(|e| std::io::Error::new(
        e.kind(),
        format!("peer closed before handshake — likely pre-2.11.8 wolfdisk (upgrade all nodes together): {}", e),
    ))?;
    if magic != WIRE_MAGIC {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "peer sent no wire handshake — it runs pre-2.11.8 wolfdisk. Mixed versions cannot sync; upgrade all nodes together.",
        ));
    }
    let mut hdr = [0u8; 4];
    stream.read_exact(&mut hdr)?;
    let wire = u16::from_le_bytes([hdr[0], hdr[1]]);
    let ver_len = u16::from_le_bytes([hdr[2], hdr[3]]) as usize;
    if ver_len > 64 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "handshake version string too long"));
    }
    let mut ver = vec![0u8; ver_len];
    stream.read_exact(&mut ver)?;
    let peer_version = String::from_utf8_lossy(&ver).to_string();
    if wire != WIRE_VERSION {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "wire version mismatch: peer runs wolfdisk {} (wire v{}), this node is {} (wire v{}) — upgrade all nodes together",
                peer_version, wire, env!("CARGO_PKG_VERSION"), WIRE_VERSION
            ),
        ));
    }
    Ok(peer_version)
}

/// Connection to a peer node
pub struct PeerConnection {
    pub node_id: String,
    pub address: String,
    stream: Mutex<TcpStream>,
}

impl PeerConnection {
    /// Connect to a peer with a connect timeout.
    /// This is critical for client nodes over WolfNet — without a timeout,
    /// TcpStream::connect() blocks for 2+ minutes on unreachable hosts,
    /// which freezes all FUSE operations and hangs Dolphin.
    pub fn connect(node_id: String, address: &str) -> std::io::Result<Self> {
        // Resolve address and connect with a 5-second timeout.
        // Over WolfNet tunnels, initial connection should complete in <1s;
        // 5s gives headroom for congested links without freezing Dolphin.
        let sock_addr: SocketAddr = address.to_socket_addrs()?.next().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "cannot resolve address")
        })?;
        let mut stream = TcpStream::connect_timeout(&sock_addr, Duration::from_secs(5))?;
        // 10s read/write timeouts — short enough to keep Dolphin responsive,
        // long enough for WAN round-trips over WolfNet.
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;
        // Disable Nagle's algorithm - each FUSE op is a synchronous round-trip,
        // so we want messages sent immediately, not buffered
        stream.set_nodelay(true)?;

        // Wire handshake before any frame (see WIRE_MAGIC docs). A peer that
        // doesn't answer it is a different wolfdisk generation — fail HERE
        // with a named error instead of looping on undecodable frames.
        write_handshake(&mut stream)?;
        let peer_version = read_handshake(&mut stream)?;
        debug!("handshake ok with {} (wolfdisk {})", address, peer_version);

        Ok(Self {
            node_id,
            address: address.to_string(),
            stream: Mutex::new(stream),
        })
    }

    /// Send a message to the peer
    pub fn send(&self, msg: &Message) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let data = encode_message(msg)?;
        let len = (data.len() as u32).to_le_bytes();

        let mut stream = self.stream.lock().unwrap();
        stream.write_all(&len)?;
        stream.write_all(&data)?;
        stream.flush()?;

        Ok(())
    }

    /// Receive a message from the peer
    pub fn recv(&self) -> Result<Message, Box<dyn std::error::Error + Send + Sync>> {
        let mut stream = self.stream.lock().unwrap();

        // Read length prefix
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf) as usize;

        // Read message data
        let mut data = vec![0u8; len];
        stream.read_exact(&mut data)?;

        let msg = decode_message(&data)?;
        Ok(msg)
    }

    /// Send and wait for response
    pub fn request(
        &self,
        msg: &Message,
    ) -> Result<Message, Box<dyn std::error::Error + Send + Sync>> {
        self.send(msg)?;
        self.recv()
    }
}

/// Manages connections to all peers
pub struct PeerManager {
    #[allow(dead_code)]
    node_id: String,
    bind_address: String,
    connections: Arc<RwLock<HashMap<String, Arc<PeerConnection>>>>,
    message_handler: Arc<dyn Fn(String, Message) -> Option<Message> + Send + Sync>,
    running: Arc<RwLock<bool>>,
}

impl PeerManager {
    /// Create a new peer manager
    pub fn new<F>(node_id: String, bind_address: String, handler: F) -> Self
    where
        F: Fn(String, Message) -> Option<Message> + Send + Sync + 'static,
    {
        Self {
            node_id,
            bind_address,
            connections: Arc::new(RwLock::new(HashMap::new())),
            message_handler: Arc::new(handler),
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Start listening for peer connections
    pub fn start(&self) -> std::io::Result<()> {
        let bind_addr: SocketAddr = self.bind_address.parse().map_err(|e| {
            // Name the offending value and the expected shape — the bare
            // AddrParseError ("invalid socket address syntax") sent an
            // operator hunting through the whole config (klas 2026-07-08).
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "invalid `bind` address '{}' ({}) — expected ip:port, e.g. 10.100.10.40:8650",
                    self.bind_address, e
                ),
            )
        })?;
        // Bind BEFORE flipping `running` true — if the address is taken we must
        // not leave the manager flagged as running. std's TcpListener already
        // sets SO_REUSEADDR, so AddrInUse here means another process is actively
        // listening on this exact port (a second wolfdisk instance, a stale
        // wolfdisk that didn't stop, or a clash with WolfStack's status-page
        // range 8550-8599). Name the address and the likely cause so the
        // operator can find it with `ss -ltnp` instead of a bare errno 98.
        let listener = TcpListener::bind(bind_addr).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!(
                        "peer port {bind_addr} is already in use — another wolfdisk \
                         is likely already listening there (you do NOT need a second \
                         wolfdisk for S3: set `[s3] enabled = true` on the existing \
                         one). If you really want a second instance, give it a \
                         different `bind`/`data_dir`. Find the holder with: \
                         ss -ltnp 'sport = :{port}'",
                        port = bind_addr.port()
                    ),
                )
            } else {
                e
            }
        })?;
        listener.set_nonblocking(true)?;
        *self.running.write().unwrap() = true;

        let handler = Arc::clone(&self.message_handler);
        let running = Arc::clone(&self.running);

        thread::spawn(move || {
            info!("Peer server listening on {}", bind_addr);

            while *running.read().unwrap() {
                match listener.accept() {
                    Ok((stream, addr)) => {
                        debug!("Accepted connection from {}", addr);
                        let handler = Arc::clone(&handler);

                        thread::spawn(move || {
                            if let Err(e) = handle_peer_connection(stream, addr, handler) {
                                debug!("Peer connection ended: {}", e);
                            }
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(100));
                    }
                    Err(e) => {
                        warn!("Accept error: {}", e);
                    }
                }
            }
        });

        Ok(())
    }

    /// Connect to a peer
    pub fn connect(
        &self,
        node_id: &str,
        address: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = PeerConnection::connect(node_id.to_string(), address)?;
        self.connections
            .write()
            .unwrap()
            .insert(node_id.to_string(), Arc::new(conn));
        info!("Connected to peer {} at {}", node_id, address);
        Ok(())
    }

    /// Send message to a specific peer
    /// Removes broken connection on failure so it can be re-established
    pub fn send_to(
        &self,
        node_id: &str,
        msg: &Message,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let result = {
            let connections = self.connections.read().unwrap();
            connections.get(node_id).map(|conn| conn.send(msg))
        };

        match result {
            Some(Err(e)) => {
                // Remove broken connection so it can be re-established
                self.connections.write().unwrap().remove(node_id);
                warn!(
                    "Failed to send to {}: {} (removed broken connection)",
                    node_id, e
                );
                Err(e)
            }
            Some(Ok(())) => Ok(()),
            None => Ok(()), // No connection exists
        }
    }

    /// Broadcast message to all peers
    /// Automatically removes broken connections so they can be re-established
    pub fn broadcast(&self, msg: &Message) {
        // specific timeout for broadcast can be handled in send if needed,
        // but for now relying on connection timeout

        // Clone connections to release lock before network I/O
        let peers: Vec<(String, Arc<PeerConnection>)> = {
            let connections = self.connections.read().unwrap();
            connections
                .iter()
                .map(|(id, conn)| (id.clone(), conn.clone()))
                .collect()
        };

        let mut failed_ids = Vec::new();

        for (id, conn) in peers {
            if let Err(e) = conn.send(msg) {
                warn!(
                    "Failed to send to {}: {} (removing broken connection)",
                    id, e
                );
                failed_ids.push(id);
            }
        }

        // Remove broken connections so broadcast thread can reconnect
        if !failed_ids.is_empty() {
            let mut connections = self.connections.write().unwrap();
            for id in &failed_ids {
                connections.remove(id);
                info!("Removed broken connection to {} (will reconnect)", id);
            }
        }
    }

    /// Remove a peer connection (e.g. when it becomes stale)
    pub fn remove_peer(&self, node_id: &str) {
        self.connections.write().unwrap().remove(node_id);
    }

    /// Get the number of active outbound connections
    pub fn connection_count(&self) -> usize {
        self.connections.read().unwrap().len()
    }

    /// Get connection to a peer
    pub fn get(&self, node_id: &str) -> Option<Arc<PeerConnection>> {
        self.connections.read().unwrap().get(node_id).cloned()
    }

    /// Get connection to leader (if known)
    pub fn leader(&self) -> Option<Arc<PeerConnection>> {
        // For now, return first connection. Will be enhanced with leader tracking.
        self.connections.read().unwrap().values().next().cloned()
    }

    /// Get or create connection to leader by ID and address
    pub fn get_or_connect_leader(
        &self,
        leader_id: &str,
        leader_addr: &str,
    ) -> Result<Arc<PeerConnection>, Box<dyn std::error::Error + Send + Sync>> {
        // Check if already connected
        {
            let connections = self.connections.read().unwrap();
            if let Some(conn) = connections.get(leader_id) {
                return Ok(conn.clone());
            }
        }

        // Connect to leader
        let conn = PeerConnection::connect(leader_id.to_string(), leader_addr)?;
        let conn = Arc::new(conn);
        self.connections
            .write()
            .unwrap()
            .insert(leader_id.to_string(), conn.clone());
        Ok(conn)
    }

    /// Remove cached connection to leader (used for reconnection on failure)
    pub fn disconnect_leader(&self, leader_id: &str) {
        self.connections.write().unwrap().remove(leader_id);
    }

    /// Stop the peer manager
    pub fn stop(&self) {
        *self.running.write().unwrap() = false;
    }
}

/// Handle incoming peer connection
fn handle_peer_connection(
    mut stream: TcpStream,
    addr: SocketAddr,
    handler: Arc<dyn Fn(String, Message) -> Option<Message> + Send + Sync>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Inbound connections can afford slightly longer timeouts since they
    // handle sync requests which transfer more data.
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    // Disable Nagle's algorithm for prompt responses
    stream.set_nodelay(true)?;

    // Wire handshake first (see WIRE_MAGIC docs): read theirs, answer with
    // ours. An old peer's first bytes are a frame-length prefix, never the
    // magic — surfaced as a named upgrade-together error instead of a
    // bincode decode failure deep in the message loop.
    match read_handshake(&mut stream) {
        Ok(peer_version) => {
            write_handshake(&mut stream)?;
            debug!("handshake ok from {} (wolfdisk {})", addr, peer_version);
        }
        Err(e) => {
            warn!("rejected peer {}: {}", addr, e);
            return Err(e.into());
        }
    }

    loop {
        // Read length prefix
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf) as usize;

        if len > 100 * 1024 * 1024 {
            // Max 100MB message
            return Err("Message too large".into());
        }

        // Read message
        let mut data = vec![0u8; len];
        stream.read_exact(&mut data)?;

        let msg = decode_message(&data)?;
        let peer_id = addr.to_string(); // Will be replaced with proper handshake

        // Handle message and optionally send response
        if let Some(response) = handler(peer_id, msg) {
            let resp_data = encode_message(&response)?;
            let resp_len = (resp_data.len() as u32).to_le_bytes();
            stream.write_all(&resp_len)?;
            stream.write_all(&resp_data)?;
            stream.flush()?;
        }
    }
}

#[cfg(test)]
mod handshake_tests {
    use super::*;
    use std::net::TcpListener;

    /// Both sides new: handshake round-trips and reports the software version.
    #[test]
    fn handshake_roundtrip() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let peer_ver = read_handshake(&mut s).unwrap();
            write_handshake(&mut s).unwrap();
            peer_ver
        });
        let mut c = TcpStream::connect(addr).unwrap();
        write_handshake(&mut c).unwrap();
        let server_ver = read_handshake(&mut c).unwrap();
        let client_seen = server.join().unwrap();
        assert_eq!(server_ver, env!("CARGO_PKG_VERSION"));
        assert_eq!(client_seen, env!("CARGO_PKG_VERSION"));
    }

    /// An old (pre-2.11.8) peer opens with a frame-length prefix, never the
    /// magic — must be rejected with the named upgrade-together error.
    #[test]
    fn old_peer_without_handshake_is_named() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            read_handshake(&mut s).map(|_| ())
        });
        let mut c = TcpStream::connect(addr).unwrap();
        // Simulate an old node: 4-byte LE length prefix of a small frame.
        c.write_all(&42u32.to_le_bytes()).unwrap();
        c.flush().unwrap();
        let err = server.join().unwrap().unwrap_err();
        assert!(err.to_string().contains("pre-2.11.8"), "got: {}", err);
    }

    /// Same magic but a different wire version: named mismatch error.
    #[test]
    fn wire_version_mismatch_is_named() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            read_handshake(&mut s).map(|_| ())
        });
        let mut c = TcpStream::connect(addr).unwrap();
        let mut buf = Vec::new();
        buf.extend_from_slice(&WIRE_MAGIC);
        buf.extend_from_slice(&1u16.to_le_bytes()); // old wire version
        buf.extend_from_slice(&3u16.to_le_bytes());
        buf.extend_from_slice(b"9.9");
        c.write_all(&buf).unwrap();
        c.flush().unwrap();
        let err = server.join().unwrap().unwrap_err();
        assert!(err.to_string().contains("wire version mismatch"), "got: {}", err);
        assert!(err.to_string().contains("9.9"), "peer version must be named: {}", err);
    }
}
