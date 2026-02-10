//! Cryptographic primitives for WolfNet
//!
//! Uses X25519 for key exchange and ChaCha20-Poly1305 for authenticated encryption.

use std::path::Path;
use x25519_dalek::{PublicKey, StaticSecret};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, aead::{Aead, KeyInit}};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use sha2::{Sha256, Digest};
use tracing::{info, debug};

/// X25519 keypair for this node
pub struct KeyPair {
    pub secret: StaticSecret,
    pub public: PublicKey,
}

impl KeyPair {
    /// Generate a new random keypair
    pub fn generate() -> Self {
        let secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    /// Load a keypair from a private key file (32 bytes, base64 encoded)
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?.trim().to_string();
        let bytes = BASE64.decode(&content)?;
        if bytes.len() != 32 {
            return Err("Invalid private key length (expected 32 bytes)".into());
        }
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&bytes);
        let secret = StaticSecret::from(key_bytes);
        let public = PublicKey::from(&secret);
        Ok(Self { secret, public })
    }

    /// Save the private key to a file
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let encoded = BASE64.encode(self.secret.to_bytes());
        std::fs::write(path, &encoded)?;
        // Restrict permissions (owner-only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        info!("Private key saved to {:?}", path);
        Ok(())
    }

    /// Load or generate a keypair
    pub fn load_or_generate(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        if path.exists() {
            info!("Loading keypair from {:?}", path);
            Self::load(path)
        } else {
            info!("Generating new keypair, saving to {:?}", path);
            let kp = Self::generate();
            kp.save(path)?;
            Ok(kp)
        }
    }

    /// Get the public key as base64 string
    pub fn public_key_base64(&self) -> String {
        BASE64.encode(self.public.as_bytes())
    }

    /// Compute a 4-byte peer ID from a public key (for packet routing)
    pub fn peer_id(public_key: &PublicKey) -> [u8; 4] {
        let hash = Sha256::digest(public_key.as_bytes());
        let mut id = [0u8; 4];
        id.copy_from_slice(&hash[..4]);
        id
    }

    /// Get this node's peer ID
    pub fn my_peer_id(&self) -> [u8; 4] {
        Self::peer_id(&self.public)
    }
}

/// Session cipher for a peer connection
pub struct SessionCipher {
    cipher: ChaCha20Poly1305,
    send_counter: u64,
    recv_counter: u64,
    /// true if our public key is lexicographically less than peer's
    is_low_side: bool,
}

impl SessionCipher {
    /// Create a session cipher from the shared secret and key ordering
    pub fn new(shared_secret: &[u8; 32], my_public: &PublicKey, peer_public: &PublicKey) -> Self {
        let key = Key::from_slice(shared_secret);
        let cipher = ChaCha20Poly1305::new(key);
        let is_low_side = my_public.as_bytes() < peer_public.as_bytes();

        debug!("Session cipher created (low_side={})", is_low_side);
        Self {
            cipher,
            send_counter: 0,
            recv_counter: 0,
            is_low_side,
        }
    }

    /// Build a nonce from counter and direction
    fn make_nonce(&self, counter: u64, is_low_side: bool) -> Nonce {
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[..8].copy_from_slice(&counter.to_le_bytes());
        // Use byte 11 as direction flag to prevent nonce reuse
        nonce_bytes[11] = if is_low_side { 0x00 } else { 0x01 };
        *Nonce::from_slice(&nonce_bytes)
    }

    /// Encrypt a packet, returns (nonce_counter, ciphertext)
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<(u64, Vec<u8>), Box<dyn std::error::Error + Send + Sync>> {
        let counter = self.send_counter;
        self.send_counter += 1;
        let nonce = self.make_nonce(counter, self.is_low_side);
        let ciphertext = self.cipher.encrypt(&nonce, plaintext)
            .map_err(|_| "Encryption failed")?;
        Ok((counter, ciphertext))
    }

    /// Decrypt a packet
    pub fn decrypt(&mut self, counter: u64, ciphertext: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        // Replay protection with restart tolerance:
        // - Accept if counter > last seen (normal forward progress)
        // - Accept if counter < 100 and recv_counter is significantly higher (peer likely restarted)
        // - Reject if counter <= recv_counter (replay) unless it's a restart
        let is_likely_restart = counter < 100 && self.recv_counter > 100;
        if counter <= self.recv_counter && self.recv_counter > 0 && !is_likely_restart {
            // Allow a small reorder window of 32 packets
            if self.recv_counter - counter > 32 {
                return Err("Replay detected: stale nonce".into());
            }
        }

        let nonce = self.make_nonce(counter, !self.is_low_side);
        let plaintext = self.cipher.decrypt(&nonce, ciphertext)
            .map_err(|_| "Decryption failed (invalid key or corrupted data)")?;

        // Only update counter if this is forward progress or a restart
        if counter > self.recv_counter || is_likely_restart {
            self.recv_counter = counter;
        }
        Ok(plaintext)
    }
}

/// Parse a base64-encoded public key into PublicKey
pub fn parse_public_key(b64: &str) -> Result<PublicKey, Box<dyn std::error::Error>> {
    let bytes = BASE64.decode(b64.trim())?;
    if bytes.len() != 32 {
        return Err(format!("Invalid public key length: {} (expected 32)", bytes.len()).into());
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(PublicKey::from(arr))
}
