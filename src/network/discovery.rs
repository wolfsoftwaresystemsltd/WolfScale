//! UDP Broadcast Discovery Module
//!
//! Enables automatic node discovery within the same network subnet.
//! Nodes broadcast their presence and listen for other nodes.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::RwLock;

use crate::error::{Error, Result};
use crate::state::ClusterMembership;

/// Discovery port (same as cluster port for simplicity)
const DISCOVERY_PORT: u16 = 7654;

/// Broadcast interval
const BROADCAST_INTERVAL: Duration = Duration::from_secs(2);

/// Discovery message prefix
const DISCOVERY_PREFIX: &str = "WOLFSCALE";

/// Discovery message version
const DISCOVERY_VERSION: u8 = 1;

/// UDP broadcast discovery for automatic node detection
pub struct Discovery {
    /// This node's ID
    node_id: String,
    /// This node's advertise address (host:port)
    advertise_address: String,
    /// Optional cluster name for filtering
    cluster_name: Option<String>,
    /// Cluster membership to update when nodes are discovered
    cluster: Arc<ClusterMembership>,
    /// Running flag
    running: Arc<RwLock<bool>>,
}

impl Discovery {
    /// Create a new discovery instance
    pub fn new(
        node_id: String,
        advertise_address: String,
        cluster_name: Option<String>,
        cluster: Arc<ClusterMembership>,
    ) -> Self {
        Self {
            node_id,
            advertise_address,
            cluster_name,
            cluster,
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Start the discovery broadcaster and listener
    /// Returns handles to the spawned tasks
    pub async fn start(&self) -> Result<(tokio::task::JoinHandle<()>, tokio::task::JoinHandle<()>)> {
        *self.running.write().await = true;

        // Create UDP socket for broadcasting
        let broadcast_socket = UdpSocket::bind("0.0.0.0:0").await
            .map_err(|e| Error::Network(format!("Failed to bind broadcast socket: {}", e)))?;
        
        broadcast_socket.set_broadcast(true)
            .map_err(|e| Error::Network(format!("Failed to enable broadcast: {}", e)))?;

        // Create UDP socket for listening on discovery port
        let listen_socket = match UdpSocket::bind(format!("0.0.0.0:{}", DISCOVERY_PORT)).await {
            Ok(s) => Some(s),
            Err(e) => {
                // Port might be in use by the main server, that's OK
                tracing::debug!("Could not bind discovery listener on port {}: {} (will use main server)", DISCOVERY_PORT, e);
                None
            }
        };

        // Start broadcaster
        let broadcaster_handle = self.start_broadcaster(broadcast_socket).await;

        // Start listener (if we have a socket)
        let listener_handle = if let Some(socket) = listen_socket {
            self.start_listener(socket).await
        } else {
            // Dummy handle that does nothing
            tokio::spawn(async {})
        };

        Ok((broadcaster_handle, listener_handle))
    }

    /// Start the broadcaster task
    async fn start_broadcaster(&self, socket: UdpSocket) -> tokio::task::JoinHandle<()> {
        let node_id = self.node_id.clone();
        let advertise_address = self.advertise_address.clone();
        let cluster_name = self.cluster_name.clone();
        let running = Arc::clone(&self.running);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(BROADCAST_INTERVAL);
            
            loop {
                interval.tick().await;

                if !*running.read().await {
                    break;
                }

                // Build discovery message
                let message = format_discovery_message(
                    &cluster_name,
                    &node_id,
                    &advertise_address,
                );

                // Broadcast to the network
                let broadcast_addr: SocketAddr = format!("255.255.255.255:{}", DISCOVERY_PORT)
                    .parse()
                    .unwrap();

                if let Err(e) = socket.send_to(message.as_bytes(), broadcast_addr).await {
                    // Broadcast might not be supported on all networks
                    tracing::trace!("Broadcast send failed (network may not support broadcast): {}", e);
                } else {
                    tracing::trace!("Discovery broadcast sent");
                }
            }
        })
    }

    /// Start the listener task
    async fn start_listener(&self, socket: UdpSocket) -> tokio::task::JoinHandle<()> {
        let node_id = self.node_id.clone();
        let cluster_name = self.cluster_name.clone();
        let cluster = Arc::clone(&self.cluster);
        let running = Arc::clone(&self.running);

        tokio::spawn(async move {
            let mut buf = [0u8; 512];

            loop {
                if !*running.read().await {
                    break;
                }

                // Use timeout to periodically check running flag
                let recv_result = tokio::time::timeout(
                    Duration::from_secs(1),
                    socket.recv_from(&mut buf),
                ).await;

                let (len, src) = match recv_result {
                    Ok(Ok((len, src))) => (len, src),
                    Ok(Err(e)) => {
                        tracing::trace!("Discovery recv error: {}", e);
                        continue;
                    }
                    Err(_) => continue, // Timeout, check running flag
                };

                // Parse the message
                let message = match std::str::from_utf8(&buf[..len]) {
                    Ok(s) => s,
                    Err(_) => continue,
                };

                if let Some((msg_cluster, msg_node_id, msg_address)) = parse_discovery_message(message) {
                    // Skip our own broadcasts
                    if msg_node_id == node_id {
                        continue;
                    }

                    // Check cluster name filter
                    if !cluster_names_match(&cluster_name, &msg_cluster) {
                        tracing::debug!(
                            "Ignoring node {} from different cluster: {:?} (we are {:?})",
                            msg_node_id, msg_cluster, cluster_name
                        );
                        continue;
                    }

                    tracing::info!(
                        "Discovered node via broadcast: {} at {} (from {})",
                        msg_node_id, msg_address, src
                    );

                    // Add to cluster membership
                    if let Err(e) = cluster.add_peer(msg_node_id.clone(), msg_address.clone()).await {
                        tracing::warn!("Failed to add discovered node {}: {}", msg_node_id, e);
                    }
                }
            }
        })
    }

    /// Stop the discovery tasks
    pub async fn stop(&self) {
        *self.running.write().await = false;
    }
}

/// Format a discovery broadcast message
fn format_discovery_message(
    cluster_name: &Option<String>,
    node_id: &str,
    advertise_address: &str,
) -> String {
    let cluster = cluster_name.as_deref().unwrap_or("");
    format!(
        "{}|{}|{}|{}|{}",
        DISCOVERY_PREFIX,
        DISCOVERY_VERSION,
        cluster,
        node_id,
        advertise_address
    )
}

/// Parse a discovery broadcast message
/// Returns (cluster_name, node_id, advertise_address)
fn parse_discovery_message(message: &str) -> Option<(Option<String>, String, String)> {
    let parts: Vec<&str> = message.split('|').collect();
    
    if parts.len() < 5 {
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

    let cluster_name = if parts[2].is_empty() {
        None
    } else {
        Some(parts[2].to_string())
    };

    let node_id = parts[3].to_string();
    let advertise_address = parts[4].to_string();

    Some((cluster_name, node_id, advertise_address))
}

/// Check if cluster names match (both empty = match, or both same value = match)
fn cluster_names_match(ours: &Option<String>, theirs: &Option<String>) -> bool {
    match (ours, theirs) {
        (None, None) => true,                        // Both unconfigured, allow
        (None, Some(_)) => true,                     // We're open, accept any
        (Some(_), None) => true,                     // They're open, accept
        (Some(a), Some(b)) => a == b,                // Both configured, must match
    }
}

/// Standalone discovery scan - used by load balancer to find nodes
/// Returns list of (node_id, address) tuples
pub async fn discover_cluster(
    cluster_name: Option<String>,
    timeout: Duration,
) -> Result<Vec<(String, String)>> {
    let socket = UdpSocket::bind(format!("0.0.0.0:{}", DISCOVERY_PORT + 1)).await
        .map_err(|e| Error::Network(format!("Failed to bind discovery socket: {}", e)))?;

    let mut discovered = Vec::new();
    let mut buf = [0u8; 512];
    let deadline = tokio::time::Instant::now() + timeout;

    tracing::info!("Scanning for WolfScale nodes via UDP broadcast ({} seconds)...", timeout.as_secs());

    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        
        let recv_result = tokio::time::timeout(
            remaining.min(Duration::from_secs(1)),
            socket.recv_from(&mut buf),
        ).await;

        let (len, _src) = match recv_result {
            Ok(Ok((len, src))) => (len, src),
            Ok(Err(_)) | Err(_) => continue,
        };

        let message = match std::str::from_utf8(&buf[..len]) {
            Ok(s) => s,
            Err(_) => continue,
        };

        if let Some((msg_cluster, msg_node_id, msg_address)) = parse_discovery_message(message) {
            // Check cluster name filter
            if !cluster_names_match(&cluster_name, &msg_cluster) {
                continue;
            }

            // Avoid duplicates
            if !discovered.iter().any(|(id, _)| id == &msg_node_id) {
                tracing::info!("  Found: {} at {}", msg_node_id, msg_address);
                discovered.push((msg_node_id, msg_address));
            }
        }
    }

    Ok(discovered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_parse_message() {
        let msg = format_discovery_message(
            &Some("test-cluster".to_string()),
            "node-1",
            "10.0.10.115:7654",
        );

        let parsed = parse_discovery_message(&msg);
        assert!(parsed.is_some());

        let (cluster, node_id, addr) = parsed.unwrap();
        assert_eq!(cluster, Some("test-cluster".to_string()));
        assert_eq!(node_id, "node-1");
        assert_eq!(addr, "10.0.10.115:7654");
    }

    #[test]
    fn test_format_parse_no_cluster() {
        let msg = format_discovery_message(
            &None,
            "node-2",
            "192.168.1.100:7654",
        );

        let parsed = parse_discovery_message(&msg);
        assert!(parsed.is_some());

        let (cluster, node_id, addr) = parsed.unwrap();
        assert_eq!(cluster, None);
        assert_eq!(node_id, "node-2");
        assert_eq!(addr, "192.168.1.100:7654");
    }

    #[test]
    fn test_cluster_name_matching() {
        // Both None - match
        assert!(cluster_names_match(&None, &None));

        // One None - always match (open policy)
        assert!(cluster_names_match(&None, &Some("foo".to_string())));
        assert!(cluster_names_match(&Some("foo".to_string()), &None));

        // Same name - match
        assert!(cluster_names_match(
            &Some("prod".to_string()),
            &Some("prod".to_string())
        ));

        // Different names - no match
        assert!(!cluster_names_match(
            &Some("prod".to_string()),
            &Some("dev".to_string())
        ));
    }
}
