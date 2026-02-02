# WolfScale Documentation

## Overview

**WolfScale** is a high-performance, Rust-based distributed MariaDB synchronization manager. It keeps multiple MariaDB database servers in sync using a Write-Ahead Log (WAL), enabling consistent data across geographically distributed or high-availability database clusters.

## What Problem Does WolfScale Solve?

When running multiple MariaDB instances that need to stay synchronized, traditional replication methods can be complex to manage and may have consistency issues. WolfScale provides:

- **Strong Consistency**: All writes go through a single leader, ensuring all nodes see the same data in the same order
- **Automatic Recovery**: Nodes that disconnect can automatically catch up when they rejoin
- **Schema Propagation**: DDL changes (CREATE, ALTER, DROP) are replicated to all nodes
- **Distributed ID Generation**: Snowflake IDs allow any node to generate unique primary keys without coordination

---

## WolfScale vs Galera Cluster

### Comparison Overview

| Aspect | WolfScale | Galera Cluster |
|--------|-----------|----------------|
| **Replication Model** | Leader-based (single writer) | Multi-master (any node can write) |
| **Conflict Handling** | No conflicts (single leader) | Certification-based conflict detection |
| **Complexity** | Simpler architecture | More complex (wsrep, flow control) |
| **Write Latency** | Low (leader commits locally) | Higher (synchronous certification) |
| **Network Tolerance** | WAL catch-up for partitions | Stricter quorum requirements |
| **Implementation** | Standalone Rust binary | Patched MariaDB (wsrep) |

### WolfScale Advantages

| Advantage | Description |
|-----------|-------------|
| **No Write Conflicts** | Single leader model eliminates certification failures |
| **Simpler Recovery** | WAL-based catch-up is straightforward vs Galera's SST/IST |
| **Lower Write Latency** | Leader commits immediately, replicates asynchronously |
| **Lightweight** | Pure Rust binary, no patched database binaries required |
| **Explicit Control** | HTTP API gives fine-grained control over write operations |
| **Easier Debugging** | Single write path makes tracing issues simpler |

### Galera Cluster Advantages

| Advantage | Description |
|-----------|-------------|
| **Multi-Master** | Write to any node in the cluster |
| **Transparent** | No application changes required |
| **Mature** | Battle-tested in production environments |
| **Built-in** | Included in MariaDB Galera Cluster distribution |

### When to Choose WolfScale

- Your application naturally routes writes to a primary location
- You want simpler operations and debugging
- You need predictable latency without certification delays
- You prefer explicit control over database operations via API
- You want to avoid patched database binaries

### When to Choose Galera

- You need true multi-master writes from any node
- Your application cannot be modified to use an API
- You require transparent drop-in replication

---

## Deployment Architecture

### Recommended Setup: Co-located Deployment

**WolfScale should be installed on the same machine as each MariaDB server.** This is the ideal configuration for several reasons:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           3-Node Cluster Example                            │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│   Server A (Leader)        Server B (Follower)      Server C (Follower)    │
│   ┌─────────────────┐      ┌─────────────────┐      ┌─────────────────┐    │
│   │  WolfScale      │      │  WolfScale      │      │  WolfScale      │    │
│   │  (node-1)       │─────▶│  (node-2)       │      │  (node-3)       │    │
│   │       │         │      │       │         │      │       │         │    │
│   │       ▼         │      │       ▼         │      │       ▼         │    │
│   │  MariaDB        │      │  MariaDB        │      │  MariaDB        │    │
│   │  (localhost)    │      │  (localhost)    │      │  (localhost)    │    │
│   └─────────────────┘      └─────────────────┘      └─────────────────┘    │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Why Co-locate WolfScale with MariaDB?

| Benefit | Description |
|---------|-------------|
| **Minimal Latency** | Local socket/localhost connections to MariaDB are faster than network connections |
| **Reliability** | No additional network hops that could fail between WolfScale and the database |
| **Simpler Networking** | Only WolfScale cluster ports need to be exposed, not MariaDB ports |
| **Better Security** | MariaDB can bind to localhost only, reducing attack surface |
| **Easier Management** | Each server is self-contained with both components |

### Node Configuration

Each node should connect to its local MariaDB:

```toml
# On each server, database connects to localhost
[database]
host = "localhost"    # or "127.0.0.1"
port = 3306
```

### Ports to Open

| Port | Purpose | Expose To |
|------|---------|-----------|
| `7654` | WolfScale cluster communication | Other cluster nodes only |
| `8080` | WolfScale HTTP API | Application servers / internal |
| `3306` | MariaDB | Localhost only (no external) |

### Alternative: Dedicated WolfScale Server (Not Recommended)

Running WolfScale on a separate server from MariaDB is possible but adds:
- Additional network latency for every database operation
- Another point of failure
- More complex firewall rules (MariaDB must be network-accessible)

Only consider this if you have constraints that prevent installation on database servers.

### Hybrid Architecture: WolfScale + Galera Clusters

WolfScale can bridge two separate Galera clusters for cross-datacenter replication:

```
┌─────────────────────────────┐         ┌─────────────────────────────┐
│   Galera Cluster A          │         │   Galera Cluster B          │
│   (Datacenter 1)            │         │   (Datacenter 2)            │
│                             │         │                             │
│  ┌─────┐ ┌─────┐ ┌─────┐    │         │  ┌─────┐ ┌─────┐ ┌─────┐    │
│  │ DB1 │ │ DB2 │ │ DB3 │    │         │  │ DB4 │ │ DB5 │ │ DB6 │   │
│  └─────┘ └─────┘ └──┬──┘    │         │  └──┬──┘ └─────┘ └─────┘   │
│              ┌──────┴─────┐ │ WolfScale│ ┌──┴───────┐               │
│              │ WolfScale  │◄├─────────┼─►│ WolfScale│               │
│              │ (Leader)   │ │   WAN   │  │(Follower)│               │
│              └────────────┘ │         │  └──────────┘               │
└─────────────────────────────┘         └─────────────────────────────┘
```

**How it works:**
1. Install WolfScale Leader on one node in Cluster A
2. Install WolfScale Follower on one node in Cluster B
3. WolfScale replicates writes between clusters over WAN
4. Galera handles replication within each cluster internally

**Benefits:**

| Benefit | Description |
|---------|-------------|
| **Cross-DC Sync** | Bridge two datacenters or regions |
| **Best of Both** | Galera for local HA, WolfScale for geo-replication |
| **Simpler WAN Traffic** | Only WolfScale traffic crosses WAN, not Galera |
| **Conflict-Free** | Single write path through leader cluster |

**Considerations:**
- All writes must go to the cluster with the WolfScale leader
- The follower cluster is effectively read-only for replicated data
- Plan for which cluster should be primary during normal operations

---

## Architecture

┌─────────────────────────────────────────────────────────────────┐
│                      WolfScale Cluster                          │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│   ┌──────────────┐    ┌──────────────┐    ┌──────────────┐      │
│   │   Leader     │──▶│  Follower 1  │──▶│  Follower N  │      │
│   │              │    │              │    │              │      │
│   │  ┌────────┐  │    │  ┌────────┐  │    │  ┌────────┐  │      │
│   │  │  WAL   │  │    │  │  WAL   │  │    │  │  WAL   │  │      │
│   │  └────────┘  │    │  └────────┘  │    │  └────────┘  │      │
│   │      │       │    │      │       │    │      │       │      │
│   │      ▼       │    │      ▼       │    │      ▼       │      │
│   │  ┌────────┐  │    │  ┌────────┐  │    │  ┌────────┐  │      │
│   │  │MariaDB │  │    │  │MariaDB │  │    │  │MariaDB │  │          │
│   │  └────────┘  │    │  └────────┘  │    │  └────────┘  │      │  
│   └──────────────┘    └──────────────┘    └──────────────┘      │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### Components

| Component | Description |
|-----------|-------------|
| **WAL (Write-Ahead Log)** | Append-only log with LZ4 compression, batching, and fsync durability |
| **Leader Node** | Coordinates all writes and replicates to followers |
| **Follower Nodes** | Receive and apply replicated writes from the leader |
| **State Tracker** | SQLite-backed persistent tracking of applied log entries |
| **Network Layer** | TCP-based cluster communication with heartbeats |
| **HTTP API** | REST API for write operations and status queries |
| **CLI** | Command-line interface for cluster management |

---

## How It Works

### 1. Write Flow

1. Client sends a write request to the leader's HTTP API
2. Leader generates a Snowflake ID (if needed) and logs the operation to the WAL
3. Leader replicates the entry to all followers
4. Followers apply the entry to their local MariaDB and acknowledge
5. Leader confirms the write once quorum is reached

### 2. WAL (Write-Ahead Log)

- **Batching**: Groups multiple operations for efficiency (configurable batch size)
- **Compression**: LZ4 compression reduces storage and network overhead
- **Segmentation**: Log is split into segments (default 64MB) for easier management
- **Retention**: Old segments can be purged after configurable retention period
- **Durability**: Optional fsync ensures writes survive crashes

### 3. Node Recovery

When a node disconnects and later rejoins:

1. The node reports its last applied LSN (Log Sequence Number) to the leader
2. The leader sends all missing entries since that LSN
3. The node applies entries in order, tracking progress in its state database
4. Once caught up, the node transitions to ACTIVE status

### 4. Snowflake ID Generation

For distributed primary key generation without coordination:

- **41 bits**: Timestamp (milliseconds) - ~69 years of unique IDs
- **10 bits**: Node ID (0-1023) - identifies which node generated the ID
- **12 bits**: Sequence (0-4095) - allows 4096 IDs per millisecond per node

---

## Configuration

### Configuration File (`wolfscale.toml`)

```toml
[node]
id = "node-1"                      # Unique node identifier
bind_address = "0.0.0.0:7654"      # Cluster communication port
data_dir = "/var/lib/wolfscale/node-1"

[database]
host = "localhost"
port = 3306
user = "wolfscale"
password = "your-password"
database = "myapp"
pool_size = 10

[wal]
batch_size = 1000                  # Entries per batch
flush_interval_ms = 100            # Flush frequency
compression = true                 # LZ4 compression
segment_size_mb = 64               # Max segment size
retention_hours = 168              # 7 days
fsync = true                       # Sync to disk

[cluster]
peers = []                         # Peer addresses
heartbeat_interval_ms = 500        # Heartbeat frequency
election_timeout_ms = 2000         # Leader election timeout

[api]
enabled = true
bind_address = "0.0.0.0:8080"      # HTTP API port
```

---

## CLI Commands

| Command | Description |
|---------|-------------|
| `wolfscale init` | Create a new configuration file |
| `wolfscale start --bootstrap` | Start as the initial leader |
| `wolfscale start` | Start as a follower |
| `wolfscale join <leader:port>` | Join an existing cluster |
| `wolfscale status` | Check cluster status |
| `wolfscale info` | Show node configuration details |
| `wolfscale validate` | Validate configuration file |

---

## Adding Nodes to the Cluster

### Step 1: Prepare the New Node

On the new machine, initialize a configuration file:

```bash
wolfscale init --node-id node-2 --output wolfscale.toml
```

### Step 2: Configure the New Node

Edit `wolfscale.toml` on the new node:

```toml
[node]
id = "node-2"                           # Must be unique across cluster
bind_address = "0.0.0.0:7654"
data_dir = "/var/lib/wolfscale/node-2"

[database]
host = "localhost"                       # Local MariaDB on this node
port = 3306
user = "wolfscale"
password = "your-password"
database = "myapp"

[cluster]
peers = ["leader-host:7654"]            # Leader's address
```

### Step 3: Join the Cluster

```bash
wolfscale join leader-host:7654
```

This will:
1. Connect to the leader and register as a follower
2. Receive all WAL entries to catch up with current state
3. Start running as an active follower

### Alternative: Install as a Service

```bash
sudo ./scripts/install-service.sh --node-id node-2
sudo nano /etc/wolfscale/wolfscale.toml  # Add leader to peers
sudo systemctl start wolfscale
```

### Configuration Comparison

| Setting | Leader (node-1) | Follower (node-2+) |
|---------|-----------------|-------------------|
| `--bootstrap` flag | Yes (first start) | No |
| `cluster.peers` | `[]` (empty) | `["leader-ip:7654"]` |
| How to start | `wolfscale start --bootstrap` | `wolfscale join leader:7654` |

### Verify the Cluster

```bash
# From any node
curl http://localhost:8080/cluster
curl http://localhost:8080/cluster/nodes
```

### Adding a Node with an Empty Database

When you add a new node with no data in its local MariaDB:

| Scenario | What Happens |
|----------|--------------|
| **New cluster (full WAL)** | ✅ Node replays entire WAL history and syncs completely |
| **Established cluster (pruned WAL)** | ⚠️ Node cannot sync entries older than retention period |

**How it works:**

1. New node reports LSN = 0 (no entries applied)
2. Leader sends all available WAL entries from the beginning
3. Node replays all entries (INSERTs, UPDATEs, DDL) in order
4. Once caught up, node becomes an active follower

**The WAL Retention Issue:**

If `retention_hours = 168` (7 days), WAL entries older than 7 days are deleted. For established clusters:

```bash
# Option 1: New cluster with complete WAL - just join
wolfscale join leader:7654

# Option 2: Established cluster with pruned WAL - restore backup first
# Step 1: Get a database dump from an existing node
mysqldump -h existing-node -u wolfscale -p myapp > backup.sql

# Step 2: Restore to the new node's local MariaDB
mysql -u wolfscale -p myapp < backup.sql

# Step 3: Now join - WolfScale catches up from the backup point
wolfscale join leader:7654
```

> **Tip:** For production clusters, consider using longer `retention_hours` or keeping database backups readily available for new node provisioning.

---

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

# DDL (Schema Changes)
curl -X POST http://localhost:8080/write/ddl \
  -H "Content-Type: application/json" \
  -d '{"ddl": "ALTER TABLE users ADD COLUMN email VARCHAR(255)"}'
```

### Status Endpoints

```bash
curl http://localhost:8080/health    # Health check
curl http://localhost:8080/status    # Node status
curl http://localhost:8080/cluster   # Cluster info
```

---

## Requirements

- **Rust**: 1.70+
- **MariaDB**: 10.5+
- **Linux**: Recommended for best performance

---

## Directory Structure

```
/var/lib/wolfscale/<node-id>/
├── wal/           # Write-ahead log segments
├── state/         # SQLite state database
└── data/          # Other runtime data
```

---

## Building from Source

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Install to system
cargo install --path .

# Run tests
cargo test
```
