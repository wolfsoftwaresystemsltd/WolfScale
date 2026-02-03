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

### Deterministic Leader Election
No voting, no split-brain. The node with the lowest ID among active nodes becomes leader immediately. Simple, predictable, reliable.

### WAL-Based Replication
Write-Ahead Log with LZ4 compression ensures all changes are durably replicated. Nodes that fall behind automatically catch up via WAL sync.

### Disaster Recovery Built-In
When a leader fails:
1. New leader is elected instantly (lowest ID wins)
2. Old leader rejoins as follower when it returns
3. Old leader syncs all missed writes before becoming eligible for leadership again

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

## Cluster Sizing

| Nodes | Fault Tolerance   | Use Case                        |
|-------|-------------------|---------------------------------|
| 1     | None              | Development only                |
| 2     | 1 node failure    | Basic HA (not recommended)      |
| 3     | 2 node failures   | Minimum for production          |
| 5     | 4 node failures   | Recommended for production      |

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

- **Website:** [wolf.uk.com](https://wolf.uk.com)
- **Issues:** [GitHub Issues](https://github.com/wolfsoftwaresystemsltd/WolfScale/issues)
- **Support Us:** [Patreon](https://www.patreon.com/15362110/join)
