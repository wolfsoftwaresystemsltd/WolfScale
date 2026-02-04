//! HTTP API Server
//!
//! REST API for write operations, status queries, and cluster management.

use std::sync::Arc;
use axum::{
    extract::{Path, State, Json},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::config::ApiConfig;
use crate::wal::{LogEntry, Value, PrimaryKey};
use crate::state::{ClusterMembership, NodeState, ClusterSummary};
use crate::error::{Error, Result};

/// HTTP client for forwarding writes to leader
static HTTP_CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("Failed to create HTTP client")
});

/// Shared application state
pub struct AppState {
    /// Node ID
    pub node_id: String,
    /// Is this node the leader
    pub is_leader: RwLock<bool>,
    /// Cluster membership
    pub cluster: Arc<ClusterMembership>,
    /// Write handler
    pub write_handler: RwLock<Option<WriteHandler>>,
    /// Data directory for WAL and state
    pub data_dir: std::path::PathBuf,
}

/// Write handler callback
pub type WriteHandler = Arc<dyn Fn(LogEntry) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u64>> + Send>> + Send + Sync>;

/// HTTP API server
pub struct HttpServer {
    config: ApiConfig,
    state: Arc<AppState>,
}

impl HttpServer {
    /// Create a new HTTP server
    pub fn new(
        config: ApiConfig,
        node_id: String,
        cluster: Arc<ClusterMembership>,
        data_dir: std::path::PathBuf,
    ) -> Self {
        let state = Arc::new(AppState {
            node_id,
            is_leader: RwLock::new(false),
            cluster,
            write_handler: RwLock::new(None),
            data_dir,
        });

        Self { config, state }
    }

    /// Create a new HTTP server with write handler
    pub fn with_write_handler(
        config: ApiConfig,
        node_id: String,
        cluster: Arc<ClusterMembership>,
        write_handler: WriteHandler,
        data_dir: std::path::PathBuf,
    ) -> Self {
        let state = Arc::new(AppState {
            node_id,
            is_leader: RwLock::new(true), // If we have write handler, we're the leader
            cluster,
            write_handler: RwLock::new(Some(write_handler)),
            data_dir,
        });

        Self { config, state }
    }

    /// Set whether this node is the leader
    pub async fn set_leader(&self, is_leader: bool) {
        *self.state.is_leader.write().await = is_leader;
    }

    /// Set the write handler
    pub async fn set_write_handler(&self, handler: WriteHandler) {
        *self.state.write_handler.write().await = Some(handler);
    }

    /// Get the state for sharing with other components
    pub fn state(&self) -> Arc<AppState> {
        Arc::clone(&self.state)
    }

    /// Create the router
    fn create_router(state: Arc<AppState>) -> Router {
        Router::new()
            // Write operations
            .route("/write", post(handle_write))
            .route("/write/insert", post(handle_insert))
            .route("/write/update", post(handle_update))
            .route("/write/delete", post(handle_delete))
            .route("/write/upsert", post(handle_upsert))
            .route("/write/ddl", post(handle_ddl))
            // Status and info
            .route("/status", get(handle_status))
            .route("/stats", get(handle_stats))
            .route("/health", get(handle_health))
            .route("/cluster", get(handle_cluster_info))
            .route("/cluster/nodes", get(handle_nodes))
            .route("/cluster/nodes/:node_id", get(handle_node_info))
            // Admin operations
            .route("/admin/promote", post(handle_promote))
            .route("/admin/demote", post(handle_demote))
            .route("/admin/reset", post(handle_reset))
            // Migration operations
            .route("/dump/info", get(handle_dump_info))
            .route("/dump", get(handle_dump))
            .with_state(state)
    }

    /// Start the HTTP server
    pub async fn start(&self) -> Result<()> {
        if !self.config.enabled {
            tracing::info!("HTTP API disabled");
            return Ok(());
        }

        let app = Self::create_router(Arc::clone(&self.state));
        
        let listener = tokio::net::TcpListener::bind(&self.config.bind_address).await?;
        tracing::info!("HTTP API listening on {}", self.config.bind_address);

        axum::serve(listener, app)
            .await
            .map_err(|e| Error::Network(format!("HTTP server error: {}", e)))?;

        Ok(())
    }
}

// ============ Request/Response Types ============

/// Write request
#[derive(Debug, Deserialize, Serialize)]
#[allow(dead_code)]
pub struct WriteRequest {
    pub table: String,
    pub operation: String,
    pub data: serde_json::Value,
}

/// Insert request
#[derive(Debug, Deserialize, Serialize)]
pub struct InsertRequest {
    pub table: String,
    pub values: std::collections::HashMap<String, serde_json::Value>,
}

/// Update request
#[derive(Debug, Deserialize, Serialize)]
pub struct UpdateRequest {
    pub table: String,
    pub set: std::collections::HashMap<String, serde_json::Value>,
    pub where_key: std::collections::HashMap<String, serde_json::Value>,
}

/// Delete request
#[derive(Debug, Deserialize, Serialize)]
pub struct DeleteRequest {
    pub table: String,
    pub where_key: std::collections::HashMap<String, serde_json::Value>,
}

/// Upsert request
#[derive(Debug, Deserialize, Serialize)]
pub struct UpsertRequest {
    pub table: String,
    pub values: std::collections::HashMap<String, serde_json::Value>,
    pub update_columns: Vec<String>,
}

/// DDL request
#[derive(Debug, Deserialize, Serialize)]
pub struct DdlRequest {
    pub ddl: String,
    pub table: Option<String>,
}

/// Write response
#[derive(Debug, Serialize)]
pub struct WriteResponse {
    pub success: bool,
    pub lsn: Option<u64>,
    pub message: Option<String>,
}

/// Status response
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub node_id: String,
    pub is_leader: bool,
    pub leader_id: Option<String>,
    pub term: u64,
    pub last_applied_lsn: u64,
    pub commit_lsn: u64,
    pub cluster_size: usize,
    pub has_quorum: bool,
}

/// Health response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub healthy: bool,
    pub node_id: String,
    pub is_leader: bool,
}

/// Stats response for throughput monitoring
#[derive(Debug, Serialize)]
pub struct StatsResponse {
    pub node_id: String,
    pub role: String,
    pub current_lsn: u64,
    pub commit_lsn: u64,
    pub uptime_seconds: u64,
    pub cluster_size: usize,
    pub active_nodes: usize,
    pub followers: Vec<FollowerStats>,
}

/// Follower stats for replication lag tracking
#[derive(Debug, Serialize)]
pub struct FollowerStats {
    pub node_id: String,
    pub last_applied_lsn: u64,
    pub lag: u64,
    pub status: String,
}

/// Cluster info response
#[derive(Debug, Serialize)]
pub struct ClusterInfoResponse {
    pub summary: ClusterSummary,
    pub nodes: Vec<NodeState>,
}

/// Error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: String,
}

// ============ Handlers ============

async fn handle_write(
    State(state): State<Arc<AppState>>,
    Json(req): Json<WriteRequest>,
) -> impl IntoResponse {
    // Forward to leader if we're not the leader
    if !*state.is_leader.read().await {
        return match forward_to_leader(&state, "/write", &req).await {
            Ok(response) => response,
            Err(error_response) => error_response,
        };
    }

    // Parse and execute write
    // This is a simplified implementation
    Json(WriteResponse {
        success: true,
        lsn: Some(0),
        message: Some("Write accepted".to_string()),
    }).into_response()
}

async fn handle_insert(
    State(state): State<Arc<AppState>>,
    Json(req): Json<InsertRequest>,
) -> impl IntoResponse {
    // Forward to leader if we're not the leader
    if !*state.is_leader.read().await {
        return match forward_to_leader(&state, "/write/insert", &req).await {
            Ok(response) => response,
            Err(error_response) => error_response,
        };
    }

    // Convert request to log entry
    let columns: Vec<String> = req.values.keys().cloned().collect();
    let values: Vec<Value> = req.values.values().map(json_to_value).collect();
    
    // Get primary key from values (assuming 'id' column)
    let pk = req.values.get("id")
        .map(|v| json_to_primary_key(v))
        .unwrap_or(PrimaryKey::Int(0));

    let _entry = LogEntry::Insert {
        table: req.table,
        columns,
        values,
        primary_key: pk,
    };

    // In a real implementation, this would call the write handler
    Json(WriteResponse {
        success: true,
        lsn: Some(1),
        message: None,
    }).into_response()
}

async fn handle_update(
    State(state): State<Arc<AppState>>,
    Json(req): Json<UpdateRequest>,
) -> impl IntoResponse {
    // Forward to leader if we're not the leader
    if !*state.is_leader.read().await {
        return match forward_to_leader(&state, "/write/update", &req).await {
            Ok(response) => response,
            Err(error_response) => error_response,
        };
    }

    let set_columns: Vec<String> = req.set.keys().cloned().collect();
    let set_values: Vec<Value> = req.set.values().map(json_to_value).collect();
    let key_columns: Vec<String> = req.where_key.keys().cloned().collect();
    
    let pk = if key_columns.len() == 1 {
        json_to_primary_key(&req.where_key[&key_columns[0]])
    } else {
        PrimaryKey::Composite(req.where_key.values().map(json_to_value).collect())
    };

    let _entry = LogEntry::Update {
        table: req.table,
        set_columns,
        set_values,
        primary_key: pk,
        key_columns,
    };

    Json(WriteResponse {
        success: true,
        lsn: Some(1),
        message: None,
    }).into_response()
}

async fn handle_delete(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DeleteRequest>,
) -> impl IntoResponse {
    // Forward to leader if we're not the leader
    if !*state.is_leader.read().await {
        return match forward_to_leader(&state, "/write/delete", &req).await {
            Ok(response) => response,
            Err(error_response) => error_response,
        };
    }

    let key_columns: Vec<String> = req.where_key.keys().cloned().collect();
    let pk = if key_columns.len() == 1 {
        json_to_primary_key(&req.where_key[&key_columns[0]])
    } else {
        PrimaryKey::Composite(req.where_key.values().map(json_to_value).collect())
    };

    let _entry = LogEntry::Delete {
        table: req.table,
        primary_key: pk,
        key_columns,
    };

    Json(WriteResponse {
        success: true,
        lsn: Some(1),
        message: None,
    }).into_response()
}

async fn handle_upsert(
    State(state): State<Arc<AppState>>,
    Json(req): Json<UpsertRequest>,
) -> impl IntoResponse {
    // Forward to leader if we're not the leader
    if !*state.is_leader.read().await {
        return match forward_to_leader(&state, "/write/upsert", &req).await {
            Ok(response) => response,
            Err(error_response) => error_response,
        };
    }

    let columns: Vec<String> = req.values.keys().cloned().collect();
    let values: Vec<Value> = req.values.values().map(json_to_value).collect();
    
    let pk = req.values.get("id")
        .map(|v| json_to_primary_key(v))
        .unwrap_or(PrimaryKey::Int(0));

    let _entry = LogEntry::Upsert {
        table: req.table,
        columns,
        values,
        update_columns: req.update_columns,
        primary_key: pk,
    };

    Json(WriteResponse {
        success: true,
        lsn: Some(1),
        message: None,
    }).into_response()
}

async fn handle_ddl(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DdlRequest>,
) -> impl IntoResponse {
    // Forward to leader if we're not the leader
    if !*state.is_leader.read().await {
        return match forward_to_leader(&state, "/write/ddl", &req).await {
            Ok(response) => response,
            Err(error_response) => error_response,
        };
    }

    // Parse DDL to determine type
    let _entry = if req.ddl.to_uppercase().starts_with("ALTER") {
        LogEntry::AlterTable {
            table: req.table.unwrap_or_default(),
            ddl: req.ddl,
        }
    } else if req.ddl.to_uppercase().starts_with("CREATE TABLE") {
        LogEntry::CreateTable {
            table: req.table.unwrap_or_default(),
            ddl: req.ddl,
        }
    } else {
        LogEntry::RawSql {
            sql: req.ddl,
            affects_table: req.table,
        }
    };

    Json(WriteResponse {
        success: true,
        lsn: Some(1),
        message: Some("DDL accepted".to_string()),
    }).into_response()
}

async fn handle_status(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let leader = state.cluster.current_leader().await;
    let self_node = state.cluster.get_self().await;
    let size = state.cluster.size().await;
    let has_quorum = state.cluster.has_quorum().await;

    // Determine if we're the leader - check both the flag and if the leader_id matches us
    let is_leader = leader.as_ref().map(|l| l.id == state.node_id).unwrap_or(false)
        || *state.is_leader.read().await;

    Json(StatusResponse {
        node_id: state.node_id.clone(),
        is_leader,
        leader_id: leader.map(|l| l.id),
        term: 1,
        last_applied_lsn: self_node.last_applied_lsn,
        commit_lsn: self_node.last_applied_lsn,
        cluster_size: size,
        has_quorum,
    })
}

async fn handle_health(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    Json(HealthResponse {
        healthy: true,
        node_id: state.node_id.clone(),
        is_leader: *state.is_leader.read().await,
    })
}

/// Stats endpoint for live throughput monitoring
async fn handle_stats(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let leader = state.cluster.current_leader().await;
    let self_node = state.cluster.get_self().await;
    let all_nodes = state.cluster.all_nodes().await;
    
    // Determine role
    let is_leader = leader.as_ref().map(|l| l.id == state.node_id).unwrap_or(false)
        || *state.is_leader.read().await;
    let role = if is_leader { "Leader" } else { "Follower" };
    
    // Calculate active nodes
    let active_count = all_nodes.iter()
        .filter(|n| n.status == crate::state::NodeStatus::Active)
        .count();
    
    // Build follower stats (only meaningful for leader)
    let current_lsn = self_node.last_applied_lsn;
    let followers: Vec<FollowerStats> = all_nodes.iter()
        .filter(|n| n.id != state.node_id)
        .map(|n| {
            let lag = if current_lsn > n.last_applied_lsn {
                current_lsn - n.last_applied_lsn
            } else {
                0
            };
            FollowerStats {
                node_id: n.id.clone(),
                last_applied_lsn: n.last_applied_lsn,
                lag,
                status: format!("{:?}", n.status),
            }
        })
        .collect();
    
    Json(StatsResponse {
        node_id: state.node_id.clone(),
        role: role.to_string(),
        current_lsn,
        commit_lsn: current_lsn, // For now, same as current
        uptime_seconds: 0, // TODO: Track actual uptime
        cluster_size: all_nodes.len(),
        active_nodes: active_count,
        followers,
    })
}

async fn handle_cluster_info(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let summary = state.cluster.summary().await;
    let nodes = state.cluster.all_nodes().await;

    Json(ClusterInfoResponse { summary, nodes })
}

async fn handle_nodes(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let nodes = state.cluster.all_nodes().await;
    Json(nodes)
}

async fn handle_node_info(
    State(state): State<Arc<AppState>>,
    Path(node_id): Path<String>,
) -> impl IntoResponse {
    match state.cluster.get_node(&node_id).await {
        Some(node) => Json(node).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Node {} not found", node_id),
                code: "NODE_NOT_FOUND".to_string(),
            }),
        ).into_response(),
    }
}

async fn handle_promote(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    // This would trigger leader election
    *state.is_leader.write().await = true;
    Json(WriteResponse {
        success: true,
        lsn: None,
        message: Some("Promotion requested".to_string()),
    })
}

async fn handle_demote(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    *state.is_leader.write().await = false;
    Json(WriteResponse {
        success: true,
        lsn: None,
        message: Some("Demotion requested".to_string()),
    })
}

/// Reset WAL and state - clears all log entries and resets LSN to 0
async fn handle_reset(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    tracing::warn!("WAL reset requested for node {}", state.node_id);
    
    // Clear WAL directory
    let wal_dir = state.data_dir.join("wal");
    let mut wal_cleared = false;
    if wal_dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&wal_dir) {
            tracing::error!("Failed to remove WAL directory: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(WriteResponse {
                success: false,
                lsn: None,
                message: Some(format!("Failed to clear WAL: {}", e)),
            }));
        }
        // Recreate empty directory
        if let Err(e) = std::fs::create_dir_all(&wal_dir) {
            tracing::error!("Failed to recreate WAL directory: {}", e);
        }
        wal_cleared = true;
    }
    
    // Clear state database
    let state_dir = state.data_dir.join("state");
    let mut state_cleared = false;
    if state_dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&state_dir) {
            tracing::error!("Failed to remove state directory: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(WriteResponse {
                success: false,
                lsn: None,
                message: Some(format!("Failed to clear state: {}", e)),
            }));
        }
        // Recreate empty directory
        if let Err(e) = std::fs::create_dir_all(&state_dir) {
            tracing::error!("Failed to recreate state directory: {}", e);
        }
        state_cleared = true;
    }
    
    tracing::warn!("WAL reset completed: wal_cleared={}, state_cleared={}", wal_cleared, state_cleared);
    
    (StatusCode::OK, Json(WriteResponse {
        success: true,
        lsn: Some(0),
        message: Some(format!("Reset complete. WAL cleared: {}, State cleared: {}. Restart required.", wal_cleared, state_cleared)),
    }))
}

// ============ Helpers ============

fn json_to_value(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(u) = n.as_u64() {
                Value::UInt(u)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::Null
            }
        }
        serde_json::Value::String(s) => Value::String(s.clone()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            Value::Json(v.clone())
        }
    }
}

fn json_to_primary_key(v: &serde_json::Value) -> PrimaryKey {
    match v {
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                PrimaryKey::Int(i)
            } else {
                PrimaryKey::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => {
            // Try to parse as UUID
            if let Ok(uuid) = uuid::Uuid::parse_str(s) {
                PrimaryKey::Uuid(uuid)
            } else {
                PrimaryKey::String(s.clone())
            }
        }
        _ => PrimaryKey::String(v.to_string()),
    }
}

/// Forward a write request to the current leader
/// Returns the leader's response or an error if forwarding fails
async fn forward_to_leader<T: Serialize>(
    state: &AppState,
    endpoint: &str,
    body: &T,
) -> std::result::Result<axum::response::Response, axum::response::Response> {
    // Get the current leader
    let leader = match state.cluster.current_leader().await {
        Some(l) => l,
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: "No leader available".to_string(),
                    code: "NO_LEADER".to_string(),
                }),
            ).into_response());
        }
    };

    // Don't forward to ourselves
    if leader.id == state.node_id {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Routing error: not leader but no other leader found".to_string(),
                code: "ROUTING_ERROR".to_string(),
            }),
        ).into_response());
    }

    // Extract host from leader address (assume same port as API or use configured API port)
    // The leader.address is the cluster port, we need to convert to API port (8080)
    let leader_host = leader.address.split(':')
        .next()
        .unwrap_or(&leader.address);
    let leader_api_url = format!("http://{}:8080{}", leader_host, endpoint);

    tracing::debug!("Forwarding write to leader at {}", leader_api_url);

    // Forward the request
    match HTTP_CLIENT
        .post(&leader_api_url)
        .json(body)
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            match response.text().await {
                Ok(body) => {
                    let response_status = StatusCode::from_u16(status.as_u16())
                        .unwrap_or(StatusCode::OK);
                    Ok((response_status, body).into_response())
                }
                Err(e) => Err((
                    StatusCode::BAD_GATEWAY,
                    Json(ErrorResponse {
                        error: format!("Failed to read leader response: {}", e),
                        code: "FORWARD_ERROR".to_string(),
                    }),
                ).into_response()),
            }
        }
        Err(e) => {
            tracing::warn!("Failed to forward to leader: {}", e);
            Err((
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    error: format!("Failed to contact leader: {}", e),
                    code: "FORWARD_ERROR".to_string(),
                }),
            ).into_response())
        }
    }
}

// ============ Migration Handlers ============

/// Dump info response
#[derive(Debug, Serialize)]
struct DumpInfoResponse {
    lsn: u64,
    database: String,
    node_id: String,
}

/// Get dump info (LSN and database name for migration)
async fn handle_dump_info(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    // Get current cluster state to find our LSN
    let self_node = state.cluster.get_self().await;
    
    Json(DumpInfoResponse {
        lsn: self_node.last_applied_lsn,
        database: "wolfscale".to_string(), // TODO: get from config
        node_id: state.node_id.clone(),
    })
}

/// Stream database dump for migration
/// This executes mysqldump and streams the output
async fn handle_dump(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    use axum::body::Body;
    use tokio::process::Command;

    // Only allow dump from leader or synced nodes
    let self_node = state.cluster.get_self().await;
    if self_node.status != crate::state::NodeStatus::Active {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Node is not in Active status - cannot provide dump".to_string(),
        ).into_response();
    }

    // Execute mysqldump
    // TODO: Get database credentials from config - for now use defaults
    let output = Command::new("mysqldump")
        .args([
            "--all-databases",
            "--single-transaction",
            "--routines",
            "--triggers",
            "--events",
            "-u", "root",
        ])
        .output()
        .await;

    match output {
        Ok(result) => {
            if result.status.success() {
                (
                    StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "application/sql")],
                    Body::from(result.stdout),
                ).into_response()
            } else {
                let stderr = String::from_utf8_lossy(&result.stderr);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("mysqldump failed: {}", stderr),
                ).into_response()
            }
        }
        Err(e) => {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to execute mysqldump: {}", e),
            ).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_to_value() {
        assert!(matches!(json_to_value(&serde_json::json!(null)), Value::Null));
        assert!(matches!(json_to_value(&serde_json::json!(true)), Value::Bool(true)));
        assert!(matches!(json_to_value(&serde_json::json!(42)), Value::Int(42)));
        assert!(matches!(json_to_value(&serde_json::json!("test")), Value::String(_)));
    }

    #[test]
    fn test_json_to_primary_key() {
        assert!(matches!(json_to_primary_key(&serde_json::json!(123)), PrimaryKey::Int(123)));
        assert!(matches!(json_to_primary_key(&serde_json::json!("abc")), PrimaryKey::String(_)));
    }
}
