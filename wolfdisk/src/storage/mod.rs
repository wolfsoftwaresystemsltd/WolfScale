//! Storage module for chunks and file index

pub mod chunks;
pub mod index;
pub mod inode;

pub use chunks::ChunkStore;
pub use index::{FileIndex, FileEntry, ChunkRef};
pub use inode::InodeTable;
