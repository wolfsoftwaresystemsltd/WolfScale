//! WolfDisk FUSE Filesystem Implementation
//!
//! Implements the fuser::Filesystem trait to provide a mountable
//! distributed filesystem.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::sync::RwLock;
use std::time::{Duration, SystemTime};

use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry,
    ReplyOpen, ReplyWrite, Request,
};
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::error::Result;
use crate::storage::{ChunkStore, FileIndex, FileEntry, InodeTable};

/// TTL for attribute caching
const TTL: Duration = Duration::from_secs(1);

/// Root inode number
const ROOT_INODE: u64 = 1;

/// WolfDisk FUSE Filesystem
pub struct WolfDiskFS {
    /// Configuration
    config: Config,

    /// Chunk storage backend
    chunk_store: ChunkStore,

    /// File metadata index
    file_index: RwLock<FileIndex>,

    /// Inode to path mapping
    inode_table: RwLock<InodeTable>,

    /// Next available inode number
    next_inode: RwLock<u64>,

    /// Open file handles (fh -> inode)
    open_files: RwLock<HashMap<u64, u64>>,

    /// Next file handle
    next_fh: RwLock<u64>,
}

impl WolfDiskFS {
    /// Create a new WolfDisk filesystem
    pub fn new(config: Config) -> Result<Self> {
        info!("Initializing WolfDisk filesystem");

        // Ensure data directories exist
        std::fs::create_dir_all(config.chunks_dir())?;
        std::fs::create_dir_all(config.index_dir())?;

        // Initialize chunk store
        let chunk_store = ChunkStore::new(config.chunks_dir(), config.replication.chunk_size)?;

        // Load or create file index
        let file_index = FileIndex::load_or_create(&config.index_dir())?;

        // Build inode table from index
        let (inode_table, max_inode) = InodeTable::from_index(&file_index);

        Ok(Self {
            config,
            chunk_store,
            file_index: RwLock::new(file_index),
            inode_table: RwLock::new(inode_table),
            next_inode: RwLock::new(max_inode + 1),
            open_files: RwLock::new(HashMap::new()),
            next_fh: RwLock::new(1),
        })
    }

    /// Allocate a new inode
    fn allocate_inode(&self) -> u64 {
        let mut next = self.next_inode.write().unwrap();
        let inode = *next;
        *next += 1;
        inode
    }

    /// Allocate a new file handle
    fn allocate_fh(&self) -> u64 {
        let mut next = self.next_fh.write().unwrap();
        let fh = *next;
        *next += 1;
        fh
    }

    /// Get root directory attributes
    fn root_attr(&self) -> FileAttr {
        FileAttr {
            ino: ROOT_INODE,
            size: 0,
            blocks: 0,
            atime: SystemTime::now(),
            mtime: SystemTime::now(),
            ctime: SystemTime::now(),
            crtime: SystemTime::now(),
            kind: FileType::Directory,
            perm: 0o755,
            nlink: 2,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    /// Convert FileEntry to FileAttr
    fn entry_to_attr(&self, entry: &FileEntry, inode: u64) -> FileAttr {
        FileAttr {
            ino: inode,
            size: entry.size,
            blocks: (entry.size + 511) / 512,
            atime: entry.accessed,
            mtime: entry.modified,
            ctime: entry.modified,
            crtime: entry.created,
            kind: if entry.is_dir { FileType::Directory } else { FileType::RegularFile },
            perm: entry.permissions as u16,
            nlink: if entry.is_dir { 2 } else { 1 },
            uid: entry.uid,
            gid: entry.gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }
}

impl Filesystem for WolfDiskFS {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let name_str = name.to_string_lossy();
        debug!("lookup: parent={}, name={}", parent, name_str);

        let inode_table = self.inode_table.read().unwrap();
        let file_index = self.file_index.read().unwrap();

        // Get parent path
        let parent_path = match inode_table.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        // Build child path
        let child_path = if parent_path.as_os_str().is_empty() || parent_path == std::path::Path::new("/") {
            std::path::PathBuf::from(name)
        } else {
            parent_path.join(name)
        };

        // Look up in index
        if let Some(entry) = file_index.get(&child_path) {
            if let Some(inode) = inode_table.get_inode(&child_path) {
                let attr = self.entry_to_attr(entry, inode);
                reply.entry(&TTL, &attr, 0);
                return;
            }
        }

        reply.error(libc::ENOENT);
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        debug!("getattr: ino={}", ino);

        if ino == ROOT_INODE {
            reply.attr(&TTL, &self.root_attr());
            return;
        }

        let inode_table = self.inode_table.read().unwrap();
        let file_index = self.file_index.read().unwrap();

        if let Some(path) = inode_table.get_path(ino) {
            if let Some(entry) = file_index.get(path) {
                let attr = self.entry_to_attr(entry, ino);
                reply.attr(&TTL, &attr);
                return;
            }
        }

        reply.error(libc::ENOENT);
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        debug!("read: ino={}, offset={}, size={}", ino, offset, size);

        let inode_table = self.inode_table.read().unwrap();
        let file_index = self.file_index.read().unwrap();

        let path = match inode_table.get_path(ino) {
            Some(p) => p.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let entry = match file_index.get(&path) {
            Some(e) => e.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        drop(file_index);
        drop(inode_table);

        // Read data from chunks
        match self.chunk_store.read(&entry.chunks, offset as u64, size as usize) {
            Ok(data) => reply.data(&data),
            Err(e) => {
                warn!("Read error: {}", e);
                reply.error(e.to_errno());
            }
        }
    }

    fn write(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        debug!("write: ino={}, offset={}, size={}", ino, offset, data.len());

        let inode_table = self.inode_table.write().unwrap();
        let mut file_index = self.file_index.write().unwrap();

        let path = match inode_table.get_path(ino) {
            Some(p) => p.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let entry = match file_index.get_mut(&path) {
            Some(e) => e,
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        // Write data to chunks
        match self.chunk_store.write(&mut entry.chunks, offset as u64, data) {
            Ok(written) => {
                // Update file size if needed
                let new_end = offset as u64 + written as u64;
                if new_end > entry.size {
                    entry.size = new_end;
                }
                entry.modified = SystemTime::now();
                reply.written(written as u32);
            }
            Err(e) => {
                warn!("Write error: {}", e);
                reply.error(e.to_errno());
            }
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        debug!("readdir: ino={}, offset={}", ino, offset);

        let inode_table = self.inode_table.read().unwrap();
        let file_index = self.file_index.read().unwrap();

        // Get directory path
        let dir_path = if ino == ROOT_INODE {
            std::path::PathBuf::new()
        } else {
            match inode_table.get_path(ino) {
                Some(p) => p.clone(),
                None => {
                    reply.error(libc::ENOENT);
                    return;
                }
            }
        };

        let mut entries = vec![
            (ino, FileType::Directory, ".".to_string()),
            (ino, FileType::Directory, "..".to_string()),
        ];

        // Find children
        for (path, entry) in file_index.iter() {
            if let Some(parent) = path.parent() {
                let parent_matches = if ino == ROOT_INODE {
                    parent.as_os_str().is_empty()
                } else {
                    parent == dir_path
                };

                if parent_matches {
                    if let Some(name) = path.file_name() {
                        let child_inode = inode_table.get_inode(path).unwrap_or(0);
                        let file_type = if entry.is_dir {
                            FileType::Directory
                        } else {
                            FileType::RegularFile
                        };
                        entries.push((child_inode, file_type, name.to_string_lossy().to_string()));
                    }
                }
            }
        }

        // Return entries starting from offset
        for (i, (inode, file_type, name)) in entries.iter().enumerate().skip(offset as usize) {
            if reply.add(*inode, (i + 1) as i64, *file_type, name) {
                break;
            }
        }

        reply.ok();
    }

    fn mkdir(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let name_str = name.to_string_lossy();
        debug!("mkdir: parent={}, name={}, mode={:o}", parent, name_str, mode);

        let mut inode_table = self.inode_table.write().unwrap();
        let mut file_index = self.file_index.write().unwrap();

        // Get parent path
        let parent_path = if parent == ROOT_INODE {
            std::path::PathBuf::new()
        } else {
            match inode_table.get_path(parent) {
                Some(p) => p.clone(),
                None => {
                    reply.error(libc::ENOENT);
                    return;
                }
            }
        };

        // Build new path
        let dir_path = parent_path.join(name);

        // Check if already exists
        if file_index.contains(&dir_path) {
            reply.error(libc::EEXIST);
            return;
        }

        // Create entry
        let now = SystemTime::now();
        let entry = FileEntry {
            size: 0,
            is_dir: true,
            permissions: mode,
            uid: req.uid(),
            gid: req.gid(),
            created: now,
            modified: now,
            accessed: now,
            chunks: Vec::new(),
        };

        // Allocate inode and add to tables
        let inode = self.allocate_inode();
        inode_table.insert(inode, dir_path.clone());
        file_index.insert(dir_path, entry.clone());

        let attr = self.entry_to_attr(&entry, inode);
        reply.entry(&TTL, &attr, 0);
    }

    fn create(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        _flags: i32,
        reply: fuser::ReplyCreate,
    ) {
        let name_str = name.to_string_lossy();
        debug!("create: parent={}, name={}, mode={:o}", parent, name_str, mode);

        let mut inode_table = self.inode_table.write().unwrap();
        let mut file_index = self.file_index.write().unwrap();

        // Get parent path
        let parent_path = if parent == ROOT_INODE {
            std::path::PathBuf::new()
        } else {
            match inode_table.get_path(parent) {
                Some(p) => p.clone(),
                None => {
                    reply.error(libc::ENOENT);
                    return;
                }
            }
        };

        // Build new path
        let file_path = parent_path.join(name);

        // Check if already exists
        if file_index.contains(&file_path) {
            reply.error(libc::EEXIST);
            return;
        }

        // Create entry
        let now = SystemTime::now();
        let entry = FileEntry {
            size: 0,
            is_dir: false,
            permissions: mode,
            uid: req.uid(),
            gid: req.gid(),
            created: now,
            modified: now,
            accessed: now,
            chunks: Vec::new(),
        };

        // Allocate inode and add to tables
        let inode = self.allocate_inode();
        inode_table.insert(inode, file_path.clone());
        file_index.insert(file_path, entry.clone());

        // Allocate file handle
        let fh = self.allocate_fh();
        self.open_files.write().unwrap().insert(fh, inode);

        let attr = self.entry_to_attr(&entry, inode);
        reply.created(&TTL, &attr, 0, fh, 0);
    }

    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        let name_str = name.to_string_lossy();
        debug!("unlink: parent={}, name={}", parent, name_str);

        let mut inode_table = self.inode_table.write().unwrap();
        let mut file_index = self.file_index.write().unwrap();

        // Get parent path
        let parent_path = if parent == ROOT_INODE {
            std::path::PathBuf::new()
        } else {
            match inode_table.get_path(parent) {
                Some(p) => p.clone(),
                None => {
                    reply.error(libc::ENOENT);
                    return;
                }
            }
        };

        let file_path = parent_path.join(name);

        // Check exists and is not a directory
        match file_index.get(&file_path) {
            Some(entry) if entry.is_dir => {
                reply.error(libc::EISDIR);
                return;
            }
            None => {
                reply.error(libc::ENOENT);
                return;
            }
            _ => {}
        }

        // Remove from index and inode table
        if let Some(entry) = file_index.remove(&file_path) {
            // Delete chunks
            for chunk in &entry.chunks {
                let _ = self.chunk_store.delete(&chunk.hash);
            }
        }
        inode_table.remove_path(&file_path);

        reply.ok();
    }

    fn rmdir(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        let name_str = name.to_string_lossy();
        debug!("rmdir: parent={}, name={}", parent, name_str);

        let mut inode_table = self.inode_table.write().unwrap();
        let mut file_index = self.file_index.write().unwrap();

        // Get parent path
        let parent_path = if parent == ROOT_INODE {
            std::path::PathBuf::new()
        } else {
            match inode_table.get_path(parent) {
                Some(p) => p.clone(),
                None => {
                    reply.error(libc::ENOENT);
                    return;
                }
            }
        };

        let dir_path = parent_path.join(name);

        // Check exists and is a directory
        match file_index.get(&dir_path) {
            Some(entry) if !entry.is_dir => {
                reply.error(libc::ENOTDIR);
                return;
            }
            None => {
                reply.error(libc::ENOENT);
                return;
            }
            _ => {}
        }

        // Check directory is empty
        for path in file_index.paths() {
            if let Some(parent) = path.parent() {
                if parent == dir_path {
                    reply.error(libc::ENOTEMPTY);
                    return;
                }
            }
        }

        // Remove from index and inode table
        file_index.remove(&dir_path);
        inode_table.remove_path(&dir_path);

        reply.ok();
    }

    fn open(&mut self, _req: &Request, ino: u64, _flags: i32, reply: ReplyOpen) {
        debug!("open: ino={}", ino);

        // Verify file exists
        let inode_table = self.inode_table.read().unwrap();
        if inode_table.get_path(ino).is_none() && ino != ROOT_INODE {
            reply.error(libc::ENOENT);
            return;
        }

        let fh = self.allocate_fh();
        self.open_files.write().unwrap().insert(fh, ino);
        reply.opened(fh, 0);
    }

    fn release(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: fuser::ReplyEmpty,
    ) {
        debug!("release: fh={}", fh);
        self.open_files.write().unwrap().remove(&fh);
        
        // Save index to persist changes
        if let Ok(index) = self.file_index.read() {
            let _ = index.save(&self.config.index_dir());
        }
        
        reply.ok();
    }
}
