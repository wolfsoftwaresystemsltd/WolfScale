//! Chunk storage with content-addressed deduplication

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use sha2::{Sha256, Digest};
use tracing::debug;

use crate::error::{Error, Result};
use super::ChunkRef;

/// Maximum number of chunks to keep in the read cache
const DEFAULT_CACHE_CAPACITY: usize = 256;

/// Content-addressed chunk storage
pub struct ChunkStore {
    /// Base directory for chunks
    base_dir: PathBuf,

    /// Target chunk size in bytes
    chunk_size: usize,

    /// In-memory read cache: hash -> chunk data
    read_cache: Mutex<ReadCache>,
}

/// Simple LRU-style read cache for chunk data
struct ReadCache {
    entries: HashMap<[u8; 32], Vec<u8>>,
    access_order: Vec<[u8; 32]>,
    capacity: usize,
}

impl ReadCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity),
            access_order: Vec::with_capacity(capacity),
            capacity,
        }
    }

    fn get(&mut self, hash: &[u8; 32]) -> Option<Vec<u8>> {
        if let Some(data) = self.entries.get(hash) {
            // Move to end of access order (most recently used)
            if let Some(pos) = self.access_order.iter().position(|h| h == hash) {
                self.access_order.remove(pos);
                self.access_order.push(*hash);
            }
            Some(data.clone())
        } else {
            None
        }
    }

    fn insert(&mut self, hash: [u8; 32], data: Vec<u8>) {
        if self.entries.contains_key(&hash) {
            return;
        }

        // Evict oldest entries if at capacity
        while self.entries.len() >= self.capacity && !self.access_order.is_empty() {
            let oldest = self.access_order.remove(0);
            self.entries.remove(&oldest);
        }

        self.entries.insert(hash, data);
        self.access_order.push(hash);
    }

    fn remove(&mut self, hash: &[u8; 32]) {
        self.entries.remove(hash);
        if let Some(pos) = self.access_order.iter().position(|h| h == hash) {
            self.access_order.remove(pos);
        }
    }
}

impl ChunkStore {
    /// Create a new chunk store
    pub fn new(base_dir: PathBuf, chunk_size: usize) -> Result<Self> {
        fs::create_dir_all(&base_dir)?;
        
        Ok(Self {
            base_dir,
            chunk_size,
            read_cache: Mutex::new(ReadCache::new(DEFAULT_CACHE_CAPACITY)),
        })
    }

    /// Get the path for a chunk by its hash
    fn chunk_path(&self, hash: &[u8; 32]) -> PathBuf {
        let hex = hex::encode(hash);
        // Use first 2 characters as subdirectory for better filesystem performance
        self.base_dir.join(&hex[0..2]).join(&hex[2..])
    }

    /// Store a chunk and return its hash
    pub fn store(&self, data: &[u8]) -> Result<[u8; 32]> {
        // Calculate hash
        let mut hasher = Sha256::new();
        hasher.update(data);
        let hash: [u8; 32] = hasher.finalize().into();

        let path = self.chunk_path(&hash);

        // Check if chunk already exists (deduplication)
        if path.exists() {
            debug!("Chunk {} already exists (deduplicated)", hex::encode(&hash));
            return Ok(hash);
        }

        // Create parent directory
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write chunk to file (no sync_all - let OS page cache handle durability)
        let mut file = File::create(&path)?;
        file.write_all(data)?;

        // Populate read cache with the data we just wrote
        if let Ok(mut cache) = self.read_cache.lock() {
            cache.insert(hash, data.to_vec());
        }

        debug!("Stored chunk {} ({} bytes)", hex::encode(&hash), data.len());
        Ok(hash)
    }
    
    /// Store a chunk with a known hash (for replication)
    pub fn store_with_hash(&self, hash: &[u8; 32], data: &[u8]) -> Result<()> {
        let path = self.chunk_path(hash);

        // Check if chunk already exists (deduplication)
        if path.exists() {
            debug!("Chunk {} already exists (deduplicated)", hex::encode(hash));
            return Ok(());
        }

        // Create parent directory
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write chunk to file (no sync_all - let OS page cache handle durability)
        let mut file = File::create(&path)?;
        file.write_all(data)?;

        // Populate read cache
        if let Ok(mut cache) = self.read_cache.lock() {
            cache.insert(*hash, data.to_vec());
        }

        debug!("Stored replicated chunk {} ({} bytes)", hex::encode(hash), data.len());
        Ok(())
    }

    /// Retrieve a chunk by its hash
    pub fn get(&self, hash: &[u8; 32]) -> Result<Vec<u8>> {
        // Check read cache first
        if let Ok(mut cache) = self.read_cache.lock() {
            if let Some(data) = cache.get(hash) {
                debug!("Cache hit for chunk {}", hex::encode(hash));
                return Ok(data);
            }
        }

        let path = self.chunk_path(hash);

        if !path.exists() {
            return Err(Error::ChunkNotFound(hex::encode(hash)));
        }

        let mut file = File::open(&path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;

        // Populate cache
        if let Ok(mut cache) = self.read_cache.lock() {
            cache.insert(*hash, data.clone());
        }

        Ok(data)
    }

    /// Delete a chunk (used for garbage collection)
    pub fn delete(&self, hash: &[u8; 32]) -> Result<()> {
        let path = self.chunk_path(hash);

        // Remove from cache
        if let Ok(mut cache) = self.read_cache.lock() {
            cache.remove(hash);
        }

        if path.exists() {
            fs::remove_file(&path)?;
            debug!("Deleted chunk {}", hex::encode(hash));
        }

        Ok(())
    }

    /// Check if a chunk exists
    pub fn exists(&self, hash: &[u8; 32]) -> bool {
        self.chunk_path(hash).exists()
    }

    /// Read data from a file's chunks at a given offset
    pub fn read(&self, chunks: &[ChunkRef], offset: u64, size: usize) -> Result<Vec<u8>> {
        if chunks.is_empty() {
            return Ok(Vec::new());
        }

        let mut result = Vec::with_capacity(size);
        let end_offset = offset + size as u64;
        let mut bytes_read = 0;

        for chunk in chunks {
            let chunk_start = chunk.offset;
            let chunk_end = chunk.offset + chunk.size as u64;

            // Skip chunks before our read range
            if chunk_end <= offset {
                continue;
            }

            // Stop if we've passed our read range
            if chunk_start >= end_offset {
                break;
            }

            // Load chunk data (will use cache if available)
            let chunk_data = self.get(&chunk.hash)?;

            // Calculate how much of this chunk to read
            let read_start = if chunk_start < offset {
                (offset - chunk_start) as usize
            } else {
                0
            };

            let read_end = if chunk_end > end_offset {
                chunk_data.len() - (chunk_end - end_offset) as usize
            } else {
                chunk_data.len()
            };

            // Append to result
            result.extend_from_slice(&chunk_data[read_start..read_end]);
            bytes_read += read_end - read_start;

            if bytes_read >= size {
                break;
            }
        }

        Ok(result)
    }

    /// Write data to a file's chunks at a given offset
    pub fn write(&self, chunks: &mut Vec<ChunkRef>, offset: u64, data: &[u8]) -> Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }

        // For simplicity in MVP, we append new chunks
        // A more sophisticated implementation would handle in-place updates

        let mut written = 0;
        let mut current_offset = offset;

        while written < data.len() {
            let remaining = data.len() - written;
            let chunk_size = remaining.min(self.chunk_size);
            let chunk_data = &data[written..written + chunk_size];

            // Store the chunk
            let hash = self.store(chunk_data)?;

            // Add chunk reference
            chunks.push(ChunkRef {
                hash,
                offset: current_offset,
                size: chunk_size as u32,
            });

            written += chunk_size;
            current_offset += chunk_size as u64;
        }

        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_chunk_store_roundtrip() {
        let dir = tempdir().unwrap();
        let store = ChunkStore::new(dir.path().to_path_buf(), 1024).unwrap();

        let data = b"Hello, WolfDisk!";
        let hash = store.store(data).unwrap();

        let retrieved = store.get(&hash).unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn test_deduplication() {
        let dir = tempdir().unwrap();
        let store = ChunkStore::new(dir.path().to_path_buf(), 1024).unwrap();

        let data = b"Duplicate content";
        let hash1 = store.store(data).unwrap();
        let hash2 = store.store(data).unwrap();

        assert_eq!(hash1, hash2);
    }
}

