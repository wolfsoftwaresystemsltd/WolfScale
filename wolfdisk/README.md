# WolfDisk

🐺💾 **Distributed File System for Linux**

WolfDisk is a distributed file system that provides easy-to-use shared and replicated storage across Linux machines. Built on the same proven consensus mechanisms as [WolfScale](https://github.com/wolfsoftwaresystemsltd/WolfScale).

## Features

- **Node Roles**: Leader, Follower, Client (mount-only), or Auto-election
- **Two Operating Modes**:
  - **Shared Mode**: Simple shared storage with single leader
  - **Replicated Mode**: Data replicated across N nodes for high availability
- **Auto-Discovery**: UDP multicast for automatic peer discovery on LAN
- **Client Mode**: Mount filesystem remotely without local storage
- **Easy Setup**: Interactive installer with configuration prompts
- **Content-Addressed Storage**: Automatic deduplication via SHA256 hashing
- **FUSE-Based**: Mount as a regular directory
- **Chunk-Based**: Large files split for efficient transfer and sync

## Quick Install

```bash
curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/wolfdisk/setup.sh | bash
```

The installer will prompt you for:
- **Node ID** — Unique identifier (defaults to hostname)
- **Role** — auto, leader, follower, or client
- **Bind IP address** — IP to listen on (auto-detected)
- **Discovery** — Auto-discovery, manual peers, or standalone
- **Mount path** — Where to mount the filesystem

## Node Roles

| Role | Storage | Replication | Use Case |
|------|---------|-------------|----------|
| **Leader** | ✅ Yes | Broadcasts to followers | Primary write node |
| **Follower** | ✅ Yes | Receives from leader | Read replicas, failover |
| **Client** | ❌ No | None (mount-only) | Access shared drive remotely |
| **Auto** | ✅ Yes | Dynamic election | Default - lowest ID becomes leader |

> 💡 **Client Mode**: Perfect for workstations that just need to access the shared filesystem without storing data locally.

## Manual Installation

### Prerequisites

- Linux with FUSE3 support
- Rust toolchain

> ⚠️ **Proxmox Users**: If running in an LXC container, you must enable FUSE in the container options: `Options → Features → FUSE`

```bash
# Ubuntu/Debian
sudo apt install libfuse3-dev fuse3

# Fedora/RHEL
sudo dnf install fuse3-devel fuse3
```

### Build

```bash
git clone https://github.com/wolfsoftwaresystemsltd/WolfScale.git
cd WolfScale/wolfdisk
cargo build --release
sudo cp target/release/wolfdisk /usr/local/bin/
```

## Usage

### Initialize Data Directory

```bash
wolfdisk init -d /var/lib/wolfdisk
```

### Mount Filesystem

```bash
# Foreground (for testing)
sudo wolfdisk mount -m /mnt/wolfdisk

# As a service
sudo systemctl start wolfdisk
```

### Check Status

```bash
wolfdisk status
```

## Configuration

Edit `/etc/wolfdisk/config.toml`:

```toml
[node]
id = "node1"
role = "auto"    # auto, leader, follower, or client
bind = "0.0.0.0:9500"
data_dir = "/var/lib/wolfdisk"

[cluster]
# Auto-discovery (recommended for LAN)
discovery = "udp://239.255.0.1:9501"

# Or manual peers
# peers = ["192.168.1.10:9500", "192.168.1.11:9500"]

[replication]
mode = "shared"      # or "replicated"
factor = 3           # Copies for replicated mode
chunk_size = 4194304 # 4MB

[mount]
path = "/mnt/wolfdisk"
allow_other = true
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Linux Applications                        │
├─────────────────────────────────────────────────────────────┤
│                   mount /mnt/wolfdisk                        │
├─────────────────────────────────────────────────────────────┤
│                     FUSE (fuser)                             │
├─────────────────────────────────────────────────────────────┤
│                   WolfDisk Core                              │
│   ┌───────────┐  ┌───────────┐  ┌─────────────────────────┐ │
│   │ File Index│  │  Chunks   │  │ Replication Engine      │ │
│   │ (metadata)│  │ (SHA256)  │  │ (leader election)       │ │
│   └───────────┘  └───────────┘  └─────────────────────────┘ │
├─────────────────────────────────────────────────────────────┤
│      Network Layer: Discovery + Peer + Protocol              │
└─────────────────────────────────────────────────────────────┘
```

## Leader Failover

WolfDisk automatically handles leader failures with fast failover:

1. **Heartbeat Detection** — Nodes monitor the leader with 2-second timeout
2. **Automatic Election** — Lowest node ID becomes the new leader
3. **Seamless Transition** — Followers continue serving reads during failover

### How Failover Works

```
Initial State:
  node-a (leader) ←→ node-b (follower) ←→ node-c (follower)

node-a goes down:
  ❌ node-a         node-b detects timeout (2s)
                    node-b becomes leader (next lowest ID)

node-a returns:
  node-a syncs from node-b (gets missed changes)
  node-a becomes leader again (lowest ID)
```

### Deterministic Election

- No voting or consensus delay
- Lowest node ID always wins
- Explicit role overrides: `role = "leader"` or `role = "follower"`

## Sync and Catchup

When a node starts or recovers from downtime, it automatically syncs with the leader:

1. **Version Tracking** — Every write increments the index version
2. **Delta Sync** — Follower sends "my version is X, give me changes since X"
3. **Incremental Updates** — Only modified/new/deleted files are transferred

### Example Sync Flow

```
Follower (version 45) → Leader: "SyncRequest(from_version=45)"
Leader (version 50)   → Follower: "SyncResponse(entries=[5 changes])"
Follower applies 5 changes, now at version 50
```

This ensures efficient catchup — a node that was down briefly only receives missed changes, not the entire index.

## Write Replication

When the leader writes a file:

1. **Local Write** — Leader stores chunks and updates index locally
2. **Broadcast** — Leader sends index update and chunks to all followers
3. **Apply** — Followers update their local index and store chunks

```
Application writes file.txt
        ↓
Leader: store chunks + update index (version++)
        ↓
Broadcast: IndexUpdate + StoreChunk to all followers
        ↓
Followers: apply updates, store chunks locally
```

No quorum required — lowest node ID is always leader.

## Read Caching

Followers cache chunks locally for fast reads:

1. **Cache Hit** — Chunk exists locally → serve immediately
2. **Cache Miss** — Request chunk from leader → cache locally → return

```
Follower read:
  chunk exists locally? → return data (fast)
  chunk missing? → fetch from leader → cache → return
```

## Client Mode (Thin Client)

Client mode mounts the filesystem without storing any data locally:

| Aspect | Leader/Follower | Client |
|--------|-----------------|--------|
| Local Storage | ✅ Stores data | ❌ No local storage |
| Reads | Local | Forwarded to leader |
| Writes | Local (leader) or forwarded | Forwarded to leader |
| Use Case | Data nodes | Workstations, containers |

### How Client Works

```
Application → /mnt/shared/file.txt
                    ↓
            WolfDisk Client
                    ↓ (network)
            Leader Node → reads/writes data
                    ↓
            Response → Application
```

Client mode is ideal for:
- Workstations accessing shared files
- Containers needing cluster access
- Read-heavy applications with low latency needs

## Commands

### wolfdisk (service)

| Command | Description |
|---------|-------------|
| `wolfdisk init` | Initialize data directory |
| `wolfdisk mount -m PATH` | Mount the filesystem |
| `wolfdisk unmount -m PATH` | Unmount the filesystem |
| `wolfdisk status` | Show node configuration |

### wolfdiskctl (control utility)

| Command | Description |
|---------|-------------|
| `wolfdiskctl status` | Show live status from running service |
| `wolfdiskctl list servers` | List all discovered servers in the cluster |
| `wolfdiskctl stats` | Live cluster statistics (refreshes every second) |

## Systemd Service

```bash
# Start
sudo systemctl start wolfdisk

# Status
sudo systemctl status wolfdisk

# Logs
sudo journalctl -u wolfdisk -f

# Enable at boot
sudo systemctl enable wolfdisk
```

## Multi-Node Example

### Server 1 (will become leader - lowest ID)
```toml
[node]
id = "node-a"
role = "auto"
```

### Server 2-N (will become followers)
```toml
[node]
id = "node-b"  # Higher ID = follower
role = "auto"
```

### Workstation (client only - no storage)
```toml
[node]
id = "desktop"
role = "client"
```

## License

MIT License - Wolf Software Systems Ltd
