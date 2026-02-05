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

/// Initial buffer size (16MB - MySQL default max_allowed_packet)
const INITIAL_BUFFER_SIZE: usize = 16 * 1024 * 1024;
/// Maximum buffer size (1GB - MySQL maximum max_allowed_packet)
const MAX_BUFFER_SIZE: usize = 1024 * 1024 * 1024;
/// Buffer growth factor
const BUFFER_GROWTH_FACTOR: usize = 2;

/// Read a complete MySQL packet from a stream with dynamic buffer sizing.
/// MySQL packets have a 4-byte header: 3 bytes length + 1 byte sequence ID.
/// This function reads the header first, determines required size, resizes buffer if needed,
/// then reads exactly the payload bytes.
#[allow(dead_code)]
async fn read_mysql_packet_dynamic(
    stream: &mut TcpStream, 
    buf: &mut Vec<u8>
) -> std::io::Result<usize> {
    use tokio::io::AsyncReadExt;
    
    // Read the 4-byte header first
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await?;
    
    // Parse payload length (3 bytes, little-endian)
    let payload_len = (header[0] as usize) 
        | ((header[1] as usize) << 8) 
        | ((header[2] as usize) << 16);
    
    let total_len = 4 + payload_len;
    
    // Check against absolute maximum
    if total_len > MAX_BUFFER_SIZE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Packet too large: {} bytes (max: {} bytes)", total_len, MAX_BUFFER_SIZE)
        ));
    }
    
    // Dynamically resize buffer if needed
    if total_len > buf.len() {
        let new_size = (total_len * BUFFER_GROWTH_FACTOR).min(MAX_BUFFER_SIZE);
        tracing::debug!("Resizing buffer from {} to {} bytes for {} byte packet", 
            buf.len(), new_size, total_len);
        buf.resize(new_size, 0);
    }
    
    // Copy header to buffer
    buf[0..4].copy_from_slice(&header);
    
    // Read exact payload bytes
    if payload_len > 0 {
        stream.read_exact(&mut buf[4..total_len]).await?;
    }
    
    // Log large packets for monitoring
    if total_len > 1024 * 1024 {
        tracing::info!("Processing large packet: {} MB", total_len / (1024 * 1024));
    }
    
    Ok(total_len)
}

/// Read data from stream into a dynamically-sized buffer.
/// Used for general reads where we don't know packet boundaries.
async fn read_with_dynamic_buffer(
    stream: &mut TcpStream,
    buf: &mut Vec<u8>,
) -> std::io::Result<usize> {
    use tokio::io::AsyncReadExt;
    
    // Try to read with current buffer
    let n = stream.read(buf).await?;
    
    // If buffer was completely filled, we might need more space
    if n == buf.len() && buf.len() < MAX_BUFFER_SIZE {
        let new_size = (buf.len() * BUFFER_GROWTH_FACTOR).min(MAX_BUFFER_SIZE);
        tracing::debug!("Buffer full, resizing from {} to {} bytes", buf.len(), new_size);
        buf.resize(new_size, 0);
    }
    
    Ok(n)
}

/// Result from forwarding a write to the leader
struct ForwardWriteResult {
    affected_rows: u64,
    last_insert_id: u64,
}

/// Forward a write query to the leader's HTTP API
async fn forward_write_to_leader(
    leader_url: &str,
    query: &str,
    database: &Option<String>,
) -> std::result::Result<ForwardWriteResult, String> {
    let client = reqwest::Client::new();
    
    let body = serde_json::json!({
        "sql": query,
        "database": database,
    });
    
    let response = client.post(leader_url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;
    
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("Leader returned error {}: {}", status, text));
    }
    
    // Parse response - leader returns JSON with affected_rows and last_insert_id
    let json: serde_json::Value = response.json().await
        .map_err(|e| format!("Failed to parse leader response: {}", e))?;
    
    Ok(ForwardWriteResult {
        affected_rows: json.get("affected_rows").and_then(|v| v.as_u64()).unwrap_or(0),
        last_insert_id: json.get("last_insert_id").and_then(|v| v.as_u64()).unwrap_or(0),
    })
}

/// Create a MySQL OK packet
fn create_mysql_ok_packet(affected_rows: u64, last_insert_id: u64) -> Vec<u8> {
    let mut payload = Vec::new();
    
    // OK packet header
    payload.push(0x00); // OK marker
    
    // Affected rows (length-encoded int)
    write_lenenc_int(&mut payload, affected_rows);
    
    // Last insert ID (length-encoded int)
    write_lenenc_int(&mut payload, last_insert_id);
    
    // Status flags (2 bytes) - autocommit
    payload.push(0x02);
    payload.push(0x00);
    
    // Warnings (2 bytes)
    payload.push(0x00);
    payload.push(0x00);
    
    // Build packet with header
    let payload_len = payload.len();
    let mut packet = Vec::new();
    packet.push((payload_len & 0xFF) as u8);
    packet.push(((payload_len >> 8) & 0xFF) as u8);
    packet.push(((payload_len >> 16) & 0xFF) as u8);
    packet.push(0x01); // Sequence ID = 1
    packet.extend(payload);
    
    packet
}

/// Create a MySQL error packet
fn create_mysql_error_packet(message: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    
    // Error packet header
    payload.push(0xFF); // Error marker
    
    // Error code (2 bytes) - generic error
    payload.push(0x00);
    payload.push(0x04);  // Error 1024
    
    // SQL state marker
    payload.push(b'#');
    
    // SQL state (5 bytes)
    payload.extend_from_slice(b"HY000");
    
    // Error message
    payload.extend_from_slice(message.as_bytes());
    
    // Build packet with header
    let payload_len = payload.len();
    let mut packet = Vec::new();
    packet.push((payload_len & 0xFF) as u8);
    packet.push(((payload_len >> 8) & 0xFF) as u8);
    packet.push(((payload_len >> 16) & 0xFF) as u8);
    packet.push(0x01); // Sequence ID = 1
    packet.extend(payload);
    
    packet
}

/// Write a length-encoded integer
fn write_lenenc_int(buf: &mut Vec<u8>, val: u64) {
    if val < 251 {
        buf.push(val as u8);
    } else if val < 65536 {
        buf.push(0xFC);
        buf.push((val & 0xFF) as u8);
        buf.push(((val >> 8) & 0xFF) as u8);
    } else if val < 16777216 {
        buf.push(0xFD);
        buf.push((val & 0xFF) as u8);
        buf.push(((val >> 8) & 0xFF) as u8);
        buf.push(((val >> 16) & 0xFF) as u8);
    } else {
        buf.push(0xFE);
        for i in 0..8 {
            buf.push(((val >> (i * 8)) & 0xFF) as u8);
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
    // MySQL HandshakeResponse41 packet structure:
    // 4 bytes: packet header (3 bytes length + 1 byte sequence)
    // 4 bytes: capability flags (lower 16 bits first, then upper 16 bits)
    // 4 bytes: max packet size
    // 1 byte: character set
    // 23 bytes: reserved (zeros)
    // string<NUL>: username
    // string<length>: auth response (length prefixed if CLIENT_SECURE_CONNECTION)
    // string<NUL>: database (if CLIENT_CONNECT_WITH_DB = 0x08 is set)
    // string<NUL>: client plugin name (if CLIENT_PLUGIN_AUTH)
    
    if packet.len() < 40 {
        return None;
    }
    
    // Skip packet header (4 bytes)
    let data = &packet[4..];
    
    if data.len() < 36 {
        return None;
    }
    
    // Read capability flags (4 bytes, little-endian)
    let cap_flags = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    
    // CLIENT_CONNECT_WITH_DB = 0x00000008
    let has_connect_with_db = (cap_flags & 0x08) != 0;
    // CLIENT_SECURE_CONNECTION = 0x00008000
    let has_secure_connection = (cap_flags & 0x8000) != 0;
    // CLIENT_PLUGIN_AUTH_LENENC_CLIENT_DATA = 0x00200000
    let has_lenenc_auth = (cap_flags & 0x00200000) != 0;
    
    if !has_connect_with_db {
        return None;
    }
    
    // Position after fixed fields: caps(4) + max_pkt(4) + charset(1) + reserved(23) = 32
    let mut pos = 32;
    
    // Skip username (NUL-terminated)
    while pos < data.len() && data[pos] != 0 {
        pos += 1;
    }
    if pos >= data.len() {
        return None;
    }
    pos += 1; // Skip NUL terminator
    
    // Skip auth response
    if pos >= data.len() {
        return None;
    }
    
    if has_secure_connection {
        // Length-prefixed auth data
        if has_lenenc_auth {
            // Length-encoded integer for auth length
            let first_byte = data[pos];
            pos += 1;
            let auth_len = if first_byte < 251 {
                first_byte as usize
            } else if first_byte == 252 && pos + 2 <= data.len() {
                let len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
                pos += 2;
                len
            } else if first_byte == 253 && pos + 3 <= data.len() {
                let len = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], 0]) as usize;
                pos += 3;
                len
            } else {
                return None;
            };
            pos += auth_len;
        } else {
            // Simple length prefix (1 byte)
            let auth_len = data[pos] as usize;
            pos += 1 + auth_len;
        }
    } else {
        // NUL-terminated auth data (old style)
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        pos += 1;
    }
    
    if pos >= data.len() {
        return None;
    }
    
    // Now we're at the database name (NUL-terminated string)
    let db_start = pos;
    while pos < data.len() && data[pos] != 0 {
        pos += 1;
    }
    
    if pos > db_start {
        if let Ok(db_name) = String::from_utf8(data[db_start..pos].to_vec()) {
            // Validate it looks like a reasonable database name
            if !db_name.is_empty() && db_name.len() < 256 {
                return Some(db_name);
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
    // IMPORTANT: With mysql -e, the client may send the query immediately after auth.
    // A single read() could get both packets. We must parse only the first packet
    // (handshake response) and save any extra bytes for the command loop.
    let mut response_buf = vec![0u8; 65536];
    let bytes_read = client.read(&mut response_buf).await?;
    if bytes_read == 0 {
        return Ok(());
    }
    
    // Parse the first packet's length from the 4-byte header
    let first_packet_len = if bytes_read >= 4 {
        let payload_len = (response_buf[0] as usize) 
            | ((response_buf[1] as usize) << 8) 
            | ((response_buf[2] as usize) << 16);
        4 + payload_len // header + payload
    } else {
        bytes_read // Not enough for header, send what we have
    };
    
    // Clamp to actual bytes read
    let first_packet_end = first_packet_len.min(bytes_read);
    
    // Try to extract database name from handshake response
    let initial_database = extract_database_from_handshake(&response_buf[..first_packet_end]);
    
    // Forward ONLY the first packet (handshake response) to backend
    backend.write_all(&response_buf[..first_packet_end]).await?;
    
    // Save any extra bytes (could be COM_QUERY from -e) for the command loop
    let mut leftover_bytes: Vec<u8> = Vec::new();
    if bytes_read > first_packet_end {
        leftover_bytes = response_buf[first_packet_end..bytes_read].to_vec();
    }

    // Phase 3: Simple auth completion - read one response from backend
    let n = backend.read(&mut handshake_buf).await?;
    if n == 0 {
        return Ok(());
    }
    client.write_all(&handshake_buf[..n]).await?;
    
    // Check if auth failed
    if n > 4 && handshake_buf[4] == 0xFF {
        return Ok(());
    }
    
    // Handle auth switch if needed (0xFE that's not EOF)
    if n > 4 && handshake_buf[4] == 0xFE && n > 9 {
        // Auth switch - relay client response
        // Note: with auth switch, leftover_bytes would be auth data, not query
        let n = if !leftover_bytes.is_empty() {
            // Use leftover bytes first
            let n = leftover_bytes.len();
            backend.write_all(&leftover_bytes).await?;
            leftover_bytes.clear();
            n
        } else {
            let n = client.read(&mut response_buf).await?;
            if n > 0 {
                backend.write_all(&response_buf[..n]).await?;
            }
            n
        };
        if n > 0 {
            // Get final auth result
            let n = backend.read(&mut handshake_buf).await?;
            if n > 0 {
                client.write_all(&handshake_buf[..n]).await?;
            }
        }
    }

    // Phase 4: Main command loop with smart routing
    // Use dynamic buffer sizing - start small, grow as needed
    let mut cmd_buf = vec![0u8; INITIAL_BUFFER_SIZE]; // Start with 16MB, grows dynamically
    let mut result_buf = vec![0u8; INITIAL_BUFFER_SIZE];
    let current_backend_addr = initial_backend_addr;
    
    // Track current database context for replication
    let mut current_database: Option<String> = initial_database;
    
    // If we have leftover bytes from auth phase (mysql -e), process them first
    let mut pending_data: Option<Vec<u8>> = if !leftover_bytes.is_empty() {
        Some(leftover_bytes)
    } else {
        None
    };
    
    loop {
        // Get packet data from pending data or read from client
        let n = if let Some(data) = pending_data.take() {
            let n = data.len();
            cmd_buf[..n].copy_from_slice(&data);
            n
        } else {
            // Use dynamic buffer reading for potentially large packets
            match read_with_dynamic_buffer(&mut client, &mut cmd_buf).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    tracing::debug!("Client read error: {}", e);
                    break;
                }
            }
        };

        // Parse the complete packet to check query type
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

        // If this is a write query, handle based on role
        if is_write {
            if let Some(ref query) = query_opt {
                tracing::debug!("WRITE detected: {}", query.chars().take(80).collect::<String>());
                
                let self_node = cluster.get_self().await;
                
                if self_node.role == NodeRole::Leader {
                    // Leader: write to WAL for replication
                    if let Some(ref wal) = wal_writer {
                        let table_name = extract_table_name(query);
                        
                        let entry = LogEntry::RawSql {
                            sql: query.clone(),
                            affects_table: table_name,
                            database: current_database.clone(),
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
                } else {
                    // Follower: forward write to leader's HTTP API
                    if let Some(leader) = cluster.current_leader().await {
                        let leader_host = leader.address.split(':').next().unwrap_or("localhost");
                        let leader_api_url = format!("http://{}:8080/sql", leader_host);
                        
                        tracing::info!("Forwarding write to leader at {}", leader_api_url);
                        
                        // Create HTTP client and forward the query
                        match forward_write_to_leader(&leader_api_url, query, &current_database).await {
                            Ok(result) => {
                                // Send success response to client
                                // This is an OK packet for MySQL protocol
                                let ok_packet = create_mysql_ok_packet(result.affected_rows, result.last_insert_id);
                                if let Err(e) = client.write_all(&ok_packet).await {
                                    tracing::error!("Failed to send OK packet to client: {}", e);
                                }
                                continue; // Skip local execution, continue with next command
                            }
                            Err(e) => {
                                tracing::error!("Failed to forward write to leader: {}", e);
                                // Send error to client
                                let err_packet = create_mysql_error_packet(&format!("Leader forwarding failed: {}", e));
                                if let Err(e) = client.write_all(&err_packet).await {
                                    tracing::error!("Failed to send error packet to client: {}", e);
                                }
                                continue;
                            }
                        }
                    } else {
                        tracing::warn!("No leader available for write forwarding, rejecting write");
                        let err_packet = create_mysql_error_packet("No leader available for write operation");
                        if let Err(e) = client.write_all(&err_packet).await {
                            tracing::error!("Failed to send error packet to client: {}", e);
                        }
                        continue;
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
        let query_start = std::time::Instant::now();
        let query_size = n;
        
        // Log large queries before sending
        if n > 1024 * 1024 {
            tracing::info!("Sending large query to backend: {} MB", n / (1024 * 1024));
        }
        
        if let Err(e) = backend.write_all(&cmd_buf[..n]).await {
            tracing::error!("Backend write error: {}", e);
            break;
        }

        // Read and relay response(s) from backend
        // For large queries, use timeout-based read to show progress
        let mut last_progress_log = std::time::Instant::now();
        loop {
            // Use a timeout to periodically log progress for long-running queries
            let read_result = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                read_with_dynamic_buffer(&mut backend, &mut result_buf)
            ).await;
            
            let rn = match read_result {
                Ok(Ok(0)) => {
                    tracing::debug!("Backend disconnected");
                    return Ok(());
                }
                Ok(Ok(n)) => n,
                Ok(Err(e)) => {
                    tracing::debug!("Backend read error: {}", e);
                    return Ok(());
                }
                Err(_timeout) => {
                    // Log progress for long-running queries
                    let elapsed = query_start.elapsed();
                    if last_progress_log.elapsed().as_secs() >= 5 {
                        tracing::info!("Still processing query ({} KB)... {}s elapsed", 
                            query_size / 1024, elapsed.as_secs());
                        last_progress_log = std::time::Instant::now();
                    }
                    continue; // Retry read
                }
            };
            
            // Log query completion time for slow queries
            let elapsed = query_start.elapsed();
            if elapsed.as_secs() > 5 {
                tracing::warn!("Slow query completed: {} bytes in {:.1}s", query_size, elapsed.as_secs_f64());
            } else if elapsed.as_secs() > 1 && query_size > 100_000 {
                tracing::info!("Query completed: {} KB in {:.1}s", query_size / 1024, elapsed.as_secs_f64());
            }

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
