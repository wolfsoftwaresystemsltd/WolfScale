# WolfScale

<div align="center">

**High-Availability MariaDB Replication with Automatic Failover**

[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Patreon](https://img.shields.io/badge/Patreon-Support%20Us-orange.svg)](https://www.patreon.com/15362110/join)

**© Wolf Software Systems Ltd** — [wolf.uk.com](https://wolf.uk.com)

</div>

---

WolfScale is a lightweight, high-availability replication layer for MariaDB clusters. It provides **automatic leader election** with deterministic failover, **WAL-based replication** for strong consistency, and a **MySQL-compatible proxy** for transparent routing—all in a single Rust binary.

## Why WolfScale?

| Feature | Benefit |
|---------|---------|
| **Zero Write Conflicts** | Single-leader model eliminates certification failures |
| **Predictable Failover** | Lowest node ID always wins—you know exactly who becomes leader |
| **Safe Node Rejoin** | Returning nodes sync via WAL before taking leadership |
| **Transparent Proxy** | Connect via MySQL protocol—no application changes needed |
| **Single Binary** | No patched databases, no complex dependencies |

## Key Features

| Category | Feature | Description |
|----------|---------|-------------|
| **Replication** | WAL-Based Sync | LZ4-compressed Write-Ahead Log for strong consistency |
| | Binlog Mode | Capture writes from MySQL/MariaDB binlog (v3.0+) |
| | Write Forwarding | Followers automatically forward writes to leader |
| **High Availability** | Deterministic Leader Election | Lowest node ID wins—predictable, instant failover |
| | Automatic Catch-Up | Returning nodes sync via WAL before leadership |
| | Health Monitoring | Leader monitors database, auto-demotes on failure |
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

### One-Line Install

```bash
curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/setup.sh | bash
```

This automatically installs dependencies, builds WolfScale, and runs the interactive configuration wizard.

### Cluster Commands

```bash
# Check cluster status
wolfctl list servers

# Sample output:
# ID          | STATUS | ROLE     | ADDRESS         | LAG
# wolftest1   | ACTIVE | LEADER   | 10.0.10.111:7654 | 0
# wolftest2   | ACTIVE | FOLLOWER | 10.0.10.112:7654 | 0
# wolftest3   | ACTIVE | FOLLOWER | 10.0.10.113:7654 | 0

# Live throughput monitoring (updates every second, Ctrl+C to exit)
wolfctl stats

# Reset WAL and state on ALL nodes (DESTRUCTIVE - requires restart)
wolfctl reset          # Interactive confirmation
wolfctl reset --force  # Skip confirmation
```

### Adding New Nodes

When adding a new node to an existing cluster with data:

```bash
# On the new node - migrate database from an existing node
wolfctl migrate --from 10.0.10.111:8080

# Then start WolfScale normally
systemctl start wolfscale
```

The new node will be in `NEEDS_MIGRATION` status until you run the migrate command.

## Architecture

| Layer        | Component                                      |
|--------------|------------------------------------------------|
| Applications | Connect via HTTP API or MySQL Protocol         |
| WolfScale    | Leader + Followers replicate via WAL           |
| Database     | Each node has local MariaDB (localhost:3306)   |

**Data Flow:** App → Any Node → Leader (for writes) → WAL → All Followers → Local MariaDB

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

## Support

- **Discord:** [Join our community](https://discord.gg/q9qMjHjUQY)
- **Website:** [wolf.uk.com](https://wolf.uk.com)
- **Issues:** [GitHub Issues](https://github.com/wolfsoftwaresystemsltd/WolfScale/issues)
- **Support Us:** [Patreon](https://www.patreon.com/15362110/join)
