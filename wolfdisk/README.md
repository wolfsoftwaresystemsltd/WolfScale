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

## Commands

| Command | Description |
|---------|-------------|
| `wolfdisk init` | Initialize data directory |
| `wolfdisk mount -m PATH` | Mount the filesystem |
| `wolfdisk unmount -m PATH` | Unmount the filesystem |
| `wolfdisk status` | Show node and cluster status |

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
