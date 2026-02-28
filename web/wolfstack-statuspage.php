<?php
$page_title = '📟 Status Pages — WolfStack Docs';
$page_desc = 'Built-in uptime monitoring and public status pages with HTTP, TCP, Ping, Container, and WolfRun monitors';
$active = 'wolfstack-statuspage.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Overview</h2>
                <p>WolfStack includes a full-featured uptime monitoring system with public-facing status pages &mdash; all built in, no third-party tools required. Create monitors for your services, build beautiful branded status pages, and keep your users informed with automatic incident tracking.</p>
                <p>Status pages are <strong>cluster-scoped</strong>: each cluster has its own pool of monitors, pages, and incidents. Configuration syncs automatically between nodes in the same cluster.</p>

                <div class="info-box" style="margin-top:1.25rem;">
                    <strong>No extra tools needed</strong> &mdash; WolfStack replaces external monitoring services like UptimeRobot, Betterstack, or Cachet. Your monitoring data stays on your own infrastructure.
                </div>
            </div>

            <div class="content-section">
                <h2>Monitor Types</h2>
                <p>WolfStack supports five types of monitors. All monitors share a global pool and can be assigned to one or more status pages.</p>

                <h3>HTTP Monitor</h3>
                <p>Checks a URL and verifies the response status code. Supports HTTPS with self-signed certificate handling.</p>
                <ul>
                    <li><strong>URL</strong> &mdash; The endpoint to check (e.g. <code>https://api.example.com/health</code>)</li>
                    <li><strong>Expected Status</strong> &mdash; HTTP status code to expect (default: 200)</li>
                    <li><strong>Timeout</strong> &mdash; How long to wait before marking as failed (default: 10s)</li>
                </ul>

                <h3>TCP Monitor</h3>
                <p>Verifies that a TCP port is open and accepting connections. Ideal for databases, mail servers, or any socket-based service.</p>
                <ul>
                    <li><strong>Host</strong> &mdash; Hostname or IP address</li>
                    <li><strong>Port</strong> &mdash; TCP port number to connect to</li>
                </ul>

                <h3>Ping (ICMP) Monitor</h3>
                <p>Simple host reachability check using ICMP ping. Quick way to verify a server is online.</p>
                <ul>
                    <li><strong>Host</strong> &mdash; Hostname or IP address to ping</li>
                </ul>

                <h3>Container Monitor</h3>
                <p>Monitors the running state of Docker or LXC containers directly on your cluster nodes.</p>
                <ul>
                    <li><strong>Container Name</strong> &mdash; Name of the Docker or LXC container</li>
                    <li><strong>Runtime</strong> &mdash; <code>docker</code> or <code>lxc</code></li>
                    <li><strong>Node</strong> &mdash; Optional target node (for multi-node clusters)</li>
                </ul>

                <h3>WolfRun Monitor</h3>
                <p>Monitors orchestrated services deployed through <a href="wolfrun.php">WolfRun</a>. Checks that enough healthy instances are running.</p>
                <ul>
                    <li><strong>Service ID</strong> &mdash; The WolfRun service to monitor</li>
                    <li><strong>Minimum Instances</strong> &mdash; Required number of healthy instances</li>
                </ul>
            </div>

            <div class="content-section">
                <h2>Monitor Configuration</h2>
                <p>Every monitor type shares these common settings:</p>
                <ul>
                    <li><strong>Name</strong> &mdash; Human-readable label shown on status pages</li>
                    <li><strong>Check Interval</strong> &mdash; How often to run checks (default: 60 seconds)</li>
                    <li><strong>Timeout</strong> &mdash; Maximum wait time per check (default: 10 seconds)</li>
                    <li><strong>Enabled</strong> &mdash; Toggle monitoring on/off without deleting the monitor</li>
                </ul>

                <h3>Status Determination</h3>
                <p>Monitor status is calculated automatically from recent check results:</p>
                <ul>
                    <li><strong style="color:#22c55e;">Up</strong> &mdash; Last check was successful</li>
                    <li><strong style="color:#f59e0b;">Degraded</strong> &mdash; Last check failed, but fewer than 3 consecutive failures</li>
                    <li><strong style="color:#ef4444;">Down</strong> &mdash; 3 or more consecutive failures</li>
                    <li><strong style="color:#8888a0;">Unknown</strong> &mdash; No check results available yet</li>
                </ul>
            </div>

            <div class="content-section">
                <h2>Public Status Pages</h2>
                <p>Create beautiful, branded status pages that your users can visit to see real-time service health. Each cluster can have multiple status pages, each with its own URL slug and selection of monitors.</p>

                <h3>Page Settings</h3>
                <ul>
                    <li><strong>Title</strong> &mdash; Page heading shown to visitors</li>
                    <li><strong>URL Slug</strong> &mdash; The public URL path (e.g. <code>/status/my-services</code>)</li>
                    <li><strong>Logo URL</strong> &mdash; Custom logo displayed at the top of the page</li>
                    <li><strong>Footer Text</strong> &mdash; Custom text shown in the page footer</li>
                    <li><strong>Monitors</strong> &mdash; Select which monitors appear on this page</li>
                    <li><strong>Enabled</strong> &mdash; Toggle page visibility on/off</li>
                </ul>

                <h3>Built-in Themes</h3>
                <p>Each status page can use one of 8 built-in themes:</p>
                <ul>
                    <li><strong>Dark</strong> &mdash; Default dark theme</li>
                    <li><strong>Light</strong> &mdash; Clean light appearance</li>
                    <li><strong>Midnight</strong> &mdash; Deep blue/dark</li>
                    <li><strong>Datacenter</strong> &mdash; Infrastructure-inspired</li>
                    <li><strong>Forest</strong> &mdash; Green tones</li>
                    <li><strong>Amber</strong> &mdash; Warm amber accents</li>
                    <li><strong>Glass</strong> &mdash; Translucent glass effect</li>
                    <li><strong>Deep Red</strong> &mdash; Bold red theme</li>
                </ul>

                <h3>Uptime Visualisation</h3>
                <p>Each monitor on the status page displays a <strong>90-day uptime bar chart</strong>. Each bar represents one day, colour-coded by uptime percentage:</p>
                <ul>
                    <li><strong style="color:#22c55e;">Green</strong> &mdash; 99.5% or higher uptime</li>
                    <li><strong style="color:#f59e0b;">Yellow</strong> &mdash; 95% to 99.5% uptime</li>
                    <li><strong style="color:#ef4444;">Red</strong> &mdash; Below 95% uptime</li>
                </ul>
                <p>Overall uptime percentage is calculated and displayed alongside each service name.</p>

                <h3>Public Index</h3>
                <p>An automatic index page at <code>/status</code> lists all enabled status pages, allowing visitors to discover all your public dashboards.</p>
            </div>

            <div class="content-section">
                <h2>Dedicated Status Page Port</h2>
                <p>WolfStack serves public status pages on a <strong>dedicated port (8550)</strong> in addition to the main dashboard port (8553). This allows you to:</p>
                <ul>
                    <li>Point a public domain directly at port 8550 for status pages only</li>
                    <li>Keep the main dashboard on a separate, firewalled port</li>
                    <li>No authentication required &mdash; status pages are designed to be public</li>
                </ul>
                <div class="info-box" style="margin-top:1rem;">
                    <strong>Tip:</strong> Use <a href="wolfproxy.php">WolfProxy</a> to reverse-proxy <code>status.yourdomain.com</code> to port 8550 for a clean public URL with SSL.
                </div>
            </div>

            <div class="content-section">
                <h2>Incident Management</h2>
                <p>WolfStack includes both automatic and manual incident tracking.</p>

                <h3>Automatic Incidents</h3>
                <p>When a monitor transitions to <strong>Degraded</strong> or <strong>Down</strong>, an incident is automatically created. When the monitor returns to <strong>Up</strong>, the incident is automatically resolved. These are clearly marked as auto-created in the admin interface.</p>

                <h3>Manual Incidents</h3>
                <p>Create incidents manually for planned maintenance or issues that aren&rsquo;t detected by monitors. Manual incidents support:</p>
                <ul>
                    <li><strong>Status Workflow</strong> &mdash; Investigating &rarr; Identified &rarr; Monitoring &rarr; Resolved</li>
                    <li><strong>Impact Levels</strong> &mdash; Critical, Major, or Minor</li>
                    <li><strong>Timestamped Updates</strong> &mdash; Add multiple updates as the situation evolves</li>
                    <li><strong>Service Association</strong> &mdash; Link incidents to one or more monitors</li>
                </ul>

                <h3>Public Display</h3>
                <p>Incidents from the last 14 days are shown on public status pages, including the incident title, status badge, impact level, and full update history.</p>
            </div>

            <div class="content-section">
                <h2>Cluster Synchronisation</h2>
                <p>Status page configuration automatically syncs between all nodes in the same cluster. This means:</p>
                <ul>
                    <li>Create a monitor on any node and it appears everywhere in the cluster</li>
                    <li>Check results are aggregated across nodes</li>
                    <li>If a node goes down, other nodes continue serving the status pages</li>
                    <li>Sync happens every 10 seconds via the existing cluster polling mechanism</li>
                </ul>
                <p>Data is cluster-scoped &mdash; nodes in different clusters never overwrite each other&rsquo;s configuration.</p>
            </div>

            <div class="content-section">
                <h2>Quick Start</h2>
                <ol class="numbered-list">
                    <li>
                        <strong>Create a monitor</strong>
                        <p>Go to <strong>Datacenter &rarr; Status Pages &rarr; Monitors</strong> and click <strong>Add Monitor</strong>. Choose a type (HTTP, TCP, Ping, Container, or WolfRun), configure the target, and save.</p>
                    </li>
                    <li>
                        <strong>Create a status page</strong>
                        <p>Switch to the <strong>Pages</strong> tab and click <strong>Add Page</strong>. Give it a title, URL slug, and select the monitors to display. Choose a theme and save.</p>
                    </li>
                    <li>
                        <strong>View your page</strong>
                        <p>Your status page is immediately available at <code>http://your-server:8550/status/your-slug</code>. Share this URL with your users.</p>
                    </li>
                    <li>
                        <strong>Set up a public domain (optional)</strong>
                        <p>Use WolfProxy or any reverse proxy to point <code>status.yourdomain.com</code> at port 8550 for a clean, SSL-secured public URL.</p>
                    </li>
                </ol>
            </div>

            <div class="content-section">
                <h2>Configuration Storage</h2>
                <p>All status page data is stored as JSON files &mdash; no database required:</p>
                <ul>
                    <li><code>/etc/wolfstack/statuspage.json</code> &mdash; Monitors, pages, and incidents</li>
                    <li><code>/etc/wolfstack/statuspage-uptime.json</code> &mdash; 90-day uptime history per monitor</li>
                </ul>
                <p>Configuration can be exported and imported via the API for backup or migration between clusters.</p>
            </div>

<div class="page-nav"><a href="wolfstack-alerting.php" class="prev">&larr; Alerting &amp; Notifications</a><a href="wolfstack-backups.php" class="next">Backup &amp; Restore &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
