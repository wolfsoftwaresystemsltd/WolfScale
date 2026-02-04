//! Binlog Event Types
//!
//! Defines the various binlog event types and their parsing.

use std::collections::HashMap;

/// Binlog event types we care about
#[derive(Debug, Clone)]
pub enum BinlogEvent {
    /// Query event - contains raw SQL (DDL, etc.)
    Query {
        database: String,
        query: String,
    },
    /// Table map event - maps table_id to database.table
    TableMap {
        table_id: u64,
        database: String,
        table: String,
        column_count: usize,
    },
    /// Write rows (INSERT)
    WriteRows {
        table_id: u64,
        rows: Vec<Vec<u8>>,
    },
    /// Update rows (UPDATE)
    UpdateRows {
        table_id: u64,
        before_rows: Vec<Vec<u8>>,
        after_rows: Vec<Vec<u8>>,
    },
    /// Delete rows (DELETE)
    DeleteRows {
        table_id: u64,
        rows: Vec<Vec<u8>>,
    },
    /// Rotate event - binlog file changed
    Rotate {
        next_file: String,
        position: u64,
    },
    /// Format description event - contains binlog format info
    FormatDescription {
        binlog_version: u16,
        server_version: String,
    },
    /// XID event - transaction commit
    Xid {
        xid: u64,
    },
    /// GTID event (MariaDB)
    Gtid {
        domain_id: u32,
        server_id: u32,
        sequence: u64,
    },
    /// Unknown/unhandled event
    Unknown {
        type_code: u8,
    },
}

/// Binlog event type codes (MariaDB/MySQL)
#[allow(dead_code)]
pub mod event_type {
    pub const UNKNOWN_EVENT: u8 = 0;
    pub const START_EVENT_V3: u8 = 1;
    pub const QUERY_EVENT: u8 = 2;
    pub const STOP_EVENT: u8 = 3;
    pub const ROTATE_EVENT: u8 = 4;
    pub const INTVAR_EVENT: u8 = 5;
    pub const LOAD_EVENT: u8 = 6;
    pub const SLAVE_EVENT: u8 = 7;
    pub const CREATE_FILE_EVENT: u8 = 8;
    pub const APPEND_BLOCK_EVENT: u8 = 9;
    pub const EXEC_LOAD_EVENT: u8 = 10;
    pub const DELETE_FILE_EVENT: u8 = 11;
    pub const NEW_LOAD_EVENT: u8 = 12;
    pub const RAND_EVENT: u8 = 13;
    pub const USER_VAR_EVENT: u8 = 14;
    pub const FORMAT_DESCRIPTION_EVENT: u8 = 15;
    pub const XID_EVENT: u8 = 16;
    pub const BEGIN_LOAD_QUERY_EVENT: u8 = 17;
    pub const EXECUTE_LOAD_QUERY_EVENT: u8 = 18;
    pub const TABLE_MAP_EVENT: u8 = 19;
    pub const PRE_GA_WRITE_ROWS_EVENT: u8 = 20;
    pub const PRE_GA_UPDATE_ROWS_EVENT: u8 = 21;
    pub const PRE_GA_DELETE_ROWS_EVENT: u8 = 22;
    pub const WRITE_ROWS_EVENT_V1: u8 = 23;
    pub const UPDATE_ROWS_EVENT_V1: u8 = 24;
    pub const DELETE_ROWS_EVENT_V1: u8 = 25;
    pub const INCIDENT_EVENT: u8 = 26;
    pub const HEARTBEAT_LOG_EVENT: u8 = 27;
    pub const IGNORABLE_LOG_EVENT: u8 = 28;
    pub const ROWS_QUERY_LOG_EVENT: u8 = 29;
    pub const WRITE_ROWS_EVENT: u8 = 30;
    pub const UPDATE_ROWS_EVENT: u8 = 31;
    pub const DELETE_ROWS_EVENT: u8 = 32;
    pub const GTID_LOG_EVENT: u8 = 33;
    pub const ANONYMOUS_GTID_LOG_EVENT: u8 = 34;
    pub const PREVIOUS_GTIDS_LOG_EVENT: u8 = 35;
    
    // MariaDB specific
    pub const MARIADB_ANNOTATE_ROWS_EVENT: u8 = 160;
    pub const MARIADB_BINLOG_CHECKPOINT_EVENT: u8 = 161;
    pub const MARIADB_GTID_EVENT: u8 = 162;
    pub const MARIADB_GTID_LIST_EVENT: u8 = 163;
    pub const MARIADB_START_ENCRYPTION_EVENT: u8 = 164;
}

/// Table map cache - maps table_id to (database, table, column_count)
#[derive(Debug, Default)]
pub struct TableMap {
    tables: HashMap<u64, (String, String, usize)>,
}

impl TableMap {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn insert(&mut self, table_id: u64, database: String, table: String, column_count: usize) {
        self.tables.insert(table_id, (database, table, column_count));
    }
    
    pub fn get(&self, table_id: u64) -> Option<&(String, String, usize)> {
        self.tables.get(&table_id)
    }
}

/// Parse a binlog event from raw bytes
pub fn parse_event(data: &[u8]) -> Result<BinlogEvent, String> {
    if data.len() < 19 {
        return Err("Event too short".to_string());
    }
    
    // Binlog event header (19 bytes for v4):
    // 4 bytes: timestamp
    // 1 byte: type_code
    // 4 bytes: server_id
    // 4 bytes: event_length
    // 4 bytes: next_position
    // 2 bytes: flags
    
    let type_code = data[4];
    let event_length = u32::from_le_bytes([data[9], data[10], data[11], data[12]]) as usize;
    
    if data.len() < event_length {
        return Err(format!("Incomplete event: have {} bytes, need {}", data.len(), event_length));
    }
    
    let payload = &data[19..event_length];
    
    match type_code {
        event_type::QUERY_EVENT => parse_query_event(payload),
        event_type::TABLE_MAP_EVENT => parse_table_map_event(payload),
        event_type::WRITE_ROWS_EVENT | event_type::WRITE_ROWS_EVENT_V1 => {
            parse_write_rows_event(payload)
        }
        event_type::UPDATE_ROWS_EVENT | event_type::UPDATE_ROWS_EVENT_V1 => {
            parse_update_rows_event(payload)
        }
        event_type::DELETE_ROWS_EVENT | event_type::DELETE_ROWS_EVENT_V1 => {
            parse_delete_rows_event(payload)
        }
        event_type::ROTATE_EVENT => parse_rotate_event(payload),
        event_type::FORMAT_DESCRIPTION_EVENT => parse_format_description_event(payload),
        event_type::XID_EVENT => parse_xid_event(payload),
        event_type::MARIADB_GTID_EVENT => parse_mariadb_gtid_event(payload),
        _ => Ok(BinlogEvent::Unknown { type_code }),
    }
}

fn parse_query_event(data: &[u8]) -> Result<BinlogEvent, String> {
    if data.len() < 13 {
        return Err("Query event too short".to_string());
    }
    
    // Query event post-header:
    // 4 bytes: thread_id
    // 4 bytes: execution_time
    // 1 byte: schema_length
    // 2 bytes: error_code
    // 2 bytes: status_vars_length (v4)
    
    let schema_length = data[8] as usize;
    let status_vars_length = u16::from_le_bytes([data[11], data[12]]) as usize;
    
    let schema_start = 13 + status_vars_length;
    if data.len() < schema_start + schema_length + 1 {
        return Err("Query event truncated".to_string());
    }
    
    let database = String::from_utf8_lossy(&data[schema_start..schema_start + schema_length]).to_string();
    let query_start = schema_start + schema_length + 1; // +1 for null terminator
    let query = String::from_utf8_lossy(&data[query_start..]).to_string();
    
    Ok(BinlogEvent::Query { database, query })
}

fn parse_table_map_event(data: &[u8]) -> Result<BinlogEvent, String> {
    if data.len() < 8 {
        return Err("Table map event too short".to_string());
    }
    
    // 6 bytes: table_id
    // 2 bytes: flags
    // 1 byte: schema_name_length
    // schema_name
    // 1 byte: null terminator
    // 1 byte: table_name_length
    // table_name
    // 1 byte: null terminator
    // ... column info
    
    let table_id = u64::from_le_bytes([data[0], data[1], data[2], data[3], data[4], data[5], 0, 0]);
    
    let schema_len = data[8] as usize;
    if data.len() < 10 + schema_len {
        return Err("Table map event truncated".to_string());
    }
    
    let database = String::from_utf8_lossy(&data[9..9 + schema_len]).to_string();
    
    let table_len_pos = 9 + schema_len + 1;
    if data.len() < table_len_pos + 1 {
        return Err("Table map event truncated".to_string());
    }
    
    let table_len = data[table_len_pos] as usize;
    let table = String::from_utf8_lossy(&data[table_len_pos + 1..table_len_pos + 1 + table_len]).to_string();
    
    // Column count is length-encoded after table name
    let col_count_pos = table_len_pos + 1 + table_len + 1;
    let column_count = if col_count_pos < data.len() {
        data[col_count_pos] as usize
    } else {
        0
    };
    
    Ok(BinlogEvent::TableMap {
        table_id,
        database,
        table,
        column_count,
    })
}

fn parse_write_rows_event(data: &[u8]) -> Result<BinlogEvent, String> {
    if data.len() < 8 {
        return Err("Write rows event too short".to_string());
    }
    
    let table_id = u64::from_le_bytes([data[0], data[1], data[2], data[3], data[4], data[5], 0, 0]);
    
    // For simplicity, we'll store the raw row data
    // A full implementation would parse the column bitmap and row data
    Ok(BinlogEvent::WriteRows {
        table_id,
        rows: vec![data[8..].to_vec()],
    })
}

fn parse_update_rows_event(data: &[u8]) -> Result<BinlogEvent, String> {
    if data.len() < 8 {
        return Err("Update rows event too short".to_string());
    }
    
    let table_id = u64::from_le_bytes([data[0], data[1], data[2], data[3], data[4], data[5], 0, 0]);
    
    Ok(BinlogEvent::UpdateRows {
        table_id,
        before_rows: vec![],
        after_rows: vec![data[8..].to_vec()],
    })
}

fn parse_delete_rows_event(data: &[u8]) -> Result<BinlogEvent, String> {
    if data.len() < 8 {
        return Err("Delete rows event too short".to_string());
    }
    
    let table_id = u64::from_le_bytes([data[0], data[1], data[2], data[3], data[4], data[5], 0, 0]);
    
    Ok(BinlogEvent::DeleteRows {
        table_id,
        rows: vec![data[8..].to_vec()],
    })
}

fn parse_rotate_event(data: &[u8]) -> Result<BinlogEvent, String> {
    if data.len() < 8 {
        return Err("Rotate event too short".to_string());
    }
    
    let position = u64::from_le_bytes([data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]]);
    let next_file = String::from_utf8_lossy(&data[8..]).trim_end_matches('\0').to_string();
    
    Ok(BinlogEvent::Rotate { next_file, position })
}

fn parse_format_description_event(data: &[u8]) -> Result<BinlogEvent, String> {
    if data.len() < 57 {
        return Err("Format description event too short".to_string());
    }
    
    let binlog_version = u16::from_le_bytes([data[0], data[1]]);
    let server_version = String::from_utf8_lossy(&data[2..52]).trim_end_matches('\0').to_string();
    
    Ok(BinlogEvent::FormatDescription {
        binlog_version,
        server_version,
    })
}

fn parse_xid_event(data: &[u8]) -> Result<BinlogEvent, String> {
    if data.len() < 8 {
        return Err("XID event too short".to_string());
    }
    
    let xid = u64::from_le_bytes([data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]]);
    
    Ok(BinlogEvent::Xid { xid })
}

fn parse_mariadb_gtid_event(data: &[u8]) -> Result<BinlogEvent, String> {
    if data.len() < 13 {
        return Err("GTID event too short".to_string());
    }
    
    let sequence = u64::from_le_bytes([data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]]);
    let domain_id = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    // Server ID is in the event header, not here
    
    Ok(BinlogEvent::Gtid {
        domain_id,
        server_id: 0, // Would need to be extracted from header
        sequence,
    })
}
