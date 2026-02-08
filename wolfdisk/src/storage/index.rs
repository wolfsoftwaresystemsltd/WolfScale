//! File metadata index

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::error::Result;

/// Reference to a chunk in storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRef {
    /// SHA256 hash of the chunk content
    pub hash: [u8; 32],

    /// Offset of this chunk within the file
    pub offset: u64,

    /// Size of this chunk in bytes
    pub size: u32,
}

/// File metadata entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// File size in bytes
    pub size: u64,

    /// Whether this is a directory
    pub is_dir: bool,

    /// File permissions
    pub permissions: u32,

    /// Owner user ID
    pub uid: u32,

    /// Owner group ID
    pub gid: u32,

    /// Creation time
    pub created: SystemTime,

    /// Modification time
    pub modified: SystemTime,

    /// Access time
    pub accessed: SystemTime,

    /// Chunk references for file content
    pub chunks: Vec<ChunkRef>,
}

/// File metadata index
#[derive(Debug, Serialize, Deserialize)]
pub struct FileIndex {
    /// Path to entry mapping
    entries: HashMap<PathBuf, FileEntry>,

    /// Index version for compatibility
    version: u32,
}

const INDEX_VERSION: u32 = 1;
const INDEX_FILENAME: &str = "index.json";

impl FileIndex {
    /// Create a new empty index
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            version: INDEX_VERSION,
        }
    }

    /// Load index from disk or create new if not exists
    pub fn load_or_create(index_dir: &Path) -> Result<Self> {
        let index_path = index_dir.join(INDEX_FILENAME);

        if index_path.exists() {
            info!("Loading file index from {:?}", index_path);
            let file = File::open(&index_path)?;
            let reader = BufReader::new(file);
            let index: FileIndex = serde_json::from_reader(reader)?;
            
            if index.version != INDEX_VERSION {
                info!("Index version mismatch, creating new index");
                return Ok(Self::new());
            }

            info!("Loaded {} entries", index.entries.len());
            Ok(index)
        } else {
            info!("No existing index, creating new");
            Ok(Self::new())
        }
    }

    /// Save index to disk
    pub fn save(&self, index_dir: &Path) -> Result<()> {
        fs::create_dir_all(index_dir)?;
        
        let index_path = index_dir.join(INDEX_FILENAME);
        let file = File::create(&index_path)?;
        let writer = BufWriter::new(file);
        serde_json::to_writer_pretty(writer, self)?;
        
        debug!("Saved file index with {} entries", self.entries.len());
        Ok(())
    }

    /// Get an entry by path
    pub fn get(&self, path: &Path) -> Option<&FileEntry> {
        self.entries.get(path)
    }

    /// Get a mutable entry by path
    pub fn get_mut(&mut self, path: &Path) -> Option<&mut FileEntry> {
        self.entries.get_mut(path)
    }

    /// Check if path exists
    pub fn contains(&self, path: &Path) -> bool {
        self.entries.contains_key(path)
    }

    /// Insert or update an entry
    pub fn insert(&mut self, path: PathBuf, entry: FileEntry) {
        self.entries.insert(path, entry);
    }

    /// Remove an entry
    pub fn remove(&mut self, path: &Path) -> Option<FileEntry> {
        self.entries.remove(path)
    }

    /// Get all paths
    pub fn paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.entries.keys()
    }

    /// Iterate over all entries
    pub fn iter(&self) -> impl Iterator<Item = (&PathBuf, &FileEntry)> {
        self.entries.iter()
    }

    /// Get entry count
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for FileIndex {
    fn default() -> Self {
        Self::new()
    }
}

// Need to implement From to convert serde_json error
impl From<serde_json::Error> for crate::error::Error {
    fn from(e: serde_json::Error) -> Self {
        crate::error::Error::Storage(format!("JSON error: {}", e))
    }
}
