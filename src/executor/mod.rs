//! Database Executor Module
//!
//! Executes log entries against MariaDB databases.

mod mariadb;
mod schema;

pub use mariadb::MariaDbExecutor;
pub use schema::SchemaManager;
