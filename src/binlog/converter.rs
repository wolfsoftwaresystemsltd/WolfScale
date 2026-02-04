//! Binlog to WAL Converter
//!
//! Converts binlog events to WolfScale WAL entries.

use super::event::{BinlogEvent, TableMap};
use crate::wal::LogEntry;

/// Convert a binlog event to a WAL LogEntry
pub fn binlog_to_wal(event: BinlogEvent, table_map: &TableMap) -> Option<LogEntry> {
    match event {
        BinlogEvent::Query { database, query } => {
            // Skip internal queries
            if query.starts_with("BEGIN") || query.starts_with("COMMIT") || query.starts_with("ROLLBACK") {
                return None;
            }
            
            // Skip Galera internal queries
            if query.contains("wsrep") || query.contains("WSREP") {
                return None;
            }
            
            // Skip SET statements that are session-specific
            if query.starts_with("SET ") && !query.contains("@@global") {
                return None;
            }
            
            Some(LogEntry::RawSql {
                sql: query,
                database: Some(database),
                affects_table: None,
            })
        }
        
        BinlogEvent::WriteRows { table_id, rows: _ } => {
            // Look up table info from table map
            if let Some((database, table, _col_count)) = table_map.get(table_id) {
                // For ROW format, we'd need to reconstruct the INSERT statement
                // This is complex - for now, we'll log a placeholder
                // A full implementation would decode the row data
                tracing::debug!(
                    "WriteRows event for {}.{} - row-based events require schema info",
                    database, table
                );
                
                // Return None for now - row events need full column type info to decode
                // The Query events will capture DDL, and STATEMENT format will work
                None
            } else {
                tracing::warn!("WriteRows for unknown table_id {}", table_id);
                None
            }
        }
        
        BinlogEvent::UpdateRows { table_id, .. } => {
            if let Some((database, table, _)) = table_map.get(table_id) {
                tracing::debug!(
                    "UpdateRows event for {}.{} - row-based events require schema info",
                    database, table
                );
            }
            None
        }
        
        BinlogEvent::DeleteRows { table_id, .. } => {
            if let Some((database, table, _)) = table_map.get(table_id) {
                tracing::debug!(
                    "DeleteRows event for {}.{} - row-based events require schema info",
                    database, table
                );
            }
            None
        }
        
        // These events don't produce WAL entries
        BinlogEvent::TableMap { .. } => None,
        BinlogEvent::Rotate { .. } => None,
        BinlogEvent::FormatDescription { .. } => None,
        BinlogEvent::Xid { .. } => None,
        BinlogEvent::Gtid { .. } => None,
        BinlogEvent::Unknown { .. } => None,
    }
}

/// Check if a query should be replicated
pub fn should_replicate_query(query: &str) -> bool {
    let query_upper = query.trim().to_uppercase();
    
    // Skip transaction control
    if query_upper.starts_with("BEGIN") 
        || query_upper.starts_with("COMMIT") 
        || query_upper.starts_with("ROLLBACK")
        || query_upper.starts_with("SAVEPOINT")
        || query_upper.starts_with("RELEASE SAVEPOINT")
    {
        return false;
    }
    
    // Skip session-specific SET statements
    if query_upper.starts_with("SET ") 
        && !query_upper.contains("@@GLOBAL")
        && !query_upper.starts_with("SET NAMES")
        && !query_upper.starts_with("SET CHARACTER")
    {
        return false;
    }
    
    // Skip internal queries
    if query_upper.contains("INFORMATION_SCHEMA")
        || query_upper.contains("PERFORMANCE_SCHEMA")
        || query_upper.contains("MYSQL.")
    {
        return false;
    }
    
    // Replicate everything else (DDL, DML, etc.)
    true
}
