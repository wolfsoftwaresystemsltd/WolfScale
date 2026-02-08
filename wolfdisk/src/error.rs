//! Error types for WolfDisk

use thiserror::Error;

/// Result type alias using WolfDisk Error
pub type Result<T> = std::result::Result<T, Error>;

/// WolfDisk error types
#[derive(Error, Debug)]
pub enum Error {
    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// TOML parsing error
    #[error("TOML error: {0}")]
    Toml(#[from] toml::de::Error),

    /// TOML serialization error
    #[error("TOML serialization error: {0}")]
    TomlSer(#[from] toml::ser::Error),

    /// Storage error
    #[error("Storage error: {0}")]
    Storage(String),

    /// Chunk not found
    #[error("Chunk not found: {0}")]
    ChunkNotFound(String),

    /// File not found
    #[error("File not found: {0}")]
    FileNotFound(String),

    /// Replication error
    #[error("Replication error: {0}")]
    Replication(String),

    /// Network error
    #[error("Network error: {0}")]
    Network(String),

    /// Invalid operation
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
}

impl Error {
    /// Convert to libc error code for FUSE
    pub fn to_errno(&self) -> libc::c_int {
        match self {
            Error::Io(e) => e.raw_os_error().unwrap_or(libc::EIO),
            Error::FileNotFound(_) => libc::ENOENT,
            Error::ChunkNotFound(_) => libc::EIO,
            Error::InvalidOperation(_) => libc::EINVAL,
            _ => libc::EIO,
        }
    }
}
