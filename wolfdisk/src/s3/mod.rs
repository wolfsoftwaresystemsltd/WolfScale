//! S3-compatible API gateway for WolfDisk
//!
//! Exposes WolfDisk's chunk-based storage as S3-compatible buckets and objects.
//! Files in WolfDisk are mapped to S3 objects where top-level directories become
//! buckets and nested paths become object keys.

pub mod server;
pub mod auth;

pub use server::S3Server;
