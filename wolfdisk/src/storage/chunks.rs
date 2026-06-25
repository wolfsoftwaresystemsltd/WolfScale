//! Chunk storage with content-addressed deduplication and an optional SSD cache tier.
//!
//! Tiers (fastest to slowest):
//!
//! 1. in-memory `ReadCache` (a few hundred hot chunks)
//! 2. SSD cache dir (`cache_dir`, optional) — fast disk
//! 3. bulk store (`base_dir`) — the HDD, always authoritative once a chunk is flushed
//!
//! Cache modes:
//!
//! - `Off` — no SSD tier (historical behaviour; `base_dir` only).
//! - `WriteThrough` — every store writes the bulk HDD (authoritative) AND mirrors to the
//!   SSD; reads check SSD before HDD. Zero data-loss risk: the HDD always holds the chunk.
//! - `WriteBack` — a store lands on the SSD first (fsync'd, marked *dirty*) and is copied
//!   to the HDD asynchronously by `flush_dirty`. A dirty chunk is never evicted before
//!   it's flushed, and `recover_dirty` re-queues any un-flushed chunk after an unclean
//!   shutdown — so a chunk is durable the moment its SSD write is fsync'd. (An SSD
//!   *hardware* failure before flush is the inherent write-back trade-off; in a
//!   replicated cluster the peer copies still cover it.)

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use sha2::{Sha256, Digest};
use tracing::{debug, warn};

use crate::error::{Error, Result};
use super::ChunkRef;

/// Maximum number of chunks to keep in the in-memory read cache.
const DEFAULT_CACHE_CAPACITY: usize = 256;

/// Unique-temp-file counter so concurrent atomic writes never collide on a temp name.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// How the optional SSD tier behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    /// No SSD tier — `base_dir` only.
    Off,
    /// Write HDD + mirror SSD; read SSD-first. HDD always authoritative.
    WriteThrough,
    /// Write SSD-first (dirty), async-flush to HDD. SSD authoritative until flushed.
    WriteBack,
}

/// Content-addressed chunk storage with an optional SSD cache tier.
pub struct ChunkStore {
    /// Bulk store (HDD). Authoritative once a chunk has been flushed here.
    base_dir: PathBuf,

    /// Optional fast tier (SSD). Present iff a cache is configured.
    cache_dir: Option<PathBuf>,

    /// Cache behaviour. `Off` whenever `cache_dir` is `None`.
    mode: CacheMode,

    /// Soft ceiling on SSD-tier bytes; cold *clean* chunks are evicted above it.
    /// 0 = unbounded (no eviction).
    max_cache_bytes: u64,

    /// Target chunk size in bytes.
    chunk_size: usize,

    /// In-memory read cache: hash -> chunk data.
    read_cache: Mutex<ReadCache>,

    /// SSD-tier bookkeeping (LRU, per-chunk size, dirty set, total bytes).
    cache_state: Mutex<CacheState>,
}

/// Simple LRU-style in-memory read cache for chunk data.
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

/// Accounting for what currently lives on the SSD tier.
#[derive(Default)]
struct CacheState {
    /// Eviction order of *clean* chunks (front = coldest). Dirty chunks are not here.
    lru: VecDeque<[u8; 32]>,
    /// Bytes each cached chunk occupies on the SSD (clean or dirty).
    sizes: HashMap<[u8; 32], u64>,
    /// Total SSD-tier bytes.
    total_bytes: u64,
    /// Chunks on the SSD not yet flushed to the HDD (write-back). Never evicted.
    dirty: HashSet<[u8; 32]>,
}

impl CacheState {
    /// Record a chunk now present on the SSD. `dirty` marks it un-flushed.
    fn track(&mut self, hash: [u8; 32], size: u64, dirty: bool) {
        // Keep total_bytes exact even if the same hash is re-tracked with a
        // different recorded size (e.g. a metadata read returned a stale length).
        match self.sizes.insert(hash, size) {
            None => self.total_bytes += size,
            Some(old) if old != size => {
                self.total_bytes = self.total_bytes.saturating_sub(old) + size;
            }
            Some(_) => {}
        }
        if dirty {
            self.dirty.insert(hash);
            // A dirty chunk must not sit in the clean-eviction queue.
            if let Some(p) = self.lru.iter().position(|h| h == &hash) {
                self.lru.remove(p);
            }
        } else {
            self.touch(&hash);
        }
    }

    /// Mark a chunk clean (flushed) and eligible for eviction.
    fn mark_clean(&mut self, hash: &[u8; 32]) {
        if self.dirty.remove(hash) && self.sizes.contains_key(hash) {
            self.lru.push_back(*hash);
        }
    }

    /// Bump a clean chunk to most-recently-used.
    fn touch(&mut self, hash: &[u8; 32]) {
        if self.dirty.contains(hash) {
            return;
        }
        if let Some(p) = self.lru.iter().position(|h| h == hash) {
            self.lru.remove(p);
        }
        self.lru.push_back(*hash);
    }

    /// Forget a chunk entirely (it left the SSD).
    fn forget(&mut self, hash: &[u8; 32]) {
        if let Some(sz) = self.sizes.remove(hash) {
            self.total_bytes = self.total_bytes.saturating_sub(sz);
        }
        self.dirty.remove(hash);
        if let Some(p) = self.lru.iter().position(|h| h == hash) {
            self.lru.remove(p);
        }
    }
}

/// Shard a hash into `<base>/<hex[0:2]>/<hex[2:]>` for a bounded fan-out per directory.
fn shard_path(base: &Path, hash: &[u8; 32]) -> PathBuf {
    let hex = hex::encode(hash);
    base.join(&hex[0..2]).join(&hex[2..])
}

/// Atomically write `data` to `path` (temp file in the same dir + rename), so a crash
/// never leaves a partially-written chunk. `fsync` forces the data (and, on most
/// filesystems, the rename) to stable storage before returning — used for the SSD
/// write-back tier where the chunk must survive an unclean shutdown.
fn write_atomic(path: &Path, data: &[u8], fsync: bool) -> Result<()> {
    let parent = path.parent();
    if let Some(p) = parent {
        fs::create_dir_all(p)?;
    }
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("chunk");
    let tmp = path.with_file_name(format!(".{}.{}.{}.tmp", fname, std::process::id(), seq));
    {
        let mut f = File::create(&tmp)?;
        f.write_all(data)?;
        if fsync {
            f.sync_all()?;
        }
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    // Durability: on ext4/xfs a rename only survives a crash once the PARENT
    // DIRECTORY entry is fsync'd. Without this the chunk data is on stable storage
    // but the directory may still point at the (now-absent) temp name — so a
    // write-back chunk we acked as durable could vanish on power loss.
    if fsync {
        if let Some(p) = parent {
            match File::open(p).and_then(|dir| dir.sync_all()) {
                Ok(()) => {}
                // Best-effort: the data is already fsync'd; only the rename's
                // crash-durability is at risk. Surface it rather than hide it.
                Err(e) => warn!("write_atomic: parent dir fsync failed for {}: {}", p.display(), e),
            }
        }
    }
    Ok(())
}

fn read_file(path: &Path) -> Result<Vec<u8>> {
    let mut f = File::open(path)?;
    let mut data = Vec::new();
    f.read_to_end(&mut data)?;
    Ok(data)
}

impl ChunkStore {
    /// Create a chunk store with no SSD tier (historical behaviour).
    pub fn new(base_dir: PathBuf, chunk_size: usize) -> Result<Self> {
        Self::with_cache(base_dir, chunk_size, None, CacheMode::Off, 0)
    }

    /// Create a chunk store, optionally backed by an SSD cache tier.
    /// `cache_dir == None` (or `mode == Off`) yields the no-cache behaviour.
    pub fn with_cache(
        base_dir: PathBuf,
        chunk_size: usize,
        cache_dir: Option<PathBuf>,
        mode: CacheMode,
        max_cache_bytes: u64,
    ) -> Result<Self> {
        fs::create_dir_all(&base_dir)?;
        // No cache dir → force Off so the rest of the code can trust the invariant.
        let (cache_dir, mode) = match cache_dir {
            Some(d) if mode != CacheMode::Off => {
                fs::create_dir_all(&d)?;
                (Some(d), mode)
            }
            _ => (None, CacheMode::Off),
        };
        Ok(Self {
            base_dir,
            cache_dir,
            mode,
            max_cache_bytes,
            chunk_size,
            read_cache: Mutex::new(ReadCache::new(DEFAULT_CACHE_CAPACITY)),
            cache_state: Mutex::new(CacheState::default()),
        })
    }

    /// True if an SSD tier is active.
    pub fn has_cache(&self) -> bool {
        self.cache_dir.is_some()
    }

    pub fn cache_mode(&self) -> CacheMode {
        self.mode
    }

    /// HDD path for a chunk.
    fn base_path(&self, hash: &[u8; 32]) -> PathBuf {
        shard_path(&self.base_dir, hash)
    }

    /// SSD path for a chunk, if a cache tier exists.
    fn cache_path(&self, hash: &[u8; 32]) -> Option<PathBuf> {
        self.cache_dir.as_ref().map(|c| shard_path(c, hash))
    }

    /// Does this chunk already exist anywhere durable (SSD or HDD)?
    fn exists_anywhere(&self, hash: &[u8; 32]) -> bool {
        if self.base_path(hash).exists() {
            return true;
        }
        if let Some(cp) = self.cache_path(hash) {
            return cp.exists();
        }
        false
    }

    /// Store a chunk and return its hash.
    pub fn store(&self, data: &[u8]) -> Result<[u8; 32]> {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let hash: [u8; 32] = hasher.finalize().into();
        self.store_with_hash(&hash, data)?;
        Ok(hash)
    }

    /// Store a chunk with a known hash (replication path, and the engine of `store`).
    pub fn store_with_hash(&self, hash: &[u8; 32], data: &[u8]) -> Result<()> {
        // Deduplication: already durable somewhere → nothing to do.
        if self.exists_anywhere(hash) {
            debug!("Chunk {} already exists (deduplicated)", hex::encode(hash));
            // Keep it warm in memory for an imminent read.
            if let Ok(mut c) = self.read_cache.lock() {
                c.insert(*hash, data.to_vec());
            }
            return Ok(());
        }

        match (self.mode, self.cache_path(hash)) {
            (CacheMode::WriteBack, Some(cpath)) => {
                // SSD first, fsync'd + marked dirty; the flusher copies it to HDD later.
                write_atomic(&cpath, data, true)?;
                if let Ok(mut s) = self.cache_state.lock() {
                    s.track(*hash, data.len() as u64, true);
                }
            }
            (CacheMode::WriteThrough, Some(cpath)) => {
                // HDD is authoritative; mirror to SSD for fast reads (best-effort).
                write_atomic(&self.base_path(hash), data, false)?;
                if write_atomic(&cpath, data, false).is_ok() {
                    if let Ok(mut s) = self.cache_state.lock() {
                        s.track(*hash, data.len() as u64, false);
                    }
                }
            }
            _ => {
                // No cache tier — HDD only.
                write_atomic(&self.base_path(hash), data, false)?;
            }
        }

        if let Ok(mut c) = self.read_cache.lock() {
            c.insert(*hash, data.to_vec());
        }
        // Opportunistically keep the SSD under its watermark.
        self.maybe_evict();
        debug!("Stored chunk {} ({} bytes)", hex::encode(hash), data.len());
        Ok(())
    }

    /// Retrieve a chunk by its hash, fastest tier first.
    pub fn get(&self, hash: &[u8; 32]) -> Result<Vec<u8>> {
        // 1. RAM.
        if let Ok(mut c) = self.read_cache.lock() {
            if let Some(data) = c.get(hash) {
                return Ok(data);
            }
        }

        // 2. SSD tier.
        if let Some(cpath) = self.cache_path(hash) {
            if cpath.exists() {
                if let Ok(data) = read_file(&cpath) {
                    if let Ok(mut s) = self.cache_state.lock() {
                        s.touch(hash);
                    }
                    if let Ok(mut c) = self.read_cache.lock() {
                        c.insert(*hash, data.clone());
                    }
                    return Ok(data);
                }
            }
        }

        // 3. HDD.
        let bpath = self.base_path(hash);
        if !bpath.exists() {
            return Err(Error::ChunkNotFound(hex::encode(hash)));
        }
        let data = read_file(&bpath)?;

        // Promote hot data to the SSD read cache (write-through/​write-back only;
        // clean, so it's freely evictable). Best-effort.
        if self.has_cache() {
            if let Some(cpath) = self.cache_path(hash) {
                if !cpath.exists() && write_atomic(&cpath, &data, false).is_ok() {
                    if let Ok(mut s) = self.cache_state.lock() {
                        s.track(*hash, data.len() as u64, false);
                    }
                    self.maybe_evict();
                }
            }
        }
        if let Ok(mut c) = self.read_cache.lock() {
            c.insert(*hash, data.clone());
        }
        Ok(data)
    }

    /// Delete a chunk from every tier (garbage collection).
    pub fn delete(&self, hash: &[u8; 32]) -> Result<()> {
        if let Ok(mut c) = self.read_cache.lock() {
            c.remove(hash);
        }
        if let Some(cpath) = self.cache_path(hash) {
            if cpath.exists() {
                let _ = fs::remove_file(&cpath);
            }
            if let Ok(mut s) = self.cache_state.lock() {
                s.forget(hash);
            }
        }
        let bpath = self.base_path(hash);
        if bpath.exists() {
            fs::remove_file(&bpath)?;
            debug!("Deleted chunk {}", hex::encode(hash));
        }
        Ok(())
    }

    /// Check if a chunk exists in any durable tier.
    pub fn exists(&self, hash: &[u8; 32]) -> bool {
        self.exists_anywhere(hash)
    }

    // ── Write-back: flush + crash recovery + eviction ──────────────────────────

    /// Copy all currently-dirty (un-flushed) chunks from the SSD to the HDD and mark
    /// them clean. Safe to call from a background thread on any cadence; a no-op
    /// unless write-back is active. Returns the number of chunks flushed.
    pub fn flush_dirty(&self) -> usize {
        if self.mode != CacheMode::WriteBack {
            return 0;
        }
        // Snapshot the dirty set without holding the lock across I/O.
        let dirty: Vec<[u8; 32]> = match self.cache_state.lock() {
            Ok(s) => s.dirty.iter().copied().collect(),
            Err(_) => return 0,
        };
        let mut flushed = 0;
        for hash in dirty {
            let cpath = match self.cache_path(&hash) {
                Some(p) => p,
                None => continue,
            };
            let bpath = self.base_path(&hash);
            if bpath.exists() {
                // Already on HDD (e.g. dedup) — just mark clean.
                if let Ok(mut s) = self.cache_state.lock() {
                    s.mark_clean(&hash);
                }
                flushed += 1;
                continue;
            }
            let data = match read_file(&cpath) {
                Ok(d) => d,
                Err(e) => {
                    // If the SSD file is simply gone and it's not on the HDD either,
                    // the chunk was deleted (GC) between snapshot and read — drop it
                    // from the dirty set so it isn't retried forever (ghost entry).
                    if !cpath.exists() && !self.base_path(&hash).exists() {
                        if let Ok(mut s) = self.cache_state.lock() { s.forget(&hash); }
                    } else {
                        warn!("flush: cannot read cached chunk {}: {} (will retry)", hex::encode(hash), e);
                    }
                    continue;
                }
            };
            // fsync the HDD copy so "clean" truly means durable on the bulk tier.
            if let Err(e) = write_atomic(&bpath, &data, true) {
                warn!("flush: cannot write chunk {} to bulk store: {}", hex::encode(hash), e);
                continue;
            }
            if let Ok(mut s) = self.cache_state.lock() {
                s.mark_clean(&hash);
            }
            flushed += 1;
        }
        if flushed > 0 {
            debug!("flush_dirty: flushed {} chunk(s) to bulk store", flushed);
        }
        // Flushing freed dirty bytes for eviction.
        self.maybe_evict();
        flushed
    }

    /// After an unclean shutdown, re-discover chunks present on the SSD but not yet on
    /// the HDD and re-mark them dirty so `flush_dirty` copies them down. Call once at
    /// startup before serving. No-op without an SSD tier.
    pub fn recover_dirty(&self) -> usize {
        let cache_dir = match &self.cache_dir {
            Some(d) => d.clone(),
            None => return 0,
        };
        let mut recovered = 0;
        // The cache is sharded `<cache>/<2 hex>/<62 hex>`.
        let shards = match fs::read_dir(&cache_dir) {
            Ok(rd) => rd,
            Err(e) => {
                warn!("recover_dirty: cannot read cache dir {}: {} — dirty chunks (if any) will NOT be flushed",
                    cache_dir.display(), e);
                return 0;
            }
        };
        for shard in shards.flatten() {
            let sp = shard.path();
            if !sp.is_dir() {
                continue;
            }
            let prefix = match sp.file_name().and_then(|n| n.to_str()) {
                Some(p) if p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()) => p.to_string(),
                _ => continue,
            };
            let files = match fs::read_dir(&sp) {
                Ok(f) => f,
                Err(_) => continue,
            };
            for file in files.flatten() {
                let fp = file.path();
                let rest = match fp.file_name().and_then(|n| n.to_str()) {
                    // Skip leftover temp files from an interrupted write_atomic.
                    Some(r) if r.len() == 62 && r.chars().all(|c| c.is_ascii_hexdigit()) => r.to_string(),
                    Some(r) if r.ends_with(".tmp") => { let _ = fs::remove_file(&fp); continue; }
                    _ => continue,
                };
                let hex = format!("{}{}", prefix, rest);
                let hash = match hex_to_hash(&hex) {
                    Some(h) => h,
                    None => continue,
                };
                // Skip entries whose size we can't read rather than tracking 0 bytes
                // (which would corrupt the eviction watermark accounting).
                let size = match file.metadata() {
                    Ok(m) => m.len(),
                    Err(_) => continue,
                };
                let on_hdd = self.base_path(&hash).exists();
                if let Ok(mut s) = self.cache_state.lock() {
                    // Track it on the SSD; dirty iff it isn't on the HDD yet.
                    s.track(hash, size, !on_hdd);
                }
                if !on_hdd {
                    recovered += 1;
                }
            }
        }
        if recovered > 0 {
            warn!("recover_dirty: {} un-flushed cached chunk(s) re-queued for the bulk store", recovered);
        }
        recovered
    }

    /// Evict cold *clean* chunks from the SSD while it's over the watermark. Dirty
    /// (un-flushed) chunks are never evicted. No-op if unbounded or under budget.
    fn maybe_evict(&self) {
        if self.max_cache_bytes == 0 || !self.has_cache() {
            return;
        }
        loop {
            let victim = {
                let mut s = match self.cache_state.lock() {
                    Ok(s) => s,
                    Err(_) => return,
                };
                if s.total_bytes <= self.max_cache_bytes {
                    return;
                }
                match s.lru.pop_front() {
                    Some(h) => {
                        // pop_front already removed it from lru; drop its accounting.
                        if let Some(sz) = s.sizes.remove(&h) {
                            s.total_bytes = s.total_bytes.saturating_sub(sz);
                        }
                        h
                    }
                    None => return, // nothing clean left to evict
                }
            };
            // The HDD copy stays; just drop the SSD mirror.
            if let Some(cpath) = self.cache_path(&victim) {
                let _ = fs::remove_file(&cpath);
            }
            if let Ok(mut c) = self.read_cache.lock() {
                c.remove(&victim);
            }
        }
    }

    /// Read data from a file's chunks at a given offset.
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

            if chunk_end <= offset {
                continue;
            }
            if chunk_start >= end_offset {
                break;
            }

            let chunk_data = self.get(&chunk.hash)?;

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

            result.extend_from_slice(&chunk_data[read_start..read_end]);
            bytes_read += read_end - read_start;

            if bytes_read >= size {
                break;
            }
        }

        Ok(result)
    }

    /// Write data to a file's chunks at a given offset.
    pub fn write(&self, chunks: &mut Vec<ChunkRef>, offset: u64, data: &[u8]) -> Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }

        let write_end = offset + data.len() as u64;

        chunks.retain(|chunk| {
            let chunk_end = chunk.offset + chunk.size as u64;
            chunk_end <= offset || chunk.offset >= write_end
        });

        let mut written = 0;
        let mut current_offset = offset;

        while written < data.len() {
            let remaining = data.len() - written;
            let chunk_size = remaining.min(self.chunk_size);
            let chunk_data = &data[written..written + chunk_size];

            let hash = self.store(chunk_data)?;

            chunks.push(ChunkRef {
                hash,
                offset: current_offset,
                size: chunk_size as u32,
            });

            written += chunk_size;
            current_offset += chunk_size as u64;
        }

        chunks.sort_by_key(|c| c.offset);

        Ok(written)
    }
}

/// Parse a 64-char hex string into a 32-byte hash.
fn hex_to_hash(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let bytes = hex::decode(hex).ok()?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    Some(arr)
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

    #[test]
    fn write_through_mirrors_to_ssd_and_keeps_hdd_authoritative() {
        let hdd = tempdir().unwrap();
        let ssd = tempdir().unwrap();
        let store = ChunkStore::with_cache(
            hdd.path().to_path_buf(), 1024,
            Some(ssd.path().to_path_buf()), CacheMode::WriteThrough, 0).unwrap();

        let data = b"write-through chunk";
        let hash = store.store(data).unwrap();

        // Present on BOTH tiers immediately.
        assert!(shard_path(hdd.path(), &hash).exists(), "HDD copy missing");
        assert!(shard_path(ssd.path(), &hash).exists(), "SSD mirror missing");
        assert_eq!(store.get(&hash).unwrap(), data);
    }

    #[test]
    fn write_back_lands_on_ssd_then_flushes_to_hdd() {
        let hdd = tempdir().unwrap();
        let ssd = tempdir().unwrap();
        let store = ChunkStore::with_cache(
            hdd.path().to_path_buf(), 1024,
            Some(ssd.path().to_path_buf()), CacheMode::WriteBack, 0).unwrap();

        let data = b"write-back chunk";
        let hash = store.store(data).unwrap();

        // Before flush: on SSD (dirty), NOT yet on HDD.
        assert!(shard_path(ssd.path(), &hash).exists(), "SSD copy missing");
        assert!(!shard_path(hdd.path(), &hash).exists(), "must not be on HDD before flush");
        assert_eq!(store.cache_state.lock().unwrap().dirty.len(), 1);

        // Flush moves it to the HDD and clears dirty.
        assert_eq!(store.flush_dirty(), 1);
        assert!(shard_path(hdd.path(), &hash).exists(), "HDD copy missing after flush");
        assert!(store.cache_state.lock().unwrap().dirty.is_empty());
        assert_eq!(store.get(&hash).unwrap(), data);
    }

    #[test]
    fn recover_dirty_requeues_unflushed_chunks() {
        let hdd = tempdir().unwrap();
        let ssd = tempdir().unwrap();
        let data = b"survives a crash";
        let hash;
        {
            let store = ChunkStore::with_cache(
                hdd.path().to_path_buf(), 1024,
                Some(ssd.path().to_path_buf()), CacheMode::WriteBack, 0).unwrap();
            hash = store.store(data).unwrap();
            // Simulate a crash: chunk is on SSD (dirty), never flushed. Drop the store.
        }
        assert!(!shard_path(hdd.path(), &hash).exists());

        // Fresh store over the same dirs (restart). Recovery must re-queue + flush it.
        let store2 = ChunkStore::with_cache(
            hdd.path().to_path_buf(), 1024,
            Some(ssd.path().to_path_buf()), CacheMode::WriteBack, 0).unwrap();
        assert_eq!(store2.recover_dirty(), 1, "un-flushed chunk should be recovered");
        assert_eq!(store2.flush_dirty(), 1);
        assert!(shard_path(hdd.path(), &hash).exists(), "recovered chunk must reach HDD");
        assert_eq!(store2.get(&hash).unwrap(), data);
    }

    #[test]
    fn delete_of_a_dirty_chunk_clears_it_from_every_tier() {
        let hdd = tempdir().unwrap();
        let ssd = tempdir().unwrap();
        let store = ChunkStore::with_cache(
            hdd.path().to_path_buf(), 1024,
            Some(ssd.path().to_path_buf()), CacheMode::WriteBack, 0).unwrap();

        let hash = store.store(b"dirty then deleted").unwrap();
        assert_eq!(store.cache_state.lock().unwrap().dirty.len(), 1);

        // GC deletes it before the flush. It must leave no ghost in the dirty set
        // and a later flush must be a clean no-op (no permanent retry loop).
        store.delete(&hash).unwrap();
        assert!(store.cache_state.lock().unwrap().dirty.is_empty(), "dirty ghost left after delete");
        assert!(!shard_path(ssd.path(), &hash).exists());
        assert_eq!(store.flush_dirty(), 0);
    }

    #[test]
    fn eviction_drops_clean_ssd_copies_but_keeps_hdd() {
        let hdd = tempdir().unwrap();
        let ssd = tempdir().unwrap();
        // Tiny budget so the 2nd chunk evicts the 1st from the SSD.
        let store = ChunkStore::with_cache(
            hdd.path().to_path_buf(), 1024,
            Some(ssd.path().to_path_buf()), CacheMode::WriteThrough, 20).unwrap();

        let h1 = store.store(b"first chunk-aaaaaa").unwrap();   // ~18 bytes
        let h2 = store.store(b"second chunk-bbbbbb").unwrap();  // pushes over 20

        // Both remain on the authoritative HDD.
        assert!(shard_path(hdd.path(), &h1).exists());
        assert!(shard_path(hdd.path(), &h2).exists());
        // The cold one was evicted from the SSD; data still served from HDD.
        assert!(!shard_path(ssd.path(), &h1).exists(), "cold SSD copy should be evicted");
        assert_eq!(store.get(&h1).unwrap(), b"first chunk-aaaaaa");
    }
}
