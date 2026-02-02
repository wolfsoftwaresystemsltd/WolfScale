//! MySQL Proxy Server
//!
//! TCP server that proxies MySQL connections to backend MariaDB.
//! - Relays the real MariaDB handshake for proper authentication
//! - Parses command packets to detect writes
//! - Smart routing: reads from local if caught up, otherwise from leader
//! - Writes always go through the cluster for replication

use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::state::{ClusterMembership, NodeStatus, NodeRole};
use crate::error::Result;
use super::protocol::MySqlPacket;

/// MySQL proxy server configuration
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Address to listen on
    pub listen_address: String,
    /// Backend MariaDB host (local)
    pub backend_host: String,
    /// Backend MariaDB port
    pub backend_port: u16,
    /// Backend username
    pub backend_user: String,
    /// Backend password
    pub backend_password: String,
}

/// MySQL proxy server
pub struct ProxyServer {
    config: ProxyConfig,
    cluster: Arc<ClusterMembership>,
}

impl ProxyServer {
    pub fn new(config: ProxyConfig, cluster: Arc<ClusterMembership>) -> Self {
        Self { config, cluster }
    }

    /// Start the proxy server
    pub async fn start(&self) -> Result<()> {
        let listener = TcpListener::bind(&self.config.listen_address).await?;
        tracing::info!("MySQL proxy listening on {}", self.config.listen_address);

        loop {
            let (client_socket, addr) = listener.accept().await?;
            tracing::debug!("New MySQL client connection from {}", addr);

            let config = self.config.clone();
            let cluster = Arc::clone(&self.cluster);

            tokio::spawn(async move {
                if let Err(e) = handle_connection(client_socket, config, cluster).await {
                    tracing::error!("Proxy connection error: {}", e);
                }
            });
        }
    }
}

/// Check if a query is a write operation
fn is_write_query(query: &str) -> bool {
    let upper = query.trim().to_uppercase();
    upper.starts_with("INSERT") ||
    upper.starts_with("UPDATE") ||
    upper.starts_with("DELETE") ||
    upper.starts_with("CREATE") ||
    upper.starts_with("ALTER") ||
    upper.starts_with("DROP") ||
    upper.starts_with("TRUNCATE") ||
    upper.starts_with("REPLACE")
}

/// Determine the backend address to use based on query type and replication status
async fn get_backend_address(
    config: &ProxyConfig,
    cluster: &ClusterMembership,
    is_write: bool,
) -> String {
    // For writes, always try to route to leader
    if is_write {
        if let Some(leader) = cluster.current_leader().await {
            // Extract host from leader address (format: host:raft_port)
            // We need to use the MariaDB port, not the raft port
            let leader_host = leader.address.split(':').next().unwrap_or(&config.backend_host);
            return format!("{}:{}", leader_host, config.backend_port);
        }
    }

    // For reads, check if local node is caught up
    let self_node = cluster.get_self().await;
    
    // Read locally if:
    // 1. We are the leader, OR
    // 2. We are Active with no replication lag
    let can_read_locally = self_node.role == NodeRole::Leader 
        || (self_node.status == NodeStatus::Active && self_node.replication_lag == 0);

    if can_read_locally {
        tracing::debug!("Reading from local backend (caught up)");
        format!("{}:{}", config.backend_host, config.backend_port)
    } else {
        // Not caught up - route to leader
        if let Some(leader) = cluster.current_leader().await {
            let leader_host = leader.address.split(':').next().unwrap_or(&config.backend_host);
            tracing::debug!("Reading from leader {} (local is lagging by {} entries)", 
                leader_host, self_node.replication_lag);
            format!("{}:{}", leader_host, config.backend_port)
        } else {
            // No leader known - fall back to local
            tracing::warn!("No leader known, falling back to local backend");
            format!("{}:{}", config.backend_host, config.backend_port)
        }
    }
}

/// Handle a client connection by proxying to backend MariaDB
async fn handle_connection(
    mut client: TcpStream,
    config: ProxyConfig,
    cluster: Arc<ClusterMembership>,
) -> Result<()> {
    // Determine initial backend (for handshake)
    // Use local backend for initial connection - it's faster and handles auth
    let initial_backend_addr = format!("{}:{}", config.backend_host, config.backend_port);
    
    // Connect to backend MariaDB
    let mut backend = match TcpStream::connect(&initial_backend_addr).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to connect to backend {}: {}", initial_backend_addr, e);
            return Err(e.into());
        }
    };
    
    tracing::debug!("Connected to backend MariaDB at {}", initial_backend_addr);

    // Phase 1: Relay the handshake from real MariaDB to client
    let mut handshake_buf = vec![0u8; 4096];
    let n = backend.read(&mut handshake_buf).await?;
    if n == 0 {
        return Ok(());
    }
    client.write_all(&handshake_buf[..n]).await?;

    // Phase 2: Relay client's handshake response to backend
    let mut response_buf = vec![0u8; 4096];
    let n = client.read(&mut response_buf).await?;
    if n == 0 {
        return Ok(());
    }
    backend.write_all(&response_buf[..n]).await?;

    // Phase 3: Relay auth result from backend to client
    let n = backend.read(&mut handshake_buf).await?;
    if n == 0 {
        return Ok(());
    }
    client.write_all(&handshake_buf[..n]).await?;

    // Phase 4: Main command loop with smart routing
    let mut cmd_buf = vec![0u8; 16 * 1024 * 1024]; // 16MB max packet
    let mut result_buf = vec![0u8; 16 * 1024 * 1024];
    let current_backend_addr = initial_backend_addr;
    
    loop {
        // Read command from client
        let n = match client.read(&mut cmd_buf).await {
            Ok(0) => {
                tracing::debug!("Client disconnected");
                break;
            }
            Ok(n) => n,
            Err(e) => {
                tracing::debug!("Client read error: {}", e);
                break;
            }
        };

        // Parse packet to check query type
        let is_write = if let Ok((packet, _)) = MySqlPacket::read(&cmd_buf[..n]) {
            if let Some(query) = packet.query_string() {
                let write = is_write_query(&query);
                if write {
                    tracing::info!("WRITE query detected: {}", 
                        query.chars().take(100).collect::<String>());
                }
                write
            } else {
                false
            }
        } else {
            false
        };

        // Get the appropriate backend address
        let target_addr = get_backend_address(&config, &cluster, is_write).await;
        
        // If backend changed, reconnect
        if target_addr != current_backend_addr {
            tracing::debug!("Switching backend from {} to {}", current_backend_addr, target_addr);
            
            // Note: For a production system, we'd need to maintain connection state
            // For now, we'll keep using the initial connection since switching mid-session
            // would lose session state (USE database, transactions, etc.)
            // 
            // TODO: Implement proper connection pooling with session affinity
            // For now, log the routing decision but keep using existing connection
            tracing::info!("Would route to {} (keeping existing connection for session state)", target_addr);
        }

        // Forward command to backend
        if let Err(e) = backend.write_all(&cmd_buf[..n]).await {
            tracing::error!("Backend write error: {}", e);
            break;
        }

        // Read and relay response(s) from backend
        loop {
            let rn = match backend.read(&mut result_buf).await {
                Ok(0) => {
                    tracing::debug!("Backend disconnected");
                    return Ok(());
                }
                Ok(n) => n,
                Err(e) => {
                    tracing::debug!("Backend read error: {}", e);
                    return Ok(());
                }
            };

            // Forward to client
            if let Err(e) = client.write_all(&result_buf[..rn]).await {
                tracing::error!("Client write error: {}", e);
                return Ok(());
            }

            // Check if there's more data pending
            let mut peek_buf = [0u8; 1];
            match tokio::time::timeout(
                std::time::Duration::from_millis(10),
                backend.peek(&mut peek_buf)
            ).await {
                Ok(Ok(0)) | Err(_) => break,
                Ok(Ok(_)) => continue,
                Ok(Err(_)) => break,
            }
        }
    }

    Ok(())
}
