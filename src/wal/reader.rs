//! WAL Reader
//!
//! Provides reading capabilities for the WAL, supporting both
//! sequential iteration and random access by LSN.

use std::collections::BTreeMap;
use std::path::PathBuf;

use super::entry::{Lsn, WalEntry};
use super::segment::{list_segments, Segment};
use super::WalPaths;
use crate::error::Result;

/// WAL Reader for accessing log entries
pub struct WalReader {
    /// WAL paths
    paths: WalPaths,
    /// Segment size in MB
    segment_size_mb: u64,
    /// Whether compression is enabled
    compression: bool,
    /// Cached segment index: LSN -> segment path
    segment_index: BTreeMap<Lsn, PathBuf>,
}

impl WalReader {
    /// Create a new WAL reader
    pub fn new(data_dir: PathBuf, segment_size_mb: u64, compression: bool) -> Result<Self> {
        let paths = WalPaths::new(data_dir.join("wal"));

        let mut reader = Self {
            paths,
            segment_size_mb,
            compression,
            segment_index: BTreeMap::new(),
        };

        reader.refresh_index()?;
        Ok(reader)
    }

    /// Refresh the segment index
    pub fn refresh_index(&mut self) -> Result<()> {
        self.segment_index.clear();

        let segments = list_segments(&self.paths.base_dir)?;
        for path in segments {
            let segment = Segment::open(path.clone(), self.segment_size_mb, self.compression)?;
            self.segment_index.insert(segment.first_lsn(), path);
        }

        Ok(())
    }

    /// Get the first LSN in the log
    pub fn first_lsn(&self) -> Option<Lsn> {
        self.segment_index.keys().next().copied()
    }

    /// Get the last LSN in the log
    pub fn last_lsn(&self) -> Result<Option<Lsn>> {
        if let Some(path) = self.segment_index.values().last() {
            let mut segment = Segment::open(path.clone(), self.segment_size_mb, self.compression)?;
            
            let mut last = None;
            for result in segment.iter() {
                if let Ok(entry) = result {
                    last = Some(entry.header.lsn);
                }
            }
            
            Ok(last)
        } else {
            Ok(None)
        }
    }

    /// Find the segment containing a specific LSN
    #[allow(dead_code)]
    fn find_segment(&self, lsn: Lsn) -> Option<&PathBuf> {
        // Find the segment with the largest first_lsn <= target lsn
        self.segment_index
            .range(..=lsn)
            .next_back()
            .map(|(_, path)| path)
    }

    /// Read entries starting from a specific LSN
    pub fn read_from(&self, from_lsn: Lsn) -> Result<Vec<WalEntry>> {
        let mut entries = Vec::new();
        
        // Find starting segment
        let start_first_lsn = self.segment_index
            .range(..=from_lsn)
            .next_back()
            .map(|(lsn, _)| *lsn);
        
        let start_lsn = match start_first_lsn {
            Some(lsn) => lsn,
            None => {
                // from_lsn is before our first segment
                if let Some(first) = self.segment_index.keys().next() {
                    *first
                } else {
                    return Ok(entries);
                }
            }
        };

        // Iterate through segments
        for (_segment_lsn, path) in self.segment_index.range(start_lsn..) {
            let mut segment = Segment::open(path.clone(), self.segment_size_mb, self.compression)?;
            
            for result in segment.iter() {
                let entry = result?;
                if entry.header.lsn >= from_lsn {
                    entries.push(entry);
                }
            }
        }

        Ok(entries)
    }

    /// Read entries in a specific LSN range (inclusive)
    pub fn read_range(&self, from_lsn: Lsn, to_lsn: Lsn) -> Result<Vec<WalEntry>> {
        let mut entries = self.read_from(from_lsn)?;
        entries.retain(|e| e.header.lsn <= to_lsn);
        Ok(entries)
    }

    /// Read entries in batches for replication
    pub fn read_batch(&self, from_lsn: Lsn, max_entries: usize) -> Result<Vec<WalEntry>> {
        let mut entries = Vec::with_capacity(max_entries);
        
        // Find starting segment
        let start_first_lsn = self.segment_index
            .range(..=from_lsn)
            .next_back()
            .map(|(lsn, _)| *lsn);
        
        let start_lsn = match start_first_lsn {
            Some(lsn) => lsn,
            None => {
                if let Some(first) = self.segment_index.keys().next() {
                    *first
                } else {
                    return Ok(entries);
                }
            }
        };

        'outer: for (_, path) in self.segment_index.range(start_lsn..) {
            let mut segment = Segment::open(path.clone(), self.segment_size_mb, self.compression)?;
            
            for result in segment.iter() {
                let entry = result?;
                if entry.header.lsn >= from_lsn {
                    entries.push(entry);
                    if entries.len() >= max_entries {
                        break 'outer;
                    }
                }
            }
        }

        Ok(entries)
    }

    /// Get a specific entry by LSN
    pub fn get(&self, lsn: Lsn) -> Result<Option<WalEntry>> {
        let entries = self.read_range(lsn, lsn)?;
        Ok(entries.into_iter().next())
    }

    /// Count total entries in the log
    pub fn count(&self) -> Result<u64> {
        let mut count = 0u64;
        
        for path in self.segment_index.values() {
            let segment = Segment::open(path.clone(), self.segment_size_mb, self.compression)?;
            count += segment.entry_count() as u64;
        }

        Ok(count)
    }

    /// Get all segment info
    pub fn segments(&self) -> Result<Vec<SegmentInfo>> {
        let mut infos = Vec::new();

        for (first_lsn, path) in &self.segment_index {
            let segment = Segment::open(path.clone(), self.segment_size_mb, self.compression)?;
            infos.push(SegmentInfo {
                id: segment.id,
                path: path.clone(),
                first_lsn: *first_lsn,
                last_lsn: segment.last_lsn(),
                entry_count: segment.entry_count(),
                sealed: segment.is_sealed(),
            });
        }

        Ok(infos)
    }

    /// Create an async stream of entries
    pub fn stream_from(&self, from_lsn: Lsn) -> impl Iterator<Item = Result<WalEntry>> + '_ {
        WalEntryIterator::new(self, from_lsn)
    }
}

/// Information about a WAL segment
#[derive(Debug, Clone)]
pub struct SegmentInfo {
    pub id: u64,
    pub path: PathBuf,
    pub first_lsn: Lsn,
    pub last_lsn: Lsn,
    pub entry_count: u32,
    pub sealed: bool,
}

/// Iterator over WAL entries starting from a specific LSN
pub struct WalEntryIterator<'a> {
    reader: &'a WalReader,
    current_segment: Option<(PathBuf, Segment)>,
    segment_iter: std::collections::btree_map::Range<'a, Lsn, PathBuf>,
    from_lsn: Lsn,
    started: bool,
}

impl<'a> WalEntryIterator<'a> {
    fn new(reader: &'a WalReader, from_lsn: Lsn) -> Self {
        // Find starting segment
        let start_lsn = reader.segment_index
            .range(..=from_lsn)
            .next_back()
            .map(|(lsn, _)| *lsn)
            .unwrap_or_else(|| {
                reader.segment_index.keys().next().copied().unwrap_or(from_lsn)
            });

        Self {
            reader,
            current_segment: None,
            segment_iter: reader.segment_index.range(start_lsn..),
            from_lsn,
            started: false,
        }
    }

    fn advance_segment(&mut self) -> Option<()> {
        let (_, path) = self.segment_iter.next()?;
        let segment = Segment::open(
            path.clone(),
            self.reader.segment_size_mb,
            self.reader.compression,
        ).ok()?;
        self.current_segment = Some((path.clone(), segment));
        Some(())
    }
}

impl<'a> Iterator for WalEntryIterator<'a> {
    type Item = Result<WalEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        // Lazy initialization
        if !self.started {
            self.started = true;
            self.advance_segment()?;
        }

        loop {
            if let Some((_, ref mut segment)) = self.current_segment {
                for result in segment.iter() {
                    match result {
                        Ok(entry) if entry.header.lsn >= self.from_lsn => {
                            return Some(Ok(entry));
                        }
                        Ok(_) => continue, // Skip entries before from_lsn
                        Err(e) => return Some(Err(e)),
                    }
                }
            }

            // Current segment exhausted, move to next
            if self.advance_segment().is_none() {
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wal::entry::{LogEntry, Value, PrimaryKey};
    use crate::wal::writer::WalWriter;
    use crate::config::WalConfig;
    use tempfile::tempdir;

    fn test_config() -> WalConfig {
        WalConfig {
            batch_size: 10,
            flush_interval_ms: 10,
            compression: true,
            segment_size_mb: 1,
            retention_hours: 0,
            fsync: false,
        }
    }

    #[tokio::test]
    async fn test_reader_basic() {
        let dir = tempdir().unwrap();
        
        // Write some entries
        let writer = WalWriter::new(
            dir.path().to_path_buf(),
            test_config(),
            "test-node".to_string(),
        ).await.unwrap();

        for i in 1..=10 {
            let entry = LogEntry::Insert {
                table: "test".to_string(),
                columns: vec!["id".to_string()],
                values: vec![Value::Int(i)],
                primary_key: PrimaryKey::Int(i),
            };
            writer.append(entry).await.unwrap();
        }
        writer.flush().await.unwrap();

        // Read entries
        let reader = WalReader::new(dir.path().to_path_buf(), 1, true).unwrap();
        let entries = reader.read_from(1).unwrap();
        assert_eq!(entries.len(), 10);
    }

    #[tokio::test]
    async fn test_reader_range() {
        let dir = tempdir().unwrap();
        
        let writer = WalWriter::new(
            dir.path().to_path_buf(),
            test_config(),
            "test-node".to_string(),
        ).await.unwrap();

        for i in 1..=20 {
            let entry = LogEntry::Insert {
                table: "test".to_string(),
                columns: vec!["id".to_string()],
                values: vec![Value::Int(i)],
                primary_key: PrimaryKey::Int(i),
            };
            writer.append(entry).await.unwrap();
        }
        writer.flush().await.unwrap();

        let reader = WalReader::new(dir.path().to_path_buf(), 1, true).unwrap();
        let entries = reader.read_range(5, 15).unwrap();
        assert_eq!(entries.len(), 11);
        assert_eq!(entries.first().unwrap().header.lsn, 5);
        assert_eq!(entries.last().unwrap().header.lsn, 15);
    }
}
