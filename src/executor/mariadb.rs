//! MariaDB Executor
//!
//! Executes log entries against a MariaDB database.

use std::sync::Arc;
use std::time::Duration;
use sqlx::{MySqlPool, Row};
use sqlx::mysql::MySqlPoolOptions;
use tokio::sync::RwLock;

use crate::config::DatabaseConfig;
use crate::wal::LogEntry;
use crate::error::{Error, Result};

/// MariaDB executor for applying log entries
pub struct MariaDbExecutor {
    /// Database-specific connection pool (may become invalid if DB is dropped)
    pool: Arc<RwLock<Option<MySqlPool>>>,
    /// Server-level connection pool (for DDL operations like DROP/CREATE DATABASE)
    server_pool: Option<MySqlPool>,
    /// Config for reconnection
    config: Option<DatabaseConfig>,
    /// Whether this is a mock executor (for testing)
    is_mock: bool,
}

impl MariaDbExecutor {
    /// Create a new executor with a connection pool
    pub async fn new(config: &DatabaseConfig) -> Result<Self> {
        // Server-level URL (no specific database)
        let server_url = format!(
            "mysql://{}:{}@{}:{}",
            config.user,
            config.password,
            config.host,
            config.port
        );

        // Database-specific URL
        let db_url = match &config.database {
            Some(db) => format!(
                "mysql://{}:{}@{}:{}/{}",
                config.user,
                config.password,
                config.host,
                config.port,
                db
            ),
            None => server_url.clone(),
        };

        // Create server-level pool for DDL operations
        let server_pool = MySqlPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(config.connect_timeout_secs))
            .connect(&server_url)
            .await?;

        // Create database-level pool for normal operations
        let db_pool = MySqlPoolOptions::new()
            .max_connections(config.pool_size)
            .acquire_timeout(Duration::from_secs(config.connect_timeout_secs))
            .connect(&db_url)
            .await?;

        Ok(Self {
            pool: Arc::new(RwLock::new(Some(db_pool))),
            server_pool: Some(server_pool),
            config: Some(config.clone()),
            is_mock: false,
        })
    }

    /// Create a mock executor for testing
    pub fn new_mock() -> Self {
        Self {
            pool: Arc::new(RwLock::new(None)),
            server_pool: None,
            config: None,
            is_mock: true,
        }
    }

    /// Check if a SQL statement is a DDL operation that affects databases
    fn is_database_ddl(stmt: &str) -> bool {
        let upper = stmt.to_uppercase();
        upper.starts_with("CREATE DATABASE") ||
        upper.starts_with("DROP DATABASE") ||
        upper.starts_with("CREATE SCHEMA") ||
        upper.starts_with("DROP SCHEMA")
    }

    /// Execute a log entry
    pub async fn execute_entry(&self, entry: &LogEntry) -> Result<()> {
        if self.is_mock {
            return Ok(());
        }

        let statements = entry.to_sql();
        
        for sql in statements {
            if sql.is_empty() {
                continue;
            }

            // Split on semicolons to handle multi-statement SQL
            // This is needed because sqlx doesn't support multi-statement queries
            // and we may have "USE dbname; CREATE TABLE..." from replication
            for single_stmt in split_sql_statements(&sql) {
                let stmt = single_stmt.trim();
                if stmt.is_empty() {
                    continue;
                }

                tracing::debug!("Executing: {}", &stmt[..stmt.len().min(100)]);
                
                // Use server pool for database-level DDL operations
                if Self::is_database_ddl(stmt) {
                    tracing::info!("Using server pool for DDL: {}", &stmt[..stmt.len().min(50)]);
                    let server_pool = self.server_pool.as_ref().ok_or_else(|| {
                        Error::Database(sqlx::Error::Configuration("No server pool".into()))
                    })?;
                    
                    sqlx::query(stmt)
                        .execute(server_pool)
                        .await
                        .map_err(|e| {
                            Error::QueryExecution(format!("Failed to execute DDL '{}...': {}", 
                                &stmt[..stmt.len().min(50)], e))
                        })?;
                    
                    // If we just created a database that matches our config, try to reconnect db pool
                    if stmt.to_uppercase().starts_with("CREATE DATABASE") {
                        if let Err(e) = self.try_reconnect_db_pool().await {
                            tracing::warn!("Could not reconnect to database pool: {}", e);
                        }
                    }
                } else {
                    // Normal statements use the database pool
                    let pool_guard = self.pool.read().await;
                    let pool = pool_guard.as_ref().ok_or_else(|| {
                        Error::Database(sqlx::Error::Configuration("No database pool".into()))
                    })?;

                    sqlx::query(stmt)
                        .execute(pool)
                        .await
                        .map_err(|e| {
                            Error::QueryExecution(format!("Failed to execute '{}...': {}", 
                                &stmt[..stmt.len().min(50)], e))
                        })?;
                }
            }
        }

        Ok(())
    }

    /// Try to reconnect the database pool after CREATE DATABASE
    async fn try_reconnect_db_pool(&self) -> Result<()> {
        let config = self.config.as_ref().ok_or_else(|| {
            Error::Database(sqlx::Error::Configuration("No config for reconnection".into()))
        })?;

        if let Some(db) = &config.database {
            let db_url = format!(
                "mysql://{}:{}@{}:{}/{}",
                config.user,
                config.password,
                config.host,
                config.port,
                db
            );

            tracing::info!("Attempting to reconnect database pool to {}", db);
            
            let new_pool = MySqlPoolOptions::new()
                .max_connections(config.pool_size)
                .acquire_timeout(Duration::from_secs(config.connect_timeout_secs))
                .connect(&db_url)
                .await?;

            let mut pool_guard = self.pool.write().await;
            *pool_guard = Some(new_pool);
            
            tracing::info!("Successfully reconnected database pool");
        }

        Ok(())
    }

    /// Execute a raw SQL statement
    pub async fn execute_raw(&self, sql: &str) -> Result<u64> {
        if self.is_mock {
            return Ok(0);
        }

        // Use server pool for database DDL
        if Self::is_database_ddl(sql) {
            let server_pool = self.server_pool.as_ref().ok_or_else(|| {
                Error::Database(sqlx::Error::Configuration("No server pool".into()))
            })?;
            let result = sqlx::query(sql)
                .execute(server_pool)
                .await?;
            return Ok(result.rows_affected());
        }

        let pool_guard = self.pool.read().await;
        let pool = pool_guard.as_ref().ok_or_else(|| {
            Error::Database(sqlx::Error::Configuration("No pool".into()))
        })?;

        let result = sqlx::query(sql)
            .execute(pool)
            .await?;

        Ok(result.rows_affected())
    }

    /// Execute multiple statements in a transaction
    pub async fn execute_transaction(&self, statements: Vec<String>) -> Result<()> {
        if self.is_mock {
            return Ok(());
        }

        let pool_guard = self.pool.read().await;
        let pool = pool_guard.as_ref().ok_or_else(|| {
            Error::Database(sqlx::Error::Configuration("No pool".into()))
        })?;

        let mut tx = pool.begin().await?;

        for sql in statements {
            if sql.is_empty() || sql == "START TRANSACTION" || sql == "COMMIT" {
                continue;
            }

            sqlx::query(&sql)
                .execute(&mut *tx)
                .await
                .map_err(|e| {
                    Error::QueryExecution(format!("Transaction failed on '{}...': {}", 
                        &sql[..sql.len().min(50)], e))
                })?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Check if connection is healthy
    pub async fn health_check(&self) -> Result<bool> {
        if self.is_mock {
            return Ok(true);
        }

        // Use server pool for health check - more reliable
        let server_pool = self.server_pool.as_ref().ok_or_else(|| {
            Error::Database(sqlx::Error::Configuration("No server pool".into()))
        })?;

        let result: (i32,) = sqlx::query_as("SELECT 1")
            .fetch_one(server_pool)
            .await?;

        Ok(result.0 == 1)
    }

    /// Get list of tables in the database
    pub async fn list_tables(&self) -> Result<Vec<String>> {
        if self.is_mock {
            return Ok(vec![]);
        }

        let pool_guard = self.pool.read().await;
        let pool = pool_guard.as_ref().ok_or_else(|| {
            Error::Database(sqlx::Error::Configuration("No pool".into()))
        })?;

        let rows = sqlx::query("SHOW TABLES")
            .fetch_all(pool)
            .await?;

        let tables: Vec<String> = rows
            .iter()
            .filter_map(|row| row.try_get::<String, _>(0).ok())
            .collect();

        Ok(tables)
    }

    /// Get table structure (columns and types)
    pub async fn describe_table(&self, table: &str) -> Result<Vec<ColumnInfo>> {
        if self.is_mock {
            return Ok(vec![]);
        }

        let pool_guard = self.pool.read().await;
        let pool = pool_guard.as_ref().ok_or_else(|| {
            Error::Database(sqlx::Error::Configuration("No pool".into()))
        })?;

        let rows = sqlx::query(&format!("DESCRIBE `{}`", table))
            .fetch_all(pool)
            .await?;

        let columns: Vec<ColumnInfo> = rows
            .iter()
            .filter_map(|row| {
                Some(ColumnInfo {
                    name: row.try_get("Field").ok()?,
                    data_type: row.try_get("Type").ok()?,
                    nullable: row.try_get::<String, _>("Null").ok()? == "YES",
                    key: row.try_get("Key").ok().unwrap_or_default(),
                    default: row.try_get("Default").ok(),
                    extra: row.try_get("Extra").ok().unwrap_or_default(),
                })
            })
            .collect();

        Ok(columns)
    }

    /// Get primary key columns for a table
    pub async fn get_primary_key(&self, table: &str) -> Result<Vec<String>> {
        if self.is_mock {
            return Ok(vec!["id".to_string()]);
        }

        let columns = self.describe_table(table).await?;
        let pk_columns: Vec<String> = columns
            .into_iter()
            .filter(|c| c.key == "PRI")
            .map(|c| c.name)
            .collect();

        Ok(pk_columns)
    }

    /// Get row count for a table
    pub async fn count_rows(&self, table: &str) -> Result<u64> {
        if self.is_mock {
            return Ok(0);
        }

        let pool_guard = self.pool.read().await;
        let pool = pool_guard.as_ref().ok_or_else(|| {
            Error::Database(sqlx::Error::Configuration("No pool".into()))
        })?;

        let row: (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM `{}`", table))
            .fetch_one(pool)
            .await?;

        Ok(row.0 as u64)
    }

    /// Close the connection pool
    pub async fn close(&self) {
        let pool_guard = self.pool.read().await;
        if let Some(pool) = pool_guard.as_ref() {
            pool.close().await;
        }
        if let Some(server_pool) = &self.server_pool {
            server_pool.close().await;
        }
    }
}

/// Column information
#[derive(Debug, Clone)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub key: String,
    pub default: Option<String>,
    pub extra: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_executor() {
        let executor = MariaDbExecutor::new_mock();
        
        assert!(executor.health_check().await.unwrap());
        
        let entry = LogEntry::Insert {
            table: "test".to_string(),
            columns: vec!["id".to_string()],
            values: vec![crate::wal::entry::Value::Int(1)],
            primary_key: crate::wal::entry::PrimaryKey::Int(1),
        };
        
        executor.execute_entry(&entry).await.unwrap();
    }

    #[test]
    fn test_sql_generation() {
        let entry = LogEntry::Insert {
            table: "users".to_string(),
            columns: vec!["id".to_string(), "name".to_string()],
            values: vec![
                crate::wal::entry::Value::Int(1),
                crate::wal::entry::Value::String("Alice".to_string()),
            ],
            primary_key: crate::wal::entry::PrimaryKey::Int(1),
        };

        let sql = entry.to_sql();
        assert_eq!(sql.len(), 1);
        assert!(sql[0].contains("INSERT INTO"));
        assert!(sql[0].contains("`users`"));
        assert!(sql[0].contains("'Alice'"));
    }
}

/// Split SQL string on semicolons, respecting string literals
/// This handles cases like: "USE db; CREATE TABLE foo (name VARCHAR(50));"
fn split_sql_statements(sql: &str) -> Vec<&str> {
    let mut statements = Vec::new();
    let mut start = 0;
    let mut in_string = false;
    let mut string_char = '"';
    let mut chars = sql.char_indices().peekable();
    
    while let Some((i, c)) = chars.next() {
        if in_string {
            if c == string_char {
                // Check for escaped quote
                if chars.peek().map(|(_, nc)| *nc == string_char).unwrap_or(false) {
                    chars.next(); // Skip escaped quote
                } else {
                    in_string = false;
                }
            }
        } else {
            match c {
                '\'' | '"' => {
                    in_string = true;
                    string_char = c;
                }
                ';' => {
                    let stmt = sql[start..i].trim();
                    if !stmt.is_empty() {
                        statements.push(stmt);
                    }
                    start = i + 1;
                }
                _ => {}
            }
        }
    }
    
    // Add remaining content
    let remaining = sql[start..].trim();
    if !remaining.is_empty() {
        statements.push(remaining);
    }
    
    statements
}
