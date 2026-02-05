//! Auto-tuning module
//!
//! Detects hardware capabilities and calculates optimal configuration values.
//! Reserves resources for MariaDB while optimizing WolfScale performance.

use sysinfo::System;

/// Tuned configuration values based on hardware detection
#[derive(Debug, Clone)]
pub struct TunedConfig {
    /// Number of WolfScale worker threads
    pub worker_threads: usize,
    /// WAL batch size
    pub batch_size: usize,
    /// Channel buffer size for WAL writer
    pub channel_buffer: usize,
    /// Replication parallel workers
    pub replication_workers: usize,
    /// Detected CPU cores
    pub detected_cores: usize,
    /// Detected RAM in MB
    pub detected_ram_mb: u64,
}

impl Default for TunedConfig {
    fn default() -> Self {
        Self {
            worker_threads: 2,
            batch_size: 1000,
            channel_buffer: 10000,
            replication_workers: 2,
            detected_cores: 4,
            detected_ram_mb: 8192,
        }
    }
}

/// Detect the number of available CPU cores
pub fn detect_cpu_cores() -> usize {
    let sys = System::new_all();
    sys.cpus().len().max(1)
}

/// Detect total RAM in megabytes
pub fn detect_ram_mb() -> u64 {
    let sys = System::new_all();
    sys.total_memory() / 1024 / 1024
}

/// Auto-tune configuration based on detected hardware
///
/// Allocation strategy:
/// - WolfScale gets 25% of CPU cores (min 1, max 8)
/// - WolfScale gets 15% of RAM for buffers
/// - MariaDB gets the remaining resources
pub fn auto_tune() -> TunedConfig {
    let cores = detect_cpu_cores();
    let ram_mb = detect_ram_mb();
    
    // CPU: Use 25% of cores for WolfScale, min 1, max 8
    // This leaves 75% for MariaDB query threads
    let worker_threads = (cores / 4).clamp(1, 8);
    
    // Replication workers: Same as worker threads, but at least 2 for parallel replication
    let replication_workers = worker_threads.max(2);
    
    // RAM: Allocate 15% for WolfScale buffers
    // MariaDB should get ~70% via innodb_buffer_pool_size
    let wolfscale_ram_mb = ram_mb * 15 / 100;
    
    // Batch size: Scale with allocated RAM
    // ~1000 entries per GB of allocated WolfScale RAM
    // Min 100, max 10000
    let batch_size = ((wolfscale_ram_mb / 1024) * 1000)
        .max(100)
        .min(10000) as usize;
    
    // Channel buffer: 10x batch size for good throughput
    let channel_buffer = batch_size * 10;
    
    let config = TunedConfig {
        worker_threads,
        batch_size,
        channel_buffer,
        replication_workers,
        detected_cores: cores,
        detected_ram_mb: ram_mb,
    };
    
    tracing::info!(
        cores = cores,
        ram_mb = ram_mb,
        worker_threads = config.worker_threads,
        batch_size = config.batch_size,
        channel_buffer = config.channel_buffer,
        replication_workers = config.replication_workers,
        "Auto-tuned configuration based on hardware"
    );
    
    config
}

/// Get a human-readable summary of the tuned configuration
pub fn tuning_summary(config: &TunedConfig) -> String {
    format!(
        "Detected: {} cores, {} MB RAM\n\
         WolfScale: {} worker threads, {} replication workers\n\
         Buffers: batch_size={}, channel_buffer={}",
        config.detected_cores,
        config.detected_ram_mb,
        config.worker_threads,
        config.replication_workers,
        config.batch_size,
        config.channel_buffer,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_auto_tune_returns_sensible_values() {
        let config = auto_tune();
        
        // Worker threads should be reasonable
        assert!(config.worker_threads >= 1);
        assert!(config.worker_threads <= 8);
        
        // Batch size should be within bounds
        assert!(config.batch_size >= 100);
        assert!(config.batch_size <= 10000);
        
        // Channel buffer should be larger than batch size
        assert!(config.channel_buffer >= config.batch_size);
    }
    
    #[test]
    fn test_detection_returns_positive_values() {
        let cores = detect_cpu_cores();
        let ram = detect_ram_mb();
        
        assert!(cores >= 1);
        assert!(ram > 0);
    }
}
