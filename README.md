# WolfScale

<div align="center">

**High-Availability MariaDB Replication with Automatic Failover**

[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Patreon](https://img.shields.io/badge/Patreon-Support%20Us-orange.svg)](https://www.patreon.com/15362110/join)

**© Wolf Software Systems Ltd** — [wolf.uk.com](https://wolf.uk.com)

</div>

---

WolfScale is a lightweight, high-availability replication layer for MariaDB stand alone servers. It provides **automatic leader election** with deterministic failover, **WAL-based replication** for strong consistency, and a **MySQL-compatible proxy** for transparent routing—all in a single Rust binary.

## Why WolfScale?

| Feature | Benefit |
|---------|---------|
| **Sub-Millisecond Replication** | Push-based replication faster than MySQL, MariaDB, or Galera |
| **Zero Write Conflicts** | Single-leader model eliminates certification failures |
| **Predictable Failover** | Lowest node ID always wins—you know exactly who becomes leader |
| **Safe Node Rejoin** | Returning nodes sync via WAL before taking leadership |
| **Zero-Config Discovery** | Nodes find each other automatically via UDP broadcast |
| **Transparent Proxy** | Connect via MySQL protocol—no application changes needed |
| **Built-in Load Balancer** | Distribute connections across cluster nodes with automatic failover |
| **Single Binary** | No patched databases, no complex dependencies |

## Key Features

| Category | Feature | Description |
|----------|---------|-------------|
| **Replication** | Push-Based Sync | Sub-millisecond replication via event-driven WAL notifications |
| | WAL-Based Sync | LZ4-compressed Write-Ahead Log for strong consistency |
| | Binlog Mode | Capture writes from MySQL/MariaDB binlog (v3.0+) |
| | Write Forwarding | Followers automatically forward writes to leader |
| **High Availability** | Deterministic Leader Election | Lowest node ID wins—predictable, instant failover |
| | Automatic Catch-Up | Returning nodes sync via WAL before leadership |
| | Health Monitoring | Leader monitors database, auto-demotes on failure |
| | Load Balancer Mode | Built-in LB for distributing connections (v5.3.6+) |
| **Connectivity** | MySQL Proxy | Native MySQL protocol on port 8007 |
| | HTTP API | RESTful API for writes and cluster management |
| | wolfctl CLI | `list servers`, `stats`, `migrate`, `binlog-setup` |
| **Deployment** | Single Binary | No patched databases, minimal dependencies |
| | Any MySQL/MariaDB | Works with standalone, Galera, Percona, RDS |
| | Geo-Distribution | Deploy nodes across regions and data centers |

### Binlog Replication Mode (v3.0+)

Capture writes directly from any MySQL-compatible database:

```bash
# Auto-detect binlog position and generate config
wolfctl binlog-setup

# Supports: MariaDB, MySQL, Galera, Percona, Amazon RDS
```

### Deterministic Leader Election
No voting, no split-brain. The node with the lowest ID among active nodes becomes leader immediately.

### WAL-Based Replication
Write-Ahead Log with LZ4 compression ensures all changes are durably replicated. Nodes that fall behind automatically catch up.

### Fastest Replication in the Industry

| System | Typical Replication Lag |
|--------|------------------------|
| MySQL Async | 100ms - seconds |
| MySQL Semi-Sync | 10-50ms |
| MariaDB Galera | 10-20ms |
| **WolfScale** | **<1ms** ⚡ |

WolfScale uses push-based replication that triggers immediately when writes are committed, rather than polling. This gives you the fastest replication available.

### MySQL Proxy Mode
```bash
# Connect like a normal MySQL client
mysql -h wolfscale-host -P 8007 -u user -p

# Writes automatically route to leader
# Reads distributed across followers
```

### Real-Time Health Monitoring
Leader continuously monitors database health. If MariaDB goes down, WolfScale steps down and fails over to the next node—automatically.

## Quick Start

### Choose Your Setup Path

> **All cluster nodes MUST have identical data before starting WolfScale.** WolfScale replicates new changes only — it does NOT sync existing data between nodes.

| Option 1: Brand New | Option 2: Backup & Restore | Option 3: Binlog Mode |
|---------------------|---------------------------|----------------------|
| **Empty databases** | **Can take source offline** | **Live database, no downtime** |
| Create the cluster | mysqldump your existing database | Use Binlog Mode |
| Point your software to the MySQL proxy | Set up empty WolfScale cluster | Replicate live from your existing database |
| Start using WolfScale immediately | Restore to leader via proxy | Works with Galera clusters too |
| | Data replicates to all nodes | Switch to WolfScale when ready |

> **Restoring Large Databases:** When restoring databases with large content (WordPress, BLOBs, vector embeddings), use `mysqldump --skip-extended-insert` to create single-row INSERT statements. This prevents proxy buffer issues with very large rows.

### One-Line Install

Run this on each server:

```bash
curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/setup.sh | bash
```

This automatically installs dependencies, builds WolfScale, and runs the interactive configuration wizard.

### Load Balancer Mode (v5.4.0+)

Run a standalone load balancer that routes MySQL connections to your WolfScale cluster:

```bash
curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/setup_lb.sh | bash
```

**Auto-Discovery (v5.4.0):** The load balancer automatically finds WolfScale nodes on your network—no configuration needed. Just run the installer and choose auto-discovery when prompted.

**Manual Mode:** You can also specify peers manually or copy `wolfscale.toml` from any cluster node:

```bash
# Copy config from a cluster node (optional)
scp user@cluster-node:/opt/wolfscale/wolfscale.toml /opt/wolfscale/

# Run load balancer installer
curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/setup_lb.sh | bash
```

The load balancer listens on port **3306** and routes traffic to healthy cluster nodes.

### Cluster Commands

```bash
# Check cluster status
wolfctl list servers

# Live throughput monitoring
wolfctl stats

# Reset WAL and state on ALL nodes (DESTRUCTIVE)
wolfctl reset --force
```

### Adding New Nodes

When adding a new node to an existing cluster with data:

```bash
# On the new node - migrate database from an existing node
wolfctl migrate --from 10.0.10.111:8080

# Then start WolfScale normally
systemctl start wolfscale
```

## Architecture

| Layer        | Component                                      |
|--------------|------------------------------------------------|
| Applications | Connect via HTTP API or MySQL Protocol         |
| WolfScale    | Leader + Followers replicate via WAL           |
| Database     | Each node has local MariaDB (localhost:3306)   |

**Write Flow:** Client → Any Node → Forwarded to Leader → Replicated to All Nodes

**Read Flow:** Client → Any Node → Local Data (or forwarded to Leader if node is behind)

## Cluster Sizing

| Nodes | Fault Tolerance   | Use Case                        |
|-------|-------------------|---------------------------------|
| 1     | None              | Development only                |
| 2     | 1 node failure    | Basic HA (not recommended)      |
| 3     | 2 node failures   | Minimum for production          |
| 5     | 4 node failures   | Recommended for production      |
| 7     | 6 node failures   | High availability               |

**Geo-Distribution:** Nodes can be deployed across different data centers or regions. Connect to your nearest node for low-latency reads - if the data isn't up-to-date, the request is automatically forwarded to the leader.

> **Note:** WolfScale doesn't use quorum - only one node needs to survive. While the cluster can run on a single remaining node, it's recommended to maintain at least 2 active nodes for redundancy.

> **Note:** The install wizard creates your configuration file automatically. See the [full documentation](docs/DOCUMENTATION.md) for advanced configuration options.

## Documentation

See [docs/DOCUMENTATION.md](docs/DOCUMENTATION.md) for complete documentation including:
- Cluster communication and heartbeat timing
- Leader election and node status transitions
- Disaster recovery and WAL catch-up
- Configuration best practices
- API reference

---

## License

MIT License - see [LICENSE](LICENSE) for details.

---

## ⚠️ Disclaimer

**USE AT YOUR OWN RISK.** This software is provided "as is" without warranty of any kind, express or implied, including but not limited to the warranties of merchantability, fitness for a particular purpose, and noninfringement. In no event shall Wolf Software Systems Ltd be liable for any claim, damages, or other liability arising from the use of this software.

By using WolfScale, you acknowledge that you are solely responsible for your data and any consequences of using this software.

## Support

- **Discord:** [Join our community](https://discord.gg/q9qMjHjUQY)
- **Website:** [wolf.uk.com](https://wolf.uk.com)
- **Issues:** [GitHub Issues](https://github.com/wolfsoftwaresystemsltd/WolfScale/issues)
- **Support Us:** [Patreon](https://www.patreon.com/15362110/join)
