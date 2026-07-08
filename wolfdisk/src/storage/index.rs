//! File metadata index

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

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

    /// Symlink target path (if this is a symlink)
    #[serde(default)]
    pub symlink_target: Option<String>,
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

/// Outcome of one attempt to read the on-disk index (internal — public
/// callers pick a recovery policy via `load_or_create`/`load_or_recover`).
enum TryLoad {
    /// No index file at all (first boot / fresh node).
    Absent,
    Valid(FileIndex),
    /// File exists and reads, but isn't valid JSON for FileIndex —
    /// truncated by an interrupted write, or garbage.
    Corrupt(serde_json::Error),
    /// Parsed fine but carries a different INDEX_VERSION. Distinct from
    /// `Absent`: the on-disk data is about to be discarded, so cluster
    /// mode must ALSO discard its replication state — an empty index
    /// claiming the old version would tell peers "in sync" forever.
    VersionMismatch(u32),
}

impl FileIndex {
    /// Create a new empty index
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            version: INDEX_VERSION,
        }
    }

    /// One load attempt, with the failure mode preserved: I/O errors
    /// (permissions, dying disk) propagate as `Err`; a file that READS
    /// fine but doesn't PARSE is reported as `Corrupt` so callers can
    /// choose their recovery policy.
    fn try_load(index_path: &Path) -> Result<TryLoad> {
        if !index_path.exists() {
            return Ok(TryLoad::Absent);
        }
        info!("Loading file index from {:?}", index_path);
        let bytes = fs::read(index_path)?;
        match serde_json::from_slice::<FileIndex>(&bytes) {
            Ok(index) if index.version != INDEX_VERSION => Ok(TryLoad::VersionMismatch(index.version)),
            Ok(index) => {
                info!("Loaded {} entries", index.entries.len());
                Ok(TryLoad::Valid(index))
            }
            Err(e) => Ok(TryLoad::Corrupt(e)),
        }
    }

    /// Load index from disk or create new if not exists. A corrupt index
    /// file is an error — used by STANDALONE mode, where there are no
    /// peers to rebuild from, so silently starting with an empty
    /// namespace would present all data as gone. Cluster mode uses
    /// `load_or_recover` instead.
    pub fn load_or_create(index_dir: &Path) -> Result<Self> {
        match Self::try_load(&index_dir.join(INDEX_FILENAME))? {
            TryLoad::Absent => {
                info!("No existing index, creating new");
                Ok(Self::new())
            }
            TryLoad::Valid(index) => Ok(index),
            TryLoad::Corrupt(e) => Err(e.into()),
            // Standalone keeps the long-standing migration behaviour:
            // an old-version index is replaced with a fresh one.
            TryLoad::VersionMismatch(v) => {
                info!("Index version mismatch (found v{}, want v{}), creating new index", v, INDEX_VERSION);
                Ok(Self::new())
            }
        }
    }

    /// Cluster-mode load: a corrupt index file (truncated in-place write,
    /// power loss) is quarantined next to itself as
    /// `index.json.corrupt-<unixtime>` and an EMPTY index returned with
    /// `recovered = true`. The caller must then discard its replication
    /// state so the node rejoins at v0 and the leader's authoritative
    /// full sync rebuilds it — an empty node still claiming its old
    /// version would tell peers "in sync" and never be repopulated.
    ///
    /// Before this, a corrupt index aborted startup via `.expect()` and
    /// systemd crash-looped the service forever (klas's node "ninni",
    /// 2026-07-08, restart counter 175).
    pub fn load_or_recover(index_dir: &Path) -> Result<(Self, bool)> {
        let index_path = index_dir.join(INDEX_FILENAME);
        match Self::try_load(&index_path)? {
            TryLoad::Absent => {
                info!("No existing index, creating new");
                Ok((Self::new(), false))
            }
            TryLoad::Valid(index) => Ok((index, false)),
            TryLoad::Corrupt(e) => {
                let quarantine = Self::quarantine_path(index_dir);
                // Quarantine, never delete — the corrupt file is the
                // post-mortem evidence. If even the rename fails, fall
                // back to failing startup rather than looping on it.
                fs::rename(&index_path, &quarantine)?;
                error!(
                    "File index {:?} is corrupt ({}) — quarantined to {:?}; \
                     starting empty and relying on the cluster full sync to rebuild this node",
                    index_path, e, quarantine
                );
                Ok((Self::new(), true))
            }
            // In cluster mode a version migration ALSO discards the local
            // data, so it must take the same recovered path — otherwise
            // the node keeps its old replication version with an empty
            // index and peers believe it's in sync (never repopulated).
            TryLoad::VersionMismatch(v) => {
                let quarantine = Self::quarantine_path(index_dir);
                fs::rename(&index_path, &quarantine)?;
                warn!(
                    "Index version mismatch (found v{}, want v{}) — old index kept at {:?}; \
                     starting empty and relying on the cluster full sync to rebuild this node",
                    v, INDEX_VERSION, quarantine
                );
                Ok((Self::new(), true))
            }
        }
    }

    /// Where a discarded index file is preserved: `index.json.corrupt-<unixtime>`.
    fn quarantine_path(index_dir: &Path) -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        index_dir.join(format!("{}.corrupt-{}", INDEX_FILENAME, ts))
    }

    /// Save index to disk atomically. Serializes, then delegates to
    /// `write_serialized` — the live index must NEVER be truncated in
    /// place: an interrupted `File::create`-then-write left a 0-byte
    /// index.json that crash-looped the node on every start (ninni,
    /// 2026-07-08).
    pub fn save(&self, index_dir: &Path) -> Result<()> {
        let bytes = self.serialize()?;
        Self::write_serialized(index_dir, &bytes)?;
        debug!("Saved file index with {} entries", self.entries.len());
        Ok(())
    }

    /// Serialize the index to bytes WITHOUT touching the filesystem, so a caller
    /// can hold the read lock only for serialization (CPU) and do the slow disk
    /// write AFTER releasing it (see the periodic persistence thread). Saving
    /// the whole index under the read lock stalled every writer for the entire
    /// disk write every 5s — on a 250k-entry index that froze replication on a
    /// bulk copy (wabil 2026-06-28).
    pub fn serialize(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec_pretty(self)
    }

    /// Write pre-serialized index bytes to disk atomically (temp + fsync +
    /// rename + directory fsync), so neither a crash mid-write nor a power
    /// loss straight after can leave a truncated/empty live index. Pair
    /// with `serialize()`.
    ///
    /// Serialized process-wide: the FUSE debounced save and the periodic
    /// persistence thread both land here on independent 5s cadences with
    /// only read locks on the index — without this gate they'd interleave
    /// writes to the SAME tmp file and could rename a half-written one
    /// into place, recreating the very corruption this path prevents.
    pub fn write_serialized(index_dir: &Path, bytes: &[u8]) -> Result<()> {
        static DISK_WRITE_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _gate = DISK_WRITE_GATE.lock().unwrap_or_else(|p| p.into_inner());

        fs::create_dir_all(index_dir)?;
        let index_path = index_dir.join(INDEX_FILENAME);
        let tmp_path = index_dir.join(format!("{}.tmp", INDEX_FILENAME));
        {
            let mut tmp = File::create(&tmp_path)?;
            tmp.write_all(bytes)?;
            // fsync BEFORE the rename: the swap is only crash-safe if the
            // new content is durable first — otherwise power loss can
            // land the rename with zero-length content, which is exactly
            // the "expected value at line 1 column 1" boot failure.
            tmp.sync_all()?;
        }
        fs::rename(&tmp_path, &index_path)?;
        // fsync the directory so the rename itself survives power loss.
        // Failure here only weakens the power-loss guarantee (the rename
        // is already visible) — log it, don't fail the save.
        match File::open(index_dir) {
            Ok(dir) => {
                if let Err(e) = dir.sync_all() {
                    warn!("Index dir fsync failed ({}); rename durability not guaranteed", e);
                }
            }
            Err(e) => warn!("Index dir open for fsync failed ({}); rename durability not guaranteed", e),
        }
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
    pub fn insert(&mut self, path: PathBuf, entry: FileEntry) -> Option<FileEntry> {
        self.entries.insert(path, entry)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> FileEntry {
        let now = SystemTime::now();
        FileEntry {
            size: 42,
            is_dir: false,
            permissions: 0o644,
            uid: 0,
            gid: 0,
            created: now,
            modified: now,
            accessed: now,
            chunks: Vec::new(),
            symlink_target: None,
        }
    }

    #[test]
    fn save_load_round_trip_and_no_tmp_leftover() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = FileIndex::new();
        index.insert(PathBuf::from("/a.txt"), entry());
        index.save(dir.path()).unwrap();

        assert!(!dir.path().join(format!("{}.tmp", INDEX_FILENAME)).exists());
        let loaded = FileIndex::load_or_create(dir.path()).unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(loaded.contains(Path::new("/a.txt")));
    }

    #[test]
    fn recover_quarantines_empty_index_file() {
        // The ninni failure: a 0-byte index.json ("expected value at line 1
        // column 1") must NOT crash-loop — quarantine + start empty.
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(INDEX_FILENAME), b"").unwrap();

        let (index, recovered) = FileIndex::load_or_recover(dir.path()).unwrap();
        assert!(recovered);
        assert!(index.is_empty());
        assert!(!dir.path().join(INDEX_FILENAME).exists(), "corrupt file must be moved aside");
        let quarantined: Vec<_> = fs::read_dir(dir.path()).unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("index.json.corrupt-"))
            .collect();
        assert_eq!(quarantined.len(), 1, "corrupt file must be preserved as evidence");
    }

    #[test]
    fn recover_quarantines_garbage_index_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(INDEX_FILENAME), b"{\"entries\": {\"trunc").unwrap();

        let (index, recovered) = FileIndex::load_or_recover(dir.path()).unwrap();
        assert!(recovered);
        assert!(index.is_empty());
    }

    #[test]
    fn recover_passes_valid_index_through() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = FileIndex::new();
        index.insert(PathBuf::from("/keep.bin"), entry());
        index.save(dir.path()).unwrap();

        let (loaded, recovered) = FileIndex::load_or_recover(dir.path()).unwrap();
        assert!(!recovered);
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn standalone_load_or_create_still_fails_on_corrupt() {
        // Standalone mode has no peers to rebuild from — swallowing the
        // corruption would present the whole namespace as empty. It must
        // surface the error and leave the file in place for the operator.
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(INDEX_FILENAME), b"").unwrap();

        assert!(FileIndex::load_or_create(dir.path()).is_err());
        assert!(dir.path().join(INDEX_FILENAME).exists(), "strict path must not move the file");
    }

    #[test]
    fn version_mismatch_standalone_creates_new_index_in_place() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(INDEX_FILENAME), b"{\"entries\":{},\"version\":999}").unwrap();

        let index = FileIndex::load_or_create(dir.path()).unwrap();
        assert!(index.is_empty());
        assert!(dir.path().join(INDEX_FILENAME).exists(), "standalone migration leaves the file for the operator");
    }

    #[test]
    fn version_mismatch_cluster_takes_recovery_path() {
        // Discarding an old-version index empties the node exactly like
        // corruption does — replication state must be discarded too, or
        // the node claims its old version with zero entries and peers
        // never repopulate it. recovered=true drives that in main.rs.
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(INDEX_FILENAME), b"{\"entries\":{},\"version\":999}").unwrap();

        let (index, recovered) = FileIndex::load_or_recover(dir.path()).unwrap();
        assert!(index.is_empty());
        assert!(recovered, "version migration empties the node → must resync from peers");
        assert!(!dir.path().join(INDEX_FILENAME).exists(), "old index moved aside");
    }
}
