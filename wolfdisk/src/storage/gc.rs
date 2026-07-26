// Written by Paul Clevett
// (C)Copyright Wolf Software Systems Ltd

//! Chunk garbage collection — mark-and-sweep.
//!
//! Chunks are content-addressed and DEDUPLICATED: two files with identical
//! content share the same chunk files. Deleting chunks synchronously when a
//! file is removed therefore corrupted every identical twin (delete file A →
//! file B's chunks vanish). Every delete/truncate/rename-overwrite path used
//! to do exactly that.
//!
//! The fix is to make the file index the single source of truth: nothing
//! deletes chunk files at operation time any more — operations only drop
//! references. This collector periodically computes the referenced set from
//! the CURRENT index (plus in-flight S3 multipart parts, whose chunks are not
//! in the index until CompleteMultipartUpload) and removes chunk files nobody
//! references.
//!
//! Two safeguards make the sweep safe against concurrent writers:
//! - **Grace window**: a chunk file younger than `grace` is never removed,
//!   covering the gap between a chunk being written/streamed and its index
//!   entry landing (and multipart parts on other nodes).
//! - **Dedup interlock** (in `ChunkStore`): a write that deduplicates against
//!   an existing chunk bumps that chunk's mtime under the store's GC lock, and
//!   the sweep re-checks age under the same lock immediately before unlinking.
//!   Without this, a write could dedup against a >grace-old orphan the very
//!   moment the sweep removes it.
//!
//! A missed sweep only ever LEAKS space until the next pass — it can never
//! remove live data.

use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use tracing::debug;

use crate::storage::chunks::ChunkStore;
use crate::storage::index::FileIndex;

/// Outcome of one sweep, for logging/telemetry.
#[derive(Debug, Default, Clone, Copy)]
pub struct GcStats {
    /// Chunk files seen across both tiers.
    pub scanned: usize,
    /// Kept: referenced by the index or multipart parts.
    pub referenced: usize,
    /// Kept: unreferenced but younger than the grace window (either at scan
    /// time or at the final under-lock recheck).
    pub young: usize,
    /// Removed.
    pub removed: usize,
    /// Bytes reclaimed by the removals.
    pub reclaimed_bytes: u64,
}

/// May an unreferenced chunk file with this mtime be swept? Pure so the
/// decision has direct tests. A file whose mtime is in the future (clock
/// skew, restored backup) reads as age zero — kept, never swept early.
pub fn sweepable(is_referenced: bool, mtime: SystemTime, now: SystemTime, grace: Duration) -> bool {
    if is_referenced {
        return false;
    }
    match now.duration_since(mtime) {
        Ok(age) => age >= grace,
        Err(_) => false, // mtime ahead of now → treat as brand new
    }
}

/// Build the referenced-hash set: every chunk of every index entry, plus any
/// extra references the caller knows about (in-flight S3 multipart parts).
pub fn referenced_chunk_hashes(
    index: &FileIndex,
    extra: impl IntoIterator<Item = [u8; 32]>,
) -> HashSet<[u8; 32]> {
    let mut set: HashSet<[u8; 32]> = HashSet::new();
    for (_, entry) in index.iter() {
        for c in &entry.chunks {
            set.insert(c.hash);
        }
    }
    set.extend(extra);
    set
}

/// One mark-and-sweep pass. `referenced` must have been built from the live
/// index (see `referenced_chunk_hashes`) — building it BEFORE the scan is
/// fine: a reference added after the snapshot belongs to a chunk that is
/// either brand new (young → kept) or was just deduplicated against (mtime
/// bumped under the store's GC lock → the under-lock recheck keeps it).
pub fn collect_garbage(
    store: &ChunkStore,
    referenced: &HashSet<[u8; 32]>,
    grace: Duration,
) -> GcStats {
    let now = SystemTime::now();
    let mut stats = GcStats::default();
    let mut candidates: Vec<[u8; 32]> = Vec::new();

    store.for_each_stored_chunk(|hash, mtime, _size| {
        stats.scanned += 1;
        if referenced.contains(hash) {
            stats.referenced += 1;
        } else if !sweepable(false, mtime, now, grace) {
            stats.young += 1;
        } else {
            candidates.push(*hash);
        }
    });

    for hash in candidates {
        // Final age recheck + unlink happen atomically w.r.t. dedup writes.
        match store.gc_delete_if_still_old(&hash, grace) {
            Some(bytes) => {
                stats.removed += 1;
                stats.reclaimed_bytes += bytes;
            }
            None => stats.young += 1, // touched (or already gone) since the scan
        }
    }

    debug!(
        "Chunk GC: scanned {}, referenced {}, young {}, removed {} ({} bytes)",
        stats.scanned, stats.referenced, stats.young, stats.removed, stats.reclaimed_bytes
    );
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn sweep_decision() {
        let now = SystemTime::now();
        let old = now - secs(7200);
        let fresh = now - secs(60);
        let future = now + secs(600);
        let grace = secs(3600);

        // Referenced chunks are never sweepable, no matter how old.
        assert!(!sweepable(true, old, now, grace));
        // Unreferenced + past the grace window → sweep.
        assert!(sweepable(false, old, now, grace));
        // Unreferenced but young → keep (in-flight write protection).
        assert!(!sweepable(false, fresh, now, grace));
        // Clock skew (mtime in the future) → keep, never sweep early.
        assert!(!sweepable(false, future, now, grace));
        // Exactly at the boundary → sweepable (>=).
        assert!(sweepable(false, now - grace, now, grace));
    }

    #[test]
    fn end_to_end_sweep_respects_references_and_grace() {
        let dir = tempfile::tempdir().unwrap();
        let store = ChunkStore::new(dir.path().to_path_buf(), 4 * 1024 * 1024).unwrap();

        let kept_ref = store.store(b"referenced data").unwrap();
        let kept_young = store.store(b"young orphan").unwrap();
        let doomed = store.store(b"old orphan").unwrap();

        // Reference one chunk via a minimal index entry.
        let mut index = FileIndex::new();
        index.insert(
            std::path::PathBuf::from("/keep"),
            crate::storage::FileEntry {
                size: 15,
                is_dir: false,
                permissions: 0o644,
                uid: 0,
                gid: 0,
                created: SystemTime::now(),
                modified: SystemTime::now(),
                accessed: SystemTime::now(),
                chunks: vec![crate::storage::ChunkRef {
                    hash: kept_ref,
                    offset: 0,
                    size: 15,
                }],
                symlink_target: None,
            },
        );
        let referenced = referenced_chunk_hashes(&index, std::iter::empty());

        // Age the doomed chunk past the grace window by back-dating its mtime.
        store.test_backdate_chunk(&doomed, secs(7200));

        let stats = collect_garbage(&store, &referenced, secs(3600));
        assert_eq!(stats.removed, 1);
        assert!(store.exists(&kept_ref), "referenced chunk must survive");
        assert!(store.exists(&kept_young), "young orphan must survive grace");
        assert!(!store.exists(&doomed), "old orphan must be swept");

        // Dedup interlock: re-storing the young orphan's content marks it
        // young again — a subsequent sweep with zero-ish grace must still see
        // the freshly-bumped mtime from the recheck path.
        store.test_backdate_chunk(&kept_young, secs(7200));
        let _ = store.store(b"young orphan").unwrap(); // dedup hit → mtime bump
        let stats2 = collect_garbage(&store, &referenced, secs(3600));
        assert_eq!(stats2.removed, 0, "deduped-against chunk must not be swept");
        assert!(store.exists(&kept_young));
    }
}
