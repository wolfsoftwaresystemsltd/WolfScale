//! Network Module
//!
//! Handles TCP communication between nodes.

mod server;
mod client;

pub use server::NetworkServer;
pub use client::NetworkClient;

use crate::replication::{Message, FrameHeader};
use crate::error::{Error, Result};

/// Read a framed message from a reader
pub async fn read_message<R: tokio::io::AsyncRead + Unpin>(reader: &mut R) -> Result<Message> {
    use tokio::io::AsyncReadExt;

    // Read header
    let mut header_bytes = [0u8; FrameHeader::SIZE];
    reader.read_exact(&mut header_bytes).await?;
    let header = FrameHeader::from_bytes(&header_bytes);

    // Read body
    let mut body = vec![0u8; header.length as usize];
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
