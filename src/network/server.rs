//! Network Server
//!
//! TCP server for accepting connections from other nodes.

use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use super::{read_message, write_message};
use crate::replication::Message;
use crate::error::{Error, Result};

/// Message handler callback type
pub type MessageHandler = Arc<dyn Fn(String, Message) -> Option<Message> + Send + Sync>;

/// Network server for cluster communication
pub struct NetworkServer {
    /// Bind address
    bind_address: String,
    /// Message handler
    handler: Option<MessageHandler>,
    /// Channel for incoming messages
    incoming_tx: mpsc::Sender<(String, Message)>,
    /// Shutdown signal
    shutdown: tokio::sync::watch::Sender<bool>,
}

impl NetworkServer {
    /// Create a new network server
    pub fn new(
        bind_address: String,
        incoming_tx: mpsc::Sender<(String, Message)>,
    ) -> Self {
        let (shutdown_tx, _) = tokio::sync::watch::channel(false);
        
        Self {
            bind_address,
            handler: None,
            incoming_tx,
            shutdown: shutdown_tx,
        }
    }

    /// Set the message handler
    pub fn set_handler(&mut self, handler: MessageHandler) {
        self.handler = Some(handler);
    }

    /// Start the server
    pub async fn start(&self) -> Result<()> {
        let listener = TcpListener::bind(&self.bind_address).await?;
        tracing::info!("Network server listening on {}", self.bind_address);

        let mut shutdown_rx = self.shutdown.subscribe();
        
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((socket, addr)) => {
                            let peer_addr = addr.to_string();
                            let incoming_tx = self.incoming_tx.clone();
                            let handler = self.handler.clone();
                            
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(socket, peer_addr.clone(), incoming_tx, handler).await {
                                    tracing::warn!("Connection error from {}: {}", peer_addr, e);
                                }
                            });
                        }
                        Err(e) => {
                            tracing::error!("Accept error: {}", e);
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
            }
        }

        tracing::info!("Network server stopped");
        Ok(())
    }

    /// Stop the server
    pub fn stop(&self) {
        let _ = self.shutdown.send(true);
    }
}

/// Handle a single connection
async fn handle_connection(
    socket: TcpStream,
    peer_addr: String,
    incoming_tx: mpsc::Sender<(String, Message)>,
    handler: Option<MessageHandler>,
) -> Result<()> {
    let (mut reader, mut writer) = socket.into_split();

    loop {
        match read_message(&mut reader).await {
            Ok(message) => {
                tracing::trace!("Received {} from {}", message.type_name(), peer_addr);

                // Try to get immediate response from handler
                if let Some(ref handler) = handler {
                    if let Some(response) = handler(peer_addr.clone(), message.clone()) {
                        write_message(&mut writer, &response).await?;
                    }
                }

                // Forward to channel for async processing
                if let Err(_) = incoming_tx.send((peer_addr.clone(), message)).await {
                    break;
                }
            }
            Err(Error::Io(ref e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // Connection closed
                break;
            }
            Err(e) => {
                tracing::warn!("Error reading message: {}", e);
                break;
            }
        }
    }

    Ok(())
}

/// Simple connection handler for request-response pattern
#[allow(dead_code)]
pub struct ConnectionHandler {
    socket: TcpStream,
}

#[allow(dead_code)]
impl ConnectionHandler {
    /// Wrap a socket
    pub fn new(socket: TcpStream) -> Self {
        Self { socket }
    }

    /// Send a message and wait for response
    pub async fn request(&mut self, message: Message) -> Result<Message> {
        let (mut reader, mut writer) = self.socket.split();
        
        write_message(&mut writer, &message).await?;
        read_message(&mut reader).await
    }

    /// Close the connection
    pub async fn close(self) -> Result<()> {
        drop(self.socket);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_server_bind() {
        let (tx, _rx) = mpsc::channel(100);
        let server = NetworkServer::new("127.0.0.1:0".to_string(), tx);
        
        // Just verify we can create a server
        assert!(!server.bind_address.is_empty());
    }
}
