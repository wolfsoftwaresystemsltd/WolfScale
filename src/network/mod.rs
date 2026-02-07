//! Network Module
//!
//! Handles TCP communication between nodes and UDP discovery.

mod server;
mod client;
pub mod discovery;

pub use server::NetworkServer;
pub use client::NetworkClient;
pub use discovery::Discovery;

use crate::replication::{Message, FrameHeader};
use crate::error::{Error, Result};

/// Maximum allowed message size (1 GB) - prevents memory exhaustion from malformed messages
const MAX_MESSAGE_SIZE: usize = 1024 * 1024 * 1024;

/// Read a framed message from a reader
pub async fn read_message<R: tokio::io::AsyncRead + Unpin>(reader: &mut R) -> Result<Message> {
    use tokio::io::AsyncReadExt;

    // Read header
    let mut header_bytes = [0u8; FrameHeader::SIZE];
    reader.read_exact(&mut header_bytes).await?;
    let header = FrameHeader::from_bytes(&header_bytes);

    // Safety check for message size - prevent memory exhaustion
    let msg_len = header.length as usize;
    if msg_len > MAX_MESSAGE_SIZE {
        return Err(Error::Network(format!(
            "Message too large: {} bytes (max {} bytes)", 
            msg_len, MAX_MESSAGE_SIZE
        )));
    }
    
    // Log large messages for debugging
    if msg_len > 10 * 1024 * 1024 {
        tracing::warn!("Receiving large message: {} MB", msg_len / (1024 * 1024));
    } else if msg_len > 1024 * 1024 {
        tracing::debug!("Receiving large message: {} KB", msg_len / 1024);
    }

    // Read body
    let mut body = vec![0u8; msg_len];
    reader.read_exact(&mut body).await?;

    // Verify checksum
    let computed_checksum = crc32fast::hash(&body);
    if computed_checksum != header.checksum {
        return Err(Error::Network("Message checksum mismatch".into()));
    }

    // Deserialize
    let message = Message::deserialize(&body)?;
    Ok(message)
}

/// Write a framed message to a writer
pub async fn write_message<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    message: &Message,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    let body = message.serialize()?;
    let header = FrameHeader::new(&body);

    writer.write_all(&header.to_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await?;

    Ok(())
}
