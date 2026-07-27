# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## THIS DIRECTORY IS A MIRROR — releases come from the WolfNet repo

The shipping source of WolfNet is **`github.com/wolfsoftwaresystemsltd/WolfNet`**,
not this directory. `wolfstack/setup.sh` clones that repo to build from source
and downloads its release binaries; nothing consumes the copy here.

This copy lags — as of 2026-07-27 it is at 0.5.27 while upstream is 0.5.30, so
`src/main.rs`, `src/peer.rs` and `src/tun.rs` differ (IPv6 subnet routing, peer
dedup). **A fix committed only here reaches no user**, and the version numbers
are not interchangeable: 0.5.28 here would collide with a real upstream release
that contains entirely different code.

To change WolfNet: commit to the WolfNet repo. Pushing to its `main` with
`Cargo.toml`/`src/**` touched builds and publishes the release automatically.

## Project Overview

WolfNet is a secure private mesh networking daemon written in Rust. It creates encrypted tunnels between machines using TUN interfaces, X25519 key exchange, and ChaCha20-Poly1305 encryption. Peers discover each other via LAN broadcast and PEX (Peer Exchange) gossip protocol.

## Build & Test

```bash
cargo build --release          # Build both binaries
cargo test                     # Run tests
cargo clippy                   # Lint
```

Produces two binaries:
- `wolfnet` — the daemon (src/main.rs)
- `wolfnetctl` — CLI control utility (src/ctl/main.rs)

Requires root/sudo to run (creates TUN interfaces, writes to /etc/wolfnet/ and /var/run/wolfnet/).

## Architecture

The daemon (`wolfnet`) runs a single-threaded poll loop over a UDP socket and TUN device fd. There is no async runtime.

**Module responsibilities:**
- `main.rs` — CLI parsing, daemon event loop (poll on UDP socket + TUN fd), packet dispatch, SIGHUP handler for config reload, status file writer
- `config.rs` — TOML config loading/saving from `/etc/wolfnet/config.toml`, `NodeStatus` struct serialized to `/var/run/wolfnet/status.json`
- `crypto.rs` — X25519 keypair generation/loading, ChaCha20-Poly1305 session encryption, peer ID derivation (first 4 bytes of SHA256 of public key)
- `peer.rs` — `Peer` struct with per-peer session state, `PeerManager` for the peer table, PEX message encoding/decoding, relay forwarding logic
- `transport.rs` — Wire protocol: packet type tags (0x01 handshake, 0x03 data, 0x04 keepalive, 0x05 discovery, 0x06 PEX), serialization/deserialization
- `tun.rs` — Linux TUN device creation via ioctl, IP/MTU configuration, interface lifecycle
- `gateway.rs` — NAT masquerading via iptables, IP forwarding toggle, external interface detection
- `ctl/main.rs` — Reads `/var/run/wolfnet/status.json` written by daemon, provides status/peers/info/purge commands

**Key data flow:** UDP packet arrives → match packet type tag → handshake/decrypt/keepalive/PEX → if data packet, decrypt and write plaintext to TUN fd. Reverse: read IP packet from TUN fd → look up destination peer by IP → encrypt → send UDP.

**IPC between daemon and ctl:** The daemon writes `/var/run/wolfnet/status.json` periodically. `wolfnetctl` reads this file. The `purge` command sends SIGHUP to the daemon process to trigger config reload.

## Configuration

Runtime config: `/etc/wolfnet/config.toml`
Status file: `/var/run/wolfnet/status.json`
Private key: `/etc/wolfnet/private.key`
Default listen port: 9600, discovery broadcast port: 9601
Default TUN interface: `wolfnet0`, subnet: `10.0.10.0/24`
