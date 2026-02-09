//! WolfNet — Secure private mesh networking over the internet
//!
//! Creates encrypted point-to-point tunnels between machines using TUN interfaces,
//! X25519 key exchange, and ChaCha20-Poly1305 authenticated encryption.

pub mod config;
pub mod crypto;
pub mod tun;
pub mod peer;
pub mod transport;
pub mod gateway;

pub use config::Config;
pub use crypto::KeyPair;
pub use peer::PeerManager;
