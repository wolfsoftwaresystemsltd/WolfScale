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

| Aspect              | WolfScale                        | Galera Cluster                        |
|---------------------|----------------------------------|---------------------------------------|
| Replication Model   | Leader-based (single writer)     | Multi-master (any node can write)     |
| Conflict Handling   | No conflicts (single leader)     | Certification-based conflict detection|
| Complexity          | Simpler architecture             | More complex (wsrep, flow control)    |
| Write Latency       | Low (leader commits locally)     | Higher (synchronous certification)    |
| Network Tolerance   | WAL catch-up for partitions      | Stricter network requirements         |
| Implementation      | Standalone Rust binary           | Patched MariaDB (wsrep)               |

### WolfScale Advantages

| Advantage              | Description                                              |
|------------------------|----------------------------------------------------------|
| No Write Conflicts     | Single leader model eliminates certification failures    |
| Simpler Recovery       | WAL-based catch-up is straightforward vs Galera SST/IST  |
| Lower Write Latency    | Leader commits immediately, replicates asynchronously    |
| Lightweight            | Pure Rust binary, no patched database binaries required  |
| Explicit Control       | HTTP API gives fine-grained control over write operations|
| Easier Debugging       | Single write path makes tracing issues simpler           |

### Galera Cluster Advantages

| Advantage              | Description                                              |
|------------------------|----------------------------------------------------------|
| Multi-Master           | Write to any node in the cluster                         |
| Transparent            | No application changes required                          |
| Mature                 | Battle-tested in production environments                 |
| Built-in               | Included in MariaDB Galera Cluster distribution          |

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

**3-Node Cluster Example:**

| Server   | Role     | WolfScale | MariaDB   | Notes                          |
|----------|----------|-----------|-----------|--------------------------------|
| Server A | Leader   | node-1    | localhost | Handles all writes             |
| Server B | Follower | node-2    | localhost | Receives replication from leader|
| Server C | Follower | node-3    | localhost | Receives replication from leader|

Each server runs both WolfScale and MariaDB locally. WolfScale connects to MariaDB via localhost.

### Why Co-locate WolfScale with MariaDB?

| Benefit             | Description                                                   |
|---------------------|---------------------------------------------------------------|
| Minimal Latency     | Local socket/localhost connections are faster than network    |
| Reliability         | No additional network hops that could fail                    |
| Simpler Networking  | Only WolfScale cluster ports need to be exposed               |
| Better Security     | MariaDB can bind to localhost only, reducing attack surface   |
| Easier Management   | Each server is self-contained with both components            |

### Node Configuration

Each node should connect to its local MariaDB:

[database]
host = "localhost"
port = 3306

### Ports to Open

| Port   | Purpose                          | Expose To                      |
|--------|----------------------------------|--------------------------------|
| 7654   | WolfScale cluster communication  | Other cluster nodes only       |
| 8080   | WolfScale HTTP API               | Application servers / internal |
| 3306   | MariaDB                          | Localhost only (no external)   |

### Alternative: Dedicated WolfScale Server (Not Recommended)

Running WolfScale on a separate server from MariaDB is possible but adds:
- Additional network latency for every database operation
- Another point of failure
- More complex firewall rules (MariaDB must be network-accessible)

Only consider this if you have constraints that prevent installation on database servers.

### Hybrid Architecture: WolfScale + Galera Clusters

WolfScale can bridge two separate Galera clusters for cross-datacenter replication:

| Datacenter 1               | Datacenter 2               |
|----------------------------|----------------------------|
| Galera Cluster A (3 nodes) | Galera Cluster B (3 nodes) |
| WolfScale Leader on DB3    | WolfScale Follower on DB4  |

WolfScale replicates between clusters over WAN. Galera handles replication within each cluster.

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

### Binlog Replication Mode (v3.0+)

For existing databases where you can't route writes through WolfScale's proxy, use **binlog mode** to capture writes directly from the binary log:

```toml
[replication]
mode = "binlog"  # Capture from binlog instead of proxy

[binlog]
server_id = 1001  # Unique ID (must not conflict with existing replica IDs)
```

**Supported Databases:**

| Database | Works? | Notes |
|----------|--------|-------|
| MariaDB standalone | ✓ | Just enable `log_bin` |
| MariaDB Galera | ✓ | Connect to any node |
| MySQL standalone | ✓ | Standard replication |
| Percona Server | ✓ | MySQL-compatible |
| Amazon RDS MySQL | ✓ | Enable binlog retention |

**How it works:**
1. WolfScale connects to the database as a replica (like a MySQL slave)
2. Reads the binary log stream in real-time
3. Converts binlog events to WAL entries
4. Replicates to WolfScale followers as normal

**Requirements:**
- Binary logging must be enabled (`log_bin = mysql-bin`)
- Recommended: `binlog_format = MIXED` or `STATEMENT`
- The `server_id` must be unique across all replicas

**Easy Setup with wolfctl:**

```bash
wolfctl binlog-setup
```

This command connects to your database, detects the current binlog position, and outputs the config snippet to add to your config.toml.

**Step-by-Step Setup:**

0. **Enable Binary Logging** (if not already enabled):
   
   Add to `/etc/mysql/mariadb.conf.d/50-server.cnf` under the `[mariadb]` section:
   ```ini
   [mariadb]
   log_bin = mysql-bin
   binlog_format = MIXED
   server_id = 1
   ```
   Then restart MariaDB:
   ```bash
   systemctl restart mariadb
   ```
   
   > **Note:** For Galera clusters, you only need to enable binlog on ONE node - the one WolfScale will connect to.

1. **Initial Data Sync** (for new clusters without existing data):
   ```bash
   # On source database - dump with binlog position
   mariadb-dump --single-transaction --master-data=2 --all-databases > dump.sql
   
   # Import on WolfScale target cluster
   mariadb < dump.sql
   ```

2. **Configure Binlog Mode:**
   ```bash
   # Detect binlog position and generate config
   wolfctl binlog-setup
   
   # Add the output to /etc/wolfscale/config.toml
   ```

3. **Start WolfScale:**
   ```bash
   systemctl restart wolfscale
   
   # Monitor replication
   wolfctl stats
   ```

**Important:** Only run WolfScale binlog mode on ONE node per source cluster - all cluster nodes have identical data.

---

## Cluster Communication

### How Nodes Communicate

WolfScale uses a heartbeat-based protocol for cluster communication. All nodes participate in health monitoring:

**Leader Heartbeats:**
1. The leader broadcasts heartbeats to all followers every 500ms (configurable)
2. Heartbeats include cluster membership so followers learn about all other nodes
3. Followers respond with acknowledgments confirming they are alive and their replication status
4. The leader tracks follower status based on received responses

**Peer-to-Peer Heartbeats:**
1. All nodes (including followers) broadcast heartbeats to all known peers
2. This enables every node to detect which other nodes are alive
3. When the leader goes down, followers already know the status of other followers
4. Proper health information is essential for correct leader election

### Heartbeat Timing

| Parameter | Default | Purpose |
|-----------|---------|---------|
| `heartbeat_interval_ms` | 500 | How often heartbeats are sent |
| Heartbeat timeout | 3x interval | Node marked unhealthy after 3 missed heartbeats |
| `election_timeout_ms` | 1500-3000 | How long to wait before starting election |

### Node Status Transitions

| Status | Meaning |
|--------|---------|
| **Active** | Node is healthy, receiving and responding to heartbeats |
| **Lagging** | Node missed recent heartbeats (timeout exceeded) |
| **Dropped** | Node has been unresponsive for extended period (removed after 30s) |
| **Joining** | Node is connecting to the cluster |
| **Syncing** | Follower is catching up on missed log entries |
| **Offline** | Node is explicitly marked as unavailable |

### Cluster Membership Sync

When a node joins the cluster:
- It connects to peers listed in `cluster.peers` configuration
- Upon receiving heartbeats from the leader, it learns about all cluster members
- It starts sending peer heartbeats to all known nodes
- All nodes eventually have a consistent view of cluster membership

### Auto-Discovery (v5.4.0+)

WolfScale nodes can automatically find each other via UDP broadcast, eliminating the need to manually configure peer addresses:

| Feature | Description |
|---------|-------------|
| **UDP Broadcast** | Nodes broadcast their presence on port 7654 every 2 seconds |
| **Automatic Join** | New nodes discover and join the cluster automatically |
| **Cluster Isolation** | Optional `cluster_name` prevents cross-cluster joins |
| **Auto-Removal** | Dead nodes are removed from membership after 30 seconds |

**Configuration:**

```toml
[cluster]
auto_discovery = true           # Enable UDP broadcast discovery (default: true)
cluster_name = "production"     # Optional: isolate clusters on same network
peers = []                      # Optional: seed nodes for faster initial discovery
```

**How it works:**
1. Node starts and broadcasts its ID and address via UDP
2. Other nodes receive the broadcast and add the new node to their membership
3. Standard TCP heartbeats take over once nodes are aware of each other
4. If a node goes down, it's marked DROPPED after heartbeat timeout
5. After 30 seconds of no heartbeat, the node is automatically removed from membership

**When to use `cluster_name`:**
- Multiple WolfScale clusters on the same network
- Separate dev/staging/production environments
- Preventing accidental cross-cluster joins

**Load Balancer Auto-Discovery:**
The load balancer can also discover cluster nodes automatically:
```bash
wolfscale load-balancer --listen 0.0.0.0:3306
# Optionally filter by cluster name:
wolfscale load-balancer --listen 0.0.0.0:3306 --cluster-name production
```

---

## Leader Election

### Deterministic Leader Election

WolfScale uses **deterministic leader election** based on node ID. No voting is required - the node with the lexicographically lowest ID among active, synced nodes becomes leader automatically.

**How it works:**
1. Followers detect missed heartbeats from the leader (timeout: approximately 3x heartbeat interval)
2. Each node checks if it has the lowest ID among active nodes
3. The node with the lowest ID becomes leader immediately
4. Other nodes recognize the new leader via heartbeats and become followers

**Benefits of deterministic election:**

| Benefit | Description |
|---------|-------------|
| **No split-brain** | Only one node can have the lowest ID |
| **Predictable failover** | You always know which node will become leader |
| **Instant transition** | No voting rounds or delays |
| **Simple implementation** | No complex consensus protocol needed |

**Example:**
- If `wolftest1` (leader) goes down, and `wolftest2` and `wolftest3` remain
- `wolftest2` will become leader because `"wolftest2" < "wolftest3"`

### Node Rejoin Behavior

When a previously failed node rejoins the cluster:

1. The node starts as a **Follower** regardless of its ID
2. It receives heartbeats from the current leader and learns the cluster state
3. It syncs any missing WAL entries from the leader (status: **Syncing**)
4. Once fully caught up, it transitions to **Active** status
5. Only then can it participate in leader election

**Important:** A node must be fully synced before it can become leader. This prevents a returning node with stale data from immediately stealing leadership.

**Example scenario:**
- `wolftest1` (lowest ID) was leader, goes down
- `wolftest2` becomes the new leader
- `wolftest1` comes back online - it joins as a follower
- `wolftest1` syncs from `wolftest2` until caught up
- Once `wolftest1` is **Active** and synced, it can reclaim leadership

### Database Health Monitoring

The leader continuously monitors local database health:

| Check | Action |
|-------|--------|
| **Database unavailable** | Leader steps down immediately |
| **Connection failure** | Triggers leader step-down |
| **Upgrade/maintenance** | Automatically fails over to next node |

This ensures that if you stop MariaDB for an upgrade, WolfScale automatically promotes another node to leader, preventing write failures.

### Disaster Recovery and WAL Catch-Up

When a node goes down while writes continue on the new leader, the returning node uses **WAL catch-up** to synchronize:

**The Scenario:**
1. `wolftest1` (leader) goes down - writes stop on its database
2. `wolftest2` becomes leader - writes continue on wolftest2's database
3. `wolftest1` returns - its database is now "behind"

**How WolfScale Handles This:**

| Step | What Happens                                                     |
|------|------------------------------------------------------------------|
| 1    | Returning node connects to current leader                        |
| 2    | Node receives heartbeat with leader's current LSN                |
| 3    | Node detects its LSN is behind and sends SyncRequest             |
| 4    | Leader reads WAL entries and sends SyncResponse                  |
| 5    | Follower applies entries to local database                       |
| 6    | Process repeats until follower catches up                        |
| 7    | Once LSN matches, node status becomes Active                     |

**Key Safety Guarantees:**
- Node cannot become leader until status is **Active**
- All missed writes are applied in order via WAL replay
- Underlying database (MariaDB) receives all changes before leadership is allowed

### Adding New Nodes to Existing Clusters

When adding a fresh node to a cluster that already has data, the WAL won't contain the complete history. Use `wolfctl migrate` to copy the database:

| Step | Command                                      |
|------|----------------------------------------------|
| 1    | Install WolfScale on new node                |
| 2    | `wolfctl migrate --from 10.0.10.111:8080`    |
| 3    | `systemctl start wolfscale`                  |

**What happens during migration:**

| Step | Action                                      |
|------|---------------------------------------------|
| 1    | Connect to source node's HTTP API           |
| 2    | Request mysqldump from source               |
| 3    | Stream and apply to local database          |
| 4    | Record source LSN as starting point         |
| 5    | Normal WAL sync takes over from that LSN    |

**NeedsMigration Status:**
- Nodes that can't catch up via WAL enter `NEEDS_MIGRATION` status
- These nodes won't participate in cluster operations
- Run `wolfctl migrate` to resolve this status
- After migration, node transitions to `Syncing` then `Active`

**When Migration is NOT Needed:**

If your new node already has the data, WolfScale will sync normally via WAL:

| Scenario                                | Action Required         |
|-----------------------------------------|-------------------------|
| Manually restored mysqldump first       | Just start WolfScale    |
| Cloned VM from existing node            | Just start WolfScale    |
| Database already has all the data       | Just start WolfScale    |
| Empty database, WAL covers the gap      | Just start WolfScale    |
| Empty database, WAL too old for gap     | Run `wolfctl migrate`   |

The `wolfctl migrate` command is only needed when the new node's database is empty AND the WAL has been rotated/truncated so it doesn't contain the historical entries needed.

---

## Configuration Best Practices

### Node ID Selection

Choose node IDs strategically since the lowest ID becomes leader during failover:

[node]
# Lower IDs get priority during leader election
# Format: aaa-location-number for predictable ordering
id = "db-dc1-001"  # Will become leader over db-dc1-002

# OR use simple names
id = "primary"     # Will become leader over "replica1"

### Recommended Configuration

[node]
id = "wolftest1"                     # Unique node ID (lowest wins election)
bind_address = "0.0.0.0:7654"        # Accept connections from any IP
advertise_address = "10.0.10.112:7654"  # CRITICAL: Your actual IP

[cluster]
bootstrap = true                      # Only ONE node should have this true
peers = [                             # List other cluster members
    "10.0.10.113:7654",
    "10.0.10.114:7654"
]

[database]
host = "localhost"
port = 3306
user = "wolfscale"
password = "secure_password"

### Critical Configuration Notes

| Setting | Importance |
|---------|------------|
| `advertise_address` | **REQUIRED** - Must be set to the node's real IP address |
| `bootstrap` | Only ONE node should have `bootstrap = true` |
| `peers` | Should list all OTHER nodes in the cluster |
| Node ID | Use consistent naming scheme; lowest ID becomes leader during failover |

---

## User Setup

Before using WolfScale, you need to create MariaDB users on **each node** in the cluster.

### Required Users

| User | Purpose | Where to Create |
|------|---------|-----------------|
| WolfScale internal user | Used by WolfScale to connect to local MariaDB | All nodes |
| Application users | Your application's database access | All nodes |

### Creating Users

Run these commands on **each node** by connecting directly to local MariaDB:

# Connect to local MariaDB as root
sudo mariadb -u root

# Create WolfScale internal user (matches config [database] section)
CREATE USER 'wolfscale'@'localhost' IDENTIFIED BY 'your_secure_password';
GRANT ALL PRIVILEGES ON *.* TO 'wolfscale'@'localhost';

# Create application user(s)
CREATE USER 'appuser'@'%' IDENTIFIED BY 'app_password';
CREATE USER 'appuser'@'localhost' IDENTIFIED BY 'app_password';
GRANT ALL PRIVILEGES ON your_database.* TO 'appuser'@'%';
GRANT ALL PRIVILEGES ON your_database.* TO 'appuser'@'localhost';

FLUSH PRIVILEGES;
EXIT;

> **Note:** The `%` wildcard allows connections from any host. Use more specific hostnames for better security.

### Why Users Must Exist on All Nodes

- Each node runs its own MariaDB instance
- WolfScale proxy connects to the local MariaDB
- When clients connect to any node, they authenticate against that node's MariaDB
- Users must have the same credentials on all nodes for seamless failover

---

## Architecture

**Cluster Structure:**

| Node     | Role     | Components                    | Replication          |
|----------|----------|-------------------------------|----------------------|
| Node 1   | Leader   | WAL + MariaDB                 | Sends to all followers|
| Node 2-N | Follower | WAL + MariaDB                 | Receives from leader |

**Data Flow:** Client Write → Leader WAL → Replicate to Followers → Apply to MariaDB

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
5. Leader confirms the write once replication is complete

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

### 5. Automatic Leader Election (Failover)

WolfScale uses Raft-style leader election to automatically promote a follower to leader when the current leader goes down.

**How it works:**

1. Followers track heartbeats from the leader
2. If no heartbeat is received within the election timeout (default: 1.5-3 seconds, randomized), a follower becomes a candidate
3. The candidate requests votes from all peers
4. A node grants its vote if the candidate has a higher or equal term and an up-to-date log
5. The candidate with a majority of votes becomes the new leader
6. The new leader starts sending heartbeats immediately

**Configuration:**

[cluster]
election_timeout_min_ms = 1000    # Fast failover (1 second)
election_timeout_max_ms = 1500    # Max timeout (1.5 seconds)
never_leader = false              # Set to true for read-only replica

# For WAN or unreliable networks, increase timeouts:
# election_timeout_min_ms = 3000  # 3 seconds
# election_timeout_max_ms = 5000  # 5 seconds

**Monitoring failover:**

# Check cluster status
curl http://localhost:8080/cluster

# View logs for election messages
wolfscale start --log-level debug

**Cluster Sizing Guide:**

| Nodes | Fault Tolerance   | Use Case                        |
|-------|-------------------|---------------------------------|
| 1     | None              | Development only                |
| 2     | 1 node failure    | Basic HA (not recommended)      |
| 3     | 2 node failures   | Minimum for production          |
| 5     | 4 node failures   | Recommended for production      |
| 7     | 6 node failures   | High availability               |

**Note:** WolfScale doesn't use quorum - only one node needs to survive. While the cluster can run on a single remaining node, it's recommended to maintain at least 2 active nodes for redundancy.

**Geo-Distribution:** Nodes can be deployed across different data centers or regions for:
- **Low-latency reads** - Connect to your nearest node
- **Disaster recovery** - Survive entire datacenter failures
- **Automatic forwarding** - If a follower doesn't have up-to-date data, reads are forwarded to the leader
- **Write forwarding** - All writes are automatically forwarded to the leader regardless of which node you connect to

### Complete 3-Node Cluster Example

All nodes use the same peer list - WolfScale ignores its own address:

**All Nodes - Same Cluster Section:**

[cluster]
peers = ["10.0.10.10:7654", "10.0.10.11:7654", "10.0.10.12:7654"]

**Node 1 (10.0.10.10):**
[node]
id = "node-1"
bind_address = "0.0.0.0:7654"
advertise_address = "10.0.10.10:7654"

**Node 2 (10.0.10.11):**
[node]
id = "node-2"
bind_address = "0.0.0.0:7654"
advertise_address = "10.0.10.11:7654"

**Node 3 (10.0.10.12):**
[node]
id = "node-3"
bind_address = "0.0.0.0:7654"
advertise_address = "10.0.10.12:7654"

> [!TIP]
> Use the same configuration file across all nodes - just change the `[node]` section for each host.

### 6. Write Forwarding

Followers automatically forward write requests to the current leader, so clients can send writes to any node:

Client -> Follower -> Leader -> Replication -> Response

**Benefits:**
- Clients don't need to track which node is the leader
- Simplifies load balancer configuration
- Transparent failover during leader elections

**How it works:**
1. Client sends write to any node (e.g., `POST /write/insert`)
2. If the node is a follower, it looks up the current leader
3. Follower proxies the request to the leader's HTTP API
4. Leader processes the write and returns response
5. Follower returns the leader's response to the client

> **Note:** For lowest latency, send writes directly to the leader when known.

### 7. MySQL Proxy Mode

Every WolfScale node includes a **built-in MySQL protocol proxy**, allowing applications to connect as if it were a regular MariaDB server:

mysql -h any-node -P 8007 -u user -p

**How it works:**
1. Application connects to any node on port 8007
2. Proxy accepts the connection using MySQL wire protocol
3. Proxy determines routing based on query type and replication status
4. SQL errors are returned back to client in MySQL format

**Routing Logic:**

| Scenario | Action |
|----------|--------|
| **Write** (INSERT/UPDATE/DELETE/CREATE/ALTER/DROP) | Always routes to leader |
| **Read** + node is leader | Returns from local database |
| **Read** + follower + caught up (lag=0) | Returns from local database |
| **Read** + follower + lagging (lag>0) | Routes to leader for fresh data |

**Write Replication:**
- When the leader receives a write through the proxy, it logs the query to the WAL
- Followers replicate the WAL entries and execute them locally
- This ensures all nodes eventually have the same data

**Benefits:**
- Every node is a MySQL entry point - no separate proxy service needed
- Applications need no code changes
- Works with any MySQL client/library
- Transparent write routing to leader
- Smart read routing based on replication status
- SQL errors passed through unchanged

**Standalone proxy (optional):**

You can also run a dedicated proxy on a separate machine:

wolfscale proxy --listen 0.0.0.0:8007 --config wolfscale.toml

---

## Configuration

### Configuration File (`wolfscale.toml`)

[node]
id = "node-1"                      # Unique node identifier
bind_address = "0.0.0.0:7654"      # What to listen on
advertise_address = "10.0.10.10:7654"  # Address other nodes use to reach this node
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
bootstrap = false                  # Set to true ONLY on initial leader
peers = ["10.0.10.11:7654", "10.0.10.12:7654"]  # All OTHER nodes (with ports!)
heartbeat_interval_ms = 500        # Heartbeat frequency
election_timeout_ms = 2000         # Leader election timeout

[api]
enabled = true
bind_address = "0.0.0.0:8080"      # HTTP API port

[proxy]
enabled = true                     # Built-in MySQL proxy (default: true)
bind_address = "0.0.0.0:3307"      # MySQL proxy port

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
| `wolfscale proxy --listen ADDR` | Start MySQL protocol proxy |

---

## Installation & Service Management

### Choose Your Setup Path

> **All cluster nodes MUST have identical data before starting WolfScale.** WolfScale replicates new changes only — it does NOT sync existing data between nodes.

| Option 1: Brand New | Option 2: Backup & Restore | Option 3: Binlog Mode |
|---------------------|---------------------------|----------------------|
| **Empty databases** | **Can take source offline** | **Live database, no downtime** |
| Create the cluster | mysqldump your existing database | Use Binlog Mode |
| Point your software to the MySQL proxy | Set up empty WolfScale cluster | Replicate live from your existing database |
| Start using WolfScale immediately | Restore to leader via proxy | Works with Galera clusters too |
| | Data replicates to all nodes | Switch to WolfScale when ready |

### Quick Start with `run.sh`

Use the included `run.sh` script for development and testing:

./run.sh start              # Start as follower node
./run.sh start --bootstrap  # Start as leader (first node)
./run.sh proxy              # Start MySQL proxy on port 8007
./run.sh status             # Check cluster status
./run.sh info               # Show node info

### Installing as a System Service

Use `install_service.sh` to install WolfScale as a systemd service:

# Install as cluster node
sudo ./install_service.sh node

# Install as MySQL proxy
sudo ./install_service.sh proxy

**Interactive Configuration:** If no `wolfscale.toml` exists, the installer will ask:
- Node ID (defaults to hostname)
- Bind address for cluster communication
- Whether this is the first/bootstrap node
- Peer addresses (one per line, format: `host:port`)
- MariaDB connection details (host, port, database, user, password)
- HTTP API port

**Service Commands:**

sudo systemctl start wolfscale      # Start
sudo systemctl stop wolfscale       # Stop
sudo systemctl enable wolfscale     # Start on boot
sudo systemctl status wolfscale     # Check status
sudo journalctl -u wolfscale -f     # View logs

**File Locations:**
| Path | Description |
|------|-------------|
| `/opt/wolfscale/wolfscale` | Binary |
| `/opt/wolfscale/wolfscale.toml` | Configuration |
| `/var/lib/wolfscale/` | Data directory |
| `/var/log/wolfscale/` | Log files |

---

## Adding Nodes to the Cluster

### Step 1: Prepare the New Node

On the new machine, initialize a configuration file:

wolfscale init --node-id node-2 --output wolfscale.toml

### Step 2: Configure the New Node

Edit `wolfscale.toml` on the new node:

[node]
id = "node-2"                           # Must be unique across cluster
bind_address = "0.0.0.0:7654"
advertise_address = "10.0.10.11:7654"   # THIS node's reachable IP
data_dir = "/var/lib/wolfscale/node-2"

[database]
host = "localhost"                       # Local MariaDB on this node
port = 3306
user = "wolfscale"
password = "your-password"
database = "myapp"

[cluster]
bootstrap = false                        # Only leader has bootstrap=true
peers = ["10.0.10.10:7654", "10.0.10.12:7654"]  # All OTHER nodes (with ports!)

### Step 3: Join the Cluster

wolfscale join leader-host:7654

This will:
1. Connect to the leader and register as a follower
2. Receive all WAL entries to catch up with current state
3. Start running as an active follower

### Alternative: Install as a Service

sudo ./scripts/install-service.sh --node-id node-2
sudo nano /etc/wolfscale/wolfscale.toml  # Add leader to peers
sudo systemctl start wolfscale

### Configuration Comparison

| Setting | Leader (node-1) | Follower (node-2+) |
|---------|-----------------|-------------------|
| `--bootstrap` flag | Yes (first start) | No |
| `cluster.peers` | `[]` (empty) | `["leader-ip:7654"]` |
| How to start | `wolfscale start --bootstrap` | `wolfscale join leader:7654` |

### Verify the Cluster

# From any node
curl http://localhost:8080/cluster
curl http://localhost:8080/cluster/nodes

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

# Option 1: New cluster with complete WAL - just join
wolfscale join leader:7654

# Option 2: Established cluster with pruned WAL - restore backup first
# Step 1: Get a database dump from an existing node
mysqldump -h existing-node -u wolfscale -p myapp > backup.sql

# Step 2: Restore to the new node's local MariaDB
mysql -u wolfscale -p myapp < backup.sql

# Step 3: Now join - WolfScale catches up from the backup point
wolfscale join leader:7654

> **Tip:** For production clusters, consider using longer `retention_hours` or keeping database backups readily available for new node provisioning.

---

## HTTP API

### Write Operations

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

### Status Endpoints

curl http://localhost:8080/health    # Health check
curl http://localhost:8080/status    # Node status
curl http://localhost:8080/cluster   # Cluster info

---

## WolfCtl CLI Tool

`wolfctl` is a command-line tool for managing and monitoring WolfScale clusters.

### Installation

The `wolfctl` binary is automatically installed to `/usr/local/bin` when using the setup script. For manual installations:

sudo cp target/release/wolfctl /usr/local/bin/

### Commands

#### List Cluster Servers

wolfctl list servers

Shows status of all nodes in the cluster:
- Node ID and address
- Status (Active, Joining, Lagging, Offline)
- Role (Leader or Follower)
- LSN and replication lag

#### Show Node Status

wolfctl status

Shows status of the local node.

#### Promote/Demote

wolfctl promote    # Request leader promotion
wolfctl demote     # Step down from leadership
wolfctl check-config         # Validate configuration file
wolfctl check-config -f /path/to/config.toml  # Check specific file

#### Live Statistics

wolfctl stats

Shows live throughput statistics, updating every second until Ctrl+C:
- Node ID and role (Leader/Follower)
- Current LSN (Log Sequence Number)
- Writes per second (calculated from LSN changes)
- Cluster node count
- Follower replication lag (when running on leader)

### Configuration Validation

The `check-config` command validates your configuration file and reports issues:

wolfctl check-config

It checks for:
- **Typos** in key names (e.g., `dvertise_address` instead of `advertise_address`)
- **Missing required fields** like `advertise_address` or `node.id`
- **Self-referencing peers** (listing your own IP in the peers list)
- **Bootstrap conflicts** (warns if bootstrap is enabled)

### Options

| Option | Description |
|--------|-------------|
| `-c, --config <PATH>` | Path to config file (default: `/etc/wolfscale/config.toml`) |
| `-e, --endpoint <URL>` | API endpoint to connect to (overrides config) |

### Examples

# Check cluster status from any node
wolfctl list servers

# Connect to a specific node's API
wolfctl -e http://192.168.1.10:8080 list servers

# Quick health check
wolfctl status

---

## Requirements

- **Rust**: 1.70+
- **MariaDB**: 10.5+
- **Linux**: Recommended for best performance

---

## Directory Structure

/var/lib/wolfscale/<node-id>/
├── wal/           # Write-ahead log segments
├── state/         # SQLite state database
└── data/          # Other runtime data

---

## Building from Source

# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Install to system
cargo install --path .

# Run tests
cargo test

---

## Backup and Restore

### Creating Backups

When creating backups with `mysqldump`, use these options for reliable replication with WolfScale:

```bash
#!/bin/bash
# backup.sh - WolfScale-compatible backup script

DATABASE="mydb"
BACKUP_FILE="backup_$(date +%Y%m%d_%H%M%S).sql"

mysqldump \
  --hex-blob \
  --routines \
  --triggers \
  --single-transaction \
  --skip-lock-tables \
  --set-gtid-purged=OFF \
  "$DATABASE" > "$BACKUP_FILE"

echo "Backup saved to: $BACKUP_FILE"
```

**Option Reference:**

| Option | Purpose |
|--------|---------|
| `--hex-blob` | **Essential** - Encodes binary/BLOB data as hex to prevent corruption |
| `--routines` | Include stored procedures and functions |
| `--triggers` | Include triggers |
| `--single-transaction` | Consistent snapshot without locking (InnoDB) |
| `--skip-lock-tables` | Avoid LOCK TABLES statements (good for replication) |
| `--set-gtid-purged=OFF` | Avoid GTID conflicts when restoring |

> **Warning:** Without `--hex-blob`, binary data (images, vectors, serialized objects) may be corrupted in the backup.

### Restoring Backups

Restore backups directly through WolfScale's MySQL proxy (port 3307) to replicate to all nodes:

```bash
# Restore through WolfScale (recommended - replicates to all nodes)
mysql -h leader-ip -P 3307 -u root -p mydb < backup.sql

# Or restore directly to MariaDB on the leader only
mysql -h localhost -P 3306 -u root -p mydb < backup.sql
```

### Full Cluster Backup

To backup all databases:

```bash
#!/bin/bash
# full_backup.sh - Complete cluster backup

BACKUP_DIR="/var/backups/wolfscale"
DATE=$(date +%Y%m%d_%H%M%S)
mkdir -p "$BACKUP_DIR"

mysqldump \
  --all-databases \
  --hex-blob \
  --routines \
  --triggers \
  --single-transaction \
  --skip-lock-tables \
  --set-gtid-purged=OFF \
  > "$BACKUP_DIR/full_backup_$DATE.sql"

# Compress for storage
gzip "$BACKUP_DIR/full_backup_$DATE.sql"

echo "Full backup saved to: $BACKUP_DIR/full_backup_$DATE.sql.gz"
```

---

## Performance Tuning

### Quick Auto-Tune

The easiest way to optimize MariaDB is to run the auto-tuner:

```bash
sudo wolfctl tune-mariadb
```

This command will:
- Detect your system's RAM and calculate optimal buffer pool size (70%)
- Detect CPU cores for thread pool configuration  
- Create `/etc/mysql/mariadb.conf.d/99-wolfscale.cnf`
- Restart MariaDB automatically

Use `--dry-run` to preview changes without applying them.

### MariaDB Tuning for WolfScale

If you prefer manual configuration, optimize your MariaDB instances with these settings:

#### InnoDB Buffer Pool (Most Impactful)

```ini
# Set to 70-80% of available RAM
innodb_buffer_pool_size = 4G

# 1 instance per GB of buffer pool
innodb_buffer_pool_instances = 4
```

#### Write Performance

```ini
# Transaction durability vs speed tradeoff
# 0 = fastest (risk of 1s data loss on crash)
# 1 = safest (sync on every commit)
# 2 = balanced (sync once per second) - RECOMMENDED
innodb_flush_log_at_trx_commit = 2

# Avoid double buffering
innodb_flush_method = O_DIRECT

# Disable if using WolfScale replication instead of binlog
sync_binlog = 0
```

#### Connections & Threading

```ini
# Enough for connection pool + WolfScale connections
max_connections = 500

# Match CPU cores
thread_pool_size = 16

# Better for many connections
thread_handling = pool-of-threads
```

#### Query Cache (Disable)

WolfScale handles consistency, so query cache can cause stale reads:

```ini
query_cache_type = 0
query_cache_size = 0
```

#### Logging (Reduce if Not Needed)

```ini
general_log = 0
slow_query_log = 0                    # Or set slow_query_time = 5
```

#### Bulk Insert Optimizations

```ini
# Fastest for bulk inserts with WolfScale
innodb_autoinc_lock_mode = 2
bulk_insert_buffer_size = 256M
```

### Recommended my.cnf Settings

Add to `/etc/mysql/mariadb.conf.d/99-wolfscale.cnf`:

```ini
[mariadb]
# Buffer pool - adjust based on available RAM
innodb_buffer_pool_size = 4G
innodb_buffer_pool_instances = 4

# Write performance
innodb_flush_log_at_trx_commit = 2
innodb_flush_method = O_DIRECT
sync_binlog = 0

# Connections
max_connections = 500
thread_pool_size = 16
thread_handling = pool-of-threads

# Disable query cache
query_cache_type = 0
query_cache_size = 0

# Bulk operations
innodb_autoinc_lock_mode = 2
bulk_insert_buffer_size = 256M
```

Then restart MariaDB: `sudo systemctl restart mariadb`

### WolfScale Tuning

#### WAL Settings

In `wolfscale.toml`:

```toml
[wal]
# Increase for bulk operations (more entries per batch)
batch_size = 5000            # Default: 1000

# Increase for more batching (tradeoff: latency)
flush_interval_ms = 500      # Default: 100

# Disable for speed (tradeoff: durability on crash)
fsync = false                # Default: true

# Enable compression for less disk I/O
compression = true           # Default: true
```

#### Connection Pool

```toml
[database]
# Increase for high-concurrency workloads
pool_size = 50               # Default: 10
```

#### Replication Settings

```toml
[cluster]
# Larger batches to followers
max_batch_entries = 5000     # Default: 1000

# Faster heartbeats for quicker failover (tradeoff: network overhead)
heartbeat_interval_ms = 250  # Default: 500
```

### Performance Tips Summary

| Optimization | Impact | Tradeoff |
|--------------|--------|----------|
| `innodb_buffer_pool_size` | **High** | RAM usage |
| `innodb_flush_log_at_trx_commit = 2` | **High** | ~1s data on crash |
| `fsync = false` | **High** | Durability on crash |
| `batch_size` increase | **Medium** | Write latency |
| `pool_size` increase | **Medium** | Connection overhead |
| `compression = true` | **Low** | CPU usage |

---

## Support

- **Discord:** [Join our community](https://discord.gg/q9qMjHjUQY)
- **Website:** [wolf.uk.com](https://wolf.uk.com)
- **Issues:** [GitHub Issues](https://github.com/wolfsoftwaresystemsltd/WolfScale/issues)
- **Support Us:** [Patreon](https://www.patreon.com/15362110/join)
