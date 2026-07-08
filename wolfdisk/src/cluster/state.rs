//! Cluster state management and leader election for WolfDisk

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

use crate::config::{Config, NodeRole};
use crate::network::discovery::{DiscoveredPeer, Discovery};

/// Cluster state for this node
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterState {
    /// Just started, finding peers
    Discovering,
    /// Following a leader
    Following,
    /// This node is the leader
    Leading,
    /// Client-only mode (no replication)
    Client,
    /// Standalone node (no peers configured)
    Standalone,
}

/// Information about a known peer
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub node_id: String,
    pub address: String,
    pub is_leader: bool,
    pub is_client: bool,
    pub last_seen: Instant,
}

/// On-disk filename for the persisted replication version + changelog,
/// stored alongside the file index.
const REPL_STATE_FILENAME: &str = "replication_state.json";

/// Serializable snapshot of the replication state that must survive a restart:
/// the monotonic index version and the recent changelog used for delta sync.
#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedReplicationState {
    version: u64,
    changelog: Vec<(u64, std::path::PathBuf, bool)>,
}

/// Cluster manager - handles leader election and state
pub struct ClusterManager {
    config: Config,
    node_id: String,
    state: Arc<RwLock<ClusterState>>,
    leader_id: Arc<RwLock<Option<String>>>,
    peers: Arc<RwLock<HashMap<String, PeerInfo>>>,
    discovery: Option<Discovery>,
    term: Arc<RwLock<u64>>,
    running: Arc<RwLock<bool>>,
    /// Last time we received a heartbeat from the leader
    last_leader_heartbeat: Arc<RwLock<Instant>>,
    /// Current index version (for sync)
    index_version: Arc<RwLock<u64>>,
    /// Whether initial sync from leader has completed
    /// Election monitor waits for this before allowing leader promotion
    initial_sync_complete: Arc<RwLock<bool>>,
    /// Changelog: (version, path, is_deleted) triples tracking which files changed at each version
    /// Used for delta sync - leader only sends entries that changed since follower's version
    /// The is_deleted flag tracks deletions so followers can remove entries they no longer need
    changelog: Arc<RwLock<std::collections::VecDeque<(u64, std::path::PathBuf, bool)>>>,
}

impl ClusterManager {
    /// Create a new cluster manager
    pub fn new(config: Config) -> Self {
        let node_id = config.node.id.clone();
        let initial_state = match config.node.role {
            NodeRole::Client => ClusterState::Client,
            _ => ClusterState::Discovering,
        };

        Self {
            config,
            node_id,
            state: Arc::new(RwLock::new(initial_state)),
            leader_id: Arc::new(RwLock::new(None)),
            peers: Arc::new(RwLock::new(HashMap::new())),
            discovery: None,
            term: Arc::new(RwLock::new(0)),
            running: Arc::new(RwLock::new(false)),
            last_leader_heartbeat: Arc::new(RwLock::new(Instant::now())),
            index_version: Arc::new(RwLock::new(0)),
            initial_sync_complete: Arc::new(RwLock::new(false)),
            changelog: Arc::new(RwLock::new(std::collections::VecDeque::new())),
        }
    }

    /// Get current cluster state
    pub fn state(&self) -> ClusterState {
        *self.state.read().unwrap()
    }

    /// Check if this node is the leader
    pub fn is_leader(&self) -> bool {
        self.state() == ClusterState::Leading
    }

    /// Get the current leader's node ID
    pub fn leader_id(&self) -> Option<String> {
        self.leader_id.read().unwrap().clone()
    }

    /// Get the current leader's address (for forwarding operations)
    pub fn leader_address(&self) -> Option<String> {
        let leader = self.leader_id.read().unwrap().clone()?;
        let peers = self.peers.read().unwrap();
        peers.get(&leader).map(|p| p.address.clone())
    }

    /// Get this node's ID
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Get list of known peers
    pub fn peers(&self) -> Vec<PeerInfo> {
        self.peers.read().unwrap().values().cloned().collect()
    }

    /// Get config reference
    pub fn config(&self) -> &crate::Config {
        &self.config
    }

    /// Add a peer address as the known leader (used when TCP probe finds leader directly)
    pub fn add_peer_as_leader(&self, address: &str) {
        let peer_id = format!("leader@{}", address);
        *self.leader_id.write().unwrap() = Some(peer_id.clone());
        self.peers.write().unwrap().insert(
            peer_id.clone(),
            PeerInfo {
                node_id: peer_id,
                address: address.to_string(),
                is_leader: true,
                is_client: false,
                last_seen: std::time::Instant::now(),
            },
        );
    }

    /// Get current index version
    pub fn index_version(&self) -> u64 {
        *self.index_version.read().unwrap()
    }

    /// Increment and return the new index version (called on writes)
    /// Records the changed path in the changelog for delta sync
    pub fn increment_index_version(&self, path: std::path::PathBuf) -> u64 {
        self.record_change(path, false)
    }

    /// Record a deletion in the changelog so followers can remove the entry
    pub fn record_deletion(&self, path: std::path::PathBuf) -> u64 {
        self.record_change(path, true)
    }

    /// Internal: record a change (create/update/delete) in the changelog
    fn record_change(&self, path: std::path::PathBuf, is_deleted: bool) -> u64 {
        let mut v = self.index_version.write().unwrap();
        *v += 1;
        let version = *v;

        let mut log = self.changelog.write().unwrap();
        log.push_back((version, path, is_deleted));

        // Cap the changelog at 10,000 entries (a follower more than 10k changes
        // behind gets a full sync instead). MUST be a VecDeque: pop_front is
        // O(1), whereas the old Vec `drain(0..n)` was O(n) PER call once at
        // capacity — and since v2.11.4 EVERY write AND EVERY setattr records a
        // change, a bulk op (cp -a / rsync / chmod -R over hundreds of thousands
        // of files) turned that into billions of element shifts while holding
        // the changelog write lock, freezing the leader and hanging the
        // followers' synchronous forwards (wabil 2026-06-29).
        while log.len() > 10_000 {
            log.pop_front();
        }

        version
    }

    /// Get the paths that changed since a given version
    /// Returns None if the changelog doesn't go back far enough (need full sync)
    /// Returns (changed_paths, deleted_paths) — changed_paths are creates/updates,
    /// deleted_paths are files that were removed
    pub fn changes_since(
        &self,
        from_version: u64,
    ) -> Option<(Vec<std::path::PathBuf>, Vec<std::path::PathBuf>)> {
        let current = self.index_version();

        // Already at (or somehow beyond) our version → genuinely nothing to send.
        if from_version >= current {
            return Some((Vec::new(), Vec::new()));
        }

        let log = self.changelog.read().unwrap();

        // We're behind (from_version < current), so the changelog MUST contain
        // every change after from_version to answer with a delta. If it's empty,
        // or its oldest entry is newer than from_version+1, there's a gap we
        // can't fill → force a full sync. The empty case is the critical one:
        // after a restart the in-memory changelog is empty while `current` was
        // restored from disk, and the old code returned Some((empty, empty))
        // here — telling the follower "nothing changed" when in fact it must
        // re-reconcile against the leader's full index (wabil 2026-06-29).
        match log.front() {
            None => return None,
            Some((oldest_version, _, _)) => {
                if from_version.saturating_add(1) < *oldest_version {
                    // Changelog truncated, need full sync
                    return None;
                }
            }
        }

        // Collect unique paths changed since from_version
        // A path's latest operation wins (if deleted then re-created, it's a change not a delete)
        let mut latest_ops: std::collections::HashMap<std::path::PathBuf, bool> =
            std::collections::HashMap::new();
        for (v, path, is_deleted) in log.iter() {
            if *v > from_version {
                latest_ops.insert(path.clone(), *is_deleted);
            }
        }

        let mut changed = Vec::new();
        let mut deleted = Vec::new();
        for (path, is_deleted) in latest_ops {
            if is_deleted {
                deleted.push(path);
            } else {
                changed.push(path);
            }
        }

        Some((changed, deleted))
    }

    /// Set the index version (used during sync)
    pub fn set_index_version(&self, version: u64) {
        *self.index_version.write().unwrap() = version;
    }

    /// Persist the replication index version + changelog next to the file index,
    /// written atomically (temp + rename). Without this the version is in-memory
    /// only, so a restart reset the leader to v0 — every node then falsely agreed
    /// "v0, in sync" while their file sets differed by a quarter-million entries,
    /// and deletions never propagated (wabil 2026-06-29).
    pub fn save_replication_state(&self, index_dir: &std::path::Path) -> std::io::Result<()> {
        // Snapshot version + changelog CONSISTENTLY: hold the version lock across
        // the changelog read, mirroring record_change's lock order (version then
        // changelog). Otherwise a concurrent record_change could bump the version
        // and append its entry between the two reads, persisting a version one
        // behind its own changelog.
        let (version, changelog) = {
            let v = self.index_version.read().unwrap();
            let log: Vec<(u64, std::path::PathBuf, bool)> =
                self.changelog.read().unwrap().iter().cloned().collect();
            (*v, log)
        };
        let state = PersistedReplicationState { version, changelog };
        let bytes = serde_json::to_vec(&state).map_err(std::io::Error::other)?;
        std::fs::create_dir_all(index_dir)?;
        let path = index_dir.join(REPL_STATE_FILENAME);
        let tmp = index_dir.join(format!("{}.tmp", REPL_STATE_FILENAME));
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&bytes)?;
            // Same durability bar as the file index: fsync before rename
            // so power loss can't materialise the rename with empty
            // content (a corrupt file here silently resets us to v0).
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &path)?;
        // ...and fsync the directory so the rename itself is durable.
        if let Ok(dir) = std::fs::File::open(index_dir) {
            let _ = dir.sync_all();
        }
        Ok(())
    }

    /// Quarantine the persisted replication state (rename to
    /// `*.corrupt-<unixtime>`). Called when the file index was corrupt and
    /// had to be rebuilt empty: restoring the old version would make this
    /// node claim vN while holding no entries, so peers would consider it
    /// in sync and never resend anything. At v0 the leader's authoritative
    /// full sync repopulates it (deletes stay double-gated on the leader
    /// side by full_sync_may_delete, so an empty rejoiner can't nuke peers).
    pub fn discard_replication_state(index_dir: &std::path::Path) {
        let path = index_dir.join(REPL_STATE_FILENAME);
        if !path.exists() {
            return;
        }
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let quarantine = index_dir.join(format!("{}.corrupt-{}", REPL_STATE_FILENAME, ts));
        match std::fs::rename(&path, &quarantine) {
            Ok(()) => warn!(
                "Discarded replication state {:?} → {:?} (file index was corrupt; forcing full resync from v0)",
                path, quarantine
            ),
            Err(e) => warn!(
                "Failed to discard replication state {:?}: {} — refusing to load it; \
                 the periodic persist will overwrite it at v0 shortly",
                path, e
            ),
        }
    }

    /// Restore the replication version + changelog on startup. A missing or
    /// corrupt file simply leaves us at v0 (the prior behaviour) — which now
    /// self-heals via the authoritative full sync regardless.
    pub fn load_replication_state(&self, index_dir: &std::path::Path) {
        let path = index_dir.join(REPL_STATE_FILENAME);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => return,
        };
        let state: PersistedReplicationState = match serde_json::from_slice(&bytes) {
            Ok(s) => s,
            Err(e) => {
                warn!("Ignoring corrupt replication state ({}); starting at v0", e);
                return;
            }
        };
        *self.index_version.write().unwrap() = state.version;
        let mut log = self.changelog.write().unwrap();
        log.clear();
        log.extend(state.changelog);
        info!(
            "Restored replication state: version {}, {} changelog entries",
            state.version,
            log.len()
        );
    }

    /// Mark initial sync as complete (allows election monitor to promote to leader)
    pub fn set_sync_complete(&self) {
        *self.initial_sync_complete.write().unwrap() = true;
        info!("Initial sync complete - node eligible for leader election");
    }

    /// Check if initial sync has completed
    pub fn is_sync_complete(&self) -> bool {
        *self.initial_sync_complete.read().unwrap()
    }

    /// Record that we received a heartbeat from the leader
    pub fn receive_leader_heartbeat(&self) {
        *self.last_leader_heartbeat.write().unwrap() = Instant::now();
    }

    /// Check if leader heartbeat has timed out (2 seconds for fast failover)
    pub fn is_leader_timeout(&self) -> bool {
        self.last_leader_heartbeat.read().unwrap().elapsed() > Duration::from_secs(2)
    }

    /// Get current term
    pub fn term(&self) -> u64 {
        *self.term.read().unwrap()
    }

    /// Start the cluster manager
    pub fn start(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        *self.running.write().unwrap() = true;

        // Client mode: run discovery to find leader, but never become leader
        let is_client = self.config.node.role == NodeRole::Client;
        if is_client {
            info!("Starting in client mode (read/write but will not become leader)");
            *self.state.write().unwrap() = ClusterState::Client;
        }

        // Start discovery if enabled (including for clients!)
        if self.config.cluster.discovery.is_some() || !self.config.cluster.peers.is_empty() {
            // Port comes from this node's own config (`discovery = "udp://ip:PORT"`),
            // NOT a compiled constant — an upgraded node keeps its existing port
            // (Golden Rule: never break existing installs).
            let discovery_port = crate::network::discovery::discovery_port_from_config(
                &self.config.cluster.discovery,
            );
            let discovery = Discovery::new(
                self.node_id.clone(),
                self.config.node.bind.clone(),
                self.config.node.role,
                discovery_port,
            )
            .with_peers(self.config.cluster.peers.clone());
            discovery.start()?;

            // Start a sync thread to copy discovered peers to our peer map
            // AND sync our leader status to Discovery for broadcasts
            let cluster_peers = Arc::clone(&self.peers);
            let cluster_state = Arc::clone(&self.state);
            let cluster_leader_id = Arc::clone(&self.leader_id);
            let running = Arc::clone(&self.running);
            let discovery_clone = discovery.clone();
            let is_client_mode = is_client;

            thread::spawn(move || {
                while *running.read().unwrap() {
                    // Sync our leader status TO discovery (for broadcasts)
                    // Clients never advertise as leader
                    if !is_client_mode {
                        let is_leading = *cluster_state.read().unwrap() == ClusterState::Leading;
                        discovery_clone.set_leader(is_leading);
                    }

                    // Get peers FROM discovery
                    let discovered = discovery_clone.peers();

                    // Update our peer map and track leader
                    let mut peers = cluster_peers.write().unwrap();
                    for dp in discovered {
                        // If client mode, also track who the leader is
                        if is_client_mode && dp.is_leader {
                            *cluster_leader_id.write().unwrap() = Some(dp.node_id.clone());
                        }

                        let is_client_peer =
                            matches!(dp.role, crate::network::discovery::DiscoveryRole::Client);
                        peers.insert(
                            dp.node_id.clone(),
                            PeerInfo {
                                node_id: dp.node_id,
                                address: dp.address,
                                is_leader: dp.is_leader,
                                is_client: is_client_peer,
                                last_seen: dp.last_seen,
                            },
                        );
                    }
                    drop(peers);

                    thread::sleep(Duration::from_millis(500));
                }
            });

            self.discovery = Some(discovery);

            info!("Discovery started for node {}", self.node_id);
        }

        // Start election monitor in background (but NOT for clients)
        if !is_client {
            self.start_election_monitor();
        }

        Ok(())
    }

    /// Start the election monitor thread
    ///
    /// Election model:
    /// 1. All nodes start as followers (Discovering state)
    /// 2. Wait for discovery to find other peers
    /// 3. After discovery delay, determine leader by lowest node ID
    /// 4. If I have lowest ID -> become leader
    /// 5. Otherwise -> stay follower and wait for leader heartbeat
    fn start_election_monitor(&self) {
        let node_id = self.node_id.clone();
        let config_role = self.config.node.role;
        let state = Arc::clone(&self.state);
        let leader_id = Arc::clone(&self.leader_id);
        let peers = Arc::clone(&self.peers);
        let running = Arc::clone(&self.running);
        let term = Arc::clone(&self.term);
        let last_leader_heartbeat = Arc::clone(&self.last_leader_heartbeat);

        // Give discovery enough time to find all peers before electing
        let discovery_delay = Duration::from_secs(5);
        let peer_stale_threshold = Duration::from_secs(4);
        let initial_sync_complete = Arc::clone(&self.initial_sync_complete);

        thread::spawn(move || {
            info!(
                "Election monitor started for node {} - waiting for discovery",
                node_id
            );

            // All nodes start as followers and wait for discovery
            thread::sleep(discovery_delay);

            while *running.read().unwrap() {
                let current_state = *state.read().unwrap();

                // Clone active peers so we don't hold the lock
                let active_peers: Vec<_> = {
                    let snapshot = peers.read().unwrap();
                    snapshot
                        .values()
                        .filter(|p| p.last_seen.elapsed() < peer_stale_threshold)
                        .cloned()
                        .collect()
                };

                // Get server peer IDs for comparison (exclude clients!)
                // Clients should never be considered for leader election
                let peer_ids: Vec<&str> = active_peers
                    .iter()
                    .filter(|p| !p.is_client)
                    .map(|p| p.node_id.as_str())
                    .collect();

                // Am I the lowest ID? (I should be leader if so)
                let i_am_lowest =
                    peer_ids.is_empty() || peer_ids.iter().all(|id| node_id.as_str() < *id);

                // Is there a visible leader already?
                let current_leader = active_peers
                    .iter()
                    .find(|p| p.is_leader)
                    .map(|p| p.node_id.clone());

                match current_state {
                    ClusterState::Discovering | ClusterState::Following => {
                        if let Some(leader) = &current_leader {
                            // There's a leader - follow them
                            *last_leader_heartbeat.write().unwrap() = std::time::Instant::now();
                            *leader_id.write().unwrap() = Some(leader.clone());

                            if current_state == ClusterState::Discovering {
                                info!("Found leader: {}", leader);
                                *state.write().unwrap() = ClusterState::Following;
                            }

                            // But if I have a lower ID, I should become leader
                            if i_am_lowest && node_id.as_str() < leader.as_str() {
                                info!(
                                    "I have lower ID than current leader {} - taking over",
                                    leader
                                );
                                *term.write().unwrap() += 1;
                                *leader_id.write().unwrap() = Some(node_id.clone());
                                *state.write().unwrap() = ClusterState::Leading;
                            }
                        } else if i_am_lowest && config_role != NodeRole::Follower {
                            // I have the lowest ID, but wait for initial sync before
                            // taking leadership. This ensures a restarting node first
                            // gets the latest data from the current leader.
                            if *initial_sync_complete.read().unwrap() || active_peers.is_empty() {
                                info!(
                                    "Becoming leader (term {}) - I have the lowest node ID",
                                    *term.read().unwrap()
                                );
                                *term.write().unwrap() += 1;
                                *leader_id.write().unwrap() = Some(node_id.clone());
                                *state.write().unwrap() = ClusterState::Leading;
                            } else {
                                debug!(
                                    "Deferring leadership: waiting for initial sync to complete"
                                );
                            }
                        } else {
                            // Not lowest ID, stay as follower and wait
                            debug!(
                                "Not becoming leader: i_am_lowest={}, config_role={:?}",
                                i_am_lowest, config_role
                            );
                            if current_state == ClusterState::Discovering {
                                *state.write().unwrap() = ClusterState::Following;
                            }
                        }
                    }
                    ClusterState::Leading => {
                        // Check if any peer has a lower ID - I should step down
                        if !i_am_lowest {
                            warn!("Stepping down - discovered peer with lower node ID");
                            *state.write().unwrap() = ClusterState::Following;
                            *last_leader_heartbeat.write().unwrap() = std::time::Instant::now();
                        }
                    }
                    ClusterState::Client | ClusterState::Standalone => {
                        // No election participation
                    }
                }

                thread::sleep(Duration::from_secs(1));
            }
        });
    }

    /// Update peer information (called from discovery)
    pub fn update_peer(&self, peer: DiscoveredPeer) {
        let is_leader = peer.is_leader;
        let info = PeerInfo {
            node_id: peer.node_id.clone(),
            address: peer.address,
            is_leader,
            is_client: matches!(peer.role, crate::network::discovery::DiscoveryRole::Client),
            last_seen: peer.last_seen,
        };

        self.peers.write().unwrap().insert(peer.node_id, info);

        // Update discovery with our leader status
        if let Some(ref discovery) = self.discovery {
            discovery.set_leader(self.is_leader());
        }
    }

    /// Write cluster status to file for wolfdiskctl
    pub fn write_status_file(&self) {
        self.write_status_file_with_stats(0, 0);
    }

    /// Write cluster status with file index stats for wolfdiskctl
    pub fn write_status_file_with_stats(&self, file_count: usize, total_size: u64) {
        use std::time::{SystemTime, UNIX_EPOCH};

        let status_dir = std::path::Path::new(&self.config.node.data_dir);
        if !status_dir.exists() {
            return; // Data dir doesn't exist yet
        }

        let status_path = status_dir.join("cluster_status.json");

        let state = self.state();
        let peers = self.peers();

        let role = if self.is_leader() {
            "leader"
        } else if state == ClusterState::Client {
            "client"
        } else {
            "follower"
        };
        let state_str = match state {
            ClusterState::Discovering => "discovering",
            ClusterState::Following => "following",
            ClusterState::Leading => "leading",
            ClusterState::Client => "client",
            ClusterState::Standalone => "standalone",
        };

        let peer_statuses: Vec<serde_json::Value> = peers
            .iter()
            .map(|p| {
                let peer_role = if p.is_leader {
                    "leader"
                } else if p.is_client {
                    "client"
                } else {
                    "follower"
                };
                serde_json::json!({
                    "node_id": p.node_id,
                    "address": p.address,
                    "role": peer_role,
                    "is_leader": p.is_leader,
                    "is_client": p.is_client,
                    "last_seen_secs_ago": p.last_seen.elapsed().as_secs()
                })
            })
            .collect();

        let status = serde_json::json!({
            "node_id": self.node_id,
            // Software version — the health UI renders it and flags
            // mixed-version clusters, which cannot sync (positional bincode
            // wire; wabil 2026-07-04).
            "version": env!("CARGO_PKG_VERSION"),
            "role": role,
            "state": state_str,
            "bind_address": self.config.node.bind,
            "leader_id": self.leader_id(),
            "index_version": self.index_version(),
            "file_count": file_count,
            "total_size": total_size,
            "peers": peer_statuses,
            "updated_at": SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
        });

        if let Ok(json) = serde_json::to_string_pretty(&status) {
            let _ = std::fs::write(&status_path, json);
        }
    }

    /// Stop the cluster manager
    pub fn stop(&self) {
        *self.running.write().unwrap() = false;
        if let Some(ref discovery) = self.discovery {
            discovery.stop();
        }
    }
}

#[cfg(test)]
mod replication_state_tests {
    use super::*;
    use std::path::PathBuf;

    fn mgr() -> ClusterManager {
        ClusterManager::new(Config::default())
    }

    #[test]
    fn changes_since_at_version_is_empty_not_none() {
        let m = mgr();
        // Genuinely caught up (from == current == 0) → "no changes", NOT a full sync.
        assert_eq!(m.changes_since(0), Some((Vec::new(), Vec::new())));
        m.increment_index_version(PathBuf::from("/a"));
        assert_eq!(m.changes_since(1), Some((Vec::new(), Vec::new())));
    }

    #[test]
    fn changes_since_reports_changes_and_deletions() {
        let m = mgr();
        m.increment_index_version(PathBuf::from("/a")); // v1 change
        m.record_deletion(PathBuf::from("/b")); // v2 delete
        let (changed, deleted) = m.changes_since(0).expect("covered by changelog");
        assert_eq!(changed, vec![PathBuf::from("/a")]);
        assert_eq!(deleted, vec![PathBuf::from("/b")]);
    }

    #[test]
    fn behind_with_empty_changelog_forces_full_sync() {
        // The restart trap: version was restored from disk (so current > 0) but
        // the in-memory changelog is empty. The old code returned Some((empty,
        // empty)) here — telling the follower "nothing changed" when it must in
        // fact reconcile against the full index. It MUST be None (full sync).
        let m = mgr();
        m.set_index_version(262_700);
        assert_eq!(m.changes_since(0), None);
        assert_eq!(m.changes_since(100), None);
    }

    #[test]
    fn replication_state_survives_save_load() {
        let dir = std::env::temp_dir().join(format!("wolfdisk_rs_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let m1 = mgr();
        m1.increment_index_version(PathBuf::from("/x")); // v1
        m1.increment_index_version(PathBuf::from("/y")); // v2
        m1.record_deletion(PathBuf::from("/x")); // v3
        m1.save_replication_state(&dir).unwrap();

        let m2 = mgr();
        assert_eq!(m2.index_version(), 0);
        m2.load_replication_state(&dir);
        assert_eq!(m2.index_version(), 3);
        // Changelog restored → delta sync still works across the restart.
        let (changed, deleted) = m2.changes_since(1).expect("changelog restored");
        assert!(changed.contains(&PathBuf::from("/y")));
        assert!(deleted.contains(&PathBuf::from("/x")));

        std::fs::remove_dir_all(&dir).ok();
    }
}
