<?php
$page_title = '🔐 WolfNet VPN — WolfStack Docs';
$page_desc = 'Built-in VPN &mdash; securely access your entire infrastructure from anywhere with WolfNet remote access';
$active = 'wolfnet-vpn.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


                <div class="content-section">
                    <h2>What Is It?</h2>
                    <p>WolfNet isn&rsquo;t just a cluster networking layer &mdash; it&rsquo;s a <strong>full
                            VPN</strong> built directly into your infrastructure. Any device running WolfNet can
                        securely join your private network from anywhere in the world, giving you instant access to
                        every server, container, and VM as if you were sat in the office.</p>
                    <p>There&rsquo;s no extra software to install, no separate VPN server to manage, and no complex
                        configuration. If you&rsquo;re already running WolfStack, <strong>you already have a
                            VPN</strong>.</p>
                </div>

                <div class="content-section">
                    <h2>Why WolfNet VPN?</h2>
                    <table>
                        <thead>
                            <tr>
                                <th>Feature</th>
                                <th>WolfNet VPN</th>
                                <th>Traditional VPN</th>
                            </tr>
                        </thead>
                        <tbody>
                            <tr>
                                <td><strong>Setup</strong></td>
                                <td>One command (<code>wolfnetctl join</code>)</td>
                                <td>Server install, certificates, client config</td>
                            </tr>
                            <tr>
                                <td><strong>Encryption</strong></td>
                                <td>X25519 + ChaCha20-Poly1305</td>
                                <td>Varies (often OpenSSL/IPSec)</td>
                            </tr>
                            <tr>
                                <td><strong>Architecture</strong></td>
                                <td>Peer-to-peer mesh</td>
                                <td>Client &rarr; Server hub</td>
                            </tr>
                            <tr>
                                <td><strong>NAT traversal</strong></td>
                                <td>Built-in relay forwarding</td>
                                <td>Requires port forwarding</td>
                            </tr>
                            <tr>
                                <td><strong>Extra software</strong></td>
                                <td>None &mdash; included with WolfStack</td>
                                <td>Separate VPN server + clients</td>
                            </tr>
                            <tr>
                                <td><strong>Performance</strong></td>
                                <td>WireGuard-class speed</td>
                                <td>Overhead from TLS tunnelling</td>
                            </tr>
                        </tbody>
                    </table>
                </div>

                <div class="content-section">
                    <h2>How It Works</h2>
                    <p>WolfNet creates a TUN-based Layer 3 encrypted mesh network. When a remote device joins, it is
                        assigned a <code>10.10.10.x</code> address on the same private subnet as your cluster. All
                        traffic between peers is encrypted end-to-end.</p>
                    <ol>
                        <li><strong>Cluster nodes</strong> run WolfNet as part of WolfStack and are interconnected
                            automatically</li>
                        <li><strong>Remote devices</strong> (laptops, workstations, phones) join the same network using
                            an invite token</li>
                        <li><strong>Relay forwarding</strong> ensures connectivity even behind NAT or restrictive
                            firewalls</li>
                        <li><strong>Routes are managed automatically</strong> &mdash; you connect and you&rsquo;re done
                        </li>
                    </ol>
                </div>

                <div class="content-section">
                    <h2>Getting Started</h2>
                    <h3>1. Generate an invite token on any WolfStack node</h3>
                    <div class="code-block">
                        <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                                onclick="copyCode(this)">Copy</button></div>
                        <pre><code>wolfnetctl invite</code></pre>
                    </div>
                    <p>This outputs a secure token that encodes the network key and the endpoint address of the inviting
                        node.</p>

                    <h3>2. Join from your remote machine</h3>
                    <p>Install WolfNet on your laptop or workstation, then join:</p>
                    <div class="code-block">
                        <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                                onclick="copyCode(this)">Copy</button></div>
                        <pre><code># Install WolfNet standalone
curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/master/wolfnet/install.sh | sudo bash

# Join the network
wolfnetctl join &lt;token&gt;</code></pre>
                    </div>

                    <h3>3. You&rsquo;re connected</h3>
                    <p>Your device receives a WolfNet IP address (e.g. <code>10.10.10.5</code>) and can immediately
                        reach all cluster resources:</p>
                    <div class="code-block">
                        <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                                onclick="copyCode(this)">Copy</button></div>
                        <pre><code># Ping a cluster node
ping 10.10.10.1

# SSH into a server
ssh admin@10.10.10.2

# Access a web service running on a container
curl http://10.10.10.3:8080

# Open the WolfStack dashboard
firefox https://10.10.10.1:9443</code></pre>
                    </div>
                </div>

                <div class="content-section">
                    <h2>Use Cases</h2>
                    <ul>
                        <li><strong>Remote work</strong> &mdash; Access your office infrastructure from home or while
                            travelling without a corporate VPN appliance</li>
                        <li><strong>Development</strong> &mdash; Connect your dev machine directly to staging/production
                            servers, databases, and containers</li>
                        <li><strong>Multi-site access</strong> &mdash; Join WolfNet networks spanning multiple clusters
                            and data centres from a single laptop</li>
                        <li><strong>Emergency admin</strong> &mdash; Quickly join the network from any machine to
                            diagnose and fix issues</li>
                        <li><strong>Team access</strong> &mdash; Generate invite tokens for team members so they can
                            securely access shared infrastructure</li>
                    </ul>
                </div>

                <div class="content-section">
                    <h2>Security</h2>
                    <ul>
                        <li><strong>X25519 key exchange</strong> &mdash; Modern elliptic-curve Diffie-Hellman for
                            forward secrecy</li>
                        <li><strong>ChaCha20-Poly1305 encryption</strong> &mdash; AEAD cipher used by WireGuard, TLS
                            1.3, and Google</li>
                        <li><strong>No central server</strong> &mdash; Peer-to-peer architecture means no single point
                            of compromise</li>
                        <li><strong>Token-based authentication</strong> &mdash; Only devices with a valid invite token
                            can join the network</li>
                        <li><strong>No third-party relay</strong> &mdash; All traffic stays on your own infrastructure
                            or goes direct between peers</li>
                    </ul>
                </div>

                <div class="content-section">
                    <h2>Managing Remote Peers</h2>
                    <p>View all connected peers and their status from the WolfStack dashboard under <strong>Global
                            WolfNet</strong>, or from the command line:</p>
                    <div class="code-block">
                        <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                                onclick="copyCode(this)">Copy</button></div>
                        <pre><code># List all connected peers
wolfnetctl peers

# Remove a peer
wolfnetctl remove &lt;peer-ip&gt;</code></pre>
                    </div>
                </div>

                <div class="page-nav"><a href="wolfnet-global.php" class="prev">&larr; Global WolfNet Access</a><a
                        href="wolfproxy.php" class="next">WolfProxy &rarr;</a></div>
            
    </main>
<?php include 'includes/footer.php'; ?>
