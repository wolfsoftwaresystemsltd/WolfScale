//! WolfNet daemon — secure private mesh networking
//!
//! Creates encrypted tunnels between machines using TUN interfaces,
//! X25519 key exchange, and ChaCha20-Poly1305 encryption.
//!
//! Supports automatic peer exchange (PEX) so joining one node
//! automatically gives you access to all its peers.

use std::net::{UdpSocket, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::os::unix::io::AsRawFd;
use std::io::{Read, Write};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tracing::{info, warn, error, debug};


use wolfnet::config::{Config, NodeStatus};
use wolfnet::crypto::KeyPair;
use wolfnet::peer::{Peer, PeerManager};
use wolfnet::tun::{self, TunDevice};
use wolfnet::transport;

#[derive(Parser)]
#[command(name = "wolfnet", version, about = "WolfNet — Secure private mesh networking")]
struct Cli {
    /// Path to config file
    #[arg(short, long, default_value = "/etc/wolfnet/config.toml")]
    config: PathBuf,

    /// Enable debug logging
    #[arg(long)]
    debug: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a new keypair
    Genkey {
        /// Output path for private key
        #[arg(short, long, default_value = "/etc/wolfnet/private.key")]
        output: PathBuf,
    },
    /// Show this node's public key
    Pubkey,
    /// Show join token (public_key@endpoint) for other nodes
    Token,
    /// Generate a default config file
    Init {
        /// WolfNet IP address for this node
        #[arg(short, long, default_value = "10.0.10.1")]
        address: String,
    },
    /// Generate an invite token for a new peer to join your network
    Invite,
    /// Join a WolfNet network using an invite token
    Join {
        /// The invite token from 'wolfnet invite'
        token: String,
    },
}

/// Encrypt plaintext into send_buf and send to peer via UDP. Zero heap allocations.
/// Copies plaintext into send_buf[13..], encrypts in-place, writes 13-byte header, sends.
fn encrypt_and_send(
    send_buf: &mut [u8],
    plaintext: &[u8],
    peer: &mut wolfnet::peer::Peer,
    my_peer_id: &[u8; 4],
    socket: &UdpSocket,
) -> bool {
    let endpoint = match peer.endpoint {
        Some(ep) => ep,
        None => return false,
    };
    if !peer.is_connected() { return false; }
    let len = plaintext.len();
    send_buf[13..13 + len].copy_from_slice(plaintext);
    match peer.encrypt_into(&mut send_buf[13..], len) {
        Ok((counter, ct_len)) => {
            send_buf[0] = transport::PKT_DATA;
            send_buf[1..5].copy_from_slice(my_peer_id);
            send_buf[5..13].copy_from_slice(&(counter as u64).to_le_bytes());
            let _ = socket.send_to(&send_buf[..13 + ct_len], endpoint);
            true
        }
        Err(_) => false,
    }
}

fn main() {
    let cli = Cli::parse();

    let filter = if cli.debug { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .init();

    // Commands that need root access (for /etc/wolfnet/)
    match &cli.command {
        Some(Commands::Invite) | Some(Commands::Join { .. }) | None => {
            if unsafe { libc::geteuid() } != 0 {
                eprintln!("✗ This command needs root access (to read /etc/wolfnet/).");
                eprintln!("  Run with: sudo wolfnet {}", std::env::args().skip(1).collect::<Vec<_>>().join(" "));
                std::process::exit(1);
            }
        }
        _ => {}
    }

    match cli.command {
        Some(Commands::Genkey { output }) => cmd_genkey(&output),
        Some(Commands::Pubkey) => cmd_pubkey(&cli.config),
        Some(Commands::Token) => cmd_token(&cli.config),
        Some(Commands::Init { address }) => cmd_init(&cli.config, &address),
        Some(Commands::Invite) => cmd_invite(&cli.config),
        Some(Commands::Join { token }) => cmd_join(&cli.config, &token),
        None => run_daemon(&cli.config),
    }
}

fn cmd_genkey(output: &PathBuf) {
    let kp = KeyPair::generate();
    match kp.save(output) {
        Ok(_) => {
            println!("Private key saved to: {:?}", output);
            println!("Public key: {}", kp.public_key_base64());
        }
        Err(e) => { error!("Failed to save key: {}", e); std::process::exit(1); }
    }
}

fn cmd_pubkey(config_path: &PathBuf) {
    let config = load_config(config_path);
    match KeyPair::load_or_generate(&config.security.private_key_file) {
        Ok(kp) => println!("{}", kp.public_key_base64()),
        Err(e) => { error!("Failed to load key: {}", e); std::process::exit(1); }
    }
}

fn cmd_token(config_path: &PathBuf) {
    let config = load_config(config_path);
    match KeyPair::load_or_generate(&config.security.private_key_file) {
        Ok(kp) => {
            let pubkey = kp.public_key_base64();
            let bind = format!("0.0.0.0:{}", config.network.listen_port);
            println!("{}@{}", pubkey, bind);
            println!("\nShare this token with peers. They can join with:");
            println!("  wolfnet join <token>");
        }
        Err(e) => { error!("{}", e); std::process::exit(1); }
    }
}

fn cmd_init(config_path: &PathBuf, address: &str) {
    let config = Config {
        network: wolfnet::config::NetworkConfig {
            address: address.to_string(),
            ..Config::default().network
        },
        ..Config::default()
    };
    match config.save(config_path) {
        Ok(_) => {
            println!("Config written to {:?}", config_path);
            println!("WolfNet IP: {}", address);
            // Also generate key
            let kp = KeyPair::generate();
            if let Err(e) = kp.save(&config.security.private_key_file) {
                warn!("Failed to save key: {}", e);
            } else {
                println!("Public key: {}", kp.public_key_base64());
            }
        }
        Err(e) => { error!("Failed to write config: {}", e); std::process::exit(1); }
    }
}

/// Resolve an endpoint string to a SocketAddr.
/// Supports both IP:port (e.g. "203.0.113.5:9600") and hostname:port (e.g. "myhome.dyndns.org:9600").
/// Rejects unroutable addresses (0.0.0.0, 127.x.x.x) that would loop back to self.
fn resolve_endpoint(ep: &str) -> Option<SocketAddr> {
    // Try direct parse first (fastest path for IP:port)
    if let Ok(addr) = ep.parse::<SocketAddr>() {
        if is_unusable_endpoint(&addr) {
            warn!("Skipping unusable endpoint '{}' (loopback/unspecified)", ep);
            return None;
        }
        return Some(addr);
    }
    // Fall back to DNS resolution (supports hostnames like myhome.dyndns.org:9600)
    match ep.to_socket_addrs() {
        Ok(mut addrs) => {
            let result = addrs.next();
            if let Some(ref addr) = result {
                if is_unusable_endpoint(addr) {
                    warn!("Skipping unusable endpoint '{}' (resolved to loopback/unspecified)", ep);
                    return None;
                }
            } else {
                warn!("DNS resolution for '{}' returned no addresses", ep);
            }
            result
        }
        Err(e) => {
            warn!("Failed to resolve endpoint '{}': {}", ep, e);
            None
        }
    }
}

/// Check if an endpoint is unusable (would send to self or nowhere)
fn is_unusable_endpoint(addr: &SocketAddr) -> bool {
    match addr {
        SocketAddr::V4(v4) => {
            let ip = v4.ip();
            ip.is_unspecified() || ip.is_loopback()
        }
        SocketAddr::V6(v6) => {
            let ip = v6.ip();
            ip.is_unspecified() || ip.is_loopback()
        }
    }
}

/// Auto-detect our public IP address
fn detect_public_ip() -> Option<String> {
    // Try multiple services in case one is down
    let services = [
        ("api.ipify.org", "GET / HTTP/1.1\r\nHost: api.ipify.org\r\nConnection: close\r\n\r\n"),
        ("ifconfig.me", "GET /ip HTTP/1.1\r\nHost: ifconfig.me\r\nConnection: close\r\n\r\n"),
    ];
    for (host, request) in &services {
        if let Ok(mut stream) = TcpStream::connect(format!("{}:80", host)) {
            stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
            if stream.write_all(request.as_bytes()).is_ok() {
                let mut response = String::new();
                let _ = stream.read_to_string(&mut response);
                // Parse HTTP response — IP is in the body after \r\n\r\n
                if let Some(body) = response.split("\r\n\r\n").nth(1) {
                    let ip = body.trim().to_string();
                    if ip.parse::<Ipv4Addr>().is_ok() {
                        return Some(ip);
                    }
                }
            }
        }
    }
    None
}

fn cmd_invite(config_path: &PathBuf) {
    let config = load_config(config_path);
    let kp = KeyPair::load_or_generate(&config.security.private_key_file).unwrap_or_else(|e| {
        error!("{}", e); std::process::exit(1);
    });

    // Auto-detect public IP
    let public_ip = detect_public_ip();
    let endpoint = match &public_ip {
        Some(ip) => format!("{}:{}", ip, config.network.listen_port),
        None => {
            eprintln!("⚠ Could not auto-detect public IP. Using local address.");
            eprintln!("  If this node is behind NAT, peers will need a relay node.");
            format!("{}:{}", config.network.address, config.network.listen_port)
        }
    };

    // Build invite token as JSON → base64
    let invite = serde_json::json!({
        "pk": kp.public_key_base64(),
        "ep": endpoint,
        "ip": config.network.address,
        "sn": config.network.subnet,
        "pt": config.network.listen_port,
    });
    let token = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        invite.to_string().as_bytes(),
    );

    println!();
    println!("  🐺 WolfNet Invite Token");
    println!("  ─────────────────────────────────────");
    println!("Your network: {}/{}", config.network.address, config.network.subnet);
    println!("Public endpoint: {}", endpoint);
    println!();
    println!("Share this token with the peer you want to invite:");
    println!();
    println!("  sudo wolfnet --config /etc/wolfnet/config.toml join {}", token);
    println!();
    println!("After they join, they'll get a reverse token for you to run.");
}

fn cmd_join(config_path: &PathBuf, token: &str) {
    use base64::Engine;
    use wolfnet::config::PeerConfig;

    // Decode token
    let decoded = base64::engine::general_purpose::STANDARD.decode(token.trim()).unwrap_or_else(|e| {
        error!("Invalid invite token: {}", e);
        std::process::exit(1);
    });
    let invite: serde_json::Value = serde_json::from_slice(&decoded).unwrap_or_else(|e| {
        error!("Invalid invite token format: {}", e);
        std::process::exit(1);
    });

    let peer_pubkey = invite["pk"].as_str().unwrap_or_else(|| {
        error!("Token missing public key"); std::process::exit(1);
    });
    let peer_endpoint = invite["ep"].as_str().unwrap_or_else(|| {
        error!("Token missing endpoint"); std::process::exit(1);
    });
    let peer_ip = invite["ip"].as_str().unwrap_or_else(|| {
        error!("Token missing IP"); std::process::exit(1);
    });
    let subnet = invite["sn"].as_u64().unwrap_or(24) as u8;

    // Load or create config
    let mut config = if config_path.exists() {
        load_config(config_path)
    } else {
        Config::default()
    };

    // Determine if this node already has a configured address
    // (i.e., not a fresh/default config). If so, preserve it.
    let default_addresses = ["10.0.10.1", "0.0.0.0"];
    let has_existing_address = config_path.exists()
        && !default_addresses.contains(&config.network.address.as_str());

    if has_existing_address {
        // Preserve existing address — this node is already part of a network

    } else {
        // Auto-assign next available IP in the subnet
        let peer_addr: Ipv4Addr = peer_ip.parse().unwrap_or_else(|_| {
            error!("Invalid peer IP in token: {}", peer_ip);
            std::process::exit(1);
        });
        let octets = peer_addr.octets();
        let mut my_last_octet = octets[3] + 1;

        // Check existing peers to avoid conflicts
        let used_ips: Vec<String> = config.peers.iter().map(|p| p.allowed_ip.clone()).collect();
        loop {
            let candidate = format!("{}.{}.{}.{}", octets[0], octets[1], octets[2], my_last_octet);
            if candidate != peer_ip && !used_ips.contains(&candidate) && candidate != config.network.address {
                config.network.address = candidate;
                break;
            }
            my_last_octet += 1;
            if my_last_octet > 254 {
                error!("No available IPs in the subnet");
                std::process::exit(1);
            }
        }
    }
    config.network.subnet = subnet;

    // Add the inviting peer (merge, don't replace)
    // Update existing peer if public key matches, otherwise add new
    let existing_idx = config.peers.iter().position(|p| p.public_key == peer_pubkey);
    match existing_idx {
        Some(idx) => {
            // Update endpoint but preserve name if already set
            let _existing_name = config.peers[idx].name.clone();
            config.peers[idx].endpoint = Some(peer_endpoint.to_string());
            config.peers[idx].allowed_ip = peer_ip.to_string();
            if config.peers[idx].name.is_none() {
                config.peers[idx].name = Some("invited-peer".to_string());
            }

        }
        None => {
            config.peers.push(PeerConfig {
                public_key: peer_pubkey.to_string(),
                endpoint: Some(peer_endpoint.to_string()),
                allowed_ip: peer_ip.to_string(),
                name: Some("invited-peer".to_string()),
            });
        }
    }

    // Generate or load our keypair
    let kp = KeyPair::load_or_generate(&config.security.private_key_file).unwrap_or_else(|e| {
        error!("Key error: {}", e);
        std::process::exit(1);
    });

    // Save config
    config.save(config_path).unwrap_or_else(|e| {
        error!("Failed to save config: {}", e);
        std::process::exit(1);
    });

    // Generate reverse invite for the other side
    let public_ip = detect_public_ip();
    let my_endpoint = match &public_ip {
        Some(ip) => format!("{}:{}", ip, config.network.listen_port),
        None => format!("{}:{}", config.network.address, config.network.listen_port),
    };
    let reverse = serde_json::json!({
        "pk": kp.public_key_base64(),
        "ep": my_endpoint,
        "ip": config.network.address,
        "sn": subnet,
        "pt": config.network.listen_port,
    });
    let reverse_token = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        reverse.to_string().as_bytes(),
    );

    println!();
    println!("  🐺 WolfNet — Joined!");
    println!("  ─────────────────────────────────────");
    println!();
    println!("✓ Config saved to {:?}", config_path);
    println!("✓ Your WolfNet IP: {}/{}", config.network.address, subnet);
    println!("✓ Peer added: {} ({})", peer_ip, peer_endpoint);
    println!();
    println!("Now run this on the inviting node to complete the link:");
    println!();
    println!("  sudo wolfnet --config /etc/wolfnet/config.toml join {}", reverse_token);
    println!();
    println!("Then restart WolfNet on both nodes:");
    println!("  sudo systemctl restart wolfnet");
}

fn load_config(path: &PathBuf) -> Config {
    if path.exists() {
        Config::load(path).unwrap_or_else(|e| {
            error!("Failed to load config: {}", e);
            std::process::exit(1);
        })
    } else {
        Config::default()
    }
}

fn run_daemon(config_path: &PathBuf) {
    let config = load_config(config_path);
    let wolfnet_ip: Ipv4Addr = config.ip_addr().unwrap_or_else(|e| {
        error!("Invalid address '{}': {}", config.network.address, e);
        std::process::exit(1);
    });

    info!("WolfNet starting — {} on {}", wolfnet_ip, config.network.interface);

    // Load or generate keypair
    let keypair = Arc::new(KeyPair::load_or_generate(&config.security.private_key_file).unwrap_or_else(|e| {
        error!("Key error: {}", e);
        std::process::exit(1);
    }));
    info!("Public key: {}", keypair.public_key_base64());

    // Create TUN device
    let tun = TunDevice::create(&config.network.interface).unwrap_or_else(|e| {
        error!("Failed to create TUN device: {}", e);
        error!("Are you running as root? (sudo wolfnet)");
        std::process::exit(1);
    });
    tun.configure(&config.network.address, config.network.subnet, config.network.mtu).unwrap_or_else(|e| {
        error!("Failed to configure TUN: {}", e);
        std::process::exit(1);
    });

    // Create UDP socket
    let bind_addr = format!("{}:{}", config.network.bind_address, config.network.listen_port);
    let socket = Arc::new(UdpSocket::bind(&bind_addr).unwrap_or_else(|e| {
        error!("Failed to bind UDP {}: {}", bind_addr, e);
        std::process::exit(1);
    }));
    socket.set_nonblocking(true).expect("Failed to set UDP socket non-blocking");
    info!("Listening on UDP {}", bind_addr);

    // Initialize peer manager and add configured peers
    let peer_manager = Arc::new(PeerManager::new());
    for pc in &config.peers {
        match wolfnet::crypto::parse_public_key(&pc.public_key) {
            Ok(pub_key) => {
                let ip: Ipv4Addr = match pc.allowed_ip.parse() {
                    Ok(ip) => ip,
                    Err(e) => { warn!("Invalid peer IP '{}': {}", pc.allowed_ip, e); continue; }
                };
                let mut peer = Peer::new(pub_key, ip);
                peer.hostname = pc.name.clone().unwrap_or_default();
                if let Some(ref ep) = pc.endpoint {
                    // Store original endpoint string for periodic re-resolution (DynDNS support)
                    peer.configured_endpoint = Some(ep.clone());
                    if let Some(addr) = resolve_endpoint(ep) {
                        peer.endpoint = Some(addr);
                    }
                }
                // Pre-establish session (we have the keys)
                peer.establish_session(&keypair.secret, &keypair.public);
                peer_manager.add_peer(peer);
            }
            Err(e) => warn!("Invalid peer public key: {}", e),
        }
    }

    // Load subnet routes (container/VM IPs → host peer IPs)
    let routes_path = PathBuf::from("/var/run/wolfnet/routes.json");
    peer_manager.load_routes(&routes_path);

    // Load CIDR-based subnet routes from WolfStack's WolfRouter config.
    // Without this, packets the kernel routes via wolfnet0 toward a
    // remote LAN (e.g. 10.10.0.0/16 advertised by peer 10.100.10.30)
    // can't be encapsulated to the right peer — TUN devices have no
    // link layer, so the kernel's "next-hop" hint is invisible to us.
    let subnet_routes_path = PathBuf::from("/var/run/wolfnet/subnet-routes.json");
    peer_manager.load_subnet_routes(&subnet_routes_path);

    // Gateway mode is only enabled explicitly in config — a gateway is a node
    // that bridges networks and relays traffic between peers that can't see each other
    let is_gateway = config.network.gateway;

    // Full gateway setup (iptables NAT rules) only when explicitly configured
    // Auto-gateway just enables IP forwarding for relay — no iptables changes
    if config.network.gateway {
        let subnet = config.cidr();
        if let Err(e) = wolfnet::gateway::enable_gateway(tun.name(), &subnet) {
            warn!("Gateway setup failed: {}", e);
        }
    }
    // Non-gateway nodes do NOT enable global ip_forward — WolfNet relay is
    // userspace (TUN → app → UDP) and doesn't need kernel forwarding.
    // Enabling it on dual-homed machines turns them into routers between
    // their physical interfaces, which can cripple the network.

    // Running flag for graceful shutdown
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc_handler(r);

    // Register SIGHUP for config hot-reload (checked in main loop)
    unsafe {
        libc::signal(libc::SIGHUP, handle_reload as *const () as libc::sighandler_t);
    }

    let hostname = hostname::get().map(|h| h.to_string_lossy().to_string()).unwrap_or_else(|_| "unknown".into());
    let start_time = Instant::now();

    // Spawn discovery threads
    if config.network.discovery {
        let r = running.clone();
        let pk = keypair.public;
        let h = hostname.clone();
        let gw = is_gateway;
        let lp = config.network.listen_port;
        std::thread::spawn(move || {
            transport::run_discovery_broadcaster(wolfnet_ip, pk, lp, h, gw, r);
        });

        let r = running.clone();
        let kp = keypair.clone();
        let pm = peer_manager.clone();
        let nid = hostname.clone();
        std::thread::spawn(move || {
            transport::run_discovery_listener(nid, kp, pm, r);
        });
    }

    // Spawn status writer thread
    {
        let r = running.clone();
        let pm = peer_manager.clone();
        let h = hostname.clone();
        let addr = config.network.address.clone();
        let pk = keypair.public_key_base64();
        let lp = config.network.listen_port;
        let gw = is_gateway;
        let iface = config.network.interface.clone();
        std::thread::spawn(move || {
            let status_path = PathBuf::from("/var/run/wolfnet/status.json");
            std::fs::create_dir_all("/var/run/wolfnet").ok();
            while r.load(Ordering::Relaxed) {
                let status = NodeStatus {
                    hostname: h.clone(),
                    address: addr.clone(),
                    public_key: pk.clone(),
                    listen_port: lp,
                    gateway: gw,
                    interface: iface.clone(),
                    uptime_secs: start_time.elapsed().as_secs(),
                    peers: pm.status(),
                };
                if let Ok(json) = serde_json::to_string_pretty(&status) {
                    let _ = std::fs::write(&status_path, json);
                }
                std::thread::sleep(Duration::from_secs(5));
            }
        });
    }

    // Main event loop — single-threaded, uses poll() on both TUN and UDP fds.
    // No separate TUN reader thread needed, no mpsc channel, no per-packet Vec allocation.
    info!("WolfNet running — {} ({}) on {}", hostname, wolfnet_ip, tun.name());
    let tun_fd = tun.raw_fd();
    let udp_fd = socket.as_raw_fd();
    let my_peer_id = keypair.my_peer_id();

    // Pre-allocated buffers — all packet processing uses these, zero heap allocations
    let mut tun_buf = [0u8; 65536];            // TUN reads (plaintext)
    let mut send_buf = [0u8; 65536 + 29];      // encrypt-in-place + 13-byte header + 16-byte tag
    let mut recv_buf = [0u8; 65536];            // UDP reads + decrypt-in-place

    let mut pfds = [
        libc::pollfd { fd: tun_fd, events: libc::POLLIN, revents: 0 },
        libc::pollfd { fd: udp_fd, events: libc::POLLIN, revents: 0 },
    ];
    // Pre-allocated broadcast buffers — reused every broadcast, zero per-packet heap allocs
    let mut broadcast_targets: Vec<(Ipv4Addr, Option<Ipv4Addr>)> = Vec::with_capacity(64);
    let mut broadcast_relayed: std::collections::HashSet<Ipv4Addr> = std::collections::HashSet::with_capacity(64);
    let mut last_handshake = Instant::now();
    let mut last_keepalive = Instant::now();
    let mut last_pex = Instant::now();
    let mut last_dns_resolve = Instant::now();
    let mut last_route_reload = Instant::now();

    while running.load(Ordering::Relaxed) {
        // Block until TUN or UDP has data (500ms timeout for periodic tasks)
        // Periodic tasks run at 10-60s intervals so 500ms is plenty responsive
        let poll_ret = unsafe { libc::poll(pfds.as_mut_ptr(), 2, 500) };

        if poll_ret > 0 {
        // If poll returned ready but neither fd has POLLIN, a persistent error
        // condition (POLLERR/POLLHUP) would cause poll to return immediately
        // every time — sleep to prevent busy-spinning
        if pfds[0].revents & libc::POLLIN == 0 && pfds[1].revents & libc::POLLIN == 0 {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        // ── 1. Outbound: TUN → encrypt → UDP ──────────────────────────────
        if pfds[0].revents & libc::POLLIN != 0 {
            loop {
                let n = unsafe { libc::read(tun_fd, tun_buf.as_mut_ptr() as *mut _, tun_buf.len()) };
                if n <= 0 { break; }
                let pkt = &tun_buf[..n as usize];

                if let Some(dest_ip) = tun::get_dest_ip(pkt) {
                    let subnet_broadcast = Ipv4Addr::new(
                        wolfnet_ip.octets()[0], wolfnet_ip.octets()[1],
                        wolfnet_ip.octets()[2], 255,
                    );

                    if dest_ip == subnet_broadcast || dest_ip == Ipv4Addr::BROADCAST {
                        // Broadcast: encrypt and send to all connected peers (single lock)
                        broadcast_relayed.clear();
                        broadcast_targets.clear();
                        peer_manager.for_each_peer_mut(|ip, peer| {
                            if ip == wolfnet_ip { return; }
                            if peer.is_connected() {
                                broadcast_targets.push((ip, None));
                            } else if let Some(relay) = peer.relay_via {
                                broadcast_targets.push((ip, Some(relay)));
                            }
                        });
                        for (ip, relay) in &broadcast_targets {
                            match relay {
                                None => {
                                    peer_manager.with_peer_by_ip(ip, |peer| {
                                        encrypt_and_send(&mut send_buf, pkt, peer, &my_peer_id, &socket);
                                    });
                                }
                                Some(relay_ip) => {
                                    if broadcast_relayed.insert(*relay_ip) {
                                        peer_manager.with_peer_by_ip(relay_ip, |relay_peer| {
                                            encrypt_and_send(&mut send_buf, pkt, relay_peer, &my_peer_id, &socket);
                                        });
                                    }
                                }
                            }
                        }
                        continue;
                    }

                    // Unicast: try direct → subnet route → relay → gateway
                    let sent = peer_manager.with_peer_by_ip(&dest_ip, |peer| {
                        encrypt_and_send(&mut send_buf, pkt, peer, &my_peer_id, &socket)
                    }).unwrap_or(false);
                    if sent { continue; }

                    if let Some(host_ip) = peer_manager.find_route(&dest_ip) {
                        let routed = peer_manager.with_peer_by_ip(&host_ip, |peer| {
                            encrypt_and_send(&mut send_buf, pkt, peer, &my_peer_id, &socket)
                        }).unwrap_or(false);
                        if routed { continue; }
                    }

                    // CIDR subnet route — longest-prefix-match against the
                    // CIDRs WolfStack advertised in subnet-routes.json. This
                    // is what lets a kernel route like
                    // `ip route add 10.10.0.0/16 via 10.100.10.30 dev wolfnet0`
                    // actually do something: the kernel's next-hop hint is
                    // invisible to a TUN, so we have to look the dest up
                    // ourselves and encapsulate to the configured gateway.
                    if let Some(gw_ip) = peer_manager.find_subnet_match(&dest_ip) {
                        let routed = peer_manager.with_peer_by_ip(&gw_ip, |peer| {
                            encrypt_and_send(&mut send_buf, pkt, peer, &my_peer_id, &socket)
                        }).unwrap_or(false);
                        if routed { continue; }
                    }

                    if let Some(relay_ip) = peer_manager.find_relay_for(&dest_ip) {
                        peer_manager.with_peer_by_ip(&relay_ip, |peer| {
                            encrypt_and_send(&mut send_buf, pkt, peer, &my_peer_id, &socket);
                        });
                        continue;
                    }

                    if let Some(gw_ip) = peer_manager.find_gateway() {
                        peer_manager.with_peer_by_ip(&gw_ip, |peer| {
                            encrypt_and_send(&mut send_buf, pkt, peer, &my_peer_id, &socket);
                        });
                    }
                } else if let Some(dest6) = tun::get_dest_ip6(pkt) {
                    // IPv6 subnet routing — the only routing path that applies
                    // to a v6 destination on the IPv4 overlay. Resolve the v6
                    // CIDR to its v4 gateway peer and encapsulate. An empty v6
                    // table (feature off) yields no match → the packet is
                    // dropped, exactly as v6 packets were before this support.
                    if let Some(gw_ip) = peer_manager.find_subnet_match_v6(&dest6) {
                        peer_manager.with_peer_by_ip(&gw_ip, |peer| {
                            encrypt_and_send(&mut send_buf, pkt, peer, &my_peer_id, &socket);
                        });
                    }
                }
            }
        }

        // ── 2. Inbound: UDP → decrypt → TUN ──────────────────────────────
        if pfds[1].revents & libc::POLLIN != 0 {
            loop {
                let (n, src) = match socket.recv_from(&mut recv_buf) {
                    Ok((n, src)) if n > 0 => (n, src),
                    _ => break,
                };

                match recv_buf[0] {
                    transport::PKT_HANDSHAKE => {
                        let data = &recv_buf[..n];
                        if let Some((pub_key, peer_ip, _peer_port, is_gw, peer_hostname)) = transport::parse_handshake(data) {
                            let endpoint = src;
                            peer_manager.update_from_discovery(&pub_key, endpoint, peer_ip, &peer_hostname, is_gw);
                            // Check if peer was already connected BEFORE re-establishing session
                            let was_connected = peer_manager.with_peer_by_ip(&peer_ip, |peer| {
                                peer.is_connected()
                            }).unwrap_or(false);
                            peer_manager.with_peer_by_ip(&peer_ip, |peer| {
                                peer.establish_session(&keypair.secret, &keypair.public);
                                peer.last_seen = Some(Instant::now());
                            });
                            // Only reply if peer was NOT already connected — prevents
                            // handshake ping-pong where two nodes bounce handshakes
                            // back and forth at wire speed, burning CPU
                            if !was_connected {
                                let reply = transport::build_handshake(&keypair, wolfnet_ip, config.network.listen_port, &hostname, is_gateway);
                                let _ = socket.send_to(&reply, src);
                            }
                        }
                    }
                    transport::PKT_DATA if n > 13 => {
                        let mut peer_id_bytes = [0u8; 4];
                        peer_id_bytes.copy_from_slice(&recv_buf[1..5]);
                        let counter = u64::from_le_bytes(recv_buf[5..13].try_into().unwrap());
                        let ct_len = n - 13;

                        let peer_ip = peer_manager.find_ip_by_endpoint_or_id(&src, &peer_id_bytes);

                        if let Some(peer_ip) = peer_ip {
                            // Decrypt in-place + check endpoint in single lock acquisition
                            let decrypt_result = peer_manager.with_peer_by_ip(&peer_ip, |peer| {
                                let result = peer.decrypt_into(counter, &mut recv_buf[13..13 + ct_len], ct_len);
                                let needs_ep_update = peer.endpoint != Some(src);
                                (result, needs_ep_update)
                            });

                            let (dec, needs_ep_update) = match decrypt_result {
                                Some((r, ep)) => (r, ep),
                                None => continue,
                            };

                            match dec {
                                Ok(pt_len) => {
                                    if needs_ep_update {
                                        peer_manager.update_endpoint_if_roaming(&peer_ip, src);
                                    }

                                    let plaintext = &recv_buf[13..13 + pt_len];

                                    // PEX message
                                    if pt_len > 1 && plaintext[0] == transport::PKT_PEER_EXCHANGE {
                                        if let Some(entries) = transport::parse_peer_exchange(plaintext) {
                                            peer_manager.add_from_pex(&entries, peer_ip, wolfnet_ip, &keypair);
                                        }
                                        continue;
                                    }

                                    // Relayed handshake — ignore
                                    if pt_len > 1 && plaintext[0] == transport::PKT_HANDSHAKE {
                                        continue;
                                    }

                                    // Route: for us, broadcast, or relay
                                    if let Some(dest_ip) = tun::get_dest_ip(plaintext) {
                                        let subnet_bcast = Ipv4Addr::new(
                                            wolfnet_ip.octets()[0], wolfnet_ip.octets()[1],
                                            wolfnet_ip.octets()[2], 255,
                                        );

                                        if dest_ip == wolfnet_ip || dest_ip == subnet_bcast || dest_ip == Ipv4Addr::BROADCAST {
                                            unsafe { libc::write(tun_fd, plaintext.as_ptr() as *const _, pt_len) };
                                        } else {
                                            // Relay: re-encrypt for destination peer
                                            let forwarded = peer_manager.with_peer_by_ip(&dest_ip, |dest_peer| {
                                                encrypt_and_send(&mut send_buf, plaintext, dest_peer, &my_peer_id, &socket)
                                            }).unwrap_or(false);

                                            if !forwarded {
                                                let mut handled = false;
                                                if let Some(host_ip) = peer_manager.find_route(&dest_ip) {
                                                    if host_ip == wolfnet_ip {
                                                        unsafe { libc::write(tun_fd, plaintext.as_ptr() as *const _, pt_len) };
                                                    } else {
                                                        peer_manager.with_peer_by_ip(&host_ip, |host_peer| {
                                                            encrypt_and_send(&mut send_buf, plaintext, host_peer, &my_peer_id, &socket);
                                                        });
                                                    }
                                                    handled = true;
                                                }
                                                if !handled {
                                                    // CIDR subnet route. If we're the configured
                                                    // gateway, write to TUN so the kernel +
                                                    // WolfStack's iptables plumbing forward the
                                                    // packet out the LAN-side iface. Otherwise
                                                    // re-encapsulate to whichever peer IS the
                                                    // gateway.
                                                    if let Some(gw_ip) = peer_manager.find_subnet_match(&dest_ip) {
                                                        if gw_ip == wolfnet_ip {
                                                            unsafe { libc::write(tun_fd, plaintext.as_ptr() as *const _, pt_len) };
                                                        } else {
                                                            peer_manager.with_peer_by_ip(&gw_ip, |gw_peer| {
                                                                encrypt_and_send(&mut send_buf, plaintext, gw_peer, &my_peer_id, &socket);
                                                            });
                                                        }
                                                    }
                                                }
                                                // else: drop — writing an unroutable packet back to TUN
                                                // creates an infinite routing loop (kernel routes it
                                                // back to wolfnet0 since dest is in our subnet)
                                            }
                                        }
                                    } else if let Some(dest6) = tun::get_dest_ip6(plaintext) {
                                        // IPv6 subnet routing on the decap/forward
                                        // path. If this node is the configured
                                        // gateway for the v6 CIDR, write to the TUN
                                        // (kernel + ip6tables forward it out the LAN
                                        // side); otherwise re-encapsulate to whichever
                                        // peer IS the gateway. Empty v6 table (feature
                                        // off) → no match → drop, exactly as before.
                                        if let Some(gw_ip) = peer_manager.find_subnet_match_v6(&dest6) {
                                            if gw_ip == wolfnet_ip {
                                                unsafe { libc::write(tun_fd, plaintext.as_ptr() as *const _, pt_len) };
                                            } else {
                                                peer_manager.with_peer_by_ip(&gw_ip, |gw_peer| {
                                                    encrypt_and_send(&mut send_buf, plaintext, gw_peer, &my_peer_id, &socket);
                                                });
                                            }
                                        }
                                    }
                                    // else: non-IP / unparseable packet — drop silently
                                }
                                Err(e) => {
                                    debug!("Decrypt failed from {} (counter={}): {}", peer_ip, counter, e);
                                }
                            }
                        }
                    }
                    transport::PKT_KEEPALIVE if n >= 5 => {
                        let mut peer_id = [0u8; 4];
                        peer_id.copy_from_slice(&recv_buf[1..5]);
                        let peer_ip = peer_manager.find_ip_by_endpoint_or_id(&src, &peer_id);

                        if let Some(ip) = peer_ip {
                            // Update endpoint if found by ID (NAT rebind).
                            // Roaming-gated: a peer with a configured
                            // static endpoint stays pinned to that even
                            // when keepalives arrive from a different src.
                            if peer_manager.find_ip_by_endpoint(&src).is_none() {
                                peer_manager.update_endpoint_if_roaming(&ip, src);
                            }
                            peer_manager.with_peer_by_ip(&ip, |peer| {
                                peer.last_seen = Some(Instant::now());
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        } // poll_ret > 0

        // 3. Periodic handshakes (every 10s)
        if last_handshake.elapsed() > Duration::from_secs(10) {
            transport::send_handshakes(&socket, &keypair, &peer_manager, wolfnet_ip, config.network.listen_port, &hostname, is_gateway);
            last_handshake = Instant::now();
        }

        // 4. Periodic keepalives (every 25s)
        if last_keepalive.elapsed() > Duration::from_secs(25) {
            transport::send_keepalives(&socket, &keypair, &peer_manager);
            last_keepalive = Instant::now();
        }

        // 5. Periodic peer exchange (every 30s)
        if last_pex.elapsed() > Duration::from_secs(30) {
            transport::send_peer_exchange(&socket, &keypair, &peer_manager, wolfnet_ip);
            last_pex = Instant::now();
        }

        // 6. Periodic DNS re-resolution for hostname-based endpoints (every 60s)
        //    This supports DynDNS — if a peer's hostname resolves to a new IP,
        //    we update the endpoint so handshakes reach them at the new address.
        if last_dns_resolve.elapsed() > Duration::from_secs(60) {
            for ip in peer_manager.all_ips() {
                let configured_ep = peer_manager.with_peer_by_ip(&ip, |peer| {
                    peer.configured_endpoint.clone()
                }).flatten();
                if let Some(ep_str) = configured_ep {
                    // Only re-resolve if it's a hostname (not a plain IP:port)
                    if ep_str.parse::<SocketAddr>().is_err() {
                        if let Some(new_addr) = resolve_endpoint(&ep_str) {
                            let current = peer_manager.with_peer_by_ip(&ip, |peer| peer.endpoint).flatten();
                            if current != Some(new_addr) {

                                peer_manager.update_endpoint(&ip, new_addr);
                            }
                        }
                    }
                }
            }
            last_dns_resolve = Instant::now();
        }

        // 6b. Periodic route file reload (every 15s) — picks up container routes
        //     from WolfStack without needing SIGHUP
        if last_route_reload.elapsed() > Duration::from_secs(15) {
            peer_manager.load_routes(&routes_path);
            peer_manager.load_subnet_routes(&subnet_routes_path);
            last_route_reload = Instant::now();
        }

        // 7. Config hot-reload on SIGHUP — add new peers without restarting
        if RELOAD_FLAG.swap(false, Ordering::SeqCst) {
            info!("SIGHUP received — reloading config...");
            match Config::load(config_path) {
                Ok(new_config) => {
                    let existing_ips = peer_manager.all_ips();
                    let mut added = 0;
                    let mut updated = 0;
                    for pc in &new_config.peers {
                        match wolfnet::crypto::parse_public_key(&pc.public_key) {
                            Ok(pub_key) => {
                                let ip: Ipv4Addr = match pc.allowed_ip.parse() {
                                    Ok(ip) => ip,
                                    Err(e) => { warn!("Reload: invalid peer IP '{}': {}", pc.allowed_ip, e); continue; }
                                };
                                if existing_ips.contains(&ip) {
                                    // Update existing peer's endpoint and hostname if changed.
                                    //
                                    // Three cases for `pc.endpoint`:
                                    //   * Some(ep) → set/refresh the in-memory endpoint.
                                    //   * None, but in-memory has one → operator (or
                                    //     WolfStack cluster-sync) intentionally removed
                                    //     the endpoint line so the peer runs roaming-only.
                                    //     We must WIPE the in-memory endpoint, otherwise
                                    //     the daemon keeps dialing the old address and
                                    //     every roaming-learned update gets clobbered.
                                    //   * None, in-memory also None → no-op.
                                    match pc.endpoint.as_ref() {
                                        Some(ep) => {
                                            if let Some(addr) = resolve_endpoint(ep) {
                                                let current_ep = peer_manager.with_peer_by_ip(&ip, |peer| peer.endpoint).flatten();
                                                if current_ep != Some(addr) {
                                                    info!("Reload: updated endpoint for {} -> {}", ip, addr);
                                                    peer_manager.update_endpoint(&ip, addr);
                                                    peer_manager.with_peer_by_ip(&ip, |peer| {
                                                        peer.configured_endpoint = Some(ep.clone());
                                                    });
                                                    updated += 1;
                                                }
                                            }
                                        }
                                        None => {
                                            if peer_manager.clear_endpoint(&ip) {
                                                info!("Reload: cleared endpoint for {} (roaming-only)", ip);
                                                updated += 1;
                                            }
                                        }
                                    }
                                    // Update hostname
                                    let new_name = pc.name.clone().unwrap_or_default();
                                    if !new_name.is_empty() {
                                        peer_manager.with_peer_by_ip(&ip, |peer| {
                                            if peer.hostname != new_name {
                                                peer.hostname = new_name.clone();
                                                updated += 1;
                                            }
                                        });
                                    }
                                } else {
                                    // New peer — add it
                                    let mut peer = Peer::new(pub_key, ip);
                                    peer.hostname = pc.name.clone().unwrap_or_default();
                                    if let Some(ref ep) = pc.endpoint {
                                        peer.configured_endpoint = Some(ep.clone());
                                        if let Some(addr) = resolve_endpoint(ep) {
                                            peer.endpoint = Some(addr);
                                        }
                                    }
                                    peer.establish_session(&keypair.secret, &keypair.public);
                                    peer_manager.add_peer(peer);
                                    added += 1;
                                }
                            }
                            Err(e) => warn!("Reload: invalid peer public key: {}", e),
                        }
                    }
                    // Purge peers NOT in the config (removes PEX ghosts)
                    let configured_ips: std::collections::HashSet<Ipv4Addr> = new_config.peers.iter()
                        .filter_map(|pc| pc.allowed_ip.parse().ok())
                        .collect();
                    let all_runtime_ips = peer_manager.all_ips();
                    let mut removed = 0;
                    for ip in &all_runtime_ips {
                        if !configured_ips.contains(ip) {
                            // Check if this peer is directly connected (LAN discovery)
                            let is_direct = peer_manager.with_peer_by_ip(ip, |p| {
                                p.relay_via.is_none() && p.is_connected()
                            }).unwrap_or(false);
                            if !is_direct {
                                peer_manager.purge_peer(ip);
                                removed += 1;
                            }
                        }
                    }
                    info!("Config reload complete: {} added, {} updated, {} purged", added, updated, removed);

                    // Also reload subnet routes
                    peer_manager.load_routes(&routes_path);
                    peer_manager.load_subnet_routes(&subnet_routes_path);
                }
                Err(e) => warn!("Config reload failed: {}", e),
            }
        }

    }

    // Cleanup
    info!("Shutting down WolfNet...");
    if config.network.gateway {
        wolfnet::gateway::disable_gateway(tun.name(), &config.cidr());
    }
    let _ = std::fs::remove_file("/var/run/wolfnet/status.json");
    info!("WolfNet stopped.");
}

fn ctrlc_handler(running: Arc<AtomicBool>) {
    let _ = ctrlc_signal(running);
}

fn ctrlc_signal(running: Arc<AtomicBool>) -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        libc::signal(libc::SIGINT, handle_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, handle_signal as *const () as libc::sighandler_t);
    }
    RUNNING.store(running);
    Ok(())
}

static RUNNING: RunningHolder = RunningHolder::new();

struct RunningHolder {
    inner: std::sync::OnceLock<Arc<AtomicBool>>,
}

impl RunningHolder {
    const fn new() -> Self { Self { inner: std::sync::OnceLock::new() } }
    fn store(&self, r: Arc<AtomicBool>) { let _ = self.inner.set(r); }
    fn signal(&self) {
        if let Some(r) = self.inner.get() { r.store(false, Ordering::SeqCst); }
    }
}

unsafe impl Sync for RunningHolder {}

extern "C" fn handle_signal(_sig: libc::c_int) {
    RUNNING.signal();
}

extern "C" fn handle_reload(_sig: libc::c_int) {
    // Safety: AtomicBool::store is signal-safe
    // We can't access the local RELOAD static from run_daemon directly,
    // so we use a global.
    RELOAD_FLAG.store(true, Ordering::SeqCst);
}

static RELOAD_FLAG: AtomicBool = AtomicBool::new(false);
