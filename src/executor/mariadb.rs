//! MariaDB Executor
//!
//! Executes log entries against a MariaDB database.

use std::time::Duration;
use sqlx::{MySqlPool, Row};
use sqlx::mysql::MySqlPoolOptions;

use crate::config::DatabaseConfig;
use crate::wal::LogEntry;
use crate::error::{Error, Result};

/// MariaDB executor for applying log entries
pub struct MariaDbExecutor {
    /// Connection pool
    pool: Option<MySqlPool>,
    /// Whether this is a mock executor (for testing)
    is_mock: bool,
}

impl MariaDbExecutor {
    /// Create a new executor with a connection pool
    pub async fn new(config: &DatabaseConfig) -> Result<Self> {
        let url = format!(
            "mysql://{}:{}@{}:{}/{}",
            config.user,
            config.password,
            config.host,
            config.port,
            config.database
        );

        let pool = MySqlPoolOptions::new()
            .max_connections(config.pool_size)
            .acquire_timeout(Duration::from_secs(config.connect_timeout_secs))
            .connect(&url)
            .await?;

        Ok(Self {
            pool: Some(pool),
            is_mock: false,
        })
    }

    /// Create a mock executor for testing
    pub fn new_mock() -> Self {
        Self {
            pool: None,
            is_mock: true,
        }
    }

    /// Execute a log entry
    pub async fn execute_entry(&self, entry: &LogEntry) -> Result<()> {
        if self.is_mock {
            return Ok(());
        }

        let pool = self.pool.as_ref().ok_or_else(|| {
            Error::Database(sqlx::Error::Configuration("No pool".into()))
        })?;

        let statements = entry.to_sql();
        
        for sql in statements {
            if sql.is_empty() {
                continue;
            }

            tracing::debug!("Executing: {}", &sql[..sql.len().min(100)]);
            
            sqlx::query(&sql)
                .execute(pool)
                .await
                .map_err(|e| {
                    Error::QueryExecution(format!("Failed to execute '{}...': {}", 
                        &sql[..sql.len().min(50)], e))
                })?;
        }

        Ok(())
    }

    /// Execute a raw SQL statement
    pub async fn execute_raw(&self, sql: &str) -> Result<u64> {
        if self.is_mock {
            return Ok(0);
        }

        let pool = self.pool.as_ref().ok_or_else(|| {
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

        let pool = self.pool.as_ref().ok_or_else(|| {
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

        let pool = self.pool.as_ref().ok_or_else(|| {
            Error::Database(sqlx::Error::Configuration("No pool".into()))
        })?;

        let result: (i32,) = sqlx::query_as("SELECT 1")
            .fetch_one(pool)
            .await?;

        Ok(result.0 == 1)
    }

    /// Get list of tables in the database
    pub async fn list_tables(&self) -> Result<Vec<String>> {
        if self.is_mock {
            return Ok(vec![]);
        }

        let pool = self.pool.as_ref().ok_or_else(|| {
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

        let pool = self.pool.as_ref().ok_or_else(|| {
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

        let pool = self.pool.as_ref().ok_or_else(|| {
            Error::Database(sqlx::Error::Configuration("No pool".into()))
        })?;

        let row: (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM `{}`", table))
            .fetch_one(pool)
            .await?;

        Ok(row.0 as u64)
    }

    /// Close the connection pool
    pub async fn close(&self) {
        if let Some(pool) = &self.pool {
            pool.close().await;
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
