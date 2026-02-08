//! Inode table for FUSE

use std::collections::HashMap;
use std::path::PathBuf;

use super::FileIndex;

/// Inode table mapping inodes to paths and vice versa
#[derive(Debug)]
pub struct InodeTable {
    /// Inode to path mapping
    inode_to_path: HashMap<u64, PathBuf>,

    /// Path to inode mapping
    path_to_inode: HashMap<PathBuf, u64>,
}

/// Root inode number
const ROOT_INODE: u64 = 1;

impl InodeTable {
    /// Create a new empty inode table with root
    pub fn new() -> Self {
        let mut table = Self {
            inode_to_path: HashMap::new(),
            path_to_inode: HashMap::new(),
        };

        // Add root directory
        table.insert(ROOT_INODE, PathBuf::new());
        table
    }

    /// Build inode table from file index
    /// Returns the table and the maximum inode number used
    pub fn from_index(index: &FileIndex) -> (Self, u64) {
        let mut table = Self::new();
        let mut max_inode = ROOT_INODE;

        // Assign inodes to all index entries
        for path in index.paths() {
            max_inode += 1;
            table.insert(max_inode, path.clone());
        }

        (table, max_inode)
    }

    /// Insert a mapping
    pub fn insert(&mut self, inode: u64, path: PathBuf) {
        self.inode_to_path.insert(inode, path.clone());
        self.path_to_inode.insert(path, inode);
    }

    /// Get path by inode
    pub fn get_path(&self, inode: u64) -> Option<&PathBuf> {
        self.inode_to_path.get(&inode)
    }

    /// Get inode by path
    pub fn get_inode(&self, path: &PathBuf) -> Option<u64> {
        self.path_to_inode.get(path).copied()
    }

    /// Remove by path
    pub fn remove_path(&mut self, path: &PathBuf) -> Option<u64> {
        if let Some(inode) = self.path_to_inode.remove(path) {
            self.inode_to_path.remove(&inode);
            Some(inode)
        } else {
            None
        }
    }

    /// Remove by inode
    pub fn remove_inode(&mut self, inode: u64) -> Option<PathBuf> {
        if let Some(path) = self.inode_to_path.remove(&inode) {
            self.path_to_inode.remove(&path);
            Some(path)
        } else {
            None
        }
    }
}

impl Default for InodeTable {
    fn default() -> Self {
        Self::new()
    }
}
