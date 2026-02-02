# WolfScale

<div align="center">

**High-Performance Distributed MariaDB Synchronization Manager**

[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Patreon](https://img.shields.io/badge/Patreon-Support%20Us-orange.svg)](https://www.patreon.com/15362110/join)

**© Wolf Software Systems Ltd** — [wolf.uk.com](https://wolf.uk.com)

</div>

---

> ⚠️ **Work in Progress** — This project is currently under active development and testing. APIs and features may change. Use in production at your own risk.

WolfScale keeps multiple MariaDB databases in sync using a Write-Ahead Log (WAL) with automatic leader election and failover. Perfect for distributed applications that need strong consistency across database replicas.

## Features

- **Write-Ahead Log (WAL)** — Durable logging with optional LZ4 compression
- **Automatic Leader Election** — Raft-style elections with automatic failover
- **Write Forwarding** — Send writes to any node; they're routed to the leader
- **MySQL Proxy Mode** — Native MySQL protocol proxy for transparent routing
- **HTTP API** — RESTful API for writes and cluster management
- **Snowflake IDs** — Distributed unique ID generation

## Installation

### Quick Install (Recommended)

Run this on any Ubuntu/Debian or Fedora/RHEL server:

```bash
curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/setup.sh | bash
```

This automatically:
- Detects your distro (apt or dnf)
- Installs all dependencies (git, build tools, OpenSSL)
- Installs Rust
- Clones, builds, and installs WolfScale
- Runs the interactive configuration wizard

<details>
<summary><strong>Manual Installation</strong></summary>

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Install dependencies (Ubuntu/Debian)
sudo apt update && sudo apt install -y git build-essential pkg-config libssl-dev

# Or for Fedora/RHEL:
# sudo dnf install -y git gcc gcc-c++ make openssl-devel pkg-config

# Clone and build
git clone https://github.com/wolfsoftwaresystemsltd/WolfScale.git
cd WolfScale
cargo build --release

# Install as service
sudo ./install_service.sh
```

</details>

### Connect

```bash
# Via MySQL proxy (recommended)
mariadb -h 127.0.0.1 -P 3307 -u your_user -p

# Check status
sudo systemctl status wolfscale
curl http://localhost:8080/health
```

## Usage

### CLI Commands

| Command | Description |
|---------|-------------|
| `wolfscale start --bootstrap` | Start as initial leader |
| `wolfscale start` | Start as follower |
| `wolfscale proxy --listen 0.0.0.0:3307` | Start MySQL proxy |
| `wolfscale status` | Check cluster status |
| `wolfscale info` | Show node configuration |

### MySQL Proxy Mode

Connect to WolfScale like a regular MySQL server:

```bash
mysql -h wolfscale-host -P 3307 -u user -p
```

Writes are automatically routed to the leader. SQL errors pass through unchanged.

### HTTP API

```bash
# Insert data
curl -X POST http://localhost:8080/write/insert \
  -H "Content-Type: application/json" \
  -d '{"table": "users", "values": {"name": "Alice", "email": "alice@example.com"}}'

# Check status
curl http://localhost:8080/status
```

## Configuration

Create `wolfscale.toml`:

```toml
[node]
id = "node-1"
bind_address = "0.0.0.0:7400"

[database]
host = "127.0.0.1"
port = 3306
user = "wolfscale"
password = "secret"

[cluster]
peers = ["192.168.1.11:7400", "192.168.1.12:7400"]
heartbeat_interval_ms = 500
election_timeout_ms = 2000

[api]
bind_address = "0.0.0.0:8080"
```

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                     Applications                        │
└─────────────────┬───────────────────────┬───────────────┘
                  │ HTTP API              │ MySQL Protocol
                  ▼                       ▼
┌─────────────────────────────────────────────────────────┐
│                    WolfScale Cluster                    │
│  ┌─────────┐      ┌─────────┐      ┌─────────┐         │
│  │ Leader  │◄────►│Follower │◄────►│Follower │         │
│  │ (Node1) │      │ (Node2) │      │ (Node3) │         │
│  └────┬────┘      └────┬────┘      └────┬────┘         │
└───────┼────────────────┼────────────────┼───────────────┘
        ▼                ▼                ▼
   ┌─────────┐      ┌─────────┐      ┌─────────┐
   │MariaDB 1│      │MariaDB 2│      │MariaDB 3│
   └─────────┘      └─────────┘      └─────────┘
```

## Documentation

See [docs/DOCUMENTATION.md](docs/DOCUMENTATION.md) for full documentation.

---

## License

MIT License - see [LICENSE](LICENSE) for details.

---

## Support

- **Website:** [wolf.uk.com](https://wolf.uk.com)
- **Issues:** [GitHub Issues](https://github.com/wolfsoftwaresystemsltd/WolfScale/issues)
- **Support Us:** [Patreon](https://www.patreon.com/15362110/join)
