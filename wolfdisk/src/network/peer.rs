//! Peer connection management for WolfDisk nodes

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, TcpListener, SocketAddr};
use std::sync::{Arc, RwLock, Mutex};
use std::time::Duration;
use std::thread;

use tracing::{debug, info, warn};

use crate::network::protocol::{Message, encode_message, decode_message};

/// Connection to a peer node
pub struct PeerConnection {
    pub node_id: String,
    pub address: String,
    stream: Mutex<TcpStream>,
}

impl PeerConnection {
    /// Connect to a peer
    pub fn connect(node_id: String, address: &str) -> std::io::Result<Self> {
        let stream = TcpStream::connect(address)?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;
        
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
    pub fn request(&self, msg: &Message) -> Result<Message, Box<dyn std::error::Error + Send + Sync>> {
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
        *self.running.write().unwrap() = true;
        
        let bind_addr: SocketAddr = self.bind_address.parse()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let listener = TcpListener::bind(bind_addr)?;
        listener.set_nonblocking(true)?;
        
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
    pub fn connect(&self, node_id: &str, address: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = PeerConnection::connect(node_id.to_string(), address)?;
        self.connections.write().unwrap().insert(node_id.to_string(), Arc::new(conn));
        info!("Connected to peer {} at {}", node_id, address);
        Ok(())
    }

    /// Send message to a specific peer
    pub fn send_to(&self, node_id: &str, msg: &Message) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let connections = self.connections.read().unwrap();
        if let Some(conn) = connections.get(node_id) {
            conn.send(msg)?;
        }
        Ok(())
    }

    /// Broadcast message to all peers
    pub fn broadcast(&self, msg: &Message) {
        let connections = self.connections.read().unwrap();
        for (id, conn) in connections.iter() {
            if let Err(e) = conn.send(msg) {
                warn!("Failed to send to {}: {}", id, e);
            }
        }
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
    pub fn get_or_connect_leader(&self, leader_id: &str, leader_addr: &str) -> Result<Arc<PeerConnection>, Box<dyn std::error::Error + Send + Sync>> {
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
        self.connections.write().unwrap().insert(leader_id.to_string(), conn.clone());
        Ok(conn)
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
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    
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
