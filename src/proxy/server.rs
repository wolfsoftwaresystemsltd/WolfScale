//! MySQL Proxy Server
//!
//! TCP server that proxies MySQL connections to backend MariaDB.
//! - Relays the real MariaDB handshake for proper authentication
//! - Parses command packets to detect writes
//! - Writes are logged to WAL for replication before execution
//! - Smart routing: reads from local if caught up, otherwise from leader

use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::state::{ClusterMembership, NodeStatus, NodeRole};
use crate::wal::{WalWriter, LogEntry};
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
    wal_writer: Option<WalWriter>,
}

impl ProxyServer {
    pub fn new(config: ProxyConfig, cluster: Arc<ClusterMembership>) -> Self {
        Self { config, cluster, wal_writer: None }
    }

    /// Create with WAL writer for replication support
    pub fn with_wal(config: ProxyConfig, cluster: Arc<ClusterMembership>, wal_writer: WalWriter) -> Self {
        Self { config, cluster, wal_writer: Some(wal_writer) }
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
            let wal_writer = self.wal_writer.clone();

            tokio::spawn(async move {
                if let Err(e) = handle_connection(client_socket, config, cluster, wal_writer).await {
                    tracing::error!("Proxy connection error: {}", e);
                }
            });
        }
    }
}

/// Check if a query is a write operation
fn is_write_query(query: &str) -> bool {
    // Strip leading comments (mysqldump adds /* ... */ style comments)
    let stripped = strip_leading_comments(query);
    let upper = stripped.trim().to_uppercase();
    
    // DDL statements (CREATE covers TABLE, VIEW, TRIGGER, PROCEDURE, FUNCTION, INDEX, DATABASE, etc.)
    upper.starts_with("CREATE") ||
    upper.starts_with("ALTER") ||
    upper.starts_with("DROP") ||
    upper.starts_with("RENAME") ||
    upper.starts_with("TRUNCATE") ||
    // DML statements
    upper.starts_with("INSERT") ||
    upper.starts_with("UPDATE") ||
    upper.starts_with("DELETE") ||
    upper.starts_with("REPLACE") ||
    upper.starts_with("LOAD") ||  // LOAD DATA INFILE
    upper.starts_with("CALL") ||  // Stored procedure calls
    // Permissions
    upper.starts_with("GRANT") ||
    upper.starts_with("REVOKE") ||
    // Lock statements
    upper.starts_with("LOCK") ||
    upper.starts_with("UNLOCK") ||
    // Session/context
    upper.starts_with("SET") ||
    upper.starts_with("USE") ||
    // Transactions (needed for consistent replication)
    upper.starts_with("START") ||  // START TRANSACTION
    upper.starts_with("BEGIN") ||
    upper.starts_with("COMMIT") ||
    upper.starts_with("ROLLBACK") ||
    upper.starts_with("SAVEPOINT") ||
    // Maintenance
    upper.starts_with("ANALYZE") ||
    upper.starts_with("OPTIMIZE") ||
    upper.starts_with("REPAIR") ||
    upper.starts_with("FLUSH")
}

/// Strip leading SQL comments from a query
fn strip_leading_comments(query: &str) -> &str {
    let mut s = query.trim();
    loop {
        // Strip /* ... */ comments
        if s.starts_with("/*") {
            if let Some(end) = s.find("*/") {
                s = s[end + 2..].trim_start();
                continue;
            }
        }
        // Strip -- comments (to end of line)
        if s.starts_with("--") {
            if let Some(end) = s.find('\n') {
                s = s[end + 1..].trim_start();
                continue;
            }
        }
        // Strip # comments (to end of line)  
        if s.starts_with("#") {
            if let Some(end) = s.find('\n') {
                s = s[end + 1..].trim_start();
                continue;
            }
        }
        break;
    }
    s
}

/// Extract database name from MySQL handshake response packet
/// When client connects with `mysql -D dbname` or `mysql dbname`, the database
/// is included in the handshake response packet, not sent as a separate command
fn extract_database_from_handshake(packet: &[u8]) -> Option<String> {
    // MySQL Handshake Response packet structure (simplified):
    // 4 bytes: packet header (length + sequence)
    // 4 bytes: client capabilities
    // 4 bytes: max packet size
    // 1 byte: charset
    // 23 bytes: reserved (zeros)
    // NUL-terminated string: username
    // Length-encoded auth response
    // NUL-terminated string: database (if CLIENT_CONNECT_WITH_DB flag is set)
    
    if packet.len() < 36 {
        return None;
    }
    
    // Skip packet header (4 bytes)
    let data = &packet[4..];
    
    if data.len() < 32 {
        return None;
    }
    
    // Check CLIENT_CONNECT_WITH_DB capability flag (bit 3 = 0x08)
    let cap_flags = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let has_db = (cap_flags & 0x08) != 0;
    
    if !has_db {
        return None;
    }
    
    // Skip: capabilities (4) + max_packet_size (4) + charset (1) + reserved (23) = 32 bytes
    let mut pos = 32;
    
    // Skip username (NUL-terminated)
    while pos < data.len() && data[pos] != 0 {
        pos += 1;
    }
    pos += 1; // Skip NUL
    
    if pos >= data.len() {
        return None;
    }
    
    // Skip auth response (length-prefixed or NUL-terminated depending on plugin)
    // Check CLIENT_PLUGIN_AUTH_LENENC_CLIENT_DATA (0x00200000) or CLIENT_SECURE_CONNECTION (0x8000)
    let secure_connection = (cap_flags & 0x8000) != 0;
    
    if secure_connection {
        // Length-prefixed auth data
        let auth_len = data[pos] as usize;
        pos += 1 + auth_len;
    } else {
        // NUL-terminated auth data
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        pos += 1;
    }
    
    if pos >= data.len() {
        return None;
    }
    
    // Now we should be at the database name (NUL-terminated)
    let db_start = pos;
    while pos < data.len() && data[pos] != 0 {
        pos += 1;
    }
    
    if pos > db_start {
        String::from_utf8(data[db_start..pos].to_vec()).ok()
    } else {
        None
    }
}

/// Query the current database from MariaDB using SELECT DATABASE()
/// This is more reliable than parsing handshake packets
async fn query_current_database(backend: &mut TcpStream, buf: &mut [u8]) -> Option<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    
    // Build COM_QUERY packet for "SELECT DATABASE()"
    let query = b"SELECT DATABASE()";
    let payload_len = 1 + query.len(); // 1 byte for command + query
    
    // MySQL packet: 3 bytes length + 1 byte seq + payload
    let mut packet = Vec::with_capacity(4 + payload_len);
    packet.extend_from_slice(&(payload_len as u32).to_le_bytes()[..3]);
    packet.push(0); // sequence number
    packet.push(0x03); // COM_QUERY
    packet.extend_from_slice(query);
    
    // Send query to backend
    if backend.write_all(&packet).await.is_err() {
        return None;
    }
    
    // Read response
    let n = match backend.read(buf).await {
        Ok(n) if n > 0 => n,
        _ => return None,
    };
    
    // Parse result set to extract database name
    // Response will be: column count packet, column def packet, EOF, row data packet, EOF
    // The database name is in the row data packet as a length-prefixed string
    // We need to find it - it's typically after several packets
    
    // Simple approach: scan for a recognizable database name pattern
    // The row data contains a length-prefixed string with the database name
    parse_database_from_result(&buf[..n])
}

/// Parse a database name from a SELECT DATABASE() result set
fn parse_database_from_result(data: &[u8]) -> Option<String> {
    // Skip through packets looking for row data
    let mut pos = 0;
    let mut packet_count = 0;
    
    while pos + 4 < data.len() && packet_count < 10 {
        // Read packet header
        let pkt_len = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], 0]) as usize;
        
        if pkt_len == 0 || pos + 4 + pkt_len > data.len() {
            break;
        }
        
        let payload = &data[pos + 4..pos + 4 + pkt_len];
        pos += 4 + pkt_len;
        packet_count += 1;
        
        // Skip column count, column definition, and EOF packets
        // Row data packet will have a length-prefixed string
        if payload.is_empty() {
            continue;
        }
        
        // Check for NULL (0xFB) or EOF (0xFE) packet
        if payload[0] == 0xFB || payload[0] == 0xFE || payload[0] == 0xFF {
            continue;
        }
        
        // Try to parse as length-prefixed string (row data)
        if packet_count >= 3 && payload[0] < 0xFB {
            let str_len = payload[0] as usize;
            if str_len > 0 && str_len < payload.len() {
                if let Ok(s) = String::from_utf8(payload[1..1 + str_len].to_vec()) {
                    // Verify it looks like a database name (alphanumeric, underscores)
                    if s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
                        return Some(s);
                    }
                }
            }
        }
    }
    
    None
}

/// Extract table name from a SQL query (best effort)
fn extract_table_name(query: &str) -> Option<String> {
    let upper = query.trim().to_uppercase();
    let query_lower = query.trim();
    
    // Try common patterns
    if upper.starts_with("INSERT INTO ") {
        let rest = &query_lower[12..];
        Some(rest.split_whitespace().next()?.trim_matches('`').to_string())
    } else if upper.starts_with("UPDATE ") {
        let rest = &query_lower[7..];
        Some(rest.split_whitespace().next()?.trim_matches('`').to_string())
    } else if upper.starts_with("DELETE FROM ") {
        let rest = &query_lower[12..];
        Some(rest.split_whitespace().next()?.trim_matches('`').to_string())
    } else if upper.starts_with("CREATE TABLE ") {
        let rest = &query_lower[13..];
        let name = rest.split_whitespace().next()?.trim_matches('`');
        Some(name.split('(').next()?.to_string())
    } else if upper.starts_with("DROP TABLE ") {
        let rest = &query_lower[11..];
        Some(rest.split_whitespace().next()?.trim_matches('`').to_string())
    } else if upper.starts_with("ALTER TABLE ") {
        let rest = &query_lower[12..];
        Some(rest.split_whitespace().next()?.trim_matches('`').to_string())
    } else if upper.starts_with("CREATE DATABASE ") || upper.starts_with("DROP DATABASE ") {
        // Database operations - return the database name
        let rest = &query_lower[16..];
        Some(rest.split_whitespace().next()?.trim_matches('`').to_string())
    } else {
        None
    }
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
    wal_writer: Option<WalWriter>,
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
    
    // Try to extract database name from handshake response
    // The database name is embedded in the handshake when client connects with -D flag or db on cmdline
    let initial_database = extract_database_from_handshake(&response_buf[..n]);
    
    backend.write_all(&response_buf[..n]).await?;

    // Phase 3: Relay auth result from backend to client
    let n = backend.read(&mut handshake_buf).await?;
    if n == 0 {
        return Ok(());
    }
    client.write_all(&handshake_buf[..n]).await?;

    // Phase 3.5: If we didn't get database from handshake, query MariaDB for it
    // This is more reliable than parsing the handshake packet
    let detected_database = if initial_database.is_some() {
        initial_database
    } else {
        query_current_database(&mut backend, &mut handshake_buf).await
    };

    // Phase 4: Main command loop with smart routing
    let mut cmd_buf = vec![0u8; 16 * 1024 * 1024]; // 16MB max packet
    let mut result_buf = vec![0u8; 16 * 1024 * 1024];
    let current_backend_addr = initial_backend_addr;
    
    // Track current database context for replication
    // This is needed because followers need to know which database to execute statements in
    // Initialize with database from handshake or query if present
    let mut current_database: Option<String> = detected_database;
    
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
        let (is_write, query_opt) = if let Ok((packet, _)) = MySqlPacket::read(&cmd_buf[..n]) {
            if let Some(query) = packet.query_string() {
                let write = is_write_query(&query);
                (write, Some(query))
            } else {
                (false, None)
            }
        } else {
            (false, None)
        };

        // Track database context changes (USE statements or COM_INIT_DB)
        if let Some(ref query) = query_opt {
            let upper = query.trim().to_uppercase();
            if upper.starts_with("USE ") || upper.starts_with("USE`") {
                // Extract database name from USE statement
                let db_name = query.trim()[3..].trim().trim_matches('`').trim_matches(';').to_string();
                if !db_name.is_empty() {
                    current_database = Some(db_name);
                }
            }
        }

        // If this is a write query and we have a WAL writer, log it for replication
        // Only log if we are the leader - followers route to leader's proxy
        if is_write {
            if let Some(ref query) = query_opt {
                tracing::info!("WRITE query: {}", query.chars().take(100).collect::<String>());
                
                // Check if we are the leader - only leader writes to WAL
                let self_node = cluster.get_self().await;
                if self_node.role == NodeRole::Leader {
                    if let Some(ref wal) = wal_writer {
                        let table_name = extract_table_name(query);
                        
                        // Build SQL with database context for replication
                        // This ensures followers execute in the correct database
                        let sql_for_wal = if let Some(ref db) = current_database {
                            let upper = query.trim().to_uppercase();
                            // Don't prepend USE for database-level operations or USE itself
                            if upper.starts_with("USE ") || 
                               upper.starts_with("CREATE DATABASE") || 
                               upper.starts_with("DROP DATABASE") ||
                               upper.starts_with("ALTER DATABASE") {
                                query.clone()
                            } else {
                                format!("USE `{}`; {}", db, query)
                            }
                        } else {
                            query.clone()
                        };
                        
                        let entry = LogEntry::RawSql {
                            sql: sql_for_wal,
                            affects_table: table_name,
                        };
                        
                        match wal.append(entry).await {
                            Ok(lsn) => {
                                tracing::debug!("Write query logged to WAL with LSN {}", lsn);
                            }
                            Err(e) => {
                                tracing::error!("Failed to log write query to WAL: {}", e);
                            }
                        }
                    }
                }
            }
        }

        // Log routing decision (but don't switch backends mid-session)
        // Switching backends would break MySQL protocol - new connection gets handshake, not query results
        // Replication happens via WAL, not by forwarding queries
        let target_addr = get_backend_address(&config, &cluster, is_write).await;
        if target_addr != current_backend_addr {
            tracing::debug!("Would route to {} (keeping session on {})", target_addr, current_backend_addr);
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
