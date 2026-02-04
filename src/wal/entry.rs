//! WAL Log Entry Types
//!
//! Defines the structure of log entries that are written to the WAL
//! and replicated across nodes.

use serde::{Deserialize, Serialize};

/// Log Sequence Number - unique identifier for each log entry
pub type Lsn = u64;

/// Primary key representation supporting multiple column types
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrimaryKey {
    /// Single integer primary key
    Int(i64),
    /// Single string primary key
    String(String),
    /// Single UUID primary key
    Uuid(uuid::Uuid),
    /// Composite primary key (multiple columns)
    Composite(Vec<Value>),
}

impl PrimaryKey {
    /// Convert to SQL WHERE clause fragment
    pub fn to_where_clause(&self, key_columns: &[String]) -> String {
        const DEFAULT_COL: &str = "id";
        match self {
            PrimaryKey::Int(v) => {
                let col = key_columns.first().map(|s| s.as_str()).unwrap_or(DEFAULT_COL);
                format!("`{}` = {}", col, v)
            }
            PrimaryKey::String(v) => {
                let col = key_columns.first().map(|s| s.as_str()).unwrap_or(DEFAULT_COL);
                format!("`{}` = '{}'", col, v.replace('\'', "''"))
            }
            PrimaryKey::Uuid(v) => {
                let col = key_columns.first().map(|s| s.as_str()).unwrap_or(DEFAULT_COL);
                format!("`{}` = '{}'", col, v)
            }
            PrimaryKey::Composite(values) => {
                let clauses: Vec<String> = key_columns
                    .iter()
                    .zip(values.iter())
                    .map(|(col, val)| format!("`{}` = {}", col, val.to_sql()))
                    .collect();
                clauses.join(" AND ")
            }
        }
    }
}

impl std::fmt::Display for PrimaryKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrimaryKey::Int(v) => write!(f, "{}", v),
            PrimaryKey::String(v) => write!(f, "{}", v),
            PrimaryKey::Uuid(v) => write!(f, "{}", v),
            PrimaryKey::Composite(v) => {
                let parts: Vec<String> = v.iter().map(|x| x.to_string()).collect();
                write!(f, "({})", parts.join(", "))
            }
        }
    }
}

/// SQL Value representation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    Uuid(uuid::Uuid),
    Timestamp(chrono::DateTime<chrono::Utc>),
    Json(serde_json::Value),
}

impl Value {
    /// Convert to SQL literal
    pub fn to_sql(&self) -> String {
        match self {
            Value::Null => "NULL".to_string(),
            Value::Bool(b) => if *b { "1" } else { "0" }.to_string(),
            Value::Int(i) => i.to_string(),
            Value::UInt(u) => u.to_string(),
            Value::Float(f) => f.to_string(),
            Value::String(s) => format!("'{}'", s.replace('\'', "''")),
            Value::Bytes(b) => format!("X'{}'", hex::encode(b)),
            Value::Uuid(u) => format!("'{}'", u),
            Value::Timestamp(t) => format!("'{}'", t.format("%Y-%m-%d %H:%M:%S%.6f")),
            Value::Json(j) => format!("'{}'", j.to_string().replace('\'', "''")),
        }
    }

    /// Check if value is NULL
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_sql())
    }
}

impl Eq for Value {}

impl std::hash::Hash for Value {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Value::Null => 0u8.hash(state),
            Value::Bool(b) => {
                1u8.hash(state);
                b.hash(state);
            }
            Value::Int(i) => {
                2u8.hash(state);
                i.hash(state);
            }
            Value::UInt(u) => {
                3u8.hash(state);
                u.hash(state);
            }
            Value::Float(f) => {
                4u8.hash(state);
                f.to_bits().hash(state);
            }
            Value::String(s) => {
                5u8.hash(state);
                s.hash(state);
            }
            Value::Bytes(b) => {
                6u8.hash(state);
                b.hash(state);
            }
            Value::Uuid(u) => {
                7u8.hash(state);
                u.hash(state);
            }
            Value::Timestamp(t) => {
                8u8.hash(state);
                t.timestamp_nanos_opt().hash(state);
            }
            Value::Json(j) => {
                9u8.hash(state);
                j.to_string().hash(state);
            }
        }
    }
}

/// Log entry header containing metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryHeader {
    /// Log Sequence Number
    pub lsn: Lsn,
    /// Term (for leader election)
    pub term: u64,
    /// Timestamp when entry was created
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Node ID that created this entry
    pub origin_node: String,
    /// CRC32 checksum of the entry body
    pub checksum: u32,
    /// Size of the entry body in bytes
    pub body_size: u32,
    /// Whether the body is compressed
    pub compressed: bool,
}

/// Log entry types representing database operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogEntry {
    /// Insert a new row
    Insert {
        table: String,
        columns: Vec<String>,
        values: Vec<Value>,
        primary_key: PrimaryKey,
    },

    /// Update existing row(s)
    Update {
        table: String,
        set_columns: Vec<String>,
        set_values: Vec<Value>,
        primary_key: PrimaryKey,
        key_columns: Vec<String>,
    },

    /// Delete row(s)
    Delete {
        table: String,
        primary_key: PrimaryKey,
        key_columns: Vec<String>,
    },

    /// Upsert (INSERT ... ON DUPLICATE KEY UPDATE)
    Upsert {
        table: String,
        columns: Vec<String>,
        values: Vec<Value>,
        update_columns: Vec<String>,
        primary_key: PrimaryKey,
    },

    /// Bulk insert multiple rows
    BulkInsert {
        table: String,
        columns: Vec<String>,
        rows: Vec<Vec<Value>>,
    },

    /// ALTER TABLE statement
    AlterTable {
        table: String,
        ddl: String,
    },

    /// CREATE TABLE statement
    CreateTable {
        table: String,
        ddl: String,
    },

    /// DROP TABLE statement
    DropTable {
        table: String,
    },

    /// CREATE INDEX statement
    CreateIndex {
        table: String,
        index_name: String,
        ddl: String,
    },

    /// DROP INDEX statement
    DropIndex {
        table: String,
        index_name: String,
    },

    /// Transaction wrapper containing multiple entries
    Transaction {
        entries: Vec<LogEntry>,
    },

    /// Raw SQL (for operations that don't fit other categories)
    RawSql {
        sql: String,
        affects_table: Option<String>,
        /// Database context (from USE statement) - prepended as USE before execution
        #[serde(default)]
        database: Option<String>,
    },

    /// No-op entry (used for leader election heartbeats)
    Noop,
}

impl LogEntry {
    /// Get the table name affected by this entry (if applicable)
    pub fn table_name(&self) -> Option<&str> {
        match self {
            LogEntry::Insert { table, .. }
            | LogEntry::Update { table, .. }
            | LogEntry::Delete { table, .. }
            | LogEntry::Upsert { table, .. }
            | LogEntry::BulkInsert { table, .. }
            | LogEntry::AlterTable { table, .. }
            | LogEntry::CreateTable { table, .. }
            | LogEntry::DropTable { table }
            | LogEntry::CreateIndex { table, .. }
            | LogEntry::DropIndex { table, .. } => Some(table),
            LogEntry::Transaction { entries } => entries.first().and_then(|e| e.table_name()),
            LogEntry::RawSql { affects_table, .. } => affects_table.as_deref(),
            LogEntry::Noop => None,
        }
    }

    /// Check if this is a DDL (schema change) operation
    pub fn is_ddl(&self) -> bool {
        matches!(
            self,
            LogEntry::AlterTable { .. }
                | LogEntry::CreateTable { .. }
                | LogEntry::DropTable { .. }
                | LogEntry::CreateIndex { .. }
                | LogEntry::DropIndex { .. }
        )
    }

    /// Check if this entry is a no-op
    pub fn is_noop(&self) -> bool {
        matches!(self, LogEntry::Noop)
    }

    /// Convert to SQL statement(s)
    pub fn to_sql(&self) -> Vec<String> {
        match self {
            LogEntry::Insert {
                table,
                columns,
                values,
                ..
            } => {
                let cols = columns
                    .iter()
                    .map(|c| format!("`{}`", c))
                    .collect::<Vec<_>>()
                    .join(", ");
                let vals = values
                    .iter()
                    .map(|v| v.to_sql())
                    .collect::<Vec<_>>()
                    .join(", ");
                vec![format!("INSERT INTO `{}` ({}) VALUES ({})", table, cols, vals)]
            }

            LogEntry::Update {
                table,
                set_columns,
                set_values,
                primary_key,
                key_columns,
            } => {
                let sets: Vec<String> = set_columns
                    .iter()
                    .zip(set_values.iter())
                    .map(|(col, val)| format!("`{}` = {}", col, val.to_sql()))
                    .collect();
                let where_clause = primary_key.to_where_clause(key_columns);
                vec![format!(
                    "UPDATE `{}` SET {} WHERE {}",
                    table,
                    sets.join(", "),
                    where_clause
                )]
            }

            LogEntry::Delete {
                table,
                primary_key,
                key_columns,
            } => {
                let where_clause = primary_key.to_where_clause(key_columns);
                vec![format!("DELETE FROM `{}` WHERE {}", table, where_clause)]
            }

            LogEntry::Upsert {
                table,
                columns,
                values,
                update_columns,
                ..
            } => {
                let cols = columns
                    .iter()
                    .map(|c| format!("`{}`", c))
                    .collect::<Vec<_>>()
                    .join(", ");
                let vals = values
                    .iter()
                    .map(|v| v.to_sql())
                    .collect::<Vec<_>>()
                    .join(", ");
                let updates: Vec<String> = update_columns
                    .iter()
                    .map(|c| format!("`{}` = VALUES(`{}`)", c, c))
                    .collect();
                vec![format!(
                    "INSERT INTO `{}` ({}) VALUES ({}) ON DUPLICATE KEY UPDATE {}",
                    table,
                    cols,
                    vals,
                    updates.join(", ")
                )]
            }

            LogEntry::BulkInsert {
                table,
                columns,
                rows,
            } => {
                let cols = columns
                    .iter()
                    .map(|c| format!("`{}`", c))
                    .collect::<Vec<_>>()
                    .join(", ");
                let row_values: Vec<String> = rows
                    .iter()
                    .map(|row| {
                        let vals = row
                            .iter()
                            .map(|v| v.to_sql())
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("({})", vals)
                    })
                    .collect();
                vec![format!(
                    "INSERT INTO `{}` ({}) VALUES {}",
                    table,
                    cols,
                    row_values.join(", ")
                )]
            }

            LogEntry::AlterTable { ddl, .. }
            | LogEntry::CreateTable { ddl, .. }
            | LogEntry::CreateIndex { ddl, .. } => vec![ddl.clone()],

            LogEntry::DropTable { table } => vec![format!("DROP TABLE IF EXISTS `{}`", table)],

            LogEntry::DropIndex { table, index_name } => {
                vec![format!("DROP INDEX `{}` ON `{}`", index_name, table)]
            }

            LogEntry::Transaction { entries } => {
                let mut sql = vec!["START TRANSACTION".to_string()];
                for entry in entries {
                    sql.extend(entry.to_sql());
                }
                sql.push("COMMIT".to_string());
                sql
            }

            LogEntry::RawSql { sql, .. } => vec![sql.clone()],

            LogEntry::Noop => vec![],
        }
    }

    /// Serialize entry to bytes
    pub fn serialize(&self) -> Result<Vec<u8>, bincode::Error> {
        bincode::serialize(self)
    }

    /// Deserialize entry from bytes
    pub fn deserialize(bytes: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(bytes)
    }
}

/// Full WAL entry with header and body
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalEntry {
    pub header: EntryHeader,
    pub entry: LogEntry,
}

impl WalEntry {
    /// Create a new WAL entry
    pub fn new(lsn: Lsn, term: u64, origin_node: String, entry: LogEntry) -> Self {
        let serialized = entry.serialize().unwrap_or_default();
        let checksum = crc32fast::hash(&serialized);

        Self {
            header: EntryHeader {
                lsn,
                term,
                timestamp: chrono::Utc::now(),
                origin_node,
                checksum,
                body_size: serialized.len() as u32,
                compressed: false,
            },
            entry,
        }
    }

    /// Verify checksum
    pub fn verify_checksum(&self) -> bool {
        let serialized = self.entry.serialize().unwrap_or_default();
        crc32fast::hash(&serialized) == self.header.checksum
    }
}

// Hex encoding for bytes (simple implementation)
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_to_sql() {
        let entry = LogEntry::Insert {
            table: "users".to_string(),
            columns: vec!["id".to_string(), "name".to_string(), "email".to_string()],
            values: vec![
                Value::Int(1),
                Value::String("Alice".to_string()),
                Value::String("alice@example.com".to_string()),
            ],
            primary_key: PrimaryKey::Int(1),
        };

        let sql = entry.to_sql();
        assert_eq!(sql.len(), 1);
        assert!(sql[0].contains("INSERT INTO `users`"));
        assert!(sql[0].contains("'Alice'"));
    }

    #[test]
    fn test_update_to_sql() {
        let entry = LogEntry::Update {
            table: "users".to_string(),
            set_columns: vec!["name".to_string()],
            set_values: vec![Value::String("Bob".to_string())],
            primary_key: PrimaryKey::Int(1),
            key_columns: vec!["id".to_string()],
        };

        let sql = entry.to_sql();
        assert!(sql[0].contains("UPDATE `users`"));
        assert!(sql[0].contains("SET `name` = 'Bob'"));
        assert!(sql[0].contains("WHERE `id` = 1"));
    }

    #[test]
    fn test_serialize_deserialize() {
        let entry = LogEntry::Delete {
            table: "users".to_string(),
            primary_key: PrimaryKey::Int(42),
            key_columns: vec!["id".to_string()],
        };

        let bytes = entry.serialize().unwrap();
        let restored = LogEntry::deserialize(&bytes).unwrap();

        match restored {
            LogEntry::Delete { table, primary_key, .. } => {
                assert_eq!(table, "users");
                assert_eq!(primary_key, PrimaryKey::Int(42));
            }
            _ => panic!("Wrong entry type after deserialize"),
        }
    }
}
