<?php
$page_title = 'WolfStack — The Universal Server Management Platform';
$page_desc = 'WolfStack — The Universal Server Management Platform. Monitor servers, manage Virtual Machines, Docker & LXC containers, control services, and edit configurations from a beautiful web dashboard.';
$page_keywords = 'server management, WolfStack, dashboard, Docker, LXC, monitoring, clustering, WolfScale, WolfDisk, WolfNet';
$page_canonical = 'https://wolfscale.org/';
$active = 'index.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>

    <main class="wiki-main">
        <div class="mobile-header">
            <a href="index.php" class="nav-logo"><img src="images/wolfstack-logo.png" alt="WolfStack" class="logo-img" style="height:32px;"></a>
            <button class="mobile-menu-btn" id="mobile-menu-btn"><span></span><span></span><span></span></button>
        </div>

        <div class="wiki-content" style="margin-left:0;">
            <!-- Hero -->
            <section style="text-align:center; padding:2rem 0 1rem; position:relative;">
                <div style="display:flex; gap:0.75rem; justify-content:center; flex-wrap:wrap; margin-bottom:1.5rem;" class="animate animate-delay-1">
                    <span class="hero-badge">Open Source</span>
                    <span class="hero-badge">FSL-1.1 License</span>
                    <span class="hero-badge">Built with Rust &#x1F980;</span>
                </div>

                <img src="images/wolfstack-logo.png" alt="WolfStack" style="width:400px;margin-bottom:0.5rem;animation:float 4s ease-in-out infinite;" class="animate animate-delay-2">
                <p style="font-size:1.1rem;color:var(--text-secondary);margin-bottom:0.5rem;font-weight:400;max-width:700px;margin-left:auto;margin-right:auto;line-height:1.7;" class="animate animate-delay-2">
                    The <strong style="color:var(--accent-primary);">all-in-one</strong> server and workstation management platform from
                    <a href="https://wolf.uk.com" target="_blank" style="color:var(--accent-primary);text-decoration:underline;">Wolf Software Systems Ltd</a>
                </p>

                <!-- CTA Buttons -->
                <div style="display:flex;gap:0.75rem;justify-content:center;flex-wrap:wrap;margin:1rem 0;" class="animate animate-delay-3">
                    <a href="https://www.patreon.com/15362110/join" target="_blank"
                       style="display:inline-flex;align-items:center;gap:0.5rem;padding:0.6rem 1.5rem;background:#16a34a;color:#fff;text-decoration:none;border-radius:50px;font-weight:600;font-size:0.85rem;transition:all 0.2s ease;box-shadow:0 3px 12px rgba(34,197,94,0.3);">
                        <svg width="16" height="16" viewBox="0 0 256 256" fill="white" xmlns="http://www.w3.org/2000/svg"><path d="M232.3 25.63c-29.45-22.29-73.6-22.15-104.94 2.52C102.06 48.12 94.34 78.36 99.6 106.5c-36.94 4.56-66.44 25.6-82.18 55.46C.98 193.24-.24 231.72 15.48 256H38.2c-20.74-28.46-19.78-72.22 6.7-99.5 22.02-22.7 55.14-31.26 84.34-20.18-5.78 38.28 15.04 77.32 51.4 92.9 21.94 9.4 47.84 9.76 69.1-2.96 24.18-14.48 39.76-42.02 39.76-70.6V69.6c.76-17.1-7.8-33.72-21.14-44h-36.08v.03zM216 155.66c0 18.7-10.14 36.34-26.14 45.48-13.68 7.82-31.26 8.92-47.96.44-25.44-12.92-38.94-43.46-30.2-70.84 25.18 14.14 56.62 16.52 84.06 4.58 7.48-3.28 14.26-8.02 20.24-13.44v33.78zM216 92.18c-20.5 22.34-55.04 29.76-83.7 16.38-18.06-8.44-31.92-24.58-36.2-44.3-4.92-22.56 2.58-47.2 21.56-62 24.32-18.94 59.78-20.68 84.2-5.44l.14.1c8.54 5.82 14.02 16.46 14 27.1v68.16z"/></svg>
                        Support us on Patreon
                    </a>
                    <a href="enterprise.php"
                       style="display:inline-flex;align-items:center;gap:0.5rem;padding:0.6rem 1.5rem;background:var(--accent-primary);color:#fff;text-decoration:none;border-radius:50px;font-weight:600;font-size:0.85rem;transition:all 0.2s ease;box-shadow:0 3px 12px rgba(220,38,38,0.3);">
                        &#x1F3E2; Enterprise Licensing
                    </a>
                </div>
            </section>

            <!-- Community Links -->
            <div style="display:flex;gap:1rem;justify-content:center;flex-wrap:wrap;margin-bottom:2rem;" class="animate animate-delay-4">
                <a href="https://www.youtube.com/@wolfsoftwaresystems" target="_blank" style="display:inline-flex;align-items:center;gap:0.5rem;padding:10px 20px;color:white;text-decoration:none;border-radius:8px;font-weight:600;font-size:0.85rem;background:#cc0000;box-shadow:0 2px 10px rgba(255,0,0,0.2);transition:all 0.2s ease;">&#x25B6; YouTube Channel</a>
                <a href="https://discord.gg/q9qMjHjUQY" target="_blank" style="display:inline-flex;align-items:center;gap:0.5rem;padding:10px 20px;color:white;text-decoration:none;border-radius:8px;font-weight:600;font-size:0.85rem;background:#5865F2;box-shadow:0 2px 10px rgba(88,101,242,0.2);transition:all 0.2s ease;">&#x1F4AC; Join Discord</a>
                <a href="https://www.reddit.com/r/WolfStack/" target="_blank" style="display:inline-flex;align-items:center;gap:0.5rem;padding:10px 20px;color:white;text-decoration:none;border-radius:8px;font-weight:600;font-size:0.85rem;background:#FF4500;box-shadow:0 2px 10px rgba(255,69,0,0.2);transition:all 0.2s ease;">&#x1F525; r/WolfStack</a>
            </div>

            <!-- Dashboard Screenshot -->
            <div class="hero-screenshot animate animate-delay-3" style="max-width:900px;margin:0 auto 2rem;">
                <img src="images/screenshots/hero-dashboard-2x.png" alt="WolfStack Dashboard">
                <p style="text-align:center;margin-top:0.75rem;font-size:0.8rem;color:var(--text-muted);font-style:italic;">WolfStack Datacenter — manage your entire fleet from a single dashboard</p>
            </div>

            <!-- Feature Highlights -->
            <div style="display:grid;grid-template-columns:repeat(auto-fit,minmax(200px,1fr));gap:1rem;margin:2rem 0;text-align:left;" class="animate animate-delay-4">
                <div class="feature-card">
                    <div class="feature-icon">&#x1F3E0;</div>
                    <h3>Unified Dashboard</h3>
                    <p>Monitor all servers, clusters, containers, and VMs from a single beautiful web interface with real-time metrics.</p>
                </div>
                <div class="feature-card">
                    <div class="feature-icon">&#x1F4E6;</div>
                    <h3>Container Management</h3>
                    <p>Create, clone, migrate, and manage Docker and LXC containers across your entire fleet. Built-in App Store.</p>
                </div>
                <div class="feature-card">
                    <div class="feature-icon">&#x1F310;</div>
                    <h3>Encrypted Mesh Network</h3>
                    <p>WolfNet creates an encrypted mesh network between all your servers automatically. Works across data centres.</p>
                </div>
                <div class="feature-card">
                    <div class="feature-icon">&#x1F7E0;</div>
                    <h3>Proxmox Integration</h3>
                    <p>Install WolfStack on top of Proxmox to manage your entire VE cluster from the WolfStack dashboard.</p>
                </div>
                <div class="feature-card">
                    <div class="feature-icon">&#x1F916;</div>
                    <h3>AI Agent</h3>
                    <p>Ask questions about your infrastructure in natural language. Get intelligent answers and automated actions.</p>
                </div>
                <div class="feature-card">
                    <div class="feature-icon">&#x1F4DF;</div>
                    <h3>Status Pages</h3>
                    <p>Built-in uptime monitoring with beautiful public status pages. HTTP, TCP, Ping, Container &amp; WolfRun monitors with 90-day history.</p>
                </div>
            </div>

            <!-- Clusters & Nodes -->
            <section class="content-section animate animate-delay-4" style="max-width:640px;margin:0 auto 1.5rem;">
                <h2>&#x1F517; Clusters &amp; Nodes</h2>
                <p>WolfStack organises your infrastructure into <strong>clusters</strong> and <strong>nodes</strong>:</p>
                <ul>
                    <li>A <strong>node</strong> is any individual server or machine running WolfStack.</li>
                    <li>A <strong>cluster</strong> is a group of nodes that are managed together as a single unit.</li>
                    <li>Add all your servers to the <strong>same cluster</strong> so they can share networking, containers, and configuration.</li>
                </ul>
                <div class="info-box">
                    <p>&#x1F4A1; You can also add multiple clusters (e.g. separate WolfStack and Proxmox clusters) to your dashboard for a complete overview.</p>
                </div>
            </section>

            <!-- Compilation Warning -->
            <section class="animate animate-delay-4" style="max-width:640px;margin:0 auto 1.5rem;">
                <div class="warning-box">
                    <p><strong>&#x26A0;&#xFE0F; Heads Up — Rust Compilation</strong></p>
                    <p>WolfStack is written in <strong>Rust</strong>, which compiles from source during installation. This will make your CPU run hard, especially on <strong>low-powered systems</strong> — this is completely normal! Don't worry, just let it finish. Once compilation is complete, everything will return to normal.</p>
                </div>
            </section>

            <!-- Quick Start -->
            <section style="padding:0 0 1.5rem;text-align:center;">
                <div style="max-width:600px;margin:0 auto;" class="animate animate-delay-4">
                    <h3 style="font-size:1.1rem;font-weight:600;margin-bottom:1rem;">&#x26A1; Quick Start</h3>
                    <ol style="text-align:left;color:var(--text-secondary);font-size:0.9rem;line-height:1.8;margin-bottom:1.25rem;padding-left:1.25rem;">
                        <li><strong>Install WolfStack on each server</strong> — run this on every machine you want to manage:
                            <div class="code-block" style="text-align:left;margin:0.5rem 0;">
                                <div class="code-header"><span>bash</span><button class="copy-btn" onclick="copyCode(this)">Copy</button></div>
                                <pre><code>curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfStack/master/setup.sh | sudo bash</code></pre>
                            </div>
                            <p style="font-size:0.82rem;color:var(--text-muted);margin:0.5rem 0 0;line-height:1.7;">
                                &#x1F4A1; If <code>sudo</code> or <code>curl</code> are not installed, install them first:<br>
                                <strong>Debian / Ubuntu:</strong> <code>apt install sudo curl</code><br>
                                <strong>RHEL / Fedora:</strong> <code>dnf install sudo curl</code>
                            </p>
                        </li>
                        <li><strong>Get the token</strong> from each server — after installation, each server displays its cluster token. You can also run <code>wolfstack --show-token</code></li>
                        <li><strong>Open the web UI</strong> on <strong>one</strong> server — navigate to <code>http://your-server-ip:8553</code> and log in. You only need to log in to one server.</li>
                        <li><strong>Add your other nodes</strong> — click the <strong>+</strong> button to add each server or Proxmox server. You're done!</li>
                        <li><strong>Update WolfNet connections</strong> — go into your cluster settings and click <strong>&#x1F517; Update WolfNet Connections</strong> to automatically set up peer-to-peer networking between all your nodes.</li>
                    </ol>
                    <p style="color:var(--text-secondary);font-size:0.85rem;margin-bottom:1rem;">
                        <strong>Note:</strong> You can add multiple WolfStack or Proxmox clusters to WolfStack's dashboard for a one-stop server management shop.
                    </p>
                    <p style="margin-top:1.25rem;color:var(--text-secondary);font-size:0.95rem;font-weight:500;">
                        Enjoy! &#x1F43A; — Please don't forget my <a href="https://www.patreon.com/15362110/join" target="_blank" style="color:#22c55e;text-decoration:underline;font-weight:600;">Patreon</a>.
                    </p>
                    <p style="margin-top:0.5rem;color:var(--text-muted);font-size:0.85rem;">
                        Works on most versions of Linux and runs fine on a single machine! The best way to learn is by trying it out.
                    </p>
                </div>
            </section>

            <!-- Comparison Chart -->
            <section style="margin-bottom:3rem;">
                <h2 style="text-align:center;font-size:1.8rem;font-weight:800;margin-bottom:0.5rem;">How Does WolfStack Compare?</h2>
                <p style="text-align:center;color:var(--text-secondary);margin-bottom:2rem;max-width:700px;margin-left:auto;margin-right:auto;">
                    WolfStack replaces multiple tools with one unified platform. Here&rsquo;s how it stacks up.
                </p>
                <div class="table-wrapper">
                    <table class="data-table" style="min-width:800px;">
                        <thead>
                            <tr>
                                <th style="text-align:left;">Feature</th>
                                <th style="text-align:center;color:var(--accent-primary);">WolfStack</th>
                                <th style="text-align:center;">Proxmox</th>
                                <th style="text-align:center;">Kubernetes</th>
                                <th style="text-align:center;">Portainer</th>
                                <th style="text-align:center;">CasaOS</th>
                                <th style="text-align:center;">Cockpit</th>
                            </tr>
                        </thead>
                        <tbody>
                            <tr><td>Docker Containers</td><td style="text-align:center;color:#22c55e;font-weight:700;">&#x2705;</td><td style="text-align:center;">&#x26A0;&#xFE0F; Limited</td><td style="text-align:center;color:#22c55e;">&#x2705;</td><td style="text-align:center;color:#22c55e;">&#x2705;</td><td style="text-align:center;color:#22c55e;">&#x2705;</td><td style="text-align:center;">&#x26A0;&#xFE0F; Plugin</td></tr>
                            <tr><td>LXC Containers</td><td style="text-align:center;color:#22c55e;font-weight:700;">&#x2705;</td><td style="text-align:center;color:#22c55e;">&#x2705;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td></tr>
                            <tr><td>VM Management</td><td style="text-align:center;color:#22c55e;font-weight:700;">&#x2705;</td><td style="text-align:center;color:#22c55e;">&#x2705;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;">&#x26A0;&#xFE0F; Basic</td></tr>
                            <tr><td>Container Orchestration</td><td style="text-align:center;color:#22c55e;font-weight:700;">&#x2705; WolfRun</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#22c55e;">&#x2705;</td><td style="text-align:center;">&#x26A0;&#xFE0F; Swarm</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td></tr>
                            <tr><td>Multi-Server Clustering</td><td style="text-align:center;color:#22c55e;font-weight:700;">&#x2705;</td><td style="text-align:center;color:#22c55e;">&#x2705;</td><td style="text-align:center;color:#22c55e;">&#x2705;</td><td style="text-align:center;">&#x26A0;&#xFE0F; Paid</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td></tr>
                            <tr><td>Encrypted Mesh Networking</td><td style="text-align:center;color:#22c55e;font-weight:700;">&#x2705; WolfNet</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;">&#x26A0;&#xFE0F; CNI plugins</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td></tr>
                            <tr><td>Built-in VPN / Remote Access</td><td style="text-align:center;color:#22c55e;font-weight:700;">&#x2705; WolfNet VPN</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td></tr>
                            <tr><td>Web Terminal</td><td style="text-align:center;color:#22c55e;font-weight:700;">&#x2705;</td><td style="text-align:center;color:#22c55e;">&#x2705;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#22c55e;">&#x2705;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#22c55e;">&#x2705;</td></tr>
                            <tr><td>File Manager</td><td style="text-align:center;color:#22c55e;font-weight:700;">&#x2705;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#22c55e;">&#x2705;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td></tr>
                            <tr><td>App Store</td><td style="text-align:center;color:#22c55e;font-weight:700;">&#x2705;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;">&#x26A0;&#xFE0F; Helm</td><td style="text-align:center;color:#22c55e;">&#x2705;</td><td style="text-align:center;color:#22c55e;">&#x2705;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td></tr>
                            <tr><td>AI Agent</td><td style="text-align:center;color:#22c55e;font-weight:700;">&#x2705;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td></tr>
                            <tr><td>Alerting (Discord/Slack/Telegram)</td><td style="text-align:center;color:#22c55e;font-weight:700;">&#x2705;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;">&#x26A0;&#xFE0F; Add-ons</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td></tr>
                            <tr><td>Public Status Pages</td><td style="text-align:center;color:#22c55e;font-weight:700;">&#x2705;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td></tr>
                            <tr><td>Database Editor</td><td style="text-align:center;color:#22c55e;font-weight:700;">&#x2705;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td></tr>
                            <tr><td>Issues Scanner</td><td style="text-align:center;color:#22c55e;font-weight:700;">&#x2705;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td><td style="text-align:center;color:#ef4444;">&#x274C;</td></tr>
                            <tr><td>Install Complexity</td><td style="text-align:center;color:#22c55e;font-weight:700;">1 command</td><td style="text-align:center;">ISO install</td><td style="text-align:center;color:#ef4444;">Very complex</td><td style="text-align:center;">Moderate</td><td style="text-align:center;color:#22c55e;">1 command</td><td style="text-align:center;color:#22c55e;">1 command</td></tr>
                            <tr><td>Written In</td><td style="text-align:center;color:#22c55e;font-weight:700;">Rust &#x1F980;</td><td style="text-align:center;">Perl/C</td><td style="text-align:center;">Go</td><td style="text-align:center;">Go</td><td style="text-align:center;">Go</td><td style="text-align:center;">Python/C</td></tr>
                            <tr><td>Price</td><td style="text-align:center;color:#22c55e;font-weight:700;">Free &amp; Open Source<br><a href="https://www.patreon.com/15362110/join" target="_blank" style="color:#22c55e;font-size:0.8rem;font-weight:500;text-decoration:underline;">Supported by Your Donations</a><br><a href="enterprise.php" style="color:#ff424d;font-size:0.8rem;font-weight:500;text-decoration:underline;">Enterprise Licensing Available</a></td><td style="text-align:center;">Free + Paid</td><td style="text-align:center;">Free</td><td style="text-align:center;">Free + Paid</td><td style="text-align:center;">Free</td><td style="text-align:center;">Free</td></tr>
                        </tbody>
                    </table>
                </div>
            </section>

            <!-- Screenshots -->
            <h2 style="text-align:center;font-size:1.3rem;font-weight:700;margin:2rem 0 0.5rem;">Screenshots</h2>
            <p style="text-align:center;color:var(--text-secondary);font-size:0.9rem;margin-bottom:1.5rem;">See WolfStack in action</p>

            <div style="display:grid;grid-template-columns:repeat(auto-fit,minmax(280px,1fr));gap:1.5rem;margin:0 0 2rem;">
                <div class="feature-card" style="padding:0;overflow:hidden;">
                    <img src="images/node-detail.png" alt="Node Monitoring" style="width:100%;display:block;">
                    <div style="padding:14px 18px;">
                        <h3 style="margin:0 0 4px;">&#x1F4CA; Node Monitoring</h3>
                        <p style="margin:0;">Real-time CPU, memory, disk, and network metrics with interactive graphs</p>
                    </div>
                </div>
                <div class="feature-card" style="padding:0;overflow:hidden;">
                    <img src="images/app-store.png" alt="App Store" style="width:100%;display:block;">
                    <div style="padding:14px 18px;">
                        <h3 style="margin:0 0 4px;">&#x1F6CD;&#xFE0F; App Store</h3>
                        <p style="margin:0;">Deploy containers and applications to any node with one click</p>
                    </div>
                </div>
                <div class="feature-card" style="padding:0;overflow:hidden;">
                    <img src="images/settings-themes.png" alt="Theme Engine" style="width:100%;display:block;">
                    <div style="padding:14px 18px;">
                        <h3 style="margin:0 0 4px;">&#x1F3A8; Theme Engine</h3>
                        <p style="margin:0;">Multiple beautiful themes — Dark, Midnight, Glass, Amber Terminal, and more</p>
                    </div>
                </div>
            </div>

            <!-- Products Section -->
            <section style="padding:2rem 0;">
                <h2 style="text-align:center;font-size:1.5rem;font-weight:700;margin-bottom:0.5rem;">The Wolf Toolkit</h2>
                <p style="text-align:center;color:var(--text-secondary);margin-bottom:2rem;font-size:0.9rem;">Everything you need to build robust, clustered server infrastructure</p>

                <div class="product-grid">
                    <!-- WolfStack -->
                    <div class="product-card flagship">
                        <div class="product-card-header">
                            <span class="product-card-icon">&#x1F4CA;</span>
                            <span class="product-card-title">WolfStack</span>
                            <span class="product-card-badge flagship">&#x2B50; Flagship</span>
                        </div>
                        <p class="product-card-desc">The central management platform for your entire infrastructure. Beautiful dashboard with real-time monitoring, <strong>Docker &amp; LXC container management</strong>, component control, config editing, multi-server clustering, and live logs — all from one premium web interface.</p>
                        <ul class="product-card-features">
                            <li>Real-time CPU, memory, disk &amp; network monitoring</li>
                            <li>Docker &amp; LXC container management with live stats</li>
                            <li>Clone &amp; migrate containers between servers</li>
                            <li>S3, NFS &amp; WolfDisk storage manager</li>
                            <li>Multi-server clustering &amp; fleet management</li>
                            <li>Built-in App Store, Status Pages, Issues Scanner, AI Agent</li>
                        </ul>
                        <div class="product-card-actions"><a href="wolfstack.php" class="btn btn-primary btn-sm">Get Started &rarr;</a></div>
                    </div>

                    <!-- Proxmox -->
                    <div class="product-card">
                        <div class="product-card-header">
                            <span class="product-card-icon">&#x1F7E0;</span>
                            <span class="product-card-title">Proxmox Integration</span>
                            <span class="product-card-badge builtin">Built In</span>
                        </div>
                        <p class="product-card-desc">Install WolfStack on top of your Proxmox servers. WolfStack automatically detects Proxmox and manages your entire VE cluster from the dashboard.</p>
                        <ul class="product-card-features">
                            <li>Install directly on Proxmox VE nodes</li>
                            <li><strong>Auto-detects</strong> Proxmox — no configuration needed</li>
                            <li>VM &amp; LXC management from WolfStack</li>
                            <li>Mix Proxmox and non-Proxmox nodes in one cluster</li>
                        </ul>
                        <div class="product-card-actions"><a href="proxmox.php" class="btn btn-primary btn-sm">View Integration &rarr;</a></div>
                    </div>

                    <!-- WolfScale -->
                    <div class="product-card">
                        <div class="product-card-header">
                            <span class="product-card-icon">&#x1F5C4;&#xFE0F;</span>
                            <span class="product-card-title">WolfScale</span>
                            <span class="product-card-badge available">Available Now</span>
                        </div>
                        <p class="product-card-desc">Database replication, clustering and load balancing — the easy way. WolfScale keeps your MariaDB and MySQL databases synchronised across any number of servers, with automatic failover.</p>
                        <ul class="product-card-features">
                            <li>MariaDB, MySQL, Percona &amp; Amazon RDS support</li>
                            <li>Built-in load balancer with read/write splitting</li>
                            <li>Automatic node discovery &amp; failover</li>
                            <li>Run on 1 to N nodes, geographically distributed</li>
                        </ul>
                        <div class="product-card-actions">
                            <a href="quickstart.php" class="btn btn-primary btn-sm">Get Started &rarr;</a>
                            <a href="features.php" class="btn btn-secondary btn-sm">Learn More</a>
                        </div>
                    </div>

                    <!-- WolfDisk -->
                    <div class="product-card">
                        <div class="product-card-header">
                            <span class="product-card-icon">&#x1F4BE;</span>
                            <span class="product-card-title">WolfDisk</span>
                            <span class="product-card-badge available">Available Now</span>
                        </div>
                        <p class="product-card-desc">Distributed filesystem that shares and replicates files across your network. Mount a shared directory on any number of Linux machines with automatic synchronisation.</p>
                        <ul class="product-card-features">
                            <li>FUSE-based — works as a regular Linux directory</li>
                            <li>Content-addressed deduplication with SHA256</li>
                            <li>S3-compatible API — mount from WolfStack</li>
                            <li>Multiple independent drives per node</li>
                        </ul>
                        <div class="product-card-actions"><a href="wolfdisk.php" class="btn btn-primary btn-sm">Get Started &rarr;</a></div>
                    </div>

                    <!-- WolfNet -->
                    <div class="product-card">
                        <div class="product-card-header">
                            <span class="product-card-icon">&#x1F310;</span>
                            <span class="product-card-title">WolfNet</span>
                            <span class="product-card-badge available">Available Now</span>
                        </div>
                        <p class="product-card-desc">Create a secure private network across the internet. Connect servers across data centres, cloud providers and on-premises infrastructure as if they were on the same local network.</p>
                        <ul class="product-card-features">
                            <li>Encrypted mesh networking (WireGuard-class crypto)</li>
                            <li>Invite/Join — connect peers with a single token</li>
                            <li>Relay forwarding — no port forwarding needed</li>
                            <li>Gateway mode with NAT for internet access</li>
                            <li><strong>Built-in VPN</strong> — remote access from anywhere</li>
                        </ul>
                        <div class="product-card-actions"><a href="wolfnet.php" class="btn btn-primary btn-sm">Get Started &rarr;</a></div>
                    </div>

                    <!-- WolfProxy -->
                    <div class="product-card">
                        <div class="product-card-header">
                            <span class="product-card-icon">&#x1F500;</span>
                            <span class="product-card-title">WolfProxy</span>
                            <span class="product-card-badge available">Available Now</span>
                        </div>
                        <p class="product-card-desc">NGINX-compatible reverse proxy with a built-in firewall. Reads your existing nginx configuration directly — just stop nginx, start WolfProxy.</p>
                        <ul class="product-card-features">
                            <li>Drop-in nginx replacement — reads sites-enabled</li>
                            <li>Built-in firewall with auto-ban</li>
                            <li>5 load balancing algorithms</li>
                            <li>Real-time monitoring dashboard</li>
                        </ul>
                        <div class="product-card-actions"><a href="wolfproxy.php" class="btn btn-primary btn-sm">Get Started &rarr;</a></div>
                    </div>

                    <!-- WolfServe -->
                    <div class="product-card">
                        <div class="product-card-header">
                            <span class="product-card-icon">&#x1F30D;</span>
                            <span class="product-card-title">WolfServe</span>
                            <span class="product-card-badge available">Available Now</span>
                        </div>
                        <p class="product-card-desc">Apache2-compatible web server that serves PHP via FastCGI. Reads your existing Apache vhost configs directly. Includes a PHP FFI bridge to call Rust from PHP.</p>
                        <ul class="product-card-features">
                            <li>Drop-in Apache2 replacement — reads vhosts</li>
                            <li>PHP via FastCGI (php-fpm)</li>
                            <li>Rust FFI bridge for PHP</li>
                            <li>Shared sessions across servers</li>
                        </ul>
                        <div class="product-card-actions"><a href="wolfserve.php" class="btn btn-primary btn-sm">Get Started &rarr;</a></div>
                    </div>

                    <!-- CodeWolf -->
                    <div class="product-card">
                        <div class="product-card-header">
                            <span class="product-card-icon">&#x1F43A;</span>
                            <span class="product-card-title">CodeWolf</span>
                            <span class="product-card-badge available">Available Now</span>
                        </div>
                        <p class="product-card-desc">Software migration service — we migrate your projects from one platform to another, including full documentation builds. Let us handle the heavy lifting.</p>
                        <ul class="product-card-features">
                            <li>Cross-platform project migration</li>
                            <li>Full documentation builds included</li>
                            <li>Minimal downtime transitions</li>
                            <li>Tailored to your stack &amp; workflow</li>
                        </ul>
                        <div class="product-card-actions"><a href="mailto:sales@wolf.uk.com" class="btn btn-primary btn-sm">Contact Sales &rarr;</a></div>
                    </div>
                </div>
            </section>

            <!-- Support Section -->
            <section class="support-section">
                <h2>&#x1F43A; Support the Pack</h2>
                <div class="container">
                    <p>WolfStack is <strong>free and open-source</strong>, funded by the community through Patreon donations. For businesses needing dedicated support, installation and ticketing — <a href="enterprise.php" style="color:#ff424d;font-weight:600;text-decoration:underline;">enterprise licensing</a> is available.</p>
                </div>
                <div class="support-buttons">
                    <a href="https://www.patreon.com/15362110/join" target="_blank" class="btn" style="background:#16a34a;color:white;box-shadow:0 3px 12px rgba(34,197,94,0.25);">
                        <svg width="16" height="16" viewBox="0 0 256 256" fill="white" xmlns="http://www.w3.org/2000/svg"><path d="M232.3 25.63c-29.45-22.29-73.6-22.15-104.94 2.52C102.06 48.12 94.34 78.36 99.6 106.5c-36.94 4.56-66.44 25.6-82.18 55.46C.98 193.24-.24 231.72 15.48 256H38.2c-20.74-28.46-19.78-72.22 6.7-99.5 22.02-22.7 55.14-31.26 84.34-20.18-5.78 38.28 15.04 77.32 51.4 92.9 21.94 9.4 47.84 9.76 69.1-2.96 24.18-14.48 39.76-42.02 39.76-70.6V69.6c.76-17.1-7.8-33.72-21.14-44h-36.08v.03zM216 155.66c0 18.7-10.14 36.34-26.14 45.48-13.68 7.82-31.26 8.92-47.96.44-25.44-12.92-38.94-43.46-30.2-70.84 25.18 14.14 56.62 16.52 84.06 4.58 7.48-3.28 14.26-8.02 20.24-13.44v33.78zM216 92.18c-20.5 22.34-55.04 29.76-83.7 16.38-18.06-8.44-31.92-24.58-36.2-44.3-4.92-22.56 2.58-47.2 21.56-62 24.32-18.94 59.78-20.68 84.2-5.44l.14.1c8.54 5.82 14.02 16.46 14 27.1v68.16z"/></svg>
                        Support on Patreon
                    </a>
                    <a href="enterprise.php" class="btn btn-primary">&#x1F3E2; Enterprise Licensing</a>
                    <a href="https://github.com/wolfsoftwaresystemsltd/WolfScale" target="_blank" class="btn btn-secondary">&#x2B50; Star on GitHub</a>
                    <a href="https://discord.gg/q9qMjHjUQY" target="_blank" class="btn" style="background:#5865F2;color:white;">&#x1F4AC; Join Discord</a>
                </div>
            </section>

            <!-- Documentation Quick Links -->
            <section style="padding:2rem 0;">
                <h2 style="text-align:center;font-size:1.25rem;font-weight:600;margin-bottom:1.5rem;color:var(--text-secondary);">Documentation</h2>
                <div class="docs-grid" style="max-width:700px;margin:0 auto;">
                    <a href="wolfstack.php" class="doc-card"><div class="doc-icon">&#x1F4CA;</div><h3>WolfStack</h3><p>Overview</p></a>
                    <a href="quickstart.php" class="doc-card"><div class="doc-icon">&#x26A1;</div><h3>Quick Start</h3><p>Install &amp; setup</p></a>
                    <a href="wolfstack-containers.php" class="doc-card"><div class="doc-icon">&#x1F4E6;</div><h3>Containers</h3><p>Docker &amp; LXC</p></a>
                    <a href="wolfnet.php" class="doc-card"><div class="doc-icon">&#x1F310;</div><h3>WolfNet</h3><p>Private network</p></a>
                    <a href="features.php" class="doc-card"><div class="doc-icon">&#x2728;</div><h3>Features</h3><p>Full list</p></a>
                </div>
            </section>

            <div class="page-nav">
                <span></span>
                <a href="wolfstack.php">WolfStack Overview &rarr;</a>
            </div>
        </div>

        <footer class="site-footer">
            <div class="footer-inner">
                <span>&copy; 2026 <a href="https://wolf.uk.com" target="_blank">Wolf Software Systems Ltd</a> &bull; FSL-1.1</span>
                <span class="footer-disclaimer">USE AT YOUR OWN RISK. This software is provided &lsquo;as is&rsquo; without warranty of any kind.</span>
            </div>
        </footer>
    </main>
</div>
<script src="script.js?v=20"></script>
</body>
</html>
