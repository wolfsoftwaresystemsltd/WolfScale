//! WAL Segment Management
//!
//! Handles individual WAL segment files with memory-mapped I/O for performance.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::entry::{Lsn, WalEntry};
use crate::error::{Error, Result};

/// Magic bytes at the start of each segment file
const SEGMENT_MAGIC: &[u8; 8] = b"WLFSCALE";

/// Segment file version
const SEGMENT_VERSION: u32 = 1;

/// Header size in bytes
const HEADER_SIZE: usize = 32;

/// Segment file header
#[derive(Debug, Clone)]
pub struct SegmentHeader {
    /// First LSN in this segment
    pub first_lsn: Lsn,
    /// Last LSN in this segment (0 if segment is active)
    pub last_lsn: Lsn,
    /// Number of entries in this segment
    pub entry_count: u32,
    /// Whether this segment is sealed (no more writes)
    pub sealed: bool,
}

impl SegmentHeader {
    /// Create header for a new segment
    pub fn new(first_lsn: Lsn) -> Self {
        Self {
            first_lsn,
            last_lsn: 0,
            entry_count: 0,
            sealed: false,
        }
    }

    /// Serialize header to bytes
    pub fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        let mut bytes = [0u8; HEADER_SIZE];
        bytes[0..8].copy_from_slice(SEGMENT_MAGIC);
        bytes[8..12].copy_from_slice(&SEGMENT_VERSION.to_le_bytes());
        bytes[12..20].copy_from_slice(&self.first_lsn.to_le_bytes());
        bytes[20..28].copy_from_slice(&self.last_lsn.to_le_bytes());
        bytes[28..32].copy_from_slice(&self.entry_count.to_le_bytes());
        // Sealed flag could be in byte 32 when we expand
        bytes
    }

    /// Parse header from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < HEADER_SIZE {
            return Err(Error::Wal("Segment header too short".into()));
        }

        if &bytes[0..8] != SEGMENT_MAGIC {
            return Err(Error::Wal("Invalid segment magic bytes".into()));
        }

        let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        if version != SEGMENT_VERSION {
            return Err(Error::Wal(format!(
                "Unsupported segment version: {}",
                version
            )));
        }

        Ok(Self {
            first_lsn: u64::from_le_bytes(bytes[12..20].try_into().unwrap()),
            last_lsn: u64::from_le_bytes(bytes[20..28].try_into().unwrap()),
            entry_count: u32::from_le_bytes(bytes[28..32].try_into().unwrap()),
            sealed: false,
        })
    }
}

/// A single WAL segment file
pub struct Segment {
    /// Segment ID (derived from first LSN)
    pub id: u64,
    /// File path
    pub path: PathBuf,
    /// File handle
    file: File,
    /// Current write position
    write_pos: u64,
    /// Segment header
    header: SegmentHeader,
    /// Maximum segment size in bytes
    max_size: u64,
    /// Whether compression is enabled
    compression: bool,
}

impl Segment {
    /// Create a new segment file
    pub fn create(path: PathBuf, first_lsn: Lsn, max_size_mb: u64, compression: bool) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;

        let header = SegmentHeader::new(first_lsn);
        let mut segment = Self {
            id: first_lsn,
            path,
            file,
            write_pos: HEADER_SIZE as u64,
            header,
            max_size: max_size_mb * 1024 * 1024,
            compression,
        };

        // Write header
        segment.write_header()?;

        Ok(segment)
    }

    /// Open an existing segment file
    pub fn open(path: PathBuf, max_size_mb: u64, compression: bool) -> Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)?;

        // Read header
        let mut header_bytes = [0u8; HEADER_SIZE];
        file.read_exact(&mut header_bytes)?;
        let header = SegmentHeader::from_bytes(&header_bytes)?;

        // Find write position (end of file for active segments)
        let write_pos = file.seek(SeekFrom::End(0))?;

        Ok(Self {
            id: header.first_lsn,
            path,
            file,
            write_pos,
            header,
            max_size: max_size_mb * 1024 * 1024,
            compression,
        })
    }

    /// Write an entry to the segment
    pub fn append(&mut self, entry: &WalEntry) -> Result<u64> {
        let serialized = bincode::serialize(entry)?;
        
        let data = if self.compression {
            lz4_flex::compress_prepend_size(&serialized)
        } else {
            serialized
        };

        // Entry format: [length: u32][compressed: u8][data: bytes][checksum: u32]
        let entry_len = data.len() as u32;
        let checksum = crc32fast::hash(&data);

        // Check if we have space
        let required_space = 4 + 1 + data.len() + 4;
        if self.write_pos + required_space as u64 > self.max_size {
            return Err(Error::Wal("Segment full".into()));
        }

        // Write entry
        self.file.seek(SeekFrom::Start(self.write_pos))?;
        self.file.write_all(&entry_len.to_le_bytes())?;
        self.file.write_all(&[self.compression as u8])?;
        self.file.write_all(&data)?;
        self.file.write_all(&checksum.to_le_bytes())?;

        let entry_pos = self.write_pos;
        self.write_pos += required_space as u64;
        self.header.entry_count += 1;
        self.header.last_lsn = entry.header.lsn;

        Ok(entry_pos)
    }

    /// Read an entry at a specific position
    pub fn read_at(&mut self, pos: u64) -> Result<WalEntry> {
        self.file.seek(SeekFrom::Start(pos))?;

        // Read length
        let mut len_bytes = [0u8; 4];
        self.file.read_exact(&mut len_bytes)?;
        let entry_len = u32::from_le_bytes(len_bytes) as usize;

        // Read compression flag
        let mut compressed_flag = [0u8; 1];
        self.file.read_exact(&mut compressed_flag)?;
        let is_compressed = compressed_flag[0] != 0;

        // Read data
        let mut data = vec![0u8; entry_len];
        self.file.read_exact(&mut data)?;

        // Read and verify checksum
        let mut checksum_bytes = [0u8; 4];
        self.file.read_exact(&mut checksum_bytes)?;
        let stored_checksum = u32::from_le_bytes(checksum_bytes);
        let computed_checksum = crc32fast::hash(&data);

        if stored_checksum != computed_checksum {
            return Err(Error::WalCorrupted {
                lsn: 0, // We don't know the LSN yet
                reason: "Checksum mismatch".into(),
            });
        }

        // Decompress if needed
        let serialized = if is_compressed {
            lz4_flex::decompress_size_prepended(&data)
                .map_err(|e| Error::Wal(format!("Decompression failed: {}", e)))?
        } else {
            data
        };

        let entry: WalEntry = bincode::deserialize(&serialized)?;
        Ok(entry)
    }

    /// Iterate over all entries in the segment
    pub fn iter(&mut self) -> SegmentIterator<'_> {
        SegmentIterator {
            segment: self,
            pos: HEADER_SIZE as u64,
        }
    }

    /// Sync segment to disk
    pub fn sync(&self) -> Result<()> {
        self.file.sync_all()?;
        Ok(())
    }

    /// Seal the segment (no more writes)
    pub fn seal(&mut self) -> Result<()> {
        self.header.sealed = true;
        self.write_header()?;
        self.sync()
    }

    /// Check if segment still has space
    pub fn has_space(&self, additional_bytes: usize) -> bool {
        self.write_pos + additional_bytes as u64 <= self.max_size
    }

    /// Check if segment is sealed
    pub fn is_sealed(&self) -> bool {
        self.header.sealed
    }

    /// Get the first LSN in this segment
    pub fn first_lsn(&self) -> Lsn {
        self.header.first_lsn
    }

    /// Get the last LSN in this segment
    pub fn last_lsn(&self) -> Lsn {
        self.header.last_lsn
    }

    /// Get entry count
    pub fn entry_count(&self) -> u32 {
        self.header.entry_count
    }

    /// Write header to file
    fn write_header(&mut self) -> Result<()> {
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&self.header.to_bytes())?;
        Ok(())
    }
}

/// Iterator over entries in a segment
pub struct SegmentIterator<'a> {
    segment: &'a mut Segment,
    pos: u64,
}

impl<'a> Iterator for SegmentIterator<'a> {
    type Item = Result<WalEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.segment.write_pos {
            return None;
        }

        let result = self.segment.read_at(self.pos);
        match &result {
            Ok(entry) => {
                // Calculate next position
                let serialized = bincode::serialize(entry).unwrap();
                let data_len = if self.segment.compression {
                    lz4_flex::compress_prepend_size(&serialized).len()
                } else {
                    serialized.len()
                };
                self.pos += 4 + 1 + data_len as u64 + 4; // len + flag + data + checksum
            }
            Err(_) => {
                // Stop iteration on error
                self.pos = self.segment.write_pos;
            }
        }

        Some(result)
    }
}

/// List all segment files in a directory
pub fn list_segments(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut segments = Vec::new();

    if !dir.exists() {
        return Ok(segments);
    }

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map_or(false, |ext| ext == "log")
            && path
                .file_stem()
                .and_then(|s| s.to_str())
                .map_or(false, |s| s.starts_with("wal_"))
        {
            segments.push(path);
        }
    }

    // Sort by segment ID (embedded in filename)
    segments.sort();
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wal::entry::{LogEntry, Value, PrimaryKey};
    use tempfile::tempdir;

    #[test]
    fn test_segment_create_and_append() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_segment.log");

        let mut segment = Segment::create(path.clone(), 1, 64, false).unwrap();

        let entry = WalEntry::new(
            1,
            1,
            "node-1".to_string(),
            LogEntry::Insert {
                table: "users".to_string(),
                columns: vec!["id".to_string(), "name".to_string()],
                values: vec![Value::Int(1), Value::String("Alice".to_string())],
                primary_key: PrimaryKey::Int(1),
            },
        );

        let pos = segment.append(&entry).unwrap();
        assert!(pos >= HEADER_SIZE as u64);

        segment.sync().unwrap();
    }

    #[test]
    fn test_segment_read_write() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_rw.log");

        let mut segment = Segment::create(path.clone(), 1, 64, true).unwrap();

        // Write entries
        for i in 1..=10 {
            let entry = WalEntry::new(
                i,
                1,
                "node-1".to_string(),
                LogEntry::Insert {
                    table: "test".to_string(),
                    columns: vec!["id".to_string()],
                    values: vec![Value::Int(i as i64)],
                    primary_key: PrimaryKey::Int(i as i64),
                },
            );
            segment.append(&entry).unwrap();
        }

        // Read back via iterator
        let mut count = 0;
        for result in segment.iter() {
            let entry = result.unwrap();
            count += 1;
            assert_eq!(entry.header.lsn, count);
        }
        assert_eq!(count, 10);
    }
}
