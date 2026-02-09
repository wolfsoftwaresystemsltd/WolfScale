# 🐺 Wolf — Server Clustering Tools Made Simple

<div align="center">

**Open-source tools for building robust, clustered server infrastructure**

[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Patreon](https://img.shields.io/badge/Patreon-Support%20Us-orange.svg)](https://www.patreon.com/15362110/join)

**[wolfscale.org](https://wolfscale.org)** • **[wolf.uk.com](https://wolf.uk.com)** • **[Discord](https://discord.gg/q9qMjHjUQY)**

© Wolf Software Systems Ltd

</div>

---

Wolf started as a database replication tool and has grown into a suite of server clustering utilities. Every tool runs as a single Rust binary, uses auto-discovery, and is designed to be simple to set up.

| Tool | Description | Status |
|------|-------------|--------|
| **[WolfScale](#wolfscale--database-replication)** | MariaDB/MySQL replication, clustering & load balancing | ✅ Available |
| **[WolfDisk](#wolfdisk--distributed-filesystem)** | Disk sharing & replication across networks | ✅ Available |
| **[WolfNet](#wolfnet--private-networking)** | Secure private networking across the internet | ✅ Available |

---

## WolfScale — Database Replication

**Database replication, clustering, and load balancing — the easy way**

WolfScale is a lightweight, high-availability replication layer for MariaDB/MySQL. It provides **automatic leader election** with deterministic failover, **WAL-based replication** for strong consistency, and a **MySQL-compatible proxy** for transparent routing—all in a single Rust binary.

Works with MySQL, Percona, and Amazon RDS • **MariaDB recommended**

### Why WolfScale?

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

### Fastest Replication in the Industry

| System | Typical Replication Lag |
|--------|------------------------|
| MySQL Async | 100ms - seconds |
| MySQL Semi-Sync | 10-50ms |
| MariaDB Galera | 10-20ms |
| **WolfScale** | **<1ms** ⚡ |

### Quick Start

> **All cluster nodes MUST have identical data before starting WolfScale.** WolfScale replicates new changes only — it does NOT sync existing data between nodes.

```bash
# Install WolfScale on each server
curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/setup.sh | bash
```

### Load Balancer

Install the load balancer directly on any server that needs database access. It auto-discovers your cluster — no configuration needed.

```bash
curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/setup_lb.sh | bash
```

```
Web Server 1 ─── WolfScale LB ───┐
             (auto-discovers)    │
                                 │
Web Server 2 ─── WolfScale LB ───┼──► WolfScale DB Cluster
             (auto-discovers)    │
                                 │
Web Server 3 ─── WolfScale LB ───┘
             (auto-discovers)
```

### Cluster Commands

```bash
wolfctl list servers     # Check cluster status
wolfctl stats            # Live throughput monitoring
wolfctl migrate --from 10.0.10.111:8080   # Migrate data to new node
wolfctl reset --force    # Reset WAL and state (DESTRUCTIVE)
```

---

## WolfDisk — Distributed Filesystem

**Disk sharing and replication across networks — the easy way**

WolfDisk is a FUSE-based distributed filesystem that shares and replicates files across Linux servers. Mount a shared directory on any number of machines and have your data automatically synchronised. Supports leader, follower, and client modes.

### Why WolfDisk?

| Feature | Benefit |
|---------|---------|
| **FUSE-Based** | Mount as a regular filesystem—works with any application |
| **Automatic Replication** | Files sync to all nodes automatically |
| **Content-Addressed Storage** | Efficient deduplication via SHA256 chunking |
| **Leader-Follower Model** | Strong consistency with automatic failover |
| **Client Mode** | Workstations can access the shared drive without local storage |
| **Multiple Drives** | Run multiple independent filesystems per node |

### Quick Start

```bash
# Interactive installer - prompts for node ID, role, and discovery
curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/wolfdisk/setup.sh | bash
```

### Node Roles

| Role | Storage | Description |
|------|---------|-------------|
| **Leader** | ✅ Local | Primary node — accepts writes, replicates to followers |
| **Follower** | ✅ Local | Receives replicated data, can become leader on failover |
| **Client** | ❌ None | Mount-only — reads/writes forwarded to leader, no local data |
| **Auto** | ✅ Local | Auto-election — lowest ID becomes leader |

See [`wolfdisk/README.md`](wolfdisk/README.md) for full documentation.

---

## WolfNet — Private Networking


WolfNet creates a secure, encrypted private network between your machines over the internet. Machines on WolfNet can see each other as if they were on the same LAN, but all traffic is encrypted with modern cryptography (X25519 + ChaCha20-Poly1305 — the same crypto as WireGuard).

### Why WolfNet?

| Feature | Benefit |
|---------|---------|
| **WireGuard-Class Crypto** | X25519 key exchange + ChaCha20-Poly1305 AEAD encryption |
| **Mesh Networking** | Every node can reach every other node directly — no single point of failure |
| **Gateway Mode** | Route internet traffic through a gateway node with NAT masquerading |
| **LAN Auto-Discovery** | Nodes find each other automatically on the same network |
| **TUN-Based** | Uses kernel TUN interfaces for near-native performance |
| **Single Binary** | No dependencies — just `wolfnet` and `wolfnetctl` |
| **Systemd Service** | Runs as a background service with automatic startup |

### Quick Start

```bash
# Interactive installer — downloads binary, generates keys, creates systemd service
curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/wolfnet/setup.sh | sudo bash
```

The installer will:
- Check for `/dev/net/tun` (with Proxmox/LXC guidance if missing)
- Download and install `wolfnet` and `wolfnetctl`
- Generate an X25519 keypair
- Prompt for WolfNet IP address, port, and gateway mode
- Create a systemd service for automatic startup

### Architecture

```
Machine A (10.0.10.1)          Machine B (10.0.10.2)
┌─────────────────┐            ┌─────────────────┐
│  wolfnet0 (TUN) │◄──────────►│  wolfnet0 (TUN) │
│  10.0.10.1/24   │  Encrypted │  10.0.10.2/24   │
│  ChaCha20-Poly  │  UDP/9600  │  ChaCha20-Poly  │
└─────────────────┘            └─────────────────┘
         ▲                              ▲
         │       Encrypted UDP          │
         └──────────┬───────────────────┘
                    │
           ┌────────▼────────┐
           │  Machine C      │
           │  (Gateway)      │
           │  10.0.10.3/24   │
           │  NAT → Internet │
           └─────────────────┘
```

### CLI Reference

```bash
# Daemon
wolfnet                          # Start the daemon (usually via systemd)
wolfnet init --address 10.0.10.1 # Generate config and keypair
wolfnet genkey                   # Generate a new X25519 keypair
wolfnet pubkey                   # Show this node's public key
wolfnet token                    # Show join token for sharing

# Control utility
wolfnetctl status                # Show node status, IP, uptime
wolfnetctl peers                 # List peers with connection status
wolfnetctl info                  # Combined status and peer list

# Service management
sudo systemctl start wolfnet     # Start service
sudo systemctl status wolfnet    # Check status
sudo journalctl -u wolfnet -f    # View logs
```

### Security

| Layer | Technology |
|-------|------------|
| Key Exchange | **X25519** (Curve25519 Diffie-Hellman) |
| Encryption | **ChaCha20-Poly1305** AEAD (256-bit) |
| Replay Protection | Counter-based nonces with monotonic validation |
| Network Isolation | iptables firewall blocks all external inbound traffic |
| Key Storage | Private keys stored with 0600 permissions |

> ⚠️ **Proxmox/LXC Users:** The TUN device (`/dev/net/tun`) is blocked by default in LXC containers. See [wolfscale.org/wolfnet.html](https://wolfscale.org/wolfnet.html) for setup instructions.

---

## Architecture (WolfScale)

| Layer        | Component                                      |
|--------------|-------------------------------------------------|
| Applications | Connect via HTTP API or MySQL Protocol          |
| WolfScale    | Leader + Followers replicate via WAL            |
| Database     | Each node has local MariaDB (localhost:3306)    |

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

**Geo-Distribution:** Nodes can be deployed across different data centres or regions. Connect to your nearest node for low-latency reads — if the data isn't up-to-date, the request is automatically forwarded to the leader.

> **Note:** WolfScale doesn't use quorum — only one node needs to survive. While the cluster can run on a single remaining node, it's recommended to maintain at least 2 active nodes for redundancy.

## Documentation

- **Website:** [wolfscale.org](https://wolfscale.org)
- **Full Docs:** [docs/DOCUMENTATION.md](docs/DOCUMENTATION.md)
- **WolfDisk Docs:** [wolfdisk/README.md](wolfdisk/README.md)
- **WolfNet Docs:** [wolfscale.org/wolfnet.html](https://wolfscale.org/wolfnet.html)

---

## Support

- ❤️ **Patreon:** [Support development](https://www.patreon.com/15362110/join)
- 💬 **Discord:** [Join our community](https://discord.gg/q9qMjHjUQY)
- 🌐 **Website:** [wolf.uk.com](https://wolf.uk.com)
- ⭐ **GitHub:** [Star this repo](https://github.com/wolfsoftwaresystemsltd/WolfScale)
- 🐛 **Issues:** [Report a bug](https://github.com/wolfsoftwaresystemsltd/WolfScale/issues)

---

## License

MIT License — see [LICENSE](LICENSE) for details.

## ⚠️ Disclaimer

**USE AT YOUR OWN RISK.** This software is provided "as is" without warranty of any kind, express or implied, including but not limited to the warranties of merchantability, fitness for a particular purpose, and noninfringement. In no event shall Wolf Software Systems Ltd be liable for any claim, damages, or other liability arising from the use of this software.

By using Wolf tools, you acknowledge that you are solely responsible for your data and any consequences of using this software.
