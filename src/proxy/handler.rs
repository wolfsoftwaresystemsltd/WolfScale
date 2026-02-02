//! Query Handler
//!
//! Routes queries to the appropriate backend - reads to local, writes to leader.

use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::state::ClusterMembership;
use crate::error::Result;
use super::protocol::{MySqlPacket, build_ok_packet, build_error_packet};

/// Query handler that routes queries appropriately
pub struct QueryHandler {
    /// Cluster membership for finding leader
    cluster: Arc<ClusterMembership>,
    /// Backend MariaDB connection info
    backend_host: String,
    backend_port: u16,
    backend_user: String,
    backend_password: String,
}

impl QueryHandler {
    pub fn new(
        cluster: Arc<ClusterMembership>,
        backend_host: String,
        backend_port: u16,
        backend_user: String,
        backend_password: String,
    ) -> Self {
        Self {
            cluster,
            backend_host,
            backend_port,
            backend_user,
            backend_password,
        }
    }

    /// Handle a query packet
    pub async fn handle_query(
        &self,
        packet: &MySqlPacket,
        client_stream: &mut TcpStream,
    ) -> Result<()> {
        let query = match packet.query_string() {
            Some(q) => q,
            None => {
                let error = build_error_packet(
                    packet.header.sequence_id + 1,
                    1064,
                    "42000",
                    "Invalid query packet",
                );
                let mut buf = Vec::new();
                error.write(&mut buf);
                client_stream.write_all(&buf).await?;
                return Ok(());
            }
        };

        tracing::debug!("Query: {}", query);

        // Check if this is a write query
        if packet.is_write_query() {
            self.handle_write_query(&query, packet.header.sequence_id, client_stream).await
        } else {
            self.handle_read_query(&query, packet.header.sequence_id, client_stream).await
        }
    }

    /// Handle a write query - forward to leader's backend
    async fn handle_write_query(
        &self,
        query: &str,
        sequence_id: u8,
        client_stream: &mut TcpStream,
    ) -> Result<()> {
        // Get the leader
        let leader = self.cluster.current_leader().await;
        
        let backend_host = if let Some(l) = leader {
            // Extract host from leader address
            l.address.split(':').next().unwrap_or(&self.backend_host).to_string()
        } else {
            self.backend_host.clone()
        };

        tracing::info!("Forwarding write to backend at {}", backend_host);

        // Connect to backend and execute
        match self.execute_on_backend(&backend_host, query).await {
            Ok((affected_rows, last_insert_id)) => {
                let ok = build_ok_packet(sequence_id + 1, affected_rows, last_insert_id);
                let mut buf = Vec::new();
                ok.write(&mut buf);
                client_stream.write_all(&buf).await?;
            }
            Err(e) => {
                let error = build_error_packet(
                    sequence_id + 1,
                    1045,
                    "HY000",
                    &format!("{}", e),
                );
                let mut buf = Vec::new();
                error.write(&mut buf);
                client_stream.write_all(&buf).await?;
            }
        }

        Ok(())
    }

    /// Handle a read query - execute on local backend
    async fn handle_read_query(
        &self,
        query: &str,
        sequence_id: u8,
        client_stream: &mut TcpStream,
    ) -> Result<()> {
        // For now, forward to local backend
        // In a full implementation, this would proxy the result set
        match self.execute_on_backend(&self.backend_host, query).await {
            Ok((affected_rows, last_insert_id)) => {
                let ok = build_ok_packet(sequence_id + 1, affected_rows, last_insert_id);
                let mut buf = Vec::new();
                ok.write(&mut buf);
                client_stream.write_all(&buf).await?;
            }
            Err(e) => {
                let error = build_error_packet(
                    sequence_id + 1,
                    1045,
                    "HY000",
                    &format!("{}", e),
                );
                let mut buf = Vec::new();
                error.write(&mut buf);
                client_stream.write_all(&buf).await?;
            }
        }

        Ok(())
    }

    /// Execute a query on a backend server
    async fn execute_on_backend(
        &self,
        host: &str,
        query: &str,
    ) -> Result<(u64, u64)> {
        // Use sqlx to execute on the backend
        let url = format!(
            "mysql://{}:{}@{}:{}/",
            self.backend_user,
            self.backend_password,
            host,
            self.backend_port
        );

        let pool = sqlx::mysql::MySqlPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .map_err(crate::Error::Database)?;

        let result = sqlx::query(query)
            .execute(&pool)
            .await
            .map_err(crate::Error::Database)?;

        Ok((result.rows_affected(), 0)) // TODO: get last_insert_id
    }
}
