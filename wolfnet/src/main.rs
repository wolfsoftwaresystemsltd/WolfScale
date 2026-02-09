//! WolfNet daemon — secure private mesh networking
//!
//! Creates encrypted tunnels between machines using TUN interfaces,
//! X25519 key exchange, and ChaCha20-Poly1305 encryption.

use std::net::{UdpSocket, Ipv4Addr, SocketAddr};
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
}

fn main() {
    let cli = Cli::parse();

    let filter = if cli.debug { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .init();

    match cli.command {
        Some(Commands::Genkey { output }) => cmd_genkey(&output),
        Some(Commands::Pubkey) => cmd_pubkey(&cli.config),
        Some(Commands::Token) => cmd_token(&cli.config),
        Some(Commands::Init { address }) => cmd_init(&cli.config, &address),
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
    let tun_fd = tun.raw_fd();

    while running.load(Ordering::Relaxed) {
        // 1. Process packets from TUN (outbound: encrypt and send via UDP)
        while let Ok(packet) = tun_rx.try_recv() {
            if let Some(dest_ip) = tun::get_dest_ip(&packet) {
                peer_manager.with_peer_by_ip(&dest_ip, |peer| {
                    if let Some(endpoint) = peer.endpoint {
                        match peer.encrypt(&packet) {
                            Ok((counter, ciphertext)) => {
                                let pkt = transport::build_data_packet(&keypair.my_peer_id(), counter, &ciphertext);
                                if let Err(e) = socket.send_to(&pkt, endpoint) {
                                    debug!("UDP send error to {}: {}", endpoint, e);
                                }
                            }
                            Err(e) => debug!("Encrypt error for {}: {}", dest_ip, e),
                        }
                    }
                });
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
                        if let Some((_peer_id, counter, ciphertext)) = transport::parse_data_packet(data) {
                            // Find peer by source address
                            if let Some(peer_ip) = peer_manager.find_ip_by_endpoint(&src) {
                                let decrypted = peer_manager.with_peer_by_ip(&peer_ip, |peer| {
                                    peer.decrypt(counter, ciphertext)
                                });
                                if let Some(Ok(plaintext)) = decrypted {
                                    unsafe { libc::write(tun_fd, plaintext.as_ptr() as *const _, plaintext.len()) };
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
