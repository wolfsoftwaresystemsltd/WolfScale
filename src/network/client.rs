//! Network Client
//!
//! TCP client for connecting to other nodes.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, RwLock};
use tokio::time::timeout;

use super::{read_message, write_message};
use crate::replication::Message;
use crate::error::{Error, Result};

/// Connection pool entry
struct PoolEntry {
    stream: TcpStream,
    last_used: std::time::Instant,
}

/// Network client for connecting to peer nodes
pub struct NetworkClient {
    /// Connection pool: address -> connection
    pool: Arc<RwLock<HashMap<String, Arc<Mutex<PoolEntry>>>>>,
    /// Connection timeout
    connect_timeout: Duration,
    /// Request timeout
    request_timeout: Duration,
    /// Max pool size per peer
    #[allow(dead_code)]
    max_connections: usize,
}

impl NetworkClient {
    /// Create a new network client
    pub fn new(connect_timeout: Duration, request_timeout: Duration) -> Self {
        Self {
            pool: Arc::new(RwLock::new(HashMap::new())),
            connect_timeout,
            request_timeout,
            max_connections: 10,
        }
    }

    /// Send a message to a peer and wait for response
    pub async fn send(&self, address: &str, message: Message) -> Result<Message> {
        let result = timeout(
            self.request_timeout,
            self.send_inner(address, message),
        ).await;

        match result {
            Ok(inner_result) => inner_result,
            Err(_) => Err(Error::ConnectionTimeout(address.to_string())),
        }
    }

    /// Send without timeout wrapper
    async fn send_inner(&self, address: &str, message: Message) -> Result<Message> {
        // Try to get existing connection
        if let Some(entry) = self.get_connection(address).await {
            let mut entry = entry.lock().await;
            
            // Split for read/write
            let (mut reader, mut writer) = entry.stream.split();
            
            if write_message(&mut writer, &message).await.is_err() {
                // Connection is dead, remove and reconnect
                drop(entry);
                self.remove_connection(address).await;
            } else {
                // Read response
                match read_message(&mut reader).await {
                    Ok(response) => {
                        entry.last_used = std::time::Instant::now();
                        return Ok(response);
                    }
                    Err(_) => {
                        drop(entry);
                        self.remove_connection(address).await;
                    }
                }
            }
        }

        // Create new connection
        let stream = self.connect(address).await?;
        let (mut reader, mut writer) = stream.into_split();

        write_message(&mut writer, &message).await?;
        let response = read_message(&mut reader).await?;

        // Note: We don't store the split connection back to pool for simplicity
        // In production, you'd want proper connection management

        Ok(response)
    }

    /// Send without waiting for response
    pub async fn send_async(&self, address: &str, message: Message) -> Result<()> {
        let stream = self.connect(address).await?;
        let (_, mut writer) = stream.into_split();
        write_message(&mut writer, &message).await?;
        Ok(())
    }

    /// Connect to an address
    async fn connect(&self, address: &str) -> Result<TcpStream> {
        let result = timeout(
            self.connect_timeout,
            TcpStream::connect(address),
        ).await;

        match result {
            Ok(Ok(stream)) => {
                stream.set_nodelay(true)?;
                Ok(stream)
            }
            Ok(Err(e)) => Err(Error::ConnectionFailed {
                address: address.to_string(),
                reason: e.to_string(),
            }),
            Err(_) => Err(Error::ConnectionTimeout(address.to_string())),
        }
    }

    /// Get a connection from the pool
    async fn get_connection(&self, address: &str) -> Option<Arc<Mutex<PoolEntry>>> {
        let pool = self.pool.read().await;
        pool.get(address).cloned()
    }

    /// Store a connection in the pool
    #[allow(dead_code)]
    async fn store_connection(&self, address: String, stream: TcpStream) -> Result<()> {
        let mut pool = self.pool.write().await;
        
        pool.insert(address, Arc::new(Mutex::new(PoolEntry {
            stream,
            last_used: std::time::Instant::now(),
        })));

        Ok(())
    }

    /// Remove a connection from the pool
    async fn remove_connection(&self, address: &str) {
        let mut pool = self.pool.write().await;
        pool.remove(address);
    }

    /// Clean up stale connections
    pub async fn cleanup_stale(&self, max_idle: Duration) {
        let mut pool = self.pool.write().await;
        let now = std::time::Instant::now();

        pool.retain(|addr, entry| {
            if let Ok(e) = entry.try_lock() {
                if now.duration_since(e.last_used) > max_idle {
                    tracing::debug!("Removing stale connection to {}", addr);
                    return false;
                }
            }
            true
        });
    }

    /// Close all connections
    pub async fn close_all(&self) {
        let mut pool = self.pool.write().await;
        pool.clear();
    }

    /// Get connection count
    pub async fn connection_count(&self) -> usize {
        self.pool.read().await.len()
    }
}

/// Simple one-shot client for single request-response
#[allow(dead_code)]
pub async fn send_once(
    address: &str,
    message: Message,
    timeout_duration: Duration,
) -> Result<Message> {
    let result = timeout(timeout_duration, async {
        let mut stream = TcpStream::connect(address).await?;
        stream.set_nodelay(true)?;

        let (mut reader, mut writer) = stream.split();
        write_message(&mut writer, &message).await?;
        read_message(&mut reader).await
    }).await;

    match result {
        Ok(inner) => inner,
        Err(_) => Err(Error::ConnectionTimeout(address.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_client_creation() {
        let client = NetworkClient::new(
            Duration::from_secs(5),
            Duration::from_secs(10),
        );

        assert_eq!(client.connection_count().await, 0);
    }

    #[tokio::test]
    async fn test_connection_failure() {
        let client = NetworkClient::new(
            Duration::from_millis(100),
            Duration::from_millis(500),
        );

        // Should fail to connect to non-existent server
        let result = client.send("127.0.0.1:99999", Message::StatusRequest).await;
        assert!(result.is_err());
    }
}
