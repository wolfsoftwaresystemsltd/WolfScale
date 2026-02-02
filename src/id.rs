//! Snowflake ID Generator
//!
//! Generates globally unique, time-ordered 64-bit IDs suitable for
//! distributed primary keys without coordination.
//!
//! ID Structure (64 bits):
//! - 1 bit: unused (sign bit)
//! - 41 bits: timestamp (milliseconds since epoch, ~69 years)
//! - 10 bits: node ID (0-1023)
//! - 12 bits: sequence (0-4095 per millisecond)

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Custom epoch: 2024-01-01 00:00:00 UTC
const WOLFSCALE_EPOCH: u64 = 1704067200000;

/// Bit allocation
#[allow(dead_code)]
const TIMESTAMP_BITS: u64 = 41;
const NODE_ID_BITS: u64 = 10;
const SEQUENCE_BITS: u64 = 12;

/// Masks
const MAX_NODE_ID: u64 = (1 << NODE_ID_BITS) - 1;
const MAX_SEQUENCE: u64 = (1 << SEQUENCE_BITS) - 1;

/// Shifts
const NODE_ID_SHIFT: u64 = SEQUENCE_BITS;
const TIMESTAMP_SHIFT: u64 = NODE_ID_BITS + SEQUENCE_BITS;

/// Snowflake ID wrapper type
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SnowflakeId(pub u64);

impl SnowflakeId {
    /// Create from raw value
    pub fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// Get the raw u64 value
    pub fn as_u64(&self) -> u64 {
        self.0
    }

    /// Extract timestamp from ID (milliseconds since WOLFSCALE_EPOCH)
    pub fn timestamp(&self) -> u64 {
        (self.0 >> TIMESTAMP_SHIFT) + WOLFSCALE_EPOCH
    }

    /// Extract node ID from ID
    pub fn node_id(&self) -> u16 {
        ((self.0 >> NODE_ID_SHIFT) & MAX_NODE_ID) as u16
    }

    /// Extract sequence from ID
    pub fn sequence(&self) -> u16 {
        (self.0 & MAX_SEQUENCE) as u16
    }
}

impl std::fmt::Display for SnowflakeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u64> for SnowflakeId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<SnowflakeId> for u64 {
    fn from(id: SnowflakeId) -> Self {
        id.0
    }
}

/// Snowflake ID Generator
///
/// Thread-safe generator that produces unique IDs for a specific node.
pub struct SnowflakeGenerator {
    node_id: u64,
    /// Packed state: upper 52 bits = last_timestamp, lower 12 bits = sequence
    state: AtomicU64,
}

impl SnowflakeGenerator {
    /// Create a new generator for the given node ID
    ///
    /// # Panics
    /// Panics if node_id > 1023
    pub fn new(node_id: u16) -> Self {
        assert!(
            (node_id as u64) <= MAX_NODE_ID,
            "Node ID must be 0-1023, got {}",
            node_id
        );

        Self {
            node_id: node_id as u64,
            state: AtomicU64::new(0),
        }
    }

    /// Generate a new unique ID
    ///
    /// This method is lock-free and thread-safe.
    pub fn generate(&self) -> SnowflakeId {
        loop {
            let current_time = Self::current_time_millis();
            let old_state = self.state.load(Ordering::Relaxed);
            let old_timestamp = old_state >> SEQUENCE_BITS;
            let old_sequence = old_state & MAX_SEQUENCE;

            let (new_timestamp, new_sequence) = if current_time > old_timestamp {
                // New millisecond, reset sequence
                (current_time, 0)
            } else if current_time == old_timestamp {
                // Same millisecond, increment sequence
                let next_seq = old_sequence + 1;
                if next_seq > MAX_SEQUENCE {
                    // Sequence overflow, wait for next millisecond
                    std::thread::yield_now();
                    continue;
                }
                (current_time, next_seq)
            } else {
                // Clock went backwards (rare), use old timestamp + next sequence
                let next_seq = old_sequence + 1;
                if next_seq > MAX_SEQUENCE {
                    // Wait for time to catch up
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                (old_timestamp, next_seq)
            };

            let new_state = (new_timestamp << SEQUENCE_BITS) | new_sequence;

            if self
                .state
                .compare_exchange(old_state, new_state, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                let id = (new_timestamp << TIMESTAMP_SHIFT)
                    | (self.node_id << NODE_ID_SHIFT)
                    | new_sequence;
                return SnowflakeId(id);
            }
            // CAS failed, retry
        }
    }

    /// Generate multiple IDs efficiently
    pub fn generate_batch(&self, count: usize) -> Vec<SnowflakeId> {
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            ids.push(self.generate());
        }
        ids
    }

    /// Get current time in milliseconds since WOLFSCALE_EPOCH
    fn current_time_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards before UNIX epoch")
            .as_millis() as u64
            - WOLFSCALE_EPOCH
    }

    /// Parse a node ID from a string (e.g., "node-5" -> 5)
    pub fn parse_node_id(node_id_str: &str) -> u16 {
        // Try to extract a number from the end of the string
        let digits: String = node_id_str
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect();

        if digits.is_empty() {
            // Hash the string to get a consistent node ID
            let hash = node_id_str
                .bytes()
                .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
            (hash % (MAX_NODE_ID + 1)) as u16
        } else {
            digits.parse::<u16>().unwrap_or(0) % (MAX_NODE_ID as u16 + 1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_generate_unique_ids() {
        let gen = SnowflakeGenerator::new(1);
        let mut ids = HashSet::new();

        for _ in 0..10000 {
            let id = gen.generate();
            assert!(ids.insert(id.0), "Duplicate ID generated: {}", id);
        }
    }

    #[test]
    fn test_ids_are_ordered() {
        let gen = SnowflakeGenerator::new(1);
        let mut last_id = 0u64;

        for _ in 0..1000 {
            let id = gen.generate();
            assert!(id.0 > last_id, "IDs should be monotonically increasing");
            last_id = id.0;
        }
    }

    #[test]
    fn test_concurrent_generation() {
        let gen = Arc::new(SnowflakeGenerator::new(1));
        let mut handles = vec![];

        for _ in 0..4 {
            let gen = Arc::clone(&gen);
            handles.push(thread::spawn(move || {
                let mut ids = Vec::new();
                for _ in 0..1000 {
                    ids.push(gen.generate().0);
                }
                ids
            }));
        }

        let mut all_ids = HashSet::new();
        for handle in handles {
            for id in handle.join().unwrap() {
                assert!(all_ids.insert(id), "Duplicate ID in concurrent test");
            }
        }

        assert_eq!(all_ids.len(), 4000);
    }

    #[test]
    fn test_id_decomposition() {
        let gen = SnowflakeGenerator::new(42);
        let id = gen.generate();

        assert_eq!(id.node_id(), 42);
        assert!(id.timestamp() > WOLFSCALE_EPOCH);
    }

    #[test]
    fn test_parse_node_id() {
        assert_eq!(SnowflakeGenerator::parse_node_id("node-5"), 5);
        assert_eq!(SnowflakeGenerator::parse_node_id("node-42"), 42);
        assert_eq!(SnowflakeGenerator::parse_node_id("server123"), 123);
        // Hashed values for non-numeric node IDs
        let id1 = SnowflakeGenerator::parse_node_id("alpha");
        let id2 = SnowflakeGenerator::parse_node_id("beta");
        assert_ne!(id1, id2);
    }
}
