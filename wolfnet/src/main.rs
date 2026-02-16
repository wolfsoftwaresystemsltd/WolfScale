//! WolfNet daemon — secure private mesh networking
//!
//! Creates encrypted tunnels between machines using TUN interfaces,
//! X25519 key exchange, and ChaCha20-Poly1305 encryption.
//!
//! Supports automatic peer exchange (PEX) so joining one node
//! automatically gives you access to all its peers.

use std::net::{UdpSocket, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs};
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
fn resolve_endpoint(ep: &str) -> Option<SocketAddr> {
    // Try direct parse first (fastest path for IP:port)
    if let Ok(addr) = ep.parse::<SocketAddr>() {
        return Some(addr);
    }
    // Fall back to DNS resolution (supports hostnames like myhome.dyndns.org:9600)
    match ep.to_socket_addrs() {
        Ok(mut addrs) => {
            let result = addrs.next();
            if let Some(addr) = result {
                info!("Resolved endpoint '{}' -> {}", ep, addr);
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
        info!("Preserving existing WolfNet address: {}", config.network.address);
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
            let existing_name = config.peers[idx].name.clone();
            config.peers[idx].endpoint = Some(peer_endpoint.to_string());
            config.peers[idx].allowed_ip = peer_ip.to_string();
            if config.peers[idx].name.is_none() {
                config.peers[idx].name = Some("invited-peer".to_string());
            }
            info!("Updated existing peer: {} ({})", 
                existing_name.unwrap_or_else(|| "unnamed".into()), peer_ip);
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
    let bind_addr = format!("0.0.0.0:{}", config.network.listen_port);
    let socket = Arc::new(UdpSocket::bind(&bind_addr).unwrap_or_else(|e| {
        error!("Failed to bind UDP {}: {}", bind_addr, e);
        std::process::exit(1);
    }));
    socket.set_read_timeout(Some(Duration::from_millis(50))).ok();
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
    } else if !config.peers.is_empty() {
        // Not a gateway, but has configured peers — enable IP forwarding
        // so we can relay packets between LAN-discovered and remote peers
        if let Err(e) = std::fs::write("/proc/sys/net/ipv4/ip_forward", "1") {
            warn!("Failed to enable IP forwarding: {}", e);
        } else {
            info!("IP forwarding enabled for relay");
        }
    }

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
                std::thread::sleep(Duration::from_secs(1));
            }
        });
    }

    // Spawn TUN reader thread
    let (tun_tx, tun_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    {
        let r = running.clone();
        let tun_fd = tun.raw_fd();
        std::thread::spawn(move || {
            let mut buf = [0u8; 65536];
            while r.load(Ordering::Relaxed) {
                let n = unsafe { libc::read(tun_fd, buf.as_mut_ptr() as *mut _, buf.len()) };
                if n > 0 {
                    let _ = tun_tx.send(buf[..n as usize].to_vec());
                } else if n < 0 {
                    let err = std::io::Error::last_os_error();
                    if err.kind() != std::io::ErrorKind::WouldBlock {
                        debug!("TUN read error: {}", err);
                    }
                    std::thread::sleep(Duration::from_micros(100));
                } else {
                    std::thread::sleep(Duration::from_micros(100));
                }
            }
        });
    }

    // Main event loop
    info!("WolfNet running — {} ({}) on {}", hostname, wolfnet_ip, tun.name());
    let mut recv_buf = [0u8; 65536];
    let mut last_handshake = Instant::now();
    let mut last_keepalive = Instant::now();
    let mut last_pex = Instant::now();
    let mut last_dns_resolve = Instant::now();
    let mut last_route_reload = Instant::now();
    let tun_fd = tun.raw_fd();

    while running.load(Ordering::Relaxed) {
        // 1. Process packets from TUN (outbound: encrypt and send via UDP)
        while let Ok(packet) = tun_rx.try_recv() {
            if let Some(dest_ip) = tun::get_dest_ip(&packet) {
                // Handle subnet broadcast — send to ALL peers (direct + relayed)
                // This enables services like WolfDisk autodiscovery across the tunnel
                let subnet_broadcast = Ipv4Addr::new(
                    wolfnet_ip.octets()[0],
                    wolfnet_ip.octets()[1],
                    wolfnet_ip.octets()[2],
                    255,
                );
                if dest_ip == subnet_broadcast || dest_ip == Ipv4Addr::BROADCAST {
                    // Collect relay info first (to avoid holding locks while sending)
                    let mut relay_targets: Vec<(Ipv4Addr, Option<Ipv4Addr>)> = Vec::new();
                    for ip in peer_manager.all_ips() {
                        if ip == wolfnet_ip { continue; }
                        let info = peer_manager.with_peer_by_ip(&ip, |peer| {
                            (peer.is_connected(), peer.relay_via)
                        });
                        if let Some((connected, relay_via)) = info {
                            if connected {
                                relay_targets.push((ip, None)); // direct
                            } else if let Some(relay) = relay_via {
                                relay_targets.push((ip, Some(relay))); // via relay
                            }
                        }
                    }
                    // Send to each peer (directly or via relay)
                    let mut relayed_via: std::collections::HashSet<Ipv4Addr> = std::collections::HashSet::new();
                    for (ip, relay) in &relay_targets {
                        match relay {
                            None => {
                                // Direct send
                                peer_manager.with_peer_by_ip(ip, |peer| {
                                    if let Some(endpoint) = peer.endpoint {
                                        if let Ok((counter, ciphertext)) = peer.encrypt(&packet) {
                                            let pkt = transport::build_data_packet(&keypair.my_peer_id(), counter, &ciphertext);
                                            let _ = socket.send_to(&pkt, endpoint);
                                        }
                                    }
                                });
                            }
                            Some(relay_ip) => {
                                // Send via relay — but only once per relay peer
                                // The relay will re-broadcast to its connected peers
                                if relayed_via.insert(*relay_ip) {
                                    peer_manager.with_peer_by_ip(relay_ip, |relay_peer| {
                                        if relay_peer.is_connected() {
                                            if let Some(endpoint) = relay_peer.endpoint {
                                                if let Ok((counter, ciphertext)) = relay_peer.encrypt(&packet) {
                                                    let pkt = transport::build_data_packet(&keypair.my_peer_id(), counter, &ciphertext);
                                                    let _ = socket.send_to(&pkt, endpoint);
                                                    debug!("Broadcast relayed via {} for relay-only peers", relay_ip);
                                                }
                                            }
                                        }
                                    });
                                }
                            }
                        }
                    }
                    continue;
                }

                // Try direct peer first
                let sent = peer_manager.with_peer_by_ip(&dest_ip, |peer| {
                    if let Some(endpoint) = peer.endpoint {
                        if peer.is_connected() {
                            match peer.encrypt(&packet) {
                                Ok((counter, ciphertext)) => {
                                    let pkt = transport::build_data_packet(&keypair.my_peer_id(), counter, &ciphertext);
                                    if let Err(e) = socket.send_to(&pkt, endpoint) {
                                        debug!("UDP send error to {}: {}", endpoint, e);
                                    }
                                    return true;
                                }
                                Err(e) => { debug!("Encrypt error for {}: {}", dest_ip, e); }
                            }
                        }
                    }
                    false
                });

                if sent.unwrap_or(false) { continue; }

                // Check subnet routes (container/VM IPs routed via a host peer)
                if let Some(host_ip) = peer_manager.find_route(&dest_ip) {
                    let routed = peer_manager.with_peer_by_ip(&host_ip, |host_peer| {
                        if let Some(endpoint) = host_peer.endpoint {
                            if host_peer.is_connected() {
                                match host_peer.encrypt(&packet) {
                                    Ok((counter, ciphertext)) => {
                                        let pkt = transport::build_data_packet(&keypair.my_peer_id(), counter, &ciphertext);
                                        let _ = socket.send_to(&pkt, endpoint);
                                        debug!("Routed packet for container {} via host {}", dest_ip, host_ip);
                                        true
                                    }
                                    Err(e) => { debug!("Encrypt error for {} via {}: {}", dest_ip, host_ip, e); false }
                                }
                            } else { false }
                        } else { false }
                    });
                    if routed.unwrap_or(false) { continue; }
                }

                // Not directly connected — try relay via PEX-learned route
                let relay_ip = peer_manager.find_relay_for(&dest_ip);
                if let Some(relay_ip) = relay_ip {
                    peer_manager.with_peer_by_ip(&relay_ip, |relay_peer| {
                        if let Some(endpoint) = relay_peer.endpoint {
                            match relay_peer.encrypt(&packet) {
                                Ok((counter, ciphertext)) => {
                                    let pkt = transport::build_data_packet(&keypair.my_peer_id(), counter, &ciphertext);
                                    if let Err(e) = socket.send_to(&pkt, endpoint) {
                                        debug!("Relay send to {} via {} failed: {}", dest_ip, relay_ip, e);
                                    } else {
                                        debug!("Relayed packet to {} via {} ({})", dest_ip, relay_ip, endpoint);
                                    }
                                }
                                Err(e) => debug!("Relay encrypt error for {} via {}: {}", dest_ip, relay_ip, e),
                            }
                        } else {
                            debug!("Relay peer {} has no endpoint", relay_ip);
                        }
                    });
                    continue;
                }

                // Fall back to gateway routing
                if let Some(gw_ip) = peer_manager.find_gateway() {
                    peer_manager.with_peer_by_ip(&gw_ip, |gw_peer| {
                        if let Some(endpoint) = gw_peer.endpoint {
                            match gw_peer.encrypt(&packet) {
                                Ok((counter, ciphertext)) => {
                                    let pkt = transport::build_data_packet(&keypair.my_peer_id(), counter, &ciphertext);
                                    if let Err(e) = socket.send_to(&pkt, endpoint) {
                                        debug!("UDP send error to gateway {}: {}", endpoint, e);
                                    }
                                }
                                Err(e) => debug!("Encrypt error for gateway {}: {}", gw_ip, e),
                            }
                        }
                    });
                }

                // Last resort: broadcast to all connected peers (simple, always works)
                // The receiving peer that owns the container will deliver it;
                // others will drop it since it's not for them.
                for peer_ip in peer_manager.all_ips() {
                    if peer_ip == wolfnet_ip { continue; }
                    peer_manager.with_peer_by_ip(&peer_ip, |peer| {
                        if peer.is_connected() {
                            if let Some(endpoint) = peer.endpoint {
                                if let Ok((counter, ciphertext)) = peer.encrypt(&packet) {
                                    let pkt = transport::build_data_packet(&keypair.my_peer_id(), counter, &ciphertext);
                                    let _ = socket.send_to(&pkt, endpoint);
                                }
                            }
                        }
                    });
                }
                debug!("Broadcast packet for unknown dest {} to all peers", dest_ip);
            }
        }

        // 2. Process packets from UDP (inbound: decrypt and write to TUN)
        match socket.recv_from(&mut recv_buf) {
            Ok((n, src)) => {
                if n == 0 { continue; }
                let data = &recv_buf[..n];
                match data[0] {
                    transport::PKT_HANDSHAKE => {
                        if let Some((pub_key, peer_ip, _peer_port, is_gw, peer_hostname)) = transport::parse_handshake(data) {
                            // Use the actual UDP source address — NOT the advertised port.
                            // Over NAT, the source port differs from listen_port.
                            let endpoint = src;
                            peer_manager.update_from_discovery(&pub_key, endpoint, peer_ip, &peer_hostname, is_gw);
                            peer_manager.with_peer_by_ip(&peer_ip, |peer| {
                                // Always re-establish session on handshake.
                                // A handshake means the peer is (re)connecting — their send
                                // counter resets to 0, so we must reset our recv counter too.
                                // Without this, a peer restart causes all their new packets
                                // to be rejected as "replay" because the old recv_counter
                                // is higher than their new send counter.
                                peer.establish_session(&keypair.secret, &keypair.public);
                                peer.last_seen = Some(Instant::now());
                            });
                            // Send handshake back
                            let reply = transport::build_handshake(&keypair, wolfnet_ip, config.network.listen_port, &hostname, is_gateway);
                            let _ = socket.send_to(&reply, src);
                        }
                    }
                    transport::PKT_DATA => {
                        if let Some((peer_id_bytes, counter, ciphertext)) = transport::parse_data_packet(data) {
                            // Find peer by source address, or fall back to peer_id (endpoint roaming)
                            let peer_ip = peer_manager.find_ip_by_endpoint(&src)
                                .or_else(|| {
                                    // Peer's IP may have changed — try to find by peer_id
                                    peer_manager.find_ip_by_id(&peer_id_bytes)
                                });

                            if let Some(peer_ip) = peer_ip {
                                let decrypted = peer_manager.with_peer_by_ip(&peer_ip, |peer| {
                                    peer.decrypt(counter, ciphertext)
                                });
                                match decrypted {
                                    Some(Ok(plaintext)) => {
                                    // Update endpoint if it changed (roaming)
                                    let known_endpoint = peer_manager.with_peer_by_ip(&peer_ip, |peer| peer.endpoint);
                                    if known_endpoint != Some(Some(src)) {
                                        info!("Peer {} roamed to new endpoint: {}", peer_ip, src);
                                        peer_manager.update_endpoint(&peer_ip, src);
                                    }

                                    // Check if this is a PEX message
                                    if plaintext.len() > 1 && plaintext[0] == transport::PKT_PEER_EXCHANGE {
                                        if let Some(entries) = transport::parse_peer_exchange(&plaintext) {
                                            info!("Received PEX from {} with {} peers", peer_ip, entries.len());
                                            peer_manager.add_from_pex(&entries, peer_ip, wolfnet_ip, &keypair);

                                            // Enable IP forwarding if we have multiple peers (we're a relay)
                                            if peer_manager.count() >= 2 {
                                                let _ = std::fs::write("/proc/sys/net/ipv4/ip_forward", "1");
                                            }
                                        }
                                        continue;
                                    }

                                    // If a relayed handshake arrives inside an encrypted data packet,
                                    // just ignore it — handshakes should only be processed when they
                                    // arrive as raw UDP packets (handled in the PKT_HANDSHAKE case above).
                                    // Processing them here corrupts endpoint info and causes session storms.
                                    if plaintext.len() > 1 && plaintext[0] == transport::PKT_HANDSHAKE {
                                        debug!("Ignoring relayed handshake inside data packet from {}", peer_ip);
                                        continue;
                                    }


                                    // Check if this packet is for us or needs relaying
                                    if let Some(dest_ip) = tun::get_dest_ip(&plaintext) {
                                        // Compute subnet broadcast address
                                        let subnet_bcast = Ipv4Addr::new(
                                            wolfnet_ip.octets()[0],
                                            wolfnet_ip.octets()[1],
                                            wolfnet_ip.octets()[2],
                                            255,
                                        );

                                        if dest_ip == wolfnet_ip {
                                            // For us — write to TUN
                                            unsafe { libc::write(tun_fd, plaintext.as_ptr() as *const _, plaintext.len()) };
                                        } else if dest_ip == subnet_bcast || dest_ip == Ipv4Addr::BROADCAST {
                                            // Broadcast: write to our TUN AND relay to all other peers
                                            unsafe { libc::write(tun_fd, plaintext.as_ptr() as *const _, plaintext.len()) };
                                            for relay_target in peer_manager.all_ips() {
                                                if relay_target == wolfnet_ip || relay_target == peer_ip { continue; }
                                                peer_manager.with_peer_by_ip(&relay_target, |dest_peer| {
                                                    if dest_peer.is_connected() {
                                                        if let Some(endpoint) = dest_peer.endpoint {
                                                            if let Ok((ctr, ct)) = dest_peer.encrypt(&plaintext) {
                                                                let pkt = transport::build_data_packet(&keypair.my_peer_id(), ctr, &ct);
                                                                let _ = socket.send_to(&pkt, endpoint);
                                                            }
                                                        }
                                                    }
                                                });
                                            }
                                        } else {
                                            // Relay: re-encrypt and forward to the destination peer
                                            let forwarded = peer_manager.with_peer_by_ip(&dest_ip, |dest_peer| {
                                                if let Some(endpoint) = dest_peer.endpoint {
                                                    match dest_peer.encrypt(&plaintext) {
                                                        Ok((ctr, ct)) => {
                                                            let pkt = transport::build_data_packet(&keypair.my_peer_id(), ctr, &ct);
                                                            let _ = socket.send_to(&pkt, endpoint);
                                                            debug!("Relayed packet from {} to {} at {} ({} bytes)", peer_ip, dest_ip, endpoint, plaintext.len());
                                                            true
                                                        }
                                                        Err(e) => {
                                                            debug!("Relay forward {} -> {} encrypt failed: {}", peer_ip, dest_ip, e);
                                                            false
                                                        }
                                                    }
                                                } else {
                                                    debug!("Relay forward {} -> {} has no endpoint", peer_ip, dest_ip);
                                                    false
                                                }
                                            });
                                            if !forwarded.unwrap_or(false) {
                                                // Check subnet routes — if the container is on us, write to TUN
                                                // If it's on another peer, forward via that peer
                                                if let Some(host_ip) = peer_manager.find_route(&dest_ip) {
                                                    if host_ip == wolfnet_ip {
                                                        // Container is on this node — write to TUN for kernel routing to bridge
                                                        unsafe { libc::write(tun_fd, plaintext.as_ptr() as *const _, plaintext.len()) };
                                                    } else {
                                                        // Forward to the host peer
                                                        peer_manager.with_peer_by_ip(&host_ip, |host_peer| {
                                                            if let Some(endpoint) = host_peer.endpoint {
                                                                if let Ok((ctr, ct)) = host_peer.encrypt(&plaintext) {
                                                                    let pkt = transport::build_data_packet(&keypair.my_peer_id(), ctr, &ct);
                                                                    let _ = socket.send_to(&pkt, endpoint);
                                                                    debug!("Subnet-routed {} -> {} via host {}", dest_ip, endpoint, host_ip);
                                                                }
                                                            }
                                                        });
                                                    }
                                                } else {
                                                    // Destination unknown — write to TUN for kernel routing
                                                    unsafe { libc::write(tun_fd, plaintext.as_ptr() as *const _, plaintext.len()) };
                                                }
                                            }
                                        }
                                    } else {
                                        unsafe { libc::write(tun_fd, plaintext.as_ptr() as *const _, plaintext.len()) };
                                    }
                                    }
                                    Some(Err(e)) => {
                                        warn!("Decrypt failed from {} (counter={}): {}", peer_ip, counter, e);
                                    }
                                    None => {
                                        debug!("Peer {} not found during decrypt", src);
                                    }
                                }
                            } else {
                                debug!("Data from unknown endpoint: {}", src);
                            }
                        }
                    }
                    transport::PKT_KEEPALIVE => {
                        let mut peer_ip_opt = peer_manager.find_ip_by_endpoint(&src);
                        
                        // If not found by endpoint, try extracting Peer ID from body
                        // This handles cases where a peer's NAT mapping changed (rebind) or
                        // the gateway restarted and lost its ephemeral endpoint mapping.
                        if peer_ip_opt.is_none() && data.len() >= 5 {
                            let mut peer_id = [0u8; 4];
                            peer_id.copy_from_slice(&data[1..5]);
                            peer_ip_opt = peer_manager.find_ip_by_id(&peer_id);
                            
                            if let Some(ip) = peer_ip_opt {
                                info!("Peer {} recovered from unknown endpoint via keepalive: {}", ip, src);
                                peer_manager.update_endpoint(&ip, src);
                            }
                        }

                        if let Some(peer_ip) = peer_ip_opt {
                            peer_manager.with_peer_by_ip(&peer_ip, |peer| {
                                peer.last_seen = Some(Instant::now());
                            });
                        }
                    }
                    _ => debug!("Unknown packet type {} from {}", data[0], src),
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => debug!("UDP recv error: {}", e),
        }

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
                                info!("DNS re-resolve: {} endpoint changed to {}", ip, new_addr);
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
                                    // Update existing peer's endpoint and hostname if changed
                                    if let Some(ref ep) = pc.endpoint {
                                        if let Some(addr) = resolve_endpoint(ep) {
                                            let current_ep = peer_manager.with_peer_by_ip(&ip, |peer| peer.endpoint).flatten();
                                            if current_ep != Some(addr) {
                                                info!("Reload: updated endpoint for {} -> {}", ip, addr);
                                                peer_manager.update_endpoint(&ip, addr);
                                                // Also update configured_endpoint for DNS re-resolution
                                                peer_manager.with_peer_by_ip(&ip, |peer| {
                                                    peer.configured_endpoint = Some(ep.clone());
                                                });
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
                    info!("Config reload complete: {} new peer(s), {} updated", added, updated);

                    // Also reload subnet routes
                    peer_manager.load_routes(&routes_path);
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
