//! Binlog Replication Module
//!
//! This module provides support for reading MariaDB binary logs and converting
//! them to WAL entries for replication. This enables WolfScale to capture
//! writes from external MariaDB/Galera clusters without requiring writes to
//! go through the WolfScale proxy.

mod client;
mod event;
mod converter;

pub use client::BinlogClient;
pub use event::BinlogEvent;
pub use converter::binlog_to_wal;
