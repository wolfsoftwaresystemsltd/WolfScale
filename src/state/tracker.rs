//! State Tracker
//!
//! Persistent storage for node state, tracking which log entries
//! have been applied to the local database.

use std::path::PathBuf;
use rusqlite::{Connection, params};
use tokio::sync::RwLock;

use crate::wal::entry::Lsn;
use crate::error::{Error, Result};

/// Persistent state tracker backed by SQLite
pub struct StateTracker {
    /// Database connection
    conn: RwLock<Connection>,
    /// Node ID
    node_id: String,
}

impl StateTracker {
    /// Create or open the state tracker database
    pub fn new(data_dir: PathBuf, node_id: String) -> Result<Self> {
        std::fs::create_dir_all(&data_dir)?;
        
        let db_path = data_dir.join("state.db");
        let conn = Connection::open(&db_path)?;

        // Initialize schema
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS node_state (
                key TEXT PRIMARY KEY,
                value_int INTEGER,
                value_text TEXT,
                updated_at TEXT DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS applied_entries (
                lsn INTEGER PRIMARY KEY,
                table_name TEXT NOT NULL,
                primary_key TEXT NOT NULL,
                applied_at TEXT DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_applied_entries_table 
                ON applied_entries(table_name);

            CREATE TABLE IF NOT EXISTS table_watermarks (
                table_name TEXT PRIMARY KEY,
                last_lsn INTEGER NOT NULL,
                updated_at TEXT DEFAULT CURRENT_TIMESTAMP
            );
            "#,
        )?;

        Ok(Self {
            conn: RwLock::new(conn),
            node_id,
        })
    }

    /// Get the last applied LSN
    pub async fn last_applied_lsn(&self) -> Result<Lsn> {
        let conn = self.conn.read().await;
        let result: std::result::Result<i64, _> = conn.query_row(
            "SELECT value_int FROM node_state WHERE key = 'last_applied_lsn'",
            [],
            |row| row.get(0),
        );

        match result {
            Ok(lsn) => Ok(lsn as Lsn),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
            Err(e) => Err(Error::State(format!("Failed to get last LSN: {}", e))),
        }
    }

    /// Set the last applied LSN
    pub async fn set_last_applied_lsn(&self, lsn: Lsn) -> Result<()> {
        let conn = self.conn.write().await;
        conn.execute(
            r#"
            INSERT INTO node_state (key, value_int) VALUES ('last_applied_lsn', ?1)
            ON CONFLICT(key) DO UPDATE SET value_int = ?1, updated_at = CURRENT_TIMESTAMP
            "#,
            params![lsn as i64],
        )?;
        Ok(())
    }

    /// Get the current term
    pub async fn current_term(&self) -> Result<u64> {
        let conn = self.conn.read().await;
        let result: std::result::Result<i64, _> = conn.query_row(
            "SELECT value_int FROM node_state WHERE key = 'current_term'",
            [],
            |row| row.get(0),
        );

        match result {
            Ok(term) => Ok(term as u64),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(1),
            Err(e) => Err(Error::State(format!("Failed to get term: {}", e))),
        }
    }

    /// Set the current term
    pub async fn set_current_term(&self, term: u64) -> Result<()> {
        let conn = self.conn.write().await;
        conn.execute(
            r#"
            INSERT INTO node_state (key, value_int) VALUES ('current_term', ?1)
            ON CONFLICT(key) DO UPDATE SET value_int = ?1, updated_at = CURRENT_TIMESTAMP
            "#,
            params![term as i64],
        )?;
        Ok(())
    }

    /// Get the voted-for node ID (for leader election)
    pub async fn voted_for(&self) -> Result<Option<String>> {
        let conn = self.conn.read().await;
        let result: std::result::Result<String, _> = conn.query_row(
            "SELECT value_text FROM node_state WHERE key = 'voted_for'",
            [],
            |row| row.get(0),
        );

        match result {
            Ok(node_id) => Ok(Some(node_id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(Error::State(format!("Failed to get voted_for: {}", e))),
        }
    }

    /// Set the voted-for node ID
    pub async fn set_voted_for(&self, node_id: Option<&str>) -> Result<()> {
        let conn = self.conn.write().await;
        match node_id {
            Some(id) => {
                conn.execute(
                    r#"
                    INSERT INTO node_state (key, value_text) VALUES ('voted_for', ?1)
                    ON CONFLICT(key) DO UPDATE SET value_text = ?1, updated_at = CURRENT_TIMESTAMP
                    "#,
                    params![id],
                )?;
            }
            None => {
                conn.execute(
                    "DELETE FROM node_state WHERE key = 'voted_for'",
                    [],
                )?;
            }
        }
        Ok(())
    }

    /// Get the current leader ID
    pub async fn current_leader(&self) -> Result<Option<String>> {
        let conn = self.conn.read().await;
        let result: std::result::Result<String, _> = conn.query_row(
            "SELECT value_text FROM node_state WHERE key = 'current_leader'",
            [],
            |row| row.get(0),
        );

        match result {
            Ok(leader_id) => Ok(Some(leader_id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(Error::State(format!("Failed to get leader: {}", e))),
        }
    }

    /// Set the current leader ID
    pub async fn set_current_leader(&self, leader_id: Option<&str>) -> Result<()> {
        let conn = self.conn.write().await;
        match leader_id {
            Some(id) => {
                conn.execute(
                    r#"
                    INSERT INTO node_state (key, value_text) VALUES ('current_leader', ?1)
                    ON CONFLICT(key) DO UPDATE SET value_text = ?1, updated_at = CURRENT_TIMESTAMP
                    "#,
                    params![id],
                )?;
            }
            None => {
                conn.execute(
                    "DELETE FROM node_state WHERE key = 'current_leader'",
                    [],
                )?;
            }
        }
        Ok(())
    }

    /// Record that an entry has been applied
    pub async fn record_applied(
        &self,
        lsn: Lsn,
        table_name: &str,
        primary_key: &str,
    ) -> Result<()> {
        let conn = self.conn.write().await;
        conn.execute(
            r#"
            INSERT OR REPLACE INTO applied_entries (lsn, table_name, primary_key)
            VALUES (?1, ?2, ?3)
            "#,
            params![lsn as i64, table_name, primary_key],
        )?;

        // Update table watermark
        conn.execute(
            r#"
            INSERT INTO table_watermarks (table_name, last_lsn)
            VALUES (?1, ?2)
            ON CONFLICT(table_name) DO UPDATE SET 
                last_lsn = MAX(last_lsn, ?2),
                updated_at = CURRENT_TIMESTAMP
            "#,
            params![table_name, lsn as i64],
        )?;

        Ok(())
    }

    /// Check if an entry has been applied
    pub async fn is_applied(&self, lsn: Lsn) -> Result<bool> {
        let conn = self.conn.read().await;
        let result: std::result::Result<i64, _> = conn.query_row(
            "SELECT COUNT(*) FROM applied_entries WHERE lsn = ?1",
            params![lsn as i64],
            |row| row.get(0),
        );

        match result {
            Ok(count) => Ok(count > 0),
            Err(e) => Err(Error::State(format!("Failed to check applied: {}", e))),
        }
    }

    /// Get the watermark (last applied LSN) for a table
    pub async fn table_watermark(&self, table_name: &str) -> Result<Lsn> {
        let conn = self.conn.read().await;
        let result: std::result::Result<i64, _> = conn.query_row(
            "SELECT last_lsn FROM table_watermarks WHERE table_name = ?1",
            params![table_name],
            |row| row.get(0),
        );

        match result {
            Ok(lsn) => Ok(lsn as Lsn),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
            Err(e) => Err(Error::State(format!("Failed to get watermark: {}", e))),
        }
    }

    /// Get all table watermarks
    pub async fn all_watermarks(&self) -> Result<Vec<(String, Lsn)>> {
        let conn = self.conn.read().await;
        let mut stmt = conn.prepare("SELECT table_name, last_lsn FROM table_watermarks")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })?;

        let mut watermarks = Vec::new();
        for result in rows {
            watermarks.push(result?);
        }

        Ok(watermarks)
    }

    /// Get count of applied entries
    pub async fn applied_count(&self) -> Result<u64> {
        let conn = self.conn.read().await;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM applied_entries",
            [],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// Get entries applied within a LSN range
    pub async fn applied_in_range(&self, from_lsn: Lsn, to_lsn: Lsn) -> Result<Vec<Lsn>> {
        let conn = self.conn.read().await;
        let mut stmt = conn.prepare(
            "SELECT lsn FROM applied_entries WHERE lsn >= ?1 AND lsn <= ?2 ORDER BY lsn"
        )?;
        let rows = stmt.query_map(params![from_lsn as i64, to_lsn as i64], |row| {
            Ok(row.get::<_, i64>(0)? as u64)
        })?;

        let mut lsns = Vec::new();
        for result in rows {
            lsns.push(result?);
        }

        Ok(lsns)
    }

    /// Clean up old applied entries (for retention)
    pub async fn cleanup_before(&self, lsn: Lsn) -> Result<u64> {
        let conn = self.conn.write().await;
        let deleted = conn.execute(
            "DELETE FROM applied_entries WHERE lsn < ?1",
            params![lsn as i64],
        )?;
        Ok(deleted as u64)
    }

    /// Get node ID
    pub fn node_id(&self) -> &str {
        &self.node_id
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::State(format!("SQLite error: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_state_tracker_basic() {
        let dir = tempdir().unwrap();
        let tracker = StateTracker::new(
            dir.path().to_path_buf(),
            "test-node".to_string(),
        ).unwrap();

        // Test LSN tracking
        assert_eq!(tracker.last_applied_lsn().await.unwrap(), 0);
        tracker.set_last_applied_lsn(100).await.unwrap();
        assert_eq!(tracker.last_applied_lsn().await.unwrap(), 100);

        // Test term
        assert_eq!(tracker.current_term().await.unwrap(), 1);
        tracker.set_current_term(5).await.unwrap();
        assert_eq!(tracker.current_term().await.unwrap(), 5);
    }

    #[tokio::test]
    async fn test_applied_entries() {
        let dir = tempdir().unwrap();
        let tracker = StateTracker::new(
            dir.path().to_path_buf(),
            "test-node".to_string(),
        ).unwrap();

        tracker.record_applied(1, "users", "1").await.unwrap();
        tracker.record_applied(2, "users", "2").await.unwrap();
        tracker.record_applied(3, "orders", "1").await.unwrap();

        assert!(tracker.is_applied(1).await.unwrap());
        assert!(tracker.is_applied(2).await.unwrap());
        assert!(!tracker.is_applied(10).await.unwrap());

        assert_eq!(tracker.table_watermark("users").await.unwrap(), 2);
        assert_eq!(tracker.table_watermark("orders").await.unwrap(), 3);
    }

    #[tokio::test]
    async fn test_voted_for() {
        let dir = tempdir().unwrap();
        let tracker = StateTracker::new(
            dir.path().to_path_buf(),
            "test-node".to_string(),
        ).unwrap();

        assert!(tracker.voted_for().await.unwrap().is_none());
        
        tracker.set_voted_for(Some("node-2")).await.unwrap();
        assert_eq!(tracker.voted_for().await.unwrap(), Some("node-2".to_string()));

        tracker.set_voted_for(None).await.unwrap();
        assert!(tracker.voted_for().await.unwrap().is_none());
    }
}
