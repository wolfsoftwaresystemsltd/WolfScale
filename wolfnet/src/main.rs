//! WolfNet daemon — secure private mesh networking
//!
//! Creates encrypted tunnels between machines using TUN interfaces,
//! X25519 key exchange, and ChaCha20-Poly1305 encryption.
//!
//! Supports automatic peer exchange (PEX) so joining one node
//! automatically gives you access to all its peers.

use std::net::{UdpSocket, Ipv4Addr, SocketAddr, TcpStream};
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
                    if let Ok(addr) = ep.parse::<SocketAddr>() {
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

    // Enable gateway if configured
    if config.network.gateway {
        let subnet = config.cidr();
        if let Err(e) = wolfnet::gateway::enable_gateway(tun.name(), &subnet) {
            warn!("Gateway setup failed: {}", e);
        }
    }

    // Running flag for graceful shutdown
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc_handler(r);

    let hostname = hostname::get().map(|h| h.to_string_lossy().to_string()).unwrap_or_else(|_| "unknown".into());
    let start_time = Instant::now();

    // Spawn discovery threads
    if config.network.discovery {
        let r = running.clone();
        let pk = keypair.public;
        let h = hostname.clone();
        let gw = config.network.gateway;
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
        let gw = config.network.gateway;
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
    let tun_fd = tun.raw_fd();

    while running.load(Ordering::Relaxed) {
        // 1. Process packets from TUN (outbound: encrypt and send via UDP)
        while let Ok(packet) = tun_rx.try_recv() {
            if let Some(dest_ip) = tun::get_dest_ip(&packet) {
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

                // Not directly connected — try relay via PEX-learned route
                let relay_ip = peer_manager.find_relay_for(&dest_ip);
                if let Some(relay_ip) = relay_ip {
                    peer_manager.with_peer_by_ip(&relay_ip, |relay_peer| {
                        if let Some(endpoint) = relay_peer.endpoint {
                            match relay_peer.encrypt(&packet) {
                                Ok((counter, ciphertext)) => {
                                    let pkt = transport::build_data_packet(&keypair.my_peer_id(), counter, &ciphertext);
                                    if let Err(e) = socket.send_to(&pkt, endpoint) {
                                        debug!("UDP send error to relay {}: {}", endpoint, e);
                                    } else {
                                        debug!("Relayed packet to {} via {}", dest_ip, relay_ip);
                                    }
                                }
                                Err(e) => debug!("Encrypt error for relay {}: {}", relay_ip, e),
                            }
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
            }
        }

        // 2. Process packets from UDP (inbound: decrypt and write to TUN)
        match socket.recv_from(&mut recv_buf) {
            Ok((n, src)) => {
                if n == 0 { continue; }
                let data = &recv_buf[..n];
                match data[0] {
                    transport::PKT_HANDSHAKE => {
                        if let Some((pub_key, peer_ip, peer_port, is_gw, peer_hostname)) = transport::parse_handshake(data) {
                            info!("Handshake from {} ({}) at {}", peer_hostname, peer_ip, src);
                            let endpoint = SocketAddr::new(src.ip(), peer_port);
                            peer_manager.update_from_discovery(&pub_key, endpoint, peer_ip, &peer_hostname, is_gw);
                            peer_manager.with_peer_by_ip(&peer_ip, |peer| {
                                if peer.cipher.is_none() {
                                    peer.establish_session(&keypair.secret, &keypair.public);
                                }
                                peer.last_seen = Some(Instant::now());
                            });
                            // Send handshake back
                            let reply = transport::build_handshake(&keypair, wolfnet_ip, config.network.listen_port, &hostname, config.network.gateway);
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
                                if let Some(Ok(plaintext)) = decrypted {
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

                                    // Check if this packet is for us or needs relaying
                                    if let Some(dest_ip) = tun::get_dest_ip(&plaintext) {
                                        if dest_ip == wolfnet_ip {
                                            // For us — write to TUN
                                            unsafe { libc::write(tun_fd, plaintext.as_ptr() as *const _, plaintext.len()) };
                                        } else {
                                            // Relay: re-encrypt and forward to the destination peer
                                            let forwarded = peer_manager.with_peer_by_ip(&dest_ip, |dest_peer| {
                                                if let Some(endpoint) = dest_peer.endpoint {
                                                    match dest_peer.encrypt(&plaintext) {
                                                        Ok((ctr, ct)) => {
                                                            let pkt = transport::build_data_packet(&keypair.my_peer_id(), ctr, &ct);
                                                            let _ = socket.send_to(&pkt, endpoint);
                                                            true
                                                        }
                                                        Err(_) => false,
                                                    }
                                                } else {
                                                    false
                                                }
                                            });
                                            if forwarded.unwrap_or(false) {
                                                debug!("Relayed packet from {} to {}", peer_ip, dest_ip);
                                            } else {
                                                // Destination unknown or unreachable, write to TUN for kernel routing
                                                unsafe { libc::write(tun_fd, plaintext.as_ptr() as *const _, plaintext.len()) };
                                            }
                                        }
                                    } else {
                                        unsafe { libc::write(tun_fd, plaintext.as_ptr() as *const _, plaintext.len()) };
                                    }
                                }
                            } else {
                                debug!("Data from unknown endpoint: {}", src);
                            }
                        }
                    }
                    transport::PKT_KEEPALIVE => {
                        if let Some(peer_ip) = peer_manager.find_ip_by_endpoint(&src) {
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
            transport::send_handshakes(&socket, &keypair, &peer_manager, wolfnet_ip, config.network.listen_port, &hostname, config.network.gateway);
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
        libc::signal(libc::SIGINT, handle_signal as libc::sighandler_t);
        libc::signal(libc::SIGTERM, handle_signal as libc::sighandler_t);
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
