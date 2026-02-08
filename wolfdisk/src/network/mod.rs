//! Network module for WolfDisk cluster communication

pub mod protocol;
pub mod discovery;
pub mod peer;

pub use protocol::{Message, encode_message, decode_message};
pub use discovery::Discovery;
pub use peer::{PeerConnection, PeerManager};
