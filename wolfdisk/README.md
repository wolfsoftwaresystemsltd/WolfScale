# WolfDisk

🐺💾 **Distributed File System for Linux**

WolfDisk is a distributed file system that provides easy-to-use shared and replicated storage across Linux machines. Built on the same proven consensus mechanisms as [WolfScale](https://github.com/wolfsoftwaresystemsltd/WolfScale).

## Features

- **Two Operating Modes**:
  - **Shared Mode**: Simple shared storage with single leader
  - **Replicated Mode**: Data replicated across N nodes for high availability

- **Easy Setup**: Single command installation
- **Content-Addressed Storage**: Automatic deduplication via SHA256 hashing
- **FUSE-Based**: Mount as a regular directory
- **Chunk-Based**: Large files split for efficient transfer and sync

## Quick Install

```bash
curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/wolfdisk/setup.sh | bash
```

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
git clone https://github.com/wolfsoftwaresystemsltd/WolfDisk.git
cd WolfDisk
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

### Configuration

Edit `/etc/wolfdisk/config.toml`:

```toml
[node]
id = "node1"
bind = "0.0.0.0:9500"
data_dir = "/var/lib/wolfdisk"

[cluster]
peers = ["192.168.1.10:9500", "192.168.1.11:9500"]

[replication]
mode = "replicated"  # or "shared"
factor = 3
chunk_size = 4194304  # 4MB

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
│   └───────────┘  └───────────┘  └─────────────────────────┘ │
├─────────────────────────────────────────────────────────────┤
│               Network Layer (WolfScale-based)                │
└─────────────────────────────────────────────────────────────┘
```

## Commands

| Command | Description |
|---------|-------------|
| `wolfdisk init` | Initialize data directory |
| `wolfdisk mount` | Mount the filesystem |
| `wolfdisk unmount` | Unmount the filesystem |
| `wolfdisk status` | Show cluster status |

## License

MIT License - Wolf Software Systems Ltd
