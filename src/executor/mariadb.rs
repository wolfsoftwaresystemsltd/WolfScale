//! MariaDB Executor
//!
//! Executes log entries against a MariaDB database.

use std::sync::Arc;
use std::time::Duration;
use std::collections::HashMap;
use sqlx::{MySqlPool, Row};
use sqlx::mysql::MySqlPoolOptions;
use tokio::sync::RwLock;

use crate::config::DatabaseConfig;
use crate::wal::LogEntry;
use crate::error::{Error, Result};

/// Safely truncate a string at char boundary (UTF-8 safe)
fn safe_truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max_chars).collect::<String>())
    }
}

/// MariaDB executor for applying log entries
pub struct MariaDbExecutor {
    /// Database-specific connection pool (may become invalid if DB is dropped)
    pool: Arc<RwLock<Option<MySqlPool>>>,
    /// Server-level connection pool (for DDL operations like DROP/CREATE DATABASE)
    server_pool: Option<MySqlPool>,
    /// Cache of database-specific pools (created on demand for replication)
    db_pools: Arc<RwLock<HashMap<String, MySqlPool>>>,
    /// Config for reconnection
    config: Option<DatabaseConfig>,
    /// Whether this is a mock executor (for testing)
    is_mock: bool,
}

impl MariaDbExecutor {
    /// Create a new executor with a connection pool
    pub async fn new(config: &DatabaseConfig) -> Result<Self> {
        // Server-level URL (no specific database) - always available
        let server_url = format!(
            "mysql://{}:{}@{}:{}",
            config.user,
            config.password,
            config.host,
            config.port
        );

        // Create server-level pool for DDL operations - this always works
        let server_pool = MySqlPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(config.connect_timeout_secs))
            .connect(&server_url)
            .await?;

        // Try to connect to database pool, but don't fail if database doesn't exist
        let db_pool = if let Some(db) = &config.database {
            let db_url = format!(
                "mysql://{}:{}@{}:{}/{}",
                config.user,
                config.password,
                config.host,
                config.port,
                db
            );

            match MySqlPoolOptions::new()
                .max_connections(config.pool_size)
                .acquire_timeout(Duration::from_secs(config.connect_timeout_secs))
                .connect(&db_url)
                .await
            {
                Ok(pool) => Some(pool),
                Err(e) => {
                    tracing::warn!("Could not connect to database '{}': {}. Will connect later.", db, e);
                    None
                }
            }
        } else {
            // No specific database configured - create a SEPARATE server-level pool for db_pool
            // (don't clone server_pool as they would share state and closing one closes both)
            match MySqlPoolOptions::new()
                .max_connections(config.pool_size)
                .acquire_timeout(Duration::from_secs(config.connect_timeout_secs))
                .connect(&server_url)
                .await
            {
                Ok(pool) => Some(pool),
                Err(e) => {
                    tracing::warn!("Could not create db_pool: {}. Will retry later.", e);
                    None
                }
            }
        };

        Ok(Self {
            pool: Arc::new(RwLock::new(db_pool)),
            server_pool: Some(server_pool),
            db_pools: Arc::new(RwLock::new(HashMap::new())),
            config: Some(config.clone()),
            is_mock: false,
        })
    }

    /// Create a mock executor for testing
    pub fn new_mock() -> Self {
        Self {
            pool: Arc::new(RwLock::new(None)),
            server_pool: None,
            db_pools: Arc::new(RwLock::new(HashMap::new())),
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

    /// Get or create a connection pool for a specific database
    /// This is used when replicating entries that target a specific database
    async fn get_or_create_db_pool(&self, database: &str) -> Result<MySqlPool> {
        // First check if we already have a pool for this database
        {
            let pools = self.db_pools.read().await;
            if let Some(pool) = pools.get(database) {
                return Ok(pool.clone());
            }
        }

        // Need to create a new pool - get config
        let config = self.config.as_ref().ok_or_else(|| {
            Error::Database(sqlx::Error::Configuration("No config available to create pool".into()))
        })?;

        let db_url = format!(
            "mysql://{}:{}@{}:{}/{}",
            config.user,
            config.password,
            config.host,
            config.port,
            database
        );

        tracing::info!("Creating on-demand connection pool for database '{}'", database);

        let pool = MySqlPoolOptions::new()
            .max_connections(config.pool_size)
            .acquire_timeout(Duration::from_secs(config.connect_timeout_secs))
            .connect(&db_url)
            .await
            .map_err(|e| {
                Error::Database(sqlx::Error::Configuration(
                    format!("Failed to connect to database '{}': {}", database, e).into()
                ))
            })?;

        // Store in cache
        {
            let mut pools = self.db_pools.write().await;
            pools.insert(database.to_string(), pool.clone());
        }

        tracing::info!("Successfully created pool for database '{}'", database);
        Ok(pool)
    }

    /// Invalidate (remove) a cached database pool - called after DROP DATABASE
    async fn invalidate_db_pool(&self, database: &str) {
        let mut pools = self.db_pools.write().await;
        if let Some(pool) = pools.remove(database) {
            tracing::info!("Invalidating pool for dropped database '{}'", database);
            pool.close().await;
        }
    }

    /// Execute a log entry
    pub async fn execute_entry(&self, entry: &LogEntry) -> Result<()> {
        if self.is_mock {
            return Ok(());
        }

        // Extract database name from entry (only RawSql has this field currently)
        let target_database = match entry {
            LogEntry::RawSql { database, .. } => database.clone(),
            _ => None,
        };

        // Get the appropriate pool for this entry
        // If entry specifies a database, try to get/create a pool for that database
        // Otherwise fall back to the main pool or server pool
        let pool_to_use: Option<MySqlPool> = if let Some(ref db_name) = target_database {
            // Entry specifies a database - create a pool for it on demand
            match self.get_or_create_db_pool(db_name).await {
                Ok(pool) => {
                    tracing::debug!("Using database-specific pool for '{}'", db_name);
                    Some(pool)
                }
                Err(e) => {
                    tracing::warn!("Could not get pool for database '{}': {}", db_name, e);
                    None
                }
            }
        } else {
            // No specific database - use the main pool
            let pool_guard = self.pool.read().await;
            pool_guard.clone()
        };

        // Acquire a connection from the chosen pool
        let mut conn_opt = if let Some(ref pool) = pool_to_use {
            match pool.acquire().await {
                Ok(c) => Some(c),
                Err(e) => {
                    tracing::warn!("Could not acquire connection from pool: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let statements = entry.to_sql();
        
        // For each SQL statement
        for sql in statements {
            if sql.is_empty() {
                continue;
            }

            // Split on semicolons to handle multi-statement SQL
            // IMPORTANT: Skip USE statements since we're now using database-specific pools
            let stmts: Vec<&str> = split_sql_statements(&sql);
            
            for single_stmt in stmts {
                let stmt = single_stmt.trim();
                if stmt.is_empty() {
                    continue;
                }

                // Skip USE statements - we handle database selection via pools now
                let upper = stmt.to_uppercase();
                if upper.starts_with("USE ") || upper.starts_with("USE`") {
                    tracing::debug!("Skipping USE statement (using database-specific pool instead): {}", stmt);
                    continue;
                }
                
                // Skip LOCK TABLES and UNLOCK TABLES - they cause metadata lock issues
                // during replication since entries execute across different connections
                if upper.starts_with("LOCK TABLES") || upper.starts_with("UNLOCK TABLES") {
                    tracing::debug!("Skipping LOCK/UNLOCK TABLES (not needed for replication): {}", safe_truncate(stmt, 50));
                    continue;
                }

                tracing::debug!("Executing on db={:?}: {}", target_database, safe_truncate(stmt, 80));
                
                // Use server pool for database-level DDL operations (CREATE/DROP DATABASE)
                if Self::is_database_ddl(stmt) {
                    tracing::info!("Executing DDL: {}", safe_truncate(stmt, 50));
                    let server_pool = self.server_pool.as_ref().ok_or_else(|| {
                        Error::Database(sqlx::Error::Configuration("No server pool".into()))
                    })?;
                    
                    // Release any held connection before DDL
                    if conn_opt.is_some() {
                        tracing::info!("Releasing connection before DDL to ensure ordering");
                        conn_opt = None;
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                    
                    // For DROP DATABASE, invalidate the cached pool for that database
                    if upper.starts_with("DROP DATABASE") {
                        // Extract database name from DROP DATABASE statement
                        let db_name = extract_database_name_from_ddl(stmt);
                        if let Some(db) = db_name {
                            self.invalidate_db_pool(&db).await;
                        }
                        // Also close the main pool if it was connected to this database
                        let mut pool_write = self.pool.write().await;
                        if let Some(p) = pool_write.take() {
                            tracing::info!("Closing db_pool before DROP DATABASE");
                            p.close().await;
                        }
                    }
                    
                    let start = std::time::Instant::now();
                    sqlx::query(stmt)
                        .execute(server_pool)
                        .await
                        .map_err(|e| {
                            Error::QueryExecution(format!("Failed to execute DDL '{}': {}", 
                                safe_truncate(stmt, 50), e))
                        })?;
                    let elapsed = start.elapsed();
                    if elapsed > Duration::from_secs(5) {
                        tracing::warn!("DDL took {:.1}s: {}", elapsed.as_secs_f64(), safe_truncate(stmt, 50));
                    }
                    
                    // After CREATE DATABASE, try to reconnect main db pool
                    if upper.starts_with("CREATE DATABASE") {
                        tracing::info!("CREATE DATABASE completed, reconnecting db_pool...");
                        if let Err(e) = self.try_reconnect_db_pool_sync().await {
                            tracing::warn!("Could not reconnect to database pool: {}", e);
                        }
                    }
                } else {
                    // Normal statements - use our held connection
                    if let Some(ref mut conn) = conn_opt {
                        sqlx::query(stmt)
                            .execute(&mut **conn)
                            .await
                            .map_err(|e| {
                                Error::QueryExecution(format!("Failed to execute '{}': {}", 
                                    safe_truncate(stmt, 50), e))
                            })?;
                    } else {
                        // No database-specific pool - try server_pool as fallback
                        if let Some(server_pool) = &self.server_pool {
                            // Acquire and hold a connection from server_pool
                            match server_pool.acquire().await {
                                Ok(mut server_conn) => {
                                    sqlx::query(stmt)
                                        .execute(&mut *server_conn)
                                        .await
                                        .map_err(|e| {
                                            Error::QueryExecution(format!("Failed to execute (fallback) '{}': {}", 
                                                safe_truncate(stmt, 50), e))
                                        })?;
                                    // Keep this connection for subsequent statements
                                    conn_opt = Some(server_conn.into());
                                }
                                Err(e) => {
                                    return Err(Error::QueryExecution(format!(
                                        "No database connection available for '{}': {}",
                                        safe_truncate(stmt, 50), e
                                    )));
                                }
                            }
                        } else {
                            return Err(Error::Database(sqlx::Error::Configuration("No connection available".into())));
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Try to reconnect the database pool synchronously (waits for result)
    /// Used when we need the pool immediately for normal writes
    async fn try_reconnect_db_pool_sync(&self) -> Result<()> {
        let config = match self.config.as_ref() {
            Some(c) => c.clone(),
            None => return Ok(()),  // No config, nothing to do
        };

        if let Some(db) = &config.database {
            let db_url = format!(
                "mysql://{}:{}@{}:{}/{}",
                config.user,
                config.password,
                config.host,
                config.port,
                db
            );

            tracing::info!("Reconnecting database pool to {}", db);
            
            match MySqlPoolOptions::new()
                .max_connections(config.pool_size)
                .acquire_timeout(Duration::from_secs(5))
                .connect(&db_url)
                .await
            {
                Ok(new_pool) => {
                    let mut pool_guard = self.pool.write().await;
                    *pool_guard = Some(new_pool);
                    tracing::info!("Successfully reconnected database pool");
                }
                Err(e) => {
                    tracing::warn!("Pool reconnection failed: {}", e);
                    return Err(Error::Database(e));
                }
            }
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
                    Error::QueryExecution(format!("Transaction failed on '{}': {}", 
                        safe_truncate(&sql, 50), e))
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

    /// Execute multiple log entries with INSERT batching for improved throughput.
    /// 
    /// This method groups consecutive INSERT statements to the same table into 
    /// multi-row INSERT statements, reducing per-statement overhead.
    /// 
    /// Safety guarantees:
    /// - Only INSERT statements are batched (UPDATE/DELETE/DDL execute individually)
    /// - Only consecutive INSERTs to the same table are combined
    /// - Batch atomicity: if any row fails, the entire batch fails
    /// - Maximum batch size prevents memory exhaustion (1000 rows or 16MB)
    pub async fn execute_entries_batched(&self, entries: &[&LogEntry]) -> Result<()> {
        if self.is_mock || entries.is_empty() {
            return Ok(());
        }

        const MAX_BATCH_ROWS: usize = 1000;
        const MAX_BATCH_BYTES: usize = 16 * 1024 * 1024; // 16MB

        // Accumulator for batching INSERTs
        let mut pending_batch: Option<InsertBatch> = None;
        let mut total_executed = 0;
        let mut total_batched = 0;

        for entry in entries {
            // Try to add to current batch
            if let Some(ref mut batch) = pending_batch {
                if let Some(insert_info) = extract_insert_info(entry) {
                    // Check if compatible with current batch
                    if batch.table == insert_info.table 
                        && batch.database == insert_info.database
                        && batch.columns == insert_info.columns
                        && batch.rows.len() < MAX_BATCH_ROWS
                        && batch.estimated_size < MAX_BATCH_BYTES
                    {
                        // Add to batch
                        batch.rows.push(insert_info.values);
                        batch.estimated_size += insert_info.estimated_size;
                        continue;
                    }
                }
                // Can't batch - flush current batch first
                let batched_count = batch.rows.len();
                self.execute_insert_batch(batch).await?;
                total_executed += 1;
                total_batched += batched_count;
                pending_batch = None;
            }

            // Try to start a new batch
            if let Some(insert_info) = extract_insert_info(entry) {
                pending_batch = Some(InsertBatch {
                    table: insert_info.table,
                    database: insert_info.database,
                    columns: insert_info.columns,
                    rows: vec![insert_info.values],
                    estimated_size: insert_info.estimated_size,
                });
            } else {
                // Not an INSERT - execute directly
                self.execute_entry(entry).await?;
                total_executed += 1;
            }
        }

        // Flush any remaining batch
        if let Some(batch) = pending_batch {
            let batched_count = batch.rows.len();
            self.execute_insert_batch(&batch).await?;
            total_executed += 1;
            total_batched += batched_count;
        }

        if total_batched > total_executed {
            tracing::info!(
                "Batched {} inserts into {} statements ({:.1}x improvement)",
                total_batched, total_executed,
                total_batched as f64 / total_executed as f64
            );
        }

        Ok(())
    }

    /// Execute a batched INSERT (multi-row INSERT statement)
    async fn execute_insert_batch(&self, batch: &InsertBatch) -> Result<()> {
        if batch.rows.is_empty() {
            return Ok(());
        }

        // Build multi-row INSERT statement
        let cols = batch.columns
            .iter()
            .map(|c| format!("`{}`", c))
            .collect::<Vec<_>>()
            .join(", ");

        let row_values: Vec<String> = batch.rows
            .iter()
            .map(|row| {
                let vals = row.iter()
                    .map(|v| v.to_sql())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({})", vals)
            })
            .collect();

        let sql = format!(
            "INSERT INTO `{}` ({}) VALUES {}",
            batch.table,
            cols,
            row_values.join(", ")
        );

        tracing::debug!(
            "Executing batched INSERT: {} rows into `{}`",
            batch.rows.len(),
            batch.table
        );

        // Get appropriate pool
        let pool_to_use = if let Some(ref db) = batch.database {
            match self.get_or_create_db_pool(db).await {
                Ok(pool) => Some(pool),
                Err(_) => None,
            }
        } else {
            self.pool.read().await.clone()
        };

        if let Some(pool) = pool_to_use {
            sqlx::query(&sql)
                .execute(&pool)
                .await
                .map_err(|e| {
                    Error::QueryExecution(format!(
                        "Batched INSERT to `{}` failed ({} rows): {}",
                        batch.table, batch.rows.len(), e
                    ))
                })?;
        } else if let Some(ref server_pool) = self.server_pool {
            sqlx::query(&sql)
                .execute(server_pool)
                .await
                .map_err(|e| {
                    Error::QueryExecution(format!(
                        "Batched INSERT to `{}` failed (fallback, {} rows): {}",
                        batch.table, batch.rows.len(), e
                    ))
                })?;
        } else {
            return Err(Error::Database(sqlx::Error::Configuration("No pool available".into())));
        }

        Ok(())
    }
}

/// Batch of INSERT statements to be combined into a multi-row INSERT
struct InsertBatch {
    table: String,
    database: Option<String>,
    columns: Vec<String>,
    rows: Vec<Vec<crate::wal::entry::Value>>,
    estimated_size: usize,
}

/// Information extracted from an INSERT entry for batching
struct InsertInfo {
    table: String,
    database: Option<String>,
    columns: Vec<String>,
    values: Vec<crate::wal::entry::Value>,
    estimated_size: usize,
}

/// Extract INSERT information from a LogEntry if it's batchable
fn extract_insert_info(entry: &LogEntry) -> Option<InsertInfo> {
    match entry {
        LogEntry::Insert { table, columns, values, .. } => {
            let estimated_size: usize = values.iter()
                .map(|v| match v {
                    crate::wal::entry::Value::String(s) => s.len(),
                    crate::wal::entry::Value::Bytes(b) => b.len(),
                    _ => 8, // Estimate for numeric types
                })
                .sum();
            
            Some(InsertInfo {
                table: table.clone(),
                database: None,
                columns: columns.clone(),
                values: values.clone(),
                estimated_size,
            })
        }
        LogEntry::RawSql { sql, database, .. } => {
            // Try to parse simple INSERT statements
            parse_raw_insert(sql, database.clone())
        }
        _ => None,
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

/// Extract database name from a CREATE/DROP DATABASE statement
fn extract_database_name_from_ddl(stmt: &str) -> Option<String> {
    let upper = stmt.trim().to_uppercase();
    let stmt_trimmed = stmt.trim();
    
    // Handle DROP DATABASE [IF EXISTS] `dbname`
    // Handle CREATE DATABASE [IF NOT EXISTS] `dbname`
    let keywords = ["DROP DATABASE", "DROP SCHEMA", "CREATE DATABASE", "CREATE SCHEMA"];
    
    for kw in &keywords {
        if upper.starts_with(kw) {
            let rest = &stmt_trimmed[kw.len()..].trim_start();
            
            // Skip IF EXISTS / IF NOT EXISTS
            let rest = if rest.to_uppercase().starts_with("IF EXISTS") {
                rest[9..].trim_start()
            } else if rest.to_uppercase().starts_with("IF NOT EXISTS") {
                rest[13..].trim_start()
            } else {
                rest
            };
            
            // Extract the database name (may be backtick-quoted or unquoted)
            let db_name = if rest.starts_with('`') {
                // Backtick-quoted
                let end = rest[1..].find('`').map(|i| i + 1)?;
                &rest[1..end]
            } else {
                // Unquoted - take until whitespace or semicolon
                rest.split(|c: char| c.is_whitespace() || c == ';')
                    .next()?
            };
            
            if !db_name.is_empty() {
                return Some(db_name.to_string());
            }
        }
    }
    
    None
}

/// Split SQL string on semicolons, respecting string literals
/// This handles cases like: "USE db; CREATE TABLE foo (name VARCHAR(50));"
fn split_sql_statements(sql: &str) -> Vec<&str> {
    let mut statements = Vec::new();
    let mut start = 0;
    let mut in_string = false;
    let mut string_char = '"';
    let mut in_backtick = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut escape_next = false;
    let mut chars = sql.char_indices().peekable();
    
    while let Some((i, c)) = chars.next() {
        // Handle escape sequences (only applies inside strings)
        if escape_next {
            escape_next = false;
            continue;
        }
        
        // Check for end of line comment
        if in_line_comment {
            if c == '\n' {
                in_line_comment = false;
            }
            continue;
        }
        
        // Check for end of block comment
        if in_block_comment {
            if c == '*' && chars.peek().map(|(_, nc)| *nc == '/').unwrap_or(false) {
                chars.next(); // consume the '/'
                in_block_comment = false;
            }
            continue;
        }
        
        // Inside a string literal
        if in_string {
            if c == '\\' {
                escape_next = true;
            } else if c == string_char {
                if chars.peek().map(|(_, nc)| *nc == string_char).unwrap_or(false) {
                    chars.next(); // Skip doubled quote
                } else {
                    in_string = false;
                }
            }
            continue;
        }
        
        // Inside backtick identifier
        if in_backtick {
            if c == '`' {
                if chars.peek().map(|(_, nc)| *nc == '`').unwrap_or(false) {
                    chars.next(); // Skip doubled backtick
                } else {
                    in_backtick = false;
                }
            }
            continue;
        }
        
        // Normal parsing - not in string, comment, or backtick
        match c {
            '\'' | '"' => {
                in_string = true;
                string_char = c;
            }
            '`' => {
                in_backtick = true;
            }
            '-' => {
                // Check for -- line comment
                if chars.peek().map(|(_, nc)| *nc == '-').unwrap_or(false) {
                    chars.next();
                    in_line_comment = true;
                }
            }
            '/' => {
                // Check for /* block comment
                if chars.peek().map(|(_, nc)| *nc == '*').unwrap_or(false) {
                    chars.next();
                    in_block_comment = true;
                }
            }
            '#' => {
                // MySQL also uses # for line comments
                in_line_comment = true;
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
    
    // Add remaining content
    let remaining = sql[start..].trim();
    if !remaining.is_empty() {
        statements.push(remaining);
    }
    
    statements
}

/// Parse a simple INSERT statement from raw SQL for batching.
/// Returns None if the SQL is not a simple INSERT or cannot be parsed.
/// 
/// Supported formats:
/// - INSERT INTO `table` (`col1`, `col2`) VALUES (val1, val2)
/// - INSERT INTO table (col1, col2) VALUES (val1, val2)
fn parse_raw_insert(sql: &str, database: Option<String>) -> Option<InsertInfo> {
    let trimmed = sql.trim();
    let upper = trimmed.to_uppercase();
    
    // Only parse simple single-row INSERTs
    if !upper.starts_with("INSERT INTO ") {
        return None;
    }
    
    // Skip complex INSERT statements that we shouldn't batch:
    // - INSERT ... SELECT
    // - INSERT ... ON DUPLICATE KEY UPDATE
    // - Multi-row VALUES
    if upper.contains(" SELECT ") 
        || upper.contains(" ON DUPLICATE KEY ")
        || upper.matches(" VALUES").count() > 1 
    {
        return None;
    }
    
    // Simple regex-like parsing for INSERT INTO `table` (cols) VALUES (vals)
    // Find table name
    let after_into = &trimmed[12..]; // Skip "INSERT INTO "
    let (table_name, rest) = parse_identifier(after_into)?;
    
    // Find column list
    let rest = rest.trim();
    if !rest.starts_with('(') {
        return None;
    }
    
    let cols_end = find_matching_paren(rest, 0)?;
    let cols_str = &rest[1..cols_end];
    let columns: Vec<String> = cols_str
        .split(',')
        .map(|c| c.trim().trim_matches('`').trim_matches('"').trim_matches('\'').to_string())
        .filter(|c| !c.is_empty())
        .collect();
    
    if columns.is_empty() {
        return None;
    }
    
    // Find VALUES
    let after_cols = rest[cols_end + 1..].trim();
    let after_cols_upper = after_cols.to_uppercase();
    if !after_cols_upper.starts_with("VALUES") {
        return None;
    }
    
    let values_rest = after_cols[6..].trim(); // Skip "VALUES"
    if !values_rest.starts_with('(') {
        return None;
    }
    
    let vals_end = find_matching_paren(values_rest, 0)?;
    let vals_str = &values_rest[1..vals_end];
    
    // Parse values - this is simplified, handles basic cases
    let values = parse_sql_values(vals_str, columns.len())?;
    
    let estimated_size: usize = values.iter()
        .map(|v| match v {
            crate::wal::entry::Value::String(s) => s.len(),
            crate::wal::entry::Value::Bytes(b) => b.len(),
            _ => 8,
        })
        .sum();
    
    Some(InsertInfo {
        table: table_name,
        database,
        columns,
        values,
        estimated_size,
    })
}

/// Parse an identifier (table/column name), handling backticks
fn parse_identifier(s: &str) -> Option<(String, &str)> {
    let s = s.trim();
    if s.starts_with('`') {
        // Backtick-quoted identifier
        let end = s[1..].find('`')?;
        Some((s[1..end + 1].to_string(), &s[end + 2..]))
    } else {
        // Unquoted identifier - ends at whitespace or (
        let end = s.find(|c: char| c.is_whitespace() || c == '(')?;
        Some((s[..end].to_string(), &s[end..]))
    }
}

/// Find the position of the matching closing parenthesis
fn find_matching_paren(s: &str, start: usize) -> Option<usize> {
    if s.chars().nth(start) != Some('(') {
        return None;
    }
    
    let mut depth = 0;
    let mut in_string = false;
    let mut string_char = '"';
    let mut escape_next = false;
    
    for (i, c) in s.char_indices().skip(start) {
        if escape_next {
            escape_next = false;
            continue;
        }
        
        if in_string {
            if c == '\\' {
                escape_next = true;
            } else if c == string_char {
                in_string = false;
            }
            continue;
        }
        
        match c {
            '\'' | '"' => {
                in_string = true;
                string_char = c;
            }
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    
    None
}

/// Parse SQL VALUES into Value types
fn parse_sql_values(vals_str: &str, expected_count: usize) -> Option<Vec<crate::wal::entry::Value>> {
    let mut values = Vec::with_capacity(expected_count);
    let mut current = String::new();
    let mut in_string = false;
    let mut string_char = '\'';
    let mut escape_next = false;
    let mut depth = 0;
    
    for c in vals_str.chars() {
        if escape_next {
            current.push(c);
            escape_next = false;
            continue;
        }
        
        if in_string {
            if c == '\\' {
                escape_next = true;
                current.push(c);
            } else if c == string_char {
                in_string = false;
                current.push(c);
            } else {
                current.push(c);
            }
            continue;
        }
        
        match c {
            '\'' | '"' => {
                in_string = true;
                string_char = c;
                current.push(c);
            }
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 => {
                if let Some(val) = parse_single_value(current.trim()) {
                    values.push(val);
                } else {
                    return None;
                }
                current.clear();
            }
            _ => {
                current.push(c);
            }
        }
    }
    
    // Push last value
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        if let Some(val) = parse_single_value(trimmed) {
            values.push(val);
        } else {
            return None;
        }
    }
    
    if values.len() == expected_count {
        Some(values)
    } else {
        None
    }
}

/// Parse a single SQL value literal into Value type
fn parse_single_value(s: &str) -> Option<crate::wal::entry::Value> {
    let s = s.trim();
    
    if s.eq_ignore_ascii_case("NULL") {
        return Some(crate::wal::entry::Value::Null);
    }
    
    // String literal
    if (s.starts_with('\'') && s.ends_with('\'')) || (s.starts_with('"') && s.ends_with('"')) {
        let inner = &s[1..s.len() - 1];
        // Unescape simple escapes
        let unescaped = inner.replace("\\'", "'").replace("\\\"", "\"").replace("\\\\", "\\");
        return Some(crate::wal::entry::Value::String(unescaped));
    }
    
    // Hex blob X'...'
    if s.starts_with("X'") && s.ends_with('\'') {
        // Skip parsing hex blobs for now - they're complex
        return None;
    }
    
    // Integer
    if let Ok(i) = s.parse::<i64>() {
        return Some(crate::wal::entry::Value::Int(i));
    }
    
    // Float
    if let Ok(f) = s.parse::<f64>() {
        return Some(crate::wal::entry::Value::Float(f));
    }
    
    // Unable to parse - skip batching this statement
    None
}
