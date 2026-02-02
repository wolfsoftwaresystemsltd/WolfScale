# WolfScale

<div align="center">

**High-Performance Distributed MariaDB Synchronization Manager**

[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Patreon](https://img.shields.io/badge/Patreon-Support%20Us-orange.svg)](https://www.patreon.com/15362110/join)

**© Wolf Software Systems Ltd** — [wolf.uk.com](https://wolf.uk.com)

</div>

---

WolfScale keeps multiple MariaDB databases in sync using a Write-Ahead Log (WAL) with automatic leader election and failover. Perfect for distributed applications that need strong consistency across database replicas.

## Features

- **Write-Ahead Log (WAL)** — Durable logging with optional LZ4 compression
- **Automatic Leader Election** — Raft-style elections with automatic failover
- **Write Forwarding** — Send writes to any node; they're routed to the leader
- **MySQL Proxy Mode** — Native MySQL protocol proxy for transparent routing
- **HTTP API** — RESTful API for writes and cluster management
- **Snowflake IDs** — Distributed unique ID generation

## Quick Start

### Build

```bash
cargo build --release
```

### Start a Cluster

**Node 1 (Leader):**
```bash
./run.sh start --bootstrap
```

**Node 2+ (Followers):**
```bash
./run.sh start
```

### Install as Service

```bash
sudo ./install_service.sh node    # Install as cluster node
sudo ./install_service.sh proxy   # Install as MySQL proxy
```

The installer will prompt for configuration if none exists.

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
database = "myapp"
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
