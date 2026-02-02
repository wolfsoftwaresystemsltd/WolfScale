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
    ) -> Self {
        let state = Arc::new(AppState {
            node_id,
            is_leader: RwLock::new(false),
            cluster,
            write_handler: RwLock::new(None),
        });

        Self { config, state }
    }

    /// Create a new HTTP server with write handler
    pub fn with_write_handler(
        config: ApiConfig,
        node_id: String,
        cluster: Arc<ClusterMembership>,
        write_handler: WriteHandler,
    ) -> Self {
        let state = Arc::new(AppState {
            node_id,
            is_leader: RwLock::new(true), // If we have write handler, we're the leader
            cluster,
            write_handler: RwLock::new(Some(write_handler)),
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
            .route("/health", get(handle_health))
            .route("/cluster", get(handle_cluster_info))
            .route("/cluster/nodes", get(handle_nodes))
            .route("/cluster/nodes/:node_id", get(handle_node_info))
            // Admin operations
            .route("/admin/promote", post(handle_promote))
            .route("/admin/demote", post(handle_demote))
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
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct WriteRequest {
    pub table: String,
    pub operation: String,
    pub data: serde_json::Value,
}

/// Insert request
#[derive(Debug, Deserialize)]
pub struct InsertRequest {
    pub table: String,
    pub values: std::collections::HashMap<String, serde_json::Value>,
}

/// Update request
#[derive(Debug, Deserialize)]
pub struct UpdateRequest {
    pub table: String,
    pub set: std::collections::HashMap<String, serde_json::Value>,
    pub where_key: std::collections::HashMap<String, serde_json::Value>,
}

/// Delete request
#[derive(Debug, Deserialize)]
pub struct DeleteRequest {
    pub table: String,
    pub where_key: std::collections::HashMap<String, serde_json::Value>,
}

/// Upsert request
#[derive(Debug, Deserialize)]
pub struct UpsertRequest {
    pub table: String,
    pub values: std::collections::HashMap<String, serde_json::Value>,
    pub update_columns: Vec<String>,
}

/// DDL request
#[derive(Debug, Deserialize)]
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
    Json(_req): Json<WriteRequest>,
) -> impl IntoResponse {
    // Check if we're the leader
    if !*state.is_leader.read().await {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Not the leader".to_string(),
                code: "NOT_LEADER".to_string(),
            }),
        ).into_response();
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
    if !*state.is_leader.read().await {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Not the leader".to_string(),
                code: "NOT_LEADER".to_string(),
            }),
        ).into_response();
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
    if !*state.is_leader.read().await {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Not the leader".to_string(),
                code: "NOT_LEADER".to_string(),
            }),
        ).into_response();
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
    if !*state.is_leader.read().await {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Not the leader".to_string(),
                code: "NOT_LEADER".to_string(),
            }),
        ).into_response();
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
    if !*state.is_leader.read().await {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Not the leader".to_string(),
                code: "NOT_LEADER".to_string(),
            }),
        ).into_response();
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
    if !*state.is_leader.read().await {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Not the leader".to_string(),
                code: "NOT_LEADER".to_string(),
            }),
        ).into_response();
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
    let is_leader = *state.is_leader.read().await;
    let leader = state.cluster.current_leader().await;
    let size = state.cluster.size().await;
    let has_quorum = state.cluster.has_quorum().await;

    Json(StatusResponse {
        node_id: state.node_id.clone(),
        is_leader,
        leader_id: leader.map(|l| l.id),
        term: 1,
        last_applied_lsn: 0,
        commit_lsn: 0,
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
