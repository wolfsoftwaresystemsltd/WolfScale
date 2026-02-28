<?php
$page_title = '🌐 WolfNet — WolfStack Docs';
$page_desc = 'WolfNet — Encrypted mesh networking for Linux. Connect servers across data centres, cloud, and home with X25519 + ChaCha20-Poly1305 encryption, automatic peer discovery, relay forwarding, and zero-config invite tokens.';
$active = 'wolfnet.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">

        <!-- ===== Overview ===== -->
        <div class="content-section">
            <h2>Overview</h2>
            <p>WolfNet creates a secure private network across the internet. Connect servers across data
                centres, cloud providers, and on-premises infrastructure as if they were on the same local
                network. Every packet is encrypted end-to-end with WireGuard-class cryptography.</p>
            <p>WolfNet works seamlessly whether your peers are on the same LAN, behind NAT, or spread
                across continents. LAN peers are discovered automatically, remote peers connect via
                public IP or DynDNS hostname, and peers behind restrictive firewalls reach each other
                through relay forwarding &mdash; all without any manual port forwarding.</p>

            <h3>Key Features</h3>
            <ul>
                <li><strong>Encrypted mesh networking</strong> &mdash; X25519 + ChaCha20-Poly1305 (WireGuard-class)</li>
                <li><strong>Invite/Join</strong> &mdash; Connect peers with a single token, no manual config</li>
                <li><strong>LAN auto-discovery</strong> &mdash; Peers on the same network find each other automatically</li>
                <li><strong>DynDNS support</strong> &mdash; Remote access via dynamic DNS hostnames, re-resolved every 60 seconds</li>
                <li><strong>Relay forwarding</strong> &mdash; No port forwarding needed &mdash; peers relay traffic automatically</li>
                <li><strong>Gateway mode</strong> &mdash; Route internet traffic through a WolfNet peer with NAT</li>
                <li><strong>Peer exchange (PEX)</strong> &mdash; Nodes share their peer lists so the mesh self-heals</li>
                <li><strong>Container integration</strong> &mdash; Docker and LXC containers get their own WolfNet addresses</li>
                <li><strong>TUN-based</strong> &mdash; Layer 3 networking with automatic route management</li>
                <li><strong>Built-in VPN</strong> &mdash; Access your entire infrastructure from anywhere</li>
                <li><strong>IBM Power ready</strong> &mdash; Pure Rust, builds natively on ppc64le</li>
            </ul>
        </div>

        <!-- ===== How It Works ===== -->
        <div class="content-section">
            <h2>How It Works</h2>
            <p>WolfNet creates a TUN virtual network interface (<code>wolfnet0</code>) on each node and assigns it
                an IP address on the <code>10.10.10.0/24</code> private subnet. All traffic between peers is
                encrypted end-to-end using modern cryptography.</p>

            <div class="code-block">
                <div class="code-header"><span class="code-lang">text</span></div>
                <pre><code>┌─────────────────────────────────────────────────────────────────────┐
│                         Your Applications                          │
│               (SSH, HTTP, databases, WolfStack UI)                 │
├─────────────────────────────────────────────────────────────────────┤
│                    wolfnet0  (10.10.10.x/24)                       │
│                       TUN virtual interface                        │
├─────────────────────────────────────────────────────────────────────┤
│                        WolfNet Daemon                              │
│  ┌──────────────┐  ┌────────────────┐  ┌────────────────────────┐  │
│  │  Peer Manager │  │   Encryption   │  │   Routing Engine      │  │
│  │  (discovery,  │  │   X25519 +     │  │   (direct, relay,     │  │
│  │   handshake,  │  │   ChaCha20-    │  │    gateway, PEX,      │  │
│  │   keepalive)  │  │   Poly1305     │  │    container routes)  │  │
│  └──────────────┘  └────────────────┘  └────────────────────────┘  │
├─────────────────────────────────────────────────────────────────────┤
│              UDP Transport  (port 9600)                             │
│    LAN broadcast discovery (port 9601)  •  Peer exchange (PEX)     │
└─────────────────────────────────────────────────────────────────────┘</code></pre>
            </div>

            <h3>Encryption</h3>
            <p>Every WolfNet node generates an <strong>X25519 keypair</strong> on first run. When two peers connect,
                they perform a Diffie-Hellman key exchange to derive a shared secret used as the
                <strong>ChaCha20-Poly1305</strong> encryption key. This is the same cryptographic foundation used
                by WireGuard and TLS 1.3.</p>
            <ul>
                <li><strong>Key exchange</strong> &mdash; X25519 (Curve25519 Diffie-Hellman)</li>
                <li><strong>Encryption</strong> &mdash; ChaCha20-Poly1305 AEAD (authenticated encryption)</li>
                <li><strong>Peer identity</strong> &mdash; SHA-256 derived 4-byte peer IDs for packet routing</li>
                <li><strong>Replay protection</strong> &mdash; Monotonic nonce counters with direction flags</li>
                <li><strong>Key storage</strong> &mdash; Private key at <code>/etc/wolfnet/private.key</code> (mode 0600)</li>
            </ul>

            <h3>Packet Flow</h3>
            <ol>
                <li>Application sends traffic to a <code>10.10.10.x</code> address</li>
                <li>The kernel routes it through the <code>wolfnet0</code> TUN interface</li>
                <li>WolfNet reads the packet, looks up the destination peer</li>
                <li>The packet is encrypted with the peer&rsquo;s shared ChaCha20-Poly1305 key</li>
                <li>The encrypted packet is sent as a UDP datagram to the peer&rsquo;s real endpoint</li>
                <li>The receiving peer decrypts and writes the packet to its local TUN interface</li>
                <li>The kernel delivers the packet to the destination application</li>
            </ol>
        </div>

        <!-- ===== Installation ===== -->
        <div class="content-section">
            <h2>Installation</h2>
            <p>WolfNet is included with WolfStack &mdash; if you have WolfStack installed, you already have WolfNet.
                For standalone installation on machines without WolfStack:</p>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code>curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/master/wolfnet/install.sh | sudo bash</code></pre>
            </div>
            <p>The installer compiles WolfNet from source using Rust, creates the systemd service, and generates
                a default configuration.</p>

            <h3>Manual Build</h3>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code>git clone https://github.com/wolfsoftwaresystemsltd/WolfScale.git
cd WolfScale/wolfnet
cargo build --release
sudo cp target/release/wolfnet /usr/local/bin/
sudo cp target/release/wolfnetctl /usr/local/bin/</code></pre>
            </div>

            <h3>Requirements</h3>
            <ul>
                <li>Linux with TUN device support (<code>/dev/net/tun</code>)</li>
                <li>Root privileges (creates network interfaces)</li>
                <li>UDP port <strong>9600</strong> open for tunnel traffic</li>
                <li>UDP port <strong>9601</strong> open for LAN discovery (optional)</li>
            </ul>
        </div>

        <!-- ===== Quick Start ===== -->
        <div class="content-section">
            <h2>Quick Start</h2>

            <h3>1. Initialize the first node</h3>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code>sudo wolfnet init --address 10.10.10.1</code></pre>
            </div>
            <p>This creates <code>/etc/wolfnet/config.toml</code> and generates an X25519 keypair.</p>

            <h3>2. Start WolfNet</h3>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code>sudo systemctl start wolfnet
sudo systemctl enable wolfnet</code></pre>
            </div>

            <h3>3. Generate an invite token</h3>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code>sudo wolfnet --config /etc/wolfnet/config.toml invite</code></pre>
            </div>
            <p>This outputs a token containing your public key, endpoint, and network details. Share it
                with the machine you want to connect.</p>

            <h3>4. Join from a second node</h3>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code># Install WolfNet on the second machine, then:
sudo wolfnet --config /etc/wolfnet/config.toml join &lt;token&gt;</code></pre>
            </div>
            <p>The joining node is automatically assigned the next available IP address (e.g. <code>10.10.10.2</code>)
                and added as a peer. It also prints a <strong>reverse token</strong> &mdash; run that command on the
                first node so both sides know about each other.</p>

            <h3>5. Restart both nodes</h3>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code>sudo systemctl restart wolfnet</code></pre>
            </div>
            <p>Both peers are now connected with encrypted tunnels. Test with:</p>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code>ping 10.10.10.2          # From node 1
wolfnetctl peers         # View connected peers
wolfnetctl status        # View this node's status</code></pre>
            </div>

            <div class="info-box">
                <p>&#x1F4A1; <strong>WolfStack users:</strong> You can also generate invite tokens and add peers from the
                    WolfStack dashboard under <strong>Networking &rarr; WolfNet</strong>, or use the cluster settings
                    page to automatically sync WolfNet connections across all cluster nodes with one click.</p>
            </div>
        </div>

        <!-- ===== Configuration ===== -->
        <div class="content-section">
            <h2>Configuration</h2>
            <p>WolfNet is configured via <code>/etc/wolfnet/config.toml</code>. The <code>wolfnet init</code>
                command or WolfStack installer creates this file for you.</p>

            <div class="code-block">
                <div class="code-header"><span class="code-lang">toml</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code># WolfNet Configuration

[network]
interface = "wolfnet0"        # TUN interface name
address = "10.10.10.1"        # This node's WolfNet IP address
subnet = 24                   # Subnet mask (CIDR)
listen_port = 9600            # UDP port for tunnel traffic
gateway = false               # Enable gateway/NAT mode
discovery = true              # Enable LAN auto-discovery broadcasts
mtu = 1400                    # TUN interface MTU

[security]
private_key_file = "/etc/wolfnet/private.key"

# Add peers — one [[peers]] block per remote node
[[peers]]
public_key = "base64_encoded_x25519_public_key"
endpoint = "203.0.113.5:9600"        # IP:port or hostname:port
allowed_ip = "10.10.10.2"            # Peer's WolfNet address
name = "server-2"                     # Friendly name (optional)

[[peers]]
public_key = "another_base64_key"
endpoint = "home.example.com:9600"   # DynDNS hostname supported
allowed_ip = "10.10.10.3"
name = "home-server"</code></pre>
            </div>

            <h3>Configuration Reference</h3>
            <table>
                <thead>
                    <tr>
                        <th>Section</th>
                        <th>Key</th>
                        <th>Default</th>
                        <th>Description</th>
                    </tr>
                </thead>
                <tbody>
                    <tr>
                        <td><code>[network]</code></td>
                        <td><code>interface</code></td>
                        <td><code>wolfnet0</code></td>
                        <td>Name of the TUN interface created by WolfNet</td>
                    </tr>
                    <tr>
                        <td></td>
                        <td><code>address</code></td>
                        <td>&mdash;</td>
                        <td>This node&rsquo;s WolfNet IP address (e.g. <code>10.10.10.1</code>)</td>
                    </tr>
                    <tr>
                        <td></td>
                        <td><code>subnet</code></td>
                        <td><code>24</code></td>
                        <td>Subnet mask in CIDR notation</td>
                    </tr>
                    <tr>
                        <td></td>
                        <td><code>listen_port</code></td>
                        <td><code>9600</code></td>
                        <td>UDP port for encrypted tunnel traffic</td>
                    </tr>
                    <tr>
                        <td></td>
                        <td><code>gateway</code></td>
                        <td><code>false</code></td>
                        <td>Enable gateway mode (NAT for internet access)</td>
                    </tr>
                    <tr>
                        <td></td>
                        <td><code>discovery</code></td>
                        <td><code>true</code></td>
                        <td>Enable UDP broadcast for LAN auto-discovery</td>
                    </tr>
                    <tr>
                        <td></td>
                        <td><code>mtu</code></td>
                        <td><code>1400</code></td>
                        <td>Maximum transmission unit for the TUN interface</td>
                    </tr>
                    <tr>
                        <td><code>[security]</code></td>
                        <td><code>private_key_file</code></td>
                        <td><code>/etc/wolfnet/private.key</code></td>
                        <td>Path to the X25519 private key file</td>
                    </tr>
                    <tr>
                        <td><code>[[peers]]</code></td>
                        <td><code>public_key</code></td>
                        <td>&mdash;</td>
                        <td>Peer&rsquo;s base64-encoded X25519 public key (required)</td>
                    </tr>
                    <tr>
                        <td></td>
                        <td><code>endpoint</code></td>
                        <td>&mdash;</td>
                        <td>Peer&rsquo;s IP:port or hostname:port (optional if using discovery)</td>
                    </tr>
                    <tr>
                        <td></td>
                        <td><code>allowed_ip</code></td>
                        <td>&mdash;</td>
                        <td>Peer&rsquo;s WolfNet IP address (required)</td>
                    </tr>
                    <tr>
                        <td></td>
                        <td><code>name</code></td>
                        <td>&mdash;</td>
                        <td>Friendly name for identification (optional)</td>
                    </tr>
                </tbody>
            </table>
        </div>

        <!-- ===== Peer Discovery ===== -->
        <div class="content-section">
            <h2>Peer Discovery</h2>
            <p>WolfNet uses multiple methods to find and connect to peers, allowing it to work across any
                network topology.</p>

            <h3>LAN Auto-Discovery</h3>
            <p>When <code>discovery = true</code> (the default), WolfNet broadcasts a UDP announcement every
                2 seconds on port <strong>9601</strong>. Any other WolfNet node on the same LAN segment will
                automatically detect it and establish a connection &mdash; no configuration needed.</p>
            <p>This means if you install WolfNet on two servers on the same network and run
                <code>wolfnet init</code> on each, they will find each other automatically.</p>

            <h3>Static Endpoints</h3>
            <p>For peers on different networks, specify their public IP address or hostname in the
                <code>endpoint</code> field:</p>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">toml</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code>[[peers]]
public_key = "abc123..."
endpoint = "203.0.113.5:9600"    # Static public IP
allowed_ip = "10.10.10.2"
name = "cloud-server"</code></pre>
            </div>

            <h3>DynDNS Hostnames</h3>
            <p>If a peer has a dynamic IP address (e.g. a home server), use a DynDNS hostname instead. WolfNet
                <strong>re-resolves hostnames every 60 seconds</strong>, so if the IP changes, the connection
                recovers automatically:</p>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">toml</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code>[[peers]]
public_key = "xyz789..."
endpoint = "myserver.duckdns.org:9600"    # DynDNS hostname
allowed_ip = "10.10.10.3"
name = "home-lab"</code></pre>
            </div>
            <div class="info-box">
                <p>&#x1F4A1; <strong>Tip:</strong> Free DynDNS services like <a href="https://www.duckdns.org"
                    target="_blank">DuckDNS</a> or <a href="https://www.noip.com" target="_blank">No-IP</a> work
                    perfectly with WolfNet. Set up the DynDNS client on your router or server, then use the hostname
                    in your WolfNet config.</p>
            </div>

            <h3>Peer Exchange (PEX)</h3>
            <p>Every 30 seconds, each WolfNet node shares its full peer list with all connected peers. This
                allows the mesh to self-heal &mdash; if node A knows node B and node C, but B and C don&rsquo;t
                know each other, A will introduce them via PEX. If direct connection isn&rsquo;t possible,
                traffic is relayed through A.</p>

            <h3>Endpoint Roaming</h3>
            <p>WolfNet identifies peers by their cryptographic key, not by IP address. If a peer&rsquo;s public IP
                changes (e.g. after a NAT rebind or ISP address change), WolfNet updates the endpoint mapping
                automatically when the next packet arrives. Combined with DynDNS re-resolution, this makes
                WolfNet resilient to IP address changes.</p>
        </div>

        <!-- ===== Remote Access Setup ===== -->
        <div class="content-section">
            <h2>Remote Access Setup</h2>
            <p>One of WolfNet&rsquo;s most powerful features is that the <strong>same configuration works for both
                local and remote access</strong>. A peer on the LAN connects via auto-discovery, and the same peer
                connecting from a coffee shop reaches your network via the configured endpoint &mdash; no config
                changes needed.</p>

            <h3>Scenario: Office + Home Server</h3>
            <p>You have a server at the office with a static IP and a server at home with a dynamic IP. You want
                both to be on the same WolfNet and accessible from anywhere.</p>

            <h4>Office Server (static IP: 203.0.113.5)</h4>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">toml</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code>[network]
address = "10.10.10.1"
listen_port = 9600
discovery = true         # Also discover LAN peers

[[peers]]
public_key = "home_server_public_key_base64"
endpoint = "home.duckdns.org:9600"
allowed_ip = "10.10.10.2"
name = "home-server"</code></pre>
            </div>

            <h4>Home Server (dynamic IP, DuckDNS: home.duckdns.org)</h4>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">toml</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code>[network]
address = "10.10.10.2"
listen_port = 9600
discovery = true

[[peers]]
public_key = "office_server_public_key_base64"
endpoint = "203.0.113.5:9600"
allowed_ip = "10.10.10.1"
name = "office-server"</code></pre>
            </div>

            <h4>Router Configuration</h4>
            <p>For the home server behind NAT, forward UDP port <strong>9600</strong> to the home server&rsquo;s
                local IP in your router settings. Alternatively, if you can&rsquo;t forward ports, the home server
                can still connect &mdash; the office server just needs to be reachable, and traffic will flow
                through the established connection.</p>

            <div class="info-box">
                <p>&#x1F4A1; <strong>No port forwarding?</strong> If neither side can open ports, use a third node
                    with a public IP as a relay. Traffic between the two will be automatically relayed through the
                    third node via peer exchange. See <a href="#relay-forwarding">Relay Forwarding</a> below.</p>
            </div>

            <h3>Scenario: Laptop VPN Access</h3>
            <p>Install WolfNet on your laptop and join the network. Whether you&rsquo;re in the office (LAN
                discovery finds peers automatically) or working remotely (connects via the configured endpoint),
                you have the same access to all <code>10.10.10.x</code> resources.</p>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code># On any WolfStack node, generate an invite:
sudo wolfnet --config /etc/wolfnet/config.toml invite

# On your laptop:
sudo wolfnet --config /etc/wolfnet/config.toml join &lt;token&gt;
sudo systemctl restart wolfnet

# Now access everything:
ssh admin@10.10.10.1                     # SSH to a server
firefox https://10.10.10.1:9443          # WolfStack dashboard
mysql -h 10.10.10.2 -u root mydb        # Database on another node
curl http://10.10.10.100:8080            # App in a container</code></pre>
            </div>

            <h3>Scenario: Multi-Site Cluster</h3>
            <p>Connect servers across multiple data centres and cloud providers. Each site discovers local
                peers automatically, and cross-site connections use static endpoints or DynDNS:</p>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">text</span></div>
                <pre><code>Site A (London)                     Site B (Frankfurt)
┌─────────────────────┐           ┌──────────────────────┐
│ 10.10.10.1 (node-a) │◄────────►│ 10.10.10.3 (node-c)  │
│ 10.10.10.2 (node-b) │  WolfNet │ 10.10.10.4 (node-d)  │
│   LAN discovery ◄─► │  tunnel  │  ◄─► LAN discovery   │
└─────────────────────┘           └──────────────────────┘
          ▲                                   ▲
          │           WolfNet tunnel           │
          └───────────────────────────────────┘
                     ▲
                     │ VPN access
              ┌──────────────┐
              │   Laptop     │
              │ 10.10.10.5   │
              └──────────────┘</code></pre>
            </div>
            <p>Only one peer at each site needs a cross-site endpoint configured. Other peers at the same site
                are discovered via LAN and reached via relay or PEX.</p>
        </div>

        <!-- ===== Invite/Join System ===== -->
        <div class="content-section">
            <h2>Invite/Join System</h2>
            <p>The invite/join system is the easiest way to connect peers. An invite token encodes everything
                the joining node needs: the inviter&rsquo;s public key, endpoint, WolfNet IP, subnet, and
                listen port.</p>

            <h3>How It Works</h3>
            <ol>
                <li><strong>Generate token:</strong> Run <code>wolfnet invite</code> on an existing node. WolfNet
                    auto-detects your public IP and encodes it into the token.</li>
                <li><strong>Join:</strong> Run <code>wolfnet join &lt;token&gt;</code> on the new node. It decodes the
                    token, assigns itself the next available IP, and adds the inviter as a peer.</li>
                <li><strong>Reverse token:</strong> The join command prints a reverse token. Run this on the
                    original node to complete the bidirectional peering.</li>
                <li><strong>Restart:</strong> Restart WolfNet on both nodes &mdash; they connect immediately.</li>
            </ol>

            <div class="code-block">
                <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code># Node A — generate invite
$ sudo wolfnet --config /etc/wolfnet/config.toml invite
Invite token:
  sudo wolfnet --config /etc/wolfnet/config.toml join eyJwa...

# Node B — join using the token
$ sudo wolfnet --config /etc/wolfnet/config.toml join eyJwa...
Joined network! This node is 10.10.10.2
Run this on the inviting node to complete the connection:
  sudo wolfnet --config /etc/wolfnet/config.toml join eyJxb...

# Node A — run the reverse token
$ sudo wolfnet --config /etc/wolfnet/config.toml join eyJxb...

# Both nodes — restart
$ sudo systemctl restart wolfnet</code></pre>
            </div>

            <div class="info-box">
                <p>&#x1F4A1; <strong>WolfStack dashboard:</strong> In the WolfStack UI, go to <strong>Networking
                    &rarr; WolfNet</strong> to generate invites, add peers, and manage connections graphically.
                    The cluster settings page also has a <strong>Update WolfNet Connections</strong> button that
                    automatically syncs WolfNet peers across all cluster nodes.</p>
            </div>
        </div>

        <!-- ===== Relay Forwarding ===== -->
        <div class="content-section" id="relay-forwarding">
            <h2>Relay Forwarding</h2>
            <p>Relay forwarding happens <strong>automatically</strong> &mdash; no configuration needed.
                When a peer cannot be reached directly (e.g. both sides are behind NAT), traffic is
                relayed through a mutually reachable peer.</p>

            <h3>How Relay Works</h3>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">text</span></div>
                <pre><code>Node A (behind NAT)          Node B (public IP)          Node C (behind NAT)
  10.10.10.1                   10.10.10.2                  10.10.10.3

  A ◄──────────────────────► B ◄──────────────────────► C
  A cannot reach C directly    B can reach both

  A sends to C:
    1. A encrypts packet for B, sends to B
    2. B decrypts, sees destination is C
    3. B re-encrypts for C, forwards to C
    4. C decrypts and receives the packet</code></pre>
            </div>

            <ol>
                <li>Node B shares its peer list with A and C via <strong>peer exchange (PEX)</strong></li>
                <li>A learns about C (with <code>relay_via = B</code>) and C learns about A</li>
                <li>When A sends traffic to C, it routes through B automatically</li>
                <li>If A and C later establish a direct connection (e.g. via port forwarding), the relay is dropped
                    in favour of the direct path</li>
            </ol>

            <p>Any WolfNet node with two or more peers automatically enables IP forwarding and can act as a
                relay. The WolfStack dashboard shows relayed peers with a <strong>purple &ldquo;Relay via&rdquo;
                badge</strong> in the Global View.</p>
        </div>

        <!-- ===== Gateway Mode ===== -->
        <div class="content-section">
            <h2>Gateway Mode</h2>
            <p>Gateway mode turns a WolfNet node into a NAT gateway, allowing other peers to route their
                internet traffic through it. This is useful when you want to give peers internet access
                through a specific exit point.</p>

            <h3>Enable Gateway</h3>
            <p>Set <code>gateway = true</code> in the gateway node&rsquo;s config:</p>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">toml</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code>[network]
address = "10.10.10.1"
gateway = true           # Enable NAT gateway</code></pre>
            </div>
            <p>You can also toggle this from the WolfStack dashboard under <strong>Networking &rarr;
                WolfNet</strong>.</p>

            <h3>What Gateway Mode Does</h3>
            <ul>
                <li>Detects the external network interface automatically</li>
                <li>Enables IP forwarding (<code>/proc/sys/net/ipv4/ip_forward</code>)</li>
                <li>Adds iptables MASQUERADE rules for NAT</li>
                <li>Allows forwarding from WolfNet to the internet</li>
                <li>Blocks inbound traffic from the internet to the WolfNet subnet</li>
                <li>All rules are cleaned up automatically on shutdown</li>
            </ul>

            <h3>Client Configuration</h3>
            <p>On a client node that should use the gateway for internet access, add a default route via the
                gateway&rsquo;s WolfNet IP:</p>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code># Route all traffic through the gateway peer
sudo ip route add default via 10.10.10.1 dev wolfnet0</code></pre>
            </div>
        </div>

        <!-- ===== Container Integration ===== -->
        <div class="content-section">
            <h2>Container Integration</h2>
            <p>WolfStack can assign WolfNet IP addresses to Docker containers and LXC containers,
                making them directly addressable across the mesh network. Container IPs are allocated
                from the <code>10.10.10.100&ndash;254</code> range.</p>

            <h3>Docker Containers</h3>
            <p>When creating a Docker container in WolfStack, assign a WolfNet IP via the
                <strong>WolfNet IP</strong> field. This is stored as a Docker label:</p>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code># WolfStack sets this automatically, but you can also do it manually:
docker create --name myapp --label wolfnet.ip=10.10.10.100 nginx</code></pre>
            </div>
            <p>WolfStack then:</p>
            <ol>
                <li>Adds the WolfNet IP as an alias on the container&rsquo;s network interface</li>
                <li>Adds a route for the WolfNet subnet inside the container</li>
                <li>Adds a host-side route so other WolfNet peers can reach the container</li>
                <li>Publishes the container route to remote peers via <code>routes.json</code></li>
            </ol>
            <p>The container is now reachable at <code>10.10.10.100</code> from any peer on the mesh.</p>

            <h3>LXC Containers</h3>
            <p>LXC containers use a marker file at <code>/var/lib/lxc/&lt;name&gt;/.wolfnet/ip</code>.
                WolfStack assigns the IP and configures routing automatically, including Proxmox LXC
                containers with dedicated WolfNet bridge interfaces.</p>

            <h3>Route Propagation</h3>
            <p>Container IP mappings are written to <code>/var/run/wolfnet/routes.json</code>. The WolfNet
                daemon reads this file and uses it to route traffic destined for containers on remote
                nodes to the correct host peer. Routes are propagated across the cluster automatically.</p>
        </div>

        <!-- ===== Platform Support ===== -->
        <div class="content-section">
            <h2>Platform Support</h2>
            <p>WolfNet is <strong>Linux only</strong>. It requires a Linux TUN device (<code>/dev/net/tun</code>)
                and runs as a root daemon. There are no clients for iOS, Android, Windows, or macOS.</p>
            <p>Supported architectures: <strong>x86_64</strong>, <strong>aarch64</strong> (ARM64),
                <strong>armv7</strong>, and <strong>ppc64le</strong> (IBM Power). Any Linux system that can
                build Rust and create a TUN interface can run WolfNet.</p>
        </div>

        <!-- ===== CLI Reference ===== -->
        <div class="content-section">
            <h2>CLI Reference</h2>

            <h3><code>wolfnet</code> (Daemon &amp; Management)</h3>
            <table>
                <thead>
                    <tr>
                        <th>Command</th>
                        <th>Description</th>
                    </tr>
                </thead>
                <tbody>
                    <tr>
                        <td><code>wolfnet</code></td>
                        <td>Run the WolfNet daemon (requires root)</td>
                    </tr>
                    <tr>
                        <td><code>wolfnet init --address &lt;ip&gt;</code></td>
                        <td>Generate default config with the given WolfNet address</td>
                    </tr>
                    <tr>
                        <td><code>wolfnet invite</code></td>
                        <td>Generate an invite token for another node (auto-detects public IP)</td>
                    </tr>
                    <tr>
                        <td><code>wolfnet join &lt;token&gt;</code></td>
                        <td>Join a WolfNet network using an invite token</td>
                    </tr>
                    <tr>
                        <td><code>wolfnet genkey --output &lt;path&gt;</code></td>
                        <td>Generate a new X25519 keypair</td>
                    </tr>
                    <tr>
                        <td><code>wolfnet pubkey</code></td>
                        <td>Print this node&rsquo;s base64 public key</td>
                    </tr>
                    <tr>
                        <td><code>wolfnet token</code></td>
                        <td>Print public key and endpoint in token format</td>
                    </tr>
                </tbody>
            </table>
            <p>Options: <code>--config &lt;path&gt;</code> (default: <code>/etc/wolfnet/config.toml</code>),
                <code>--debug</code> (verbose logging)</p>

            <h3><code>wolfnetctl</code> (Status &amp; Monitoring)</h3>
            <table>
                <thead>
                    <tr>
                        <th>Command</th>
                        <th>Description</th>
                    </tr>
                </thead>
                <tbody>
                    <tr>
                        <td><code>wolfnetctl status</code></td>
                        <td>Show this node&rsquo;s status (IP, port, gateway, uptime, peer count)</td>
                    </tr>
                    <tr>
                        <td><code>wolfnetctl list servers</code></td>
                        <td>List all nodes on the network with their roles</td>
                    </tr>
                    <tr>
                        <td><code>wolfnetctl peers</code></td>
                        <td>Detailed peer info: hostname, IP, endpoint, status, traffic, relay info</td>
                    </tr>
                    <tr>
                        <td><code>wolfnetctl info</code></td>
                        <td>Combined status + peers output</td>
                    </tr>
                </tbody>
            </table>
            <p><code>wolfnetctl</code> reads from <code>/var/run/wolfnet/status.json</code> &mdash; the daemon must
                be running.</p>
        </div>

        <!-- ===== Systemd Service ===== -->
        <div class="content-section">
            <h2>Systemd Service</h2>
            <p>The installer creates a systemd service automatically. Common commands:</p>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code># Start WolfNet
sudo systemctl start wolfnet

# Check status
sudo systemctl status wolfnet

# View logs
sudo journalctl -u wolfnet -f

# Enable on boot
sudo systemctl enable wolfnet

# Restart after config change
sudo systemctl restart wolfnet

# Reload config without restart (sends SIGHUP)
sudo kill -HUP $(pidof wolfnet)</code></pre>
            </div>
        </div>

        <!-- ===== Network Ports ===== -->
        <div class="content-section">
            <h2>Network Ports</h2>
            <table>
                <thead>
                    <tr>
                        <th>Port</th>
                        <th>Protocol</th>
                        <th>Purpose</th>
                    </tr>
                </thead>
                <tbody>
                    <tr>
                        <td>9600</td>
                        <td>UDP</td>
                        <td>Encrypted tunnel traffic (handshakes, data, keepalives, PEX)</td>
                    </tr>
                    <tr>
                        <td>9601</td>
                        <td>UDP</td>
                        <td>LAN auto-discovery broadcasts (local network only)</td>
                    </tr>
                </tbody>
            </table>
            <div class="info-box">
                <p>&#x1F4A1; <strong>Firewall tip:</strong> Only port <strong>9600/UDP</strong> needs to be open
                    for remote access. Port 9601 is used for LAN discovery only and does not need to be exposed
                    to the internet.</p>
            </div>
        </div>

        <!-- ===== File Locations ===== -->
        <div class="content-section">
            <h2>File Locations</h2>
            <table>
                <thead>
                    <tr>
                        <th>Path</th>
                        <th>Purpose</th>
                    </tr>
                </thead>
                <tbody>
                    <tr>
                        <td><code>/etc/wolfnet/config.toml</code></td>
                        <td>Main configuration file</td>
                    </tr>
                    <tr>
                        <td><code>/etc/wolfnet/private.key</code></td>
                        <td>X25519 private key (base64, mode 0600)</td>
                    </tr>
                    <tr>
                        <td><code>/var/run/wolfnet/status.json</code></td>
                        <td>Live daemon status (updated every 5 seconds)</td>
                    </tr>
                    <tr>
                        <td><code>/var/run/wolfnet/routes.json</code></td>
                        <td>Container/VM IP &rarr; host WolfNet IP routing table</td>
                    </tr>
                </tbody>
            </table>
        </div>

        <!-- ===== Security ===== -->
        <div class="content-section">
            <h2>Security</h2>
            <ul>
                <li><strong>X25519 key exchange</strong> &mdash; Modern elliptic-curve Diffie-Hellman providing
                    forward secrecy</li>
                <li><strong>ChaCha20-Poly1305 AEAD</strong> &mdash; Authenticated encryption used by WireGuard,
                    TLS 1.3, and Google&rsquo;s QUIC protocol</li>
                <li><strong>No central server</strong> &mdash; Peer-to-peer mesh with no single point of compromise</li>
                <li><strong>No third-party relay</strong> &mdash; All traffic stays on your own infrastructure</li>
                <li><strong>Token-based authentication</strong> &mdash; Only devices with a valid invite token can join</li>
                <li><strong>Replay protection</strong> &mdash; Monotonic nonce counters prevent packet replay attacks</li>
                <li><strong>Key isolation</strong> &mdash; Private key stored with mode 0600, never transmitted</li>
                <li><strong>Session re-establishment</strong> &mdash; Sessions are re-established on every handshake,
                    resetting all counters</li>
            </ul>
        </div>

        <!-- ===== Troubleshooting ===== -->
        <div class="content-section">
            <h2>Troubleshooting</h2>

            <h3>Peers not connecting</h3>
            <ul>
                <li>Check that UDP port <strong>9600</strong> is open in your firewall on both sides</li>
                <li>Verify the endpoint address is correct: <code>wolfnetctl peers</code> shows the last known endpoint</li>
                <li>For DynDNS, ensure the hostname resolves correctly: <code>dig myhost.duckdns.org</code></li>
                <li>Check logs for handshake errors: <code>sudo journalctl -u wolfnet -f</code></li>
                <li>Run with <code>--debug</code> for verbose output: <code>sudo wolfnet --debug</code></li>
            </ul>

            <h3>Connected but can&rsquo;t ping</h3>
            <ul>
                <li>Verify both nodes have <code>wolfnet0</code> interface up: <code>ip addr show wolfnet0</code></li>
                <li>Check for conflicting firewall rules blocking the <code>10.10.10.0/24</code> subnet</li>
                <li>Ensure the peer shows as connected: <code>wolfnetctl peers</code></li>
            </ul>

            <h3>LAN discovery not working</h3>
            <ul>
                <li>Both nodes must be on the same Layer 2 network (same switch/VLAN)</li>
                <li>Check that <code>discovery = true</code> in config on both nodes</li>
                <li>Some cloud providers and corporate networks block UDP broadcast &mdash; use static endpoints instead</li>
            </ul>

            <h3>Connection drops after IP change</h3>
            <ul>
                <li>Use a DynDNS hostname instead of a static IP in the <code>endpoint</code> field</li>
                <li>WolfNet re-resolves hostnames every 60 seconds automatically</li>
                <li>Keepalives are sent every 25 seconds to maintain NAT mappings</li>
            </ul>

            <h3>WolfNet status shows no peers</h3>
            <div class="code-block">
                <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                        onclick="copyCode(this)">Copy</button></div>
                <pre><code># Check if daemon is running
sudo systemctl status wolfnet

# Check status file
cat /var/run/wolfnet/status.json | python3 -m json.tool

# Check config
cat /etc/wolfnet/config.toml

# View public key (must match what peers have)
sudo wolfnet --config /etc/wolfnet/config.toml pubkey</code></pre>
            </div>

            <h3>Container not reachable via WolfNet</h3>
            <ul>
                <li>Verify the container has a WolfNet IP assigned in WolfStack</li>
                <li>Check that routes are published: <code>cat /var/run/wolfnet/routes.json</code></li>
                <li>Ensure iptables FORWARD rules allow traffic between <code>wolfnet0</code> and the container bridge</li>
                <li>Restart WolfStack to reapply container routes: <code>sudo systemctl restart wolfstack</code></li>
            </ul>
        </div>

        <div class="page-nav"><a href="wolfdisk.php" class="prev">&larr; WolfDisk</a><a
                href="wolfnet-global.php" class="next">Global View &rarr;</a></div>

    </main>
<?php include 'includes/footer.php'; ?>
