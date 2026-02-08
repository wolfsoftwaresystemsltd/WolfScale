//! Storage module for chunks and file index

mod chunks;
mod index;
mod inode;

pub use chunks::ChunkStore;
pub use index::{FileIndex, FileEntry, ChunkRef};
pub use inode::InodeTable;
