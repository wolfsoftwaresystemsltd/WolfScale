//! WolfDisk - Distributed File System
//!
//! A distributed file system for Linux that provides easy-to-use
//! shared and replicated storage.

pub mod config;
pub mod error;
pub mod fuse;
pub mod storage;

pub use config::Config;
pub use error::{Error, Result};
