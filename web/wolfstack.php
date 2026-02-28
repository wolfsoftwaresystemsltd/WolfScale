<?php
$page_title = 'WolfStack Overview — WolfStack Docs';
$page_desc = 'WolfStack — The Universal Server Management Platform. Overview, installation, and feature guide.';
$active = 'wolfstack.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">

                <div class="content-section">
                    <h2>What is WolfStack?</h2>
                    <p>WolfStack is an all-in-one server management platform that lets you monitor, manage, and control
                        your entire infrastructure from a single beautiful web dashboard. Whether you have one machine
                        or hundreds, WolfStack scales with you.</p>

                    <p>Built entirely in <strong>Rust</strong> for maximum performance and reliability, WolfStack
                        installs on any Linux distribution and auto-adapts to your system. It comes with <a
                            href="wolfnet.php">WolfNet</a> — an encrypted mesh network that connects all your servers
                        automatically, even across different data centres.</p>

                    <img src="images/dashboard-overview.png" alt="WolfStack Dashboard"
                        style="width: 100%; border-radius: 10px; margin: 1.5rem 0; border: 1px solid var(--border-color); box-shadow: 0 8px 32px rgba(0,0,0,0.3);">

                    <h3>Key Capabilities</h3>
                    <ul>
                        <li><strong>Real-time monitoring</strong> — CPU, memory, disk, and network metrics with
                            interactive graphs</li>
                        <li><strong>Container management</strong> — Create, clone, migrate, and manage Docker and LXC
                            containers</li>
                        <li><strong>Multi-server clustering</strong> — Manage your entire fleet from one dashboard</li>
                        <li><strong>Proxmox integration</strong> — Install on top of Proxmox to manage VE clusters</li>
                        <li><strong>WolfRun orchestration</strong> — Schedule and scale containers across nodes,
                            replacing Kubernetes</li>
                        <li><strong>File & config management</strong> — Browse and edit files on any node via the web UI
                        </li>
                        <li><strong>Web terminal</strong> — Full SSH terminal in your browser for any node or container
                        </li>
                        <li><strong>App Store</strong> — Deploy containers and apps to any node with one click</li>
                        <li><strong>Issues Scanner</strong> — AI-powered proactive monitoring for hardware and service
                            issues</li>
                        <li><strong>Alerting</strong> — Discord, Slack, and Telegram notifications for threshold
                            breaches</li>
                        <li><strong>Status Pages</strong> — Built-in uptime monitoring with public status pages and incident tracking</li>
                        <li><strong>AI Agent</strong> — Ask questions about your infrastructure in natural language</li>
                        <li><strong>Beautiful themes</strong> — Dark, Glass, Midnight, Amber Terminal, and more</li>
                    </ul>
                </div>

                <div class="content-section">
                    <h2>⚡ Quick Start</h2>
                    <h3>Step 1: Install WolfStack</h3>
                    <p>Run this on every machine you want to manage:</p>
                    <div class="code-block">
                        <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                                onclick="copyCode(this)">Copy</button></div>
                        <pre><code>curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfStack/master/setup.sh | sudo bash</code></pre>
                    </div>
                    <p>The installer automatically detects your Linux distribution and installs WolfStack as a systemd
                        service.</p>

                    <h3>Step 2: Get the Token</h3>
                    <p>After installation, each server displays its cluster token. You can also retrieve it with:</p>
                    <div class="code-block">
                        <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                                onclick="copyCode(this)">Copy</button></div>
                        <pre><code>wolfstack --show-token</code></pre>
                    </div>

                    <h3>Step 3: Open the Web UI</h3>
                    <p>Navigate to <code>http://your-server-ip:8553</code> and log in with your Linux credentials. You
                        only need to connect to <strong>one</strong> server — it manages the rest.</p>

                    <h3>Step 4: Add Your Other Nodes</h3>
                    <p>Click the <strong>+</strong> button in the sidebar to add each server using its token. You can
                        add both WolfStack and Proxmox nodes.</p>

                    <h3>Step 5: Connect WolfNet</h3>
                    <p>Go into your cluster settings and click <strong>🔗 Update WolfNet Connections</strong> to
                        automatically set up encrypted peer-to-peer networking between all your nodes.</p>

                    <p><strong>That's it!</strong> You now have a fully managed, encrypted cluster. 🐺</p>
                </div>

                <div class="content-section">
                    <h2>System Requirements</h2>
                    <h3>Supported Platforms</h3>
                    <ul>
                        <li>Debian 11, 12, 13 (Trixie)</li>
                        <li>Ubuntu 20.04, 22.04, 24.04, 25.04, 25.10</li>
                        <li>AlmaLinux 8, 9</li>
                        <li>Rocky Linux 8, 9</li>
                        <li>Fedora 38+</li>
                        <li>Arch Linux</li>
                        <li>Proxmox VE 7, 8</li>
                        <li>Any Linux with glibc 2.31+</li>
                    </ul>

                    <h3>Minimum Requirements</h3>
                    <ul>
                        <li><strong>CPU:</strong> 1 core (any architecture supported by Rust)</li>
                        <li><strong>RAM:</strong> 256 MB free</li>
                        <li><strong>Disk:</strong> 50 MB for the binary</li>
                        <li><strong>Network:</strong> One open port (8553 default)</li>
                    </ul>
                </div>

                <div class="content-section">
                    <h2>Dashboard Features</h2>
                    <img src="images/node-detail.png" alt="Node Detail View"
                        style="width: 100%; border-radius: 10px; margin-bottom: 1.5rem; border: 1px solid var(--border-color); box-shadow: 0 8px 32px rgba(0,0,0,0.3);">

                    <h3>Datacenter View</h3>
                    <p>The datacenter view shows a global map of your infrastructure with real-time status for every
                        node. At a glance, see CPU usage, memory consumption, disk space, and uptime for your entire
                        fleet.</p>

                    <h3>Node Detail</h3>
                    <p>Click any node to see detailed metrics including interactive CPU, memory, disk, and network
                        graphs. View running services, manage containers, browse files, and open a web terminal — all
                        from one page.</p>

                    <h3>Themes</h3>
                    <img src="images/settings-themes.png" alt="WolfStack Themes"
                        style="width: 100%; border-radius: 10px; margin-bottom: 1rem; border: 1px solid var(--border-color); box-shadow: 0 8px 32px rgba(0,0,0,0.3);">
                    <p>WolfStack includes multiple beautiful themes including WolfStack Dark, Midnight, Glass
                        (glassmorphism), Amber Terminal, and more. Switch themes from the Settings page.</p>
                </div>

                <div class="content-section">
                    <h2>What's Included</h2>
                    <p>WolfStack comes with a suite of integrated tools:</p>
                    <ul>
                        <li><a href="wolfstack-containers.php"><strong>Container Management</strong></a> — Docker & LXC
                            with cloning, migration, and resource limits</li>
                        <li><a href="wolfstack-storage.php"><strong>Storage Manager</strong></a> — S3/R2, NFS, WolfDisk
                            mounts from the dashboard</li>
                        <li><a href="wolfstack-files.php"><strong>File Manager</strong></a> — Browse, edit, upload, and
                            download files on any node</li>
                        <li><a href="wolfstack-networking.php"><strong>Networking</strong></a> — IP management, port
                            forwarding, firewall rules</li>
                        <li><a href="wolfstack-clustering.php"><strong>Multi-Server Clustering</strong></a> — Join
                            nodes into clusters with auto-discovery</li>
                        <li><a href="wolfstack-mysql.php"><strong>MariaDB/MySQL Editor</strong></a> — Browse tables,
                            run queries, manage databases</li>
                        <li><a href="wolfstack-security.php"><strong>Security</strong></a> — Linux PAM authentication,
                            API tokens, audit logging</li>
                        <li><a href="wolfstack-certificates.php"><strong>Certificates</strong></a> — SSL/TLS
                            certificate management</li>
                        <li><a href="wolfstack-cron.php"><strong>Cron Jobs</strong></a> — Schedule and manage cron
                            tasks on any node</li>
                        <li><a href="wolfstack-terminal.php"><strong>Terminal</strong></a> — Full web-based SSH
                            terminal</li>
                        <li><a href="wolfstack-issues.php"><strong>Issues Scanner</strong></a> — AI-powered server
                            health monitoring</li>
                        <li><a href="wolfstack-alerting.php"><strong>Alerting</strong></a> — Discord, Slack, Telegram
                            notifications</li>
                        <li><a href="wolfstack-statuspage.php"><strong>Status Pages</strong></a> — Uptime monitoring
                            with public status pages</li>
                        <li><a href="wolfrun.php"><strong>WolfRun Orchestration</strong></a> — Schedule, scale, and
                            manage
                            services across your cluster — replaces Kubernetes</li>
                        <li><a href="proxmox.php"><strong>Proxmox Integration</strong></a> — Install on Proxmox to
                            manage
                            VE clusters from the WolfStack dashboard</li>
                        <li><a href="app-store.php"><strong>App Store</strong></a> — One-click container deployment
                        </li>
                        <li><a href="wolfstack-ai.php"><strong>AI Agent</strong></a> — Natural language infrastructure
                            queries</li>
                        <li><a href="wolfstack-settings.php"><strong>Settings</strong></a> — Themes, alerting, Docker
                            registries, node and cluster configuration</li>
                    </ul>
                </div>

                <div class="page-nav">
                    <a href="index.php" class="prev">← Home</a>
                    <a href="wolfstack-containers.php" class="next">Container Management →</a>
                </div>
            
    </main>
<?php include 'includes/footer.php'; ?>
