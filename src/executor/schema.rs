//! Schema Manager
//!
//! Handles schema change tracking and validation.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

use super::MariaDbExecutor;
use crate::error::{Error, Result};

/// Schema version information
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaVersion {
    /// Version number
    pub version: u64,
    /// Timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Description
    pub description: String,
    /// Hash of the schema
    pub hash: String,
}

/// Table schema information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableSchema {
    /// Table name
    pub name: String,
    /// Columns
    pub columns: Vec<ColumnSchema>,
    /// Primary key columns
    pub primary_key: Vec<String>,
    /// Indexes
    pub indexes: Vec<IndexSchema>,
}

/// Column schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnSchema {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub auto_increment: bool,
}

/// Index schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexSchema {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
}

/// Schema manager for tracking and applying schema changes
pub struct SchemaManager {
    /// Cached table schemas
    schemas: HashMap<String, TableSchema>,
    /// Current schema version
    version: u64,
}

impl SchemaManager {
    /// Create a new schema manager
    pub fn new() -> Self {
        Self {
            schemas: HashMap::new(),
            version: 0,
        }
    }

    /// Load schemas from the database
    pub async fn load_from_database(&mut self, executor: &MariaDbExecutor) -> Result<()> {
        let tables = executor.list_tables().await?;

        for table in tables {
            let columns = executor.describe_table(&table).await?;
            let pk = executor.get_primary_key(&table).await?;

            let schema = TableSchema {
                name: table.clone(),
                columns: columns
                    .into_iter()
                    .map(|c| ColumnSchema {
                        name: c.name,
                        data_type: c.data_type,
                        nullable: c.nullable,
                        default_value: c.default,
                        auto_increment: c.extra.contains("auto_increment"),
                    })
                    .collect(),
                primary_key: pk,
                indexes: vec![], // Would need additional query for indexes
            };

            self.schemas.insert(table, schema);
        }

        self.version += 1;
        Ok(())
    }

    /// Get schema for a table
    pub fn get_table(&self, name: &str) -> Option<&TableSchema> {
        self.schemas.get(name)
    }

    /// Validate a DDL statement
    pub fn validate_ddl(&self, ddl: &str) -> Result<DdlChange> {
        let ddl_upper = ddl.to_uppercase();

        if ddl_upper.starts_with("CREATE TABLE") {
            Ok(DdlChange::CreateTable {
                table: Self::extract_table_name(ddl)?,
            })
        } else if ddl_upper.starts_with("DROP TABLE") {
            Ok(DdlChange::DropTable {
                table: Self::extract_table_name(ddl)?,
            })
        } else if ddl_upper.starts_with("ALTER TABLE") {
            let table = Self::extract_table_name(ddl)?;
            let change_type = if ddl_upper.contains("ADD COLUMN") || ddl_upper.contains("ADD `") {
                AlterType::AddColumn
            } else if ddl_upper.contains("DROP COLUMN") || ddl_upper.contains("DROP `") {
                AlterType::DropColumn
            } else if ddl_upper.contains("MODIFY") || ddl_upper.contains("CHANGE") {
                AlterType::ModifyColumn
            } else if ddl_upper.contains("ADD INDEX") || ddl_upper.contains("ADD KEY") {
                AlterType::AddIndex
            } else if ddl_upper.contains("DROP INDEX") || ddl_upper.contains("DROP KEY") {
                AlterType::DropIndex
            } else {
                AlterType::Other
            };

            Ok(DdlChange::AlterTable { table, change_type })
        } else if ddl_upper.starts_with("CREATE INDEX") {
            Ok(DdlChange::CreateIndex {
                table: Self::extract_table_from_index(ddl)?,
            })
        } else if ddl_upper.starts_with("DROP INDEX") {
            Ok(DdlChange::DropIndex {
                table: Self::extract_table_from_index(ddl)?,
            })
        } else {
            Err(Error::Schema(format!("Unknown DDL type: {}", &ddl[..ddl.len().min(50)])))
        }
    }

    /// Check if a schema change is safe (won't cause data loss)
    pub fn is_safe_change(&self, change: &DdlChange) -> bool {
        match change {
            DdlChange::CreateTable { .. } => true,
            DdlChange::DropTable { table } => {
                // Unsafe if table exists and has data
                // For safety, always treat as unsafe
                tracing::warn!("DROP TABLE {} is potentially unsafe", table);
                false
            }
            DdlChange::AlterTable { change_type, .. } => {
                match change_type {
                    AlterType::AddColumn => true,
                    AlterType::AddIndex => true,
                    AlterType::DropColumn => false, // Data loss
                    AlterType::DropIndex => true,
                    AlterType::ModifyColumn => false, // Could cause data loss
                    AlterType::Other => false,
                }
            }
            DdlChange::CreateIndex { .. } => true,
            DdlChange::DropIndex { .. } => true,
        }
    }

    /// Extract table name from DDL
    fn extract_table_name(ddl: &str) -> Result<String> {
        // Simple extraction - look for backtick-quoted or unquoted table name
        let ddl = ddl.to_uppercase();
        
        let table_pos = ddl.find("TABLE")
            .ok_or_else(|| Error::Schema("Could not find TABLE keyword".into()))?;
        
        let after_table = &ddl[table_pos + 5..].trim_start();
        
        // Skip "IF EXISTS" or "IF NOT EXISTS"
        let after_if = if after_table.starts_with("IF") {
            let exists_pos = after_table.find("EXISTS")
                .ok_or_else(|| Error::Schema("Invalid IF clause".into()))?;
            &after_table[exists_pos + 6..].trim_start()
        } else {
            after_table
        };

        // Extract first word (table name)
        let end = after_if
            .find(|c: char| c.is_whitespace() || c == '(' || c == ';')
            .unwrap_or(after_if.len());

        let table_name = &after_if[..end];
        let table_name = table_name.trim_matches('`').trim_matches('"');

        Ok(table_name.to_lowercase())
    }

    /// Extract table name from index DDL
    fn extract_table_from_index(ddl: &str) -> Result<String> {
        let ddl = ddl.to_uppercase();
        
        if let Some(on_pos) = ddl.find(" ON ") {
            let after_on = &ddl[on_pos + 4..].trim_start();
            let end = after_on
                .find(|c: char| c.is_whitespace() || c == '(' || c == ';')
                .unwrap_or(after_on.len());
            
            let table_name = &after_on[..end];
            let table_name = table_name.trim_matches('`').trim_matches('"');
            Ok(table_name.to_lowercase())
        } else {
            Err(Error::Schema("Could not find ON keyword in index DDL".into()))
        }
    }

    /// Get current schema version
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Get all table names
    pub fn tables(&self) -> Vec<&str> {
        self.schemas.keys().map(|s| s.as_str()).collect()
    }

    /// Compute a hash of the current schema
    pub fn compute_hash(&self) -> String {
        let mut hasher = crc32fast::Hasher::new();
        
        let mut tables: Vec<_> = self.schemas.keys().collect();
        tables.sort();
        
        for table in tables {
            if let Some(schema) = self.schemas.get(table) {
                hasher.update(table.as_bytes());
                for col in &schema.columns {
                    hasher.update(col.name.as_bytes());
                    hasher.update(col.data_type.as_bytes());
                }
            }
        }

        format!("{:08x}", hasher.finalize())
    }
}

impl Default for SchemaManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Type of DDL change
#[derive(Debug, Clone)]
pub enum DdlChange {
    CreateTable { table: String },
    DropTable { table: String },
    AlterTable { table: String, change_type: AlterType },
    CreateIndex { table: String },
    DropIndex { table: String },
}

/// Alter table change type
#[derive(Debug, Clone)]
pub enum AlterType {
    AddColumn,
    DropColumn,
    ModifyColumn,
    AddIndex,
    DropIndex,
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_ddl() {
        let manager = SchemaManager::new();

        let change = manager.validate_ddl("CREATE TABLE `users` (id INT)").unwrap();
        match change {
            DdlChange::CreateTable { table } => assert_eq!(table, "users"),
            _ => panic!("Expected CreateTable"),
        }

        let change = manager.validate_ddl("ALTER TABLE users ADD COLUMN name VARCHAR(255)").unwrap();
        match change {
            DdlChange::AlterTable { table, change_type } => {
                assert_eq!(table, "users");
                assert!(matches!(change_type, AlterType::AddColumn));
            }
            _ => panic!("Expected AlterTable"),
        }
    }

    #[test]
    fn test_safe_change() {
        let manager = SchemaManager::new();

        let change = DdlChange::CreateTable { table: "test".to_string() };
        assert!(manager.is_safe_change(&change));

        let change = DdlChange::DropTable { table: "test".to_string() };
        assert!(!manager.is_safe_change(&change));

        let change = DdlChange::AlterTable { 
            table: "test".to_string(), 
            change_type: AlterType::AddColumn 
        };
        assert!(manager.is_safe_change(&change));
    }
}
