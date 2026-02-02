//! MySQL Wire Protocol Implementation
//!
//! Handles parsing and building MySQL protocol packets.

use bytes::{Buf, BufMut, BytesMut};
use std::io;

/// MySQL packet types
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PacketType {
    /// COM_QUIT (0x01)
    Quit,
    /// COM_QUERY (0x03)
    Query,
    /// COM_PING (0x0e)
    Ping,
    /// Initial handshake from server
    Handshake,
    /// Handshake response from client
    HandshakeResponse,
    /// OK packet
    Ok,
    /// Error packet
    Error,
    /// EOF packet
    Eof,
    /// Unknown command
    Unknown(u8),
}

impl From<u8> for PacketType {
    fn from(cmd: u8) -> Self {
        match cmd {
            0x01 => PacketType::Quit,
            0x03 => PacketType::Query,
            0x0e => PacketType::Ping,
            _ => PacketType::Unknown(cmd),
        }
    }
}

/// MySQL packet header (4 bytes)
#[derive(Debug, Clone)]
pub struct PacketHeader {
    /// Payload length (3 bytes)
    pub length: u32,
    /// Sequence ID (1 byte)
    pub sequence_id: u8,
}

impl PacketHeader {
    pub fn read(data: &[u8]) -> io::Result<Self> {
        if data.len() < 4 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Not enough data for header"));
        }
        
        let length = (data[0] as u32) | ((data[1] as u32) << 8) | ((data[2] as u32) << 16);
        let sequence_id = data[3];
        
        Ok(Self { length, sequence_id })
    }

    pub fn write(&self, buf: &mut Vec<u8>) {
        buf.push((self.length & 0xff) as u8);
        buf.push(((self.length >> 8) & 0xff) as u8);
        buf.push(((self.length >> 16) & 0xff) as u8);
        buf.push(self.sequence_id);
    }
}

/// MySQL packet
#[derive(Debug, Clone)]
pub struct MySqlPacket {
    pub header: PacketHeader,
    pub payload: Vec<u8>,
}

impl MySqlPacket {
    /// Create a new packet
    pub fn new(sequence_id: u8, payload: Vec<u8>) -> Self {
        Self {
            header: PacketHeader {
                length: payload.len() as u32,
                sequence_id,
            },
            payload,
        }
    }

    /// Read a packet from a buffer
    pub fn read(data: &[u8]) -> io::Result<(Self, usize)> {
        let header = PacketHeader::read(data)?;
        let total_len = 4 + header.length as usize;
        
        if data.len() < total_len {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Not enough data for packet"));
        }
        
        let payload = data[4..total_len].to_vec();
        
        Ok((Self { header, payload }, total_len))
    }

    /// Write packet to buffer
    pub fn write(&self, buf: &mut Vec<u8>) {
        self.header.write(buf);
        buf.extend_from_slice(&self.payload);
    }

    /// Get command type (first byte of payload for command packets)
    pub fn command(&self) -> Option<PacketType> {
        self.payload.first().map(|&b| PacketType::from(b))
    }

    /// Get query string (for COM_QUERY packets)
    pub fn query_string(&self) -> Option<String> {
        if self.payload.len() > 1 && self.payload[0] == 0x03 {
            String::from_utf8(self.payload[1..].to_vec()).ok()
        } else {
            None
        }
    }

    /// Check if this is a write query
    pub fn is_write_query(&self) -> bool {
        if let Some(query) = self.query_string() {
            let upper = query.trim().to_uppercase();
            upper.starts_with("INSERT") ||
            upper.starts_with("UPDATE") ||
            upper.starts_with("DELETE") ||
            upper.starts_with("CREATE") ||
            upper.starts_with("ALTER") ||
            upper.starts_with("DROP") ||
            upper.starts_with("TRUNCATE") ||
            upper.starts_with("REPLACE")
        } else {
            false
        }
    }
}

/// Build an OK packet
pub fn build_ok_packet(sequence_id: u8, affected_rows: u64, last_insert_id: u64) -> MySqlPacket {
    let mut payload = Vec::new();
    payload.push(0x00); // OK header
    write_lenenc_int(&mut payload, affected_rows);
    write_lenenc_int(&mut payload, last_insert_id);
    payload.push(0x00); // status flags (2 bytes)
    payload.push(0x00);
    payload.push(0x00); // warnings (2 bytes)
    payload.push(0x00);
    
    MySqlPacket::new(sequence_id, payload)
}

/// Build an error packet
pub fn build_error_packet(sequence_id: u8, error_code: u16, sql_state: &str, message: &str) -> MySqlPacket {
    let mut payload = Vec::new();
    payload.push(0xff); // Error header
    payload.push((error_code & 0xff) as u8);
    payload.push(((error_code >> 8) & 0xff) as u8);
    payload.push(b'#'); // SQL state marker
    payload.extend_from_slice(sql_state.as_bytes());
    payload.extend_from_slice(message.as_bytes());
    
    MySqlPacket::new(sequence_id, payload)
}

/// Build initial handshake packet (server -> client)
pub fn build_handshake_packet(server_version: &str) -> MySqlPacket {
    let mut payload = Vec::new();
    
    // Protocol version
    payload.push(10);
    
    // Server version (null-terminated)
    payload.extend_from_slice(server_version.as_bytes());
    payload.push(0);
    
    // Connection ID (4 bytes)
    payload.extend_from_slice(&[1, 0, 0, 0]);
    
    // Auth plugin data part 1 (8 bytes)
    payload.extend_from_slice(b"12345678");
    
    // Filler
    payload.push(0);
    
    // Capability flags lower 2 bytes
    let capabilities: u32 = 0x0000_a20f; // Basic capabilities
    payload.push((capabilities & 0xff) as u8);
    payload.push(((capabilities >> 8) & 0xff) as u8);
    
    // Character set (utf8mb4)
    payload.push(45);
    
    // Status flags
    payload.push(0x02);
    payload.push(0x00);
    
    // Capability flags upper 2 bytes
    payload.push(((capabilities >> 16) & 0xff) as u8);
    payload.push(((capabilities >> 24) & 0xff) as u8);
    
    // Auth plugin data length (or 0)
    payload.push(21);
    
    // Reserved (10 bytes)
    payload.extend_from_slice(&[0u8; 10]);
    
    // Auth plugin data part 2 (12 bytes + null)
    payload.extend_from_slice(b"123456789012");
    payload.push(0);
    
    // Auth plugin name
    payload.extend_from_slice(b"mysql_native_password");
    payload.push(0);
    
    MySqlPacket::new(0, payload)
}

/// Write a length-encoded integer
fn write_lenenc_int(buf: &mut Vec<u8>, value: u64) {
    if value < 251 {
        buf.push(value as u8);
    } else if value < 65536 {
        buf.push(0xfc);
        buf.push((value & 0xff) as u8);
        buf.push(((value >> 8) & 0xff) as u8);
    } else if value < 16777216 {
        buf.push(0xfd);
        buf.push((value & 0xff) as u8);
        buf.push(((value >> 8) & 0xff) as u8);
        buf.push(((value >> 16) & 0xff) as u8);
    } else {
        buf.push(0xfe);
        for i in 0..8 {
            buf.push(((value >> (i * 8)) & 0xff) as u8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packet_header() {
        let data = [0x05, 0x00, 0x00, 0x01]; // length=5, seq=1
        let header = PacketHeader::read(&data).unwrap();
        assert_eq!(header.length, 5);
        assert_eq!(header.sequence_id, 1);
    }

    #[test]
    fn test_is_write_query() {
        let mut payload = vec![0x03]; // COM_QUERY
        payload.extend_from_slice(b"INSERT INTO test VALUES (1)");
        let packet = MySqlPacket::new(0, payload);
        assert!(packet.is_write_query());
        
        let mut payload = vec![0x03];
        payload.extend_from_slice(b"SELECT * FROM test");
        let packet = MySqlPacket::new(0, payload);
        assert!(!packet.is_write_query());
    }
}
