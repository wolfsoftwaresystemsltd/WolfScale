# WolfScale

A high-performance, Rust-based distributed MariaDB synchronization manager that keeps multiple databases in sync using a Write-Ahead Log (WAL).

## Features

- **High-Performance WAL**: Append-only log with optional LZ4 compression, configurable batching, and fsync for durability
- **Leader-Based Replication**: One node coordinates writes, ensuring strong consistency across all nodes
- **Automatic Node Recovery**: Dropped nodes can rejoin and automatically catch up with missed entries
- **Schema Propagation**: ALTER TABLE, CREATE TABLE, and DROP TABLE statements are replicated across all nodes
- **Record Tracking**: Each node tracks which records have been applied, enabling precise synchronization
- **Snowflake IDs**: Distributed primary key generation that doesn't require coordination
- **HTTP API**: REST API for write operations, status queries, and cluster management
- **CLI Interface**: Easy-to-use command-line interface for cluster operations

## Quick Start

### 1. Install

```bash
cargo install --path .
```

### 2. Initialize Configuration

```bash
wolfscale init --node-id node-1 --output wolfscale.toml
```

### 3. Configure Database

Edit `wolfscale.toml` to set your MariaDB connection details:

```toml
[database]
host = "localhost"
port = 3306
user = "wolfscale"
password = "your-password"
database = "myapp"
```

### 4. Start the First Node (Bootstrap)

```bash
wolfscale start --bootstrap
```

### 5. Start Additional Nodes

On other machines, initialize and start:

```bash
# On node-2
wolfscale init --node-id node-2
wolfscale join node-1.example.com:7654
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                      WolfScale Cluster                          │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│   ┌──────────────┐    ┌──────────────┐    ┌──────────────┐     │
│   │   Leader     │    │  Follower 1  │    │  Follower N  │     │
│   │              │───▶│              │───▶│              │     │
│   │  ┌────────┐  │    │  ┌────────┐  │    │  ┌────────┐  │     │
│   │  │  WAL   │  │    │  │  WAL   │  │    │  │  WAL   │  │     │
│   │  └────────┘  │    │  └────────┘  │    │  └────────┘  │     │
│   │      │       │    │      │       │    │      │       │     │
│   │      ▼       │    │      ▼       │    │      ▼       │     │
│   │  ┌────────┐  │    │  ┌────────┐  │    │  ┌────────┐  │     │
│   │  │MariaDB │  │    │  │MariaDB │  │    │  │MariaDB │  │     │
│   │  └────────┘  │    │  └────────┘  │    │  └────────┘  │     │
│   └──────────────┘    └──────────────┘    └──────────────┘     │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

## Commands

### Start a Node

```bash
# Start as leader (bootstrap)
wolfscale start --bootstrap

# Start as follower
wolfscale start
```

### Join a Cluster

```bash
wolfscale join leader-address:7654
```

### Check Status

```bash
wolfscale status --address localhost:8080
```

### Validate Configuration

```bash
wolfscale validate --config wolfscale.toml
```

### Show Node Info

```bash
wolfscale info --config wolfscale.toml
```

## HTTP API

### Write Operations

```bash
# Insert
curl -X POST http://localhost:8080/write/insert \
  -H "Content-Type: application/json" \
  -d '{"table": "users", "values": {"id": 1, "name": "Alice"}}'

# Update
curl -X POST http://localhost:8080/write/update \
  -H "Content-Type: application/json" \
  -d '{"table": "users", "set": {"name": "Bob"}, "where_key": {"id": 1}}'

# Delete
curl -X POST http://localhost:8080/write/delete \
  -H "Content-Type: application/json" \
  -d '{"table": "users", "where_key": {"id": 1}}'

# DDL
curl -X POST http://localhost:8080/write/ddl \
  -H "Content-Type: application/json" \
  -d '{"ddl": "ALTER TABLE users ADD COLUMN email VARCHAR(255)"}'
```

### Status Endpoints

```bash
# Health check
curl http://localhost:8080/health

# Node status
curl http://localhost:8080/status

# Cluster info
curl http://localhost:8080/cluster

# All nodes
curl http://localhost:8080/cluster/nodes
```

## Configuration Reference

See [wolfscale.toml.example](wolfscale.toml.example) for a complete configuration reference.

### Key Settings

| Setting | Description | Default |
|---------|-------------|---------|
| `node.id` | Unique node identifier | Required |
| `node.bind_address` | Address for cluster communication | Required |
| `wal.batch_size` | Entries per batch | 1000 |
| `wal.compression` | Enable LZ4 compression | true |
| `wal.fsync` | Sync writes to disk | true |
| `cluster.heartbeat_interval_ms` | Heartbeat frequency | 500 |
| `cluster.election_timeout_ms` | Leader election timeout | 2000 |

## Node Recovery

When a node drops and rejoins:

1. The node reports its last applied LSN to the leader
2. The leader sends all missing entries since that LSN
3. The node applies entries in order, tracking progress
4. Once caught up, the node transitions to ACTIVE status

## Primary Key Strategy

WolfScale uses Snowflake IDs for distributed primary key generation:

- 41 bits: timestamp (milliseconds, ~69 years)
- 10 bits: node ID (0-1023)
- 12 bits: sequence (0-4095 per millisecond)

This allows each node to generate unique IDs without coordination.

## Requirements

- Rust 1.70+
- MariaDB 10.5+
- Linux (for best performance with io_uring)

## Building

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run tests
cargo test

# Run with integration tests (requires MariaDB)
cargo test --features integration
```

## License

MIT License - see LICENSE file for details.
