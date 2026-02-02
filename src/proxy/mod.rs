//! MySQL Protocol Proxy
//!
//! Implements a MySQL/MariaDB wire protocol proxy that allows applications
//! to connect to WolfScale as if it were a regular MySQL server.

mod server;
mod protocol;
mod handler;

pub use server::{ProxyServer, ProxyConfig};
pub use protocol::{MySqlPacket, PacketType};
pub use handler::QueryHandler;
