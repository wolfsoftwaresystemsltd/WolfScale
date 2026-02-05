//! Binlog Client
//!
//! Connects to MariaDB as a replica and streams binlog events.

use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::config::{DatabaseConfig, BinlogConfig};
use crate::wal::WalWriter;
use crate::error::Result;

use super::event::{parse_event, TableMap, BinlogEvent};
use super::converter::{binlog_to_wal, should_replicate_query};

/// Binlog replication client
pub struct BinlogClient {
    db_config: DatabaseConfig,
    binlog_config: BinlogConfig,
    wal_writer: Arc<WalWriter>,
}

impl BinlogClient {
    /// Create a new binlog client
    pub fn new(
        db_config: DatabaseConfig,
        binlog_config: BinlogConfig,
        wal_writer: Arc<WalWriter>,
    ) -> Self {
        Self {
            db_config,
            binlog_config,
            wal_writer,
        }
    }
    
    /// Start streaming binlog events
    pub async fn start(&self) -> Result<()> {
        tracing::info!(
            "Starting binlog client, connecting to {}:{} as server_id {}",
            self.db_config.host,
            self.db_config.port,
            self.binlog_config.server_id
        );
        
        // Connect to MariaDB
        let addr = format!("{}:{}", self.db_config.host, self.db_config.port);
        let mut stream = TcpStream::connect(&addr).await?;
        
        // Read the initial handshake packet
        let mut buf = vec![0u8; 65536];
        let n = stream.read(&mut buf).await?;
        tracing::debug!("Received handshake: {} bytes", n);
        
        // Parse handshake and authenticate
        self.authenticate(&mut stream, &buf[..n]).await?;
        
        // Get current binlog position if not specified
        let (binlog_file, binlog_pos) = self.get_binlog_position(&mut stream).await?;
        
        tracing::info!("Starting binlog replication from {}:{}", binlog_file, binlog_pos);
        
        // Register as a replica
        self.register_slave(&mut stream).await?;
        
        // Start binlog dump
        self.send_binlog_dump(&mut stream, &binlog_file, binlog_pos).await?;
        
        // Process binlog events
        let mut table_map = TableMap::new();
        let mut current_file = binlog_file;
        
        loop {
            // Read packet length (3 bytes) + sequence (1 byte)
            let mut header = [0u8; 4];
            if let Err(e) = stream.read_exact(&mut header).await {
                tracing::error!("Failed to read packet header: {}", e);
                break;
            }
            
            let packet_len = u32::from_le_bytes([header[0], header[1], header[2], 0]) as usize;
            
            if packet_len == 0 {
                continue;
            }
            
            // Read packet body
            let mut packet = vec![0u8; packet_len];
            if let Err(e) = stream.read_exact(&mut packet).await {
                tracing::error!("Failed to read packet body: {}", e);
                break;
            }
            
            // First byte is the packet type
            match packet[0] {
                0x00 => {
                    // OK packet with binlog event
                    if packet.len() > 1 {
                        match parse_event(&packet[1..]) {
                            Ok(event) => {
                                self.handle_event(event, &mut table_map, &mut current_file).await?;
                            }
                            Err(e) => {
                                tracing::warn!("Failed to parse event: {}", e);
                            }
                        }
                    }
                }
                0xFE => {
                    // EOF packet
                    tracing::info!("Received EOF from binlog stream");
                    break;
                }
                0xFF => {
                    // Error packet
                    let error_code = u16::from_le_bytes([packet[1], packet[2]]);
                    let error_msg = String::from_utf8_lossy(&packet[9..]);
                    tracing::error!("MySQL error {}: {}", error_code, error_msg);
                    break;
                }
                _ => {
                    tracing::debug!("Unknown packet type: 0x{:02X}", packet[0]);
                }
            }
        }
        
        Ok(())
    }
    
    async fn authenticate(&self, stream: &mut TcpStream, handshake: &[u8]) -> Result<()> {
        // Parse handshake to get auth data
        // MySQL handshake format: protocol_version, server_version, thread_id, auth_plugin_data, ...
        
        if handshake.len() < 10 {
            return Err(crate::Error::Network("Handshake too short".to_string()));
        }
        
        // Skip to auth_plugin_data_part_1 (offset depends on server version string)
        let version_end = handshake[1..].iter().position(|&b| b == 0).unwrap_or(0) + 1;
        let auth_start = version_end + 5; // +4 for thread_id, +1 for null terminator
        
        if auth_start + 8 > handshake.len() {
            return Err(crate::Error::Network("Cannot find auth data".to_string()));
        }
        
        let auth_data_1 = &handshake[auth_start..auth_start + 8];
        
        // Build authentication response (simplified - native password auth)
        let mut response = Vec::new();
        
        // Capability flags (4 bytes)
        let capabilities: u32 = 0x000FA68D; // CLIENT_PROTOCOL_41 | CLIENT_SECURE_CONNECTION | ...
        response.extend_from_slice(&capabilities.to_le_bytes());
        
        // Max packet size (4 bytes)
        response.extend_from_slice(&16777216u32.to_le_bytes());
        
        // Character set (1 byte) - utf8
        response.push(33);
        
        // Reserved (23 bytes of nulls)
        response.extend_from_slice(&[0u8; 23]);
        
        // Username (null-terminated)
        response.extend_from_slice(self.db_config.user.as_bytes());
        response.push(0);
        
        // Auth response (length-prefixed)
        // For native password: SHA1(password) XOR SHA1(auth_data + SHA1(SHA1(password)))
        // Simplified: just send empty for now if no password
        if self.db_config.password.is_empty() {
            response.push(0);
        } else {
            // Simple password auth - this is a placeholder
            // A full implementation would compute the auth hash
            let scramble = self.scramble_password(&self.db_config.password, auth_data_1);
            response.push(scramble.len() as u8);
            response.extend_from_slice(&scramble);
        }
        
        // Build packet
        let mut packet = Vec::new();
        let len = response.len() as u32;
        packet.push((len & 0xFF) as u8);
        packet.push(((len >> 8) & 0xFF) as u8);
        packet.push(((len >> 16) & 0xFF) as u8);
        packet.push(1); // Sequence number
        packet.extend(response);
        
        stream.write_all(&packet).await?;
        
        // Read response
        let mut buf = [0u8; 1024];
        let n = stream.read(&mut buf).await?;
        
        if n > 4 && buf[4] == 0xFF {
            // Error
            let error_msg = String::from_utf8_lossy(&buf[13..n]);
            return Err(crate::Error::Network(format!("Auth failed: {}", error_msg)));
        }
        
        tracing::debug!("Authentication successful");
        Ok(())
    }
    
    fn scramble_password(&self, password: &str, auth_data: &[u8]) -> Vec<u8> {
        use sha1::{Sha1, Digest};
        
        // MySQL native password authentication
        // SHA1(password) XOR SHA1(auth_data + SHA1(SHA1(password)))
        
        let mut hasher = Sha1::new();
        hasher.update(password.as_bytes());
        let stage1 = hasher.finalize_reset();
        
        hasher.update(&stage1);
        let stage2 = hasher.finalize_reset();
        
        hasher.update(auth_data);
        hasher.update(&stage2);
        let stage3 = hasher.finalize();
        
        stage1.iter().zip(stage3.iter()).map(|(a, b)| a ^ b).collect()
    }
    
    async fn get_binlog_position(&self, stream: &mut TcpStream) -> Result<(String, u64)> {
        // Check config first - if user specified position, use it
        if let (Some(file), Some(pos)) = (&self.binlog_config.start_file, self.binlog_config.start_position) {
            tracing::info!("Using configured binlog position: {}:{}", file, pos);
            return Ok((file.clone(), pos));
        }
        
        // Query SHOW MASTER STATUS to get current binlog position
        tracing::debug!("Querying SHOW MASTER STATUS to detect binlog position");
        self.send_query(stream, "SHOW MASTER STATUS").await?;
        
        // Read the complete response (may come in multiple packets)
        let mut response_data = Vec::new();
        let mut buf = vec![0u8; 65536];
        
        // Read packets until we get EOF or error
        loop {
            let n = stream.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            response_data.extend_from_slice(&buf[..n]);
            
            // Check if we've received the EOF packet (less data than buffer = done)
            if n < buf.len() {
                break;
            }
        }
        
        tracing::debug!("Received {} bytes from SHOW MASTER STATUS", response_data.len());
        
        // Parse the MySQL result set to extract file and position
        // Result format: column_count packet, column definitions, EOF, row data, EOF
        if let Some((file, pos)) = self.parse_master_status_result(&response_data) {
            tracing::info!("Detected binlog position from SHOW MASTER STATUS: {}:{}", file, pos);
            return Ok((file, pos));
        }
        
        // Fallback if parsing fails - use config values or defaults
        let file = self.binlog_config.start_file.clone()
            .unwrap_or_else(|| "mysql-bin.000001".to_string());
        let pos = self.binlog_config.start_position.unwrap_or(4);
        
        tracing::warn!("Could not parse SHOW MASTER STATUS, using defaults: {}:{}", file, pos);
        Ok((file, pos))
    }
    
    /// Parse SHOW MASTER STATUS result set to extract file and position
    fn parse_master_status_result(&self, data: &[u8]) -> Option<(String, u64)> {
        // MySQL protocol result set format:
        // 1. Column count packet (4-byte header + length-encoded column count)
        // 2. Column definition packets (one per column)
        // 3. EOF packet
        // 4. Row data packet(s) (length-encoded strings for each column)
        // 5. EOF packet
        //
        // SHOW MASTER STATUS returns: File, Position, Binlog_Do_DB, Binlog_Ignore_DB, ...
        // We need first two columns: File (string) and Position (number as string)
        
        if data.len() < 10 {
            return None;
        }
        
        // Skip packets until we find the row data
        // We're looking for the pattern: column_count, N column defs, EOF, ROW DATA, EOF
        // The row data packet starts with 0x00 (OK) but column packets don't
        
        // Find text data that looks like a binlog filename
        // Search for "mysql-bin" or similar patterns followed by position
        let _data_str = String::from_utf8_lossy(data);
        
        // Try to find binlog file pattern (usually mysql-bin.XXXXXX or similar)
        // Look for .000 pattern which is common in binlog filenames
        for i in 0..data.len().saturating_sub(20) {
            // Look for a length byte followed by text that contains "bin" and ".0"
            if data[i] > 5 && data[i] < 100 {
                let len = data[i] as usize;
                if i + 1 + len <= data.len() {
                    let potential_file = &data[i + 1..i + 1 + len];
                    if let Ok(file_str) = std::str::from_utf8(potential_file) {
                        // Check if this looks like a binlog filename
                        if (file_str.contains("-bin") || file_str.contains("binlog")) 
                            && file_str.contains(".0") {
                            // Next field should be the position
                            let pos_start = i + 1 + len;
                            if pos_start < data.len() {
                                let pos_len = data[pos_start] as usize;
                                if pos_start + 1 + pos_len <= data.len() {
                                    let pos_bytes = &data[pos_start + 1..pos_start + 1 + pos_len];
                                    if let Ok(pos_str) = std::str::from_utf8(pos_bytes) {
                                        if let Ok(position) = pos_str.parse::<u64>() {
                                            return Some((file_str.to_string(), position));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        
        // Alternative: parse more carefully by skipping packets
        let mut offset = 0;
        while offset + 4 < data.len() {
            // Read packet header
            let pkt_len = u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], 0]) as usize;
            let _seq = data[offset + 3];
            offset += 4;
            
            if pkt_len == 0 || offset + pkt_len > data.len() {
                break;
            }
            
            let pkt_data = &data[offset..offset + pkt_len];
            
            // Check for row data (not column definition, not EOF)
            // Row data starts with length-encoded strings
            if pkt_data.len() > 2 && pkt_data[0] != 0xFE && pkt_data[0] != 0xFF && pkt_data[0] != 0x00 {
                // This might be a column definition or row - check for binlog pattern
                if let Some((file, pos)) = self.try_parse_row(pkt_data) {
                    return Some((file, pos));
                }
            }
            
            offset += pkt_len;
        }
        
        None
    }
    
    /// Try to parse a row packet from SHOW MASTER STATUS
    fn try_parse_row(&self, data: &[u8]) -> Option<(String, u64)> {
        if data.is_empty() {
            return None;
        }
        
        // First column: File (length-encoded string)
        let (file_str, consumed) = self.read_length_encoded_string(data)?;
        
        // Check if it looks like a binlog file
        if !file_str.contains("-bin") && !file_str.contains("binlog") {
            return None;
        }
        
        // Second column: Position (length-encoded string containing a number)
        let remaining = &data[consumed..];
        let (pos_str, _) = self.read_length_encoded_string(remaining)?;
        
        let position = pos_str.parse::<u64>().ok()?;
        
        Some((file_str, position))
    }
    
    /// Read a length-encoded string from MySQL protocol data
    fn read_length_encoded_string(&self, data: &[u8]) -> Option<(String, usize)> {
        if data.is_empty() {
            return None;
        }
        
        let first_byte = data[0];
        
        // Length encoding:
        // 0-250: 1-byte length
        // 251: NULL
        // 252: 2-byte length follows
        // 253: 3-byte length follows
        // 254: 8-byte length follows
        
        let (len, header_size) = if first_byte <= 250 {
            (first_byte as usize, 1)
        } else if first_byte == 252 && data.len() >= 3 {
            let len = u16::from_le_bytes([data[1], data[2]]) as usize;
            (len, 3)
        } else if first_byte == 253 && data.len() >= 4 {
            let len = u32::from_le_bytes([data[1], data[2], data[3], 0]) as usize;
            (len, 4)
        } else if first_byte == 254 && data.len() >= 9 {
            let len = u64::from_le_bytes([
                data[1], data[2], data[3], data[4],
                data[5], data[6], data[7], data[8]
            ]) as usize;
            (len, 9)
        } else {
            return None;
        };
        
        if data.len() < header_size + len {
            return None;
        }
        
        let string_data = &data[header_size..header_size + len];
        let string = String::from_utf8_lossy(string_data).to_string();
        
        Some((string, header_size + len))
    }
    
    async fn send_query(&self, stream: &mut TcpStream, query: &str) -> Result<()> {
        let mut packet = Vec::new();
        
        // Packet length
        let len = (query.len() + 1) as u32;
        packet.push((len & 0xFF) as u8);
        packet.push(((len >> 8) & 0xFF) as u8);
        packet.push(((len >> 16) & 0xFF) as u8);
        packet.push(0); // Sequence number
        
        // COM_QUERY
        packet.push(0x03);
        packet.extend(query.as_bytes());
        
        stream.write_all(&packet).await?;
        Ok(())
    }
    
    async fn register_slave(&self, stream: &mut TcpStream) -> Result<()> {
        // COM_REGISTER_SLAVE
        let server_id = self.binlog_config.server_id;
        
        let mut payload = Vec::new();
        payload.push(0x15); // COM_REGISTER_SLAVE
        payload.extend_from_slice(&server_id.to_le_bytes());
        
        // Empty hostname, user, password, port
        payload.push(0); // hostname length
        payload.push(0); // user length
        payload.push(0); // password length
        payload.extend_from_slice(&0u16.to_le_bytes()); // port
        payload.extend_from_slice(&0u32.to_le_bytes()); // replication rank
        payload.extend_from_slice(&0u32.to_le_bytes()); // master_id
        
        let mut packet = Vec::new();
        let len = payload.len() as u32;
        packet.push((len & 0xFF) as u8);
        packet.push(((len >> 8) & 0xFF) as u8);
        packet.push(((len >> 16) & 0xFF) as u8);
        packet.push(0); // Sequence number
        packet.extend(payload);
        
        stream.write_all(&packet).await?;
        
        // Read response
        let mut buf = [0u8; 1024];
        let n = stream.read(&mut buf).await?;
        
        if n > 4 && buf[4] == 0xFF {
            let error_msg = String::from_utf8_lossy(&buf[13..n]);
            return Err(crate::Error::Network(format!("Register slave failed: {}", error_msg)));
        }
        
        tracing::debug!("Registered as slave with server_id {}", server_id);
        Ok(())
    }
    
    async fn send_binlog_dump(&self, stream: &mut TcpStream, file: &str, position: u64) -> Result<()> {
        // COM_BINLOG_DUMP
        let server_id = self.binlog_config.server_id;
        
        let mut payload = Vec::new();
        payload.push(0x12); // COM_BINLOG_DUMP
        payload.extend_from_slice(&(position as u32).to_le_bytes()); // binlog position
        payload.extend_from_slice(&0u16.to_le_bytes()); // flags
        payload.extend_from_slice(&server_id.to_le_bytes()); // server_id
        payload.extend(file.as_bytes()); // binlog file
        
        let mut packet = Vec::new();
        let len = payload.len() as u32;
        packet.push((len & 0xFF) as u8);
        packet.push(((len >> 8) & 0xFF) as u8);
        packet.push(((len >> 16) & 0xFF) as u8);
        packet.push(0); // Sequence number
        packet.extend(payload);
        
        stream.write_all(&packet).await?;
        
        tracing::debug!("Sent COM_BINLOG_DUMP for {}:{}", file, position);
        Ok(())
    }
    
    async fn handle_event(
        &self,
        event: BinlogEvent,
        table_map: &mut TableMap,
        current_file: &mut String,
    ) -> Result<()> {
        match &event {
            BinlogEvent::TableMap { table_id, database, table, column_count } => {
                table_map.insert(*table_id, database.clone(), table.clone(), *column_count);
                tracing::debug!("TableMap: {} -> {}.{}", table_id, database, table);
            }
            
            BinlogEvent::Rotate { next_file, position } => {
                tracing::info!("Binlog rotate to {}:{}", next_file, position);
                *current_file = next_file.clone();
            }
            
            BinlogEvent::FormatDescription { binlog_version, server_version } => {
                tracing::info!("Binlog format: version {} from {}", binlog_version, server_version);
            }
            
            BinlogEvent::Query { database, query } => {
                if should_replicate_query(query) {
                    tracing::debug!("Replicating query in [{}]: {}", database, 
                        query.chars().take(100).collect::<String>());
                }
            }
            
            BinlogEvent::Xid { xid } => {
                tracing::trace!("Transaction committed: XID {}", xid);
            }
            
            _ => {}
        }
        
        // Convert to WAL entry
        if let Some(entry) = binlog_to_wal(event, table_map) {
            match self.wal_writer.append(entry).await {
                Ok(lsn) => {
                    tracing::debug!("Wrote binlog event to WAL with LSN {}", lsn);
                }
                Err(e) => {
                    tracing::error!("Failed to write to WAL: {}", e);
                }
            }
        }
        
        Ok(())
    }
}
