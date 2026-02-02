//! MySQL Proxy Server
//!
//! TCP server that accepts MySQL client connections and proxies queries.

use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::state::ClusterMembership;
use crate::error::Result;
use super::protocol::{
    MySqlPacket, PacketType, build_handshake_packet, build_ok_packet, build_error_packet,
};
use super::handler::QueryHandler;

/// MySQL proxy server configuration
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Address to listen on
    pub listen_address: String,
    /// Backend MariaDB host
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
            let (socket, addr) = listener.accept().await?;
            tracing::info!("New MySQL client connection from {}", addr);

            let handler = QueryHandler::new(
                Arc::clone(&self.cluster),
                self.config.backend_host.clone(),
                self.config.backend_port,
                self.config.backend_user.clone(),
                self.config.backend_password.clone(),
            );

            tokio::spawn(async move {
                if let Err(e) = handle_client(socket, handler).await {
                    tracing::error!("Client handler error: {}", e);
                }
            });
        }
    }
}

/// Handle a single client connection
async fn handle_client(mut socket: TcpStream, handler: QueryHandler) -> Result<()> {
    // Send handshake
    let handshake = build_handshake_packet("5.7.0-WolfScale-Proxy");
    let mut buf = Vec::new();
    handshake.write(&mut buf);
    socket.write_all(&buf).await?;

    // Read handshake response
    let mut response_buf = vec![0u8; 4096];
    let n = socket.read(&mut response_buf).await?;
    if n == 0 {
        return Ok(());
    }

    // Parse handshake response (we just accept it for now)
    tracing::debug!("Received handshake response ({} bytes)", n);

    // Send OK to complete authentication
    let ok = build_ok_packet(2, 0, 0);
    buf.clear();
    ok.write(&mut buf);
    socket.write_all(&buf).await?;

    // Main command loop
    loop {
        let mut cmd_buf = vec![0u8; 65536];
        let n = socket.read(&mut cmd_buf).await?;
        if n == 0 {
            tracing::debug!("Client disconnected");
            break;
        }

        // Parse packet
        let (packet, _) = match MySqlPacket::read(&cmd_buf[..n]) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to parse packet: {}", e);
                continue;
            }
        };

        match packet.command() {
            Some(PacketType::Quit) => {
                tracing::debug!("Client sent QUIT");
                break;
            }
            Some(PacketType::Query) => {
                if let Err(e) = handler.handle_query(&packet, &mut socket).await {
                    tracing::error!("Query handler error: {}", e);
                    let error = build_error_packet(
                        packet.header.sequence_id + 1,
                        1105,
                        "HY000",
                        &format!("Internal error: {}", e),
                    );
                    buf.clear();
                    error.write(&mut buf);
                    socket.write_all(&buf).await?;
                }
            }
            Some(PacketType::Ping) => {
                let ok = build_ok_packet(packet.header.sequence_id + 1, 0, 0);
                buf.clear();
                ok.write(&mut buf);
                socket.write_all(&buf).await?;
            }
            Some(PacketType::Unknown(cmd)) => {
                tracing::debug!("Unknown command: 0x{:02x}", cmd);
                let error = build_error_packet(
                    packet.header.sequence_id + 1,
                    1047,
                    "HY000",
                    "Unknown command",
                );
                buf.clear();
                error.write(&mut buf);
                socket.write_all(&buf).await?;
            }
            _ => {}
        }
    }

    Ok(())
}
