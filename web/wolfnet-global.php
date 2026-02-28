<?php
$page_title = '🌍 Global View — WolfStack Docs';
$page_desc = 'Global View is WolfStack\'s fleet dashboard — manage all Docker containers, LXC containers, and VMs across every node in your cluster from one unified interface.';
$active = 'wolfnet-global.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


                <div class="content-section">
                    <h2>Overview</h2>
                    <p>Global View is WolfStack's fleet management dashboard. It scans all nodes in your cluster and presents every Docker container, LXC container, and VM in a single unified table &mdash; organised by <strong>cluster &rarr; server &rarr; containers &rarr; VMs</strong>. You get full control over every workload without switching between individual node pages.</p>
                    <div class="info-box" style="background:rgba(16,185,129,0.08);border-left:4px solid #10b981;padding:12px 16px;border-radius:6px;margin:16px 0;">
                        <strong>Formerly "Global WolfNet"</strong> &mdash; this view was renamed and expanded in v11.18. It now goes far beyond WolfNet IP scanning to serve as a full fleet management console.
                    </div>
                </div>

                <div class="content-section">
                    <h2>Accessing Global View</h2>
                    <ol>
                        <li>Open the WolfStack dashboard on any node</li>
                        <li>In the left sidebar under <strong>Datacenter</strong>, click <strong>Global View</strong></li>
                        <li>Click <strong>Scan Network</strong> to discover all WolfNet peers and node IPs</li>
                        <li>The <strong>Fleet Containers &amp; VMs</strong> section loads automatically after the scan</li>
                    </ol>
                </div>

                <div class="content-section">
                    <h2>Fleet Dashboard</h2>
                    <p>After a scan, Global View shows a unified table of every workload across your infrastructure:</p>

                    <h3>Table Hierarchy</h3>
                    <p>Containers and VMs are grouped under their parent node and cluster:</p>
                    <pre style="background:var(--bg-secondary);padding:16px;border-radius:8px;font-size:13px;line-height:1.6;overflow-x:auto;">
🏢 <strong>Production Cluster</strong>
  🖥️ <strong>web-server-01</strong> (10.10.10.1)
      🐳 Docker  nginx-proxy       ● running   172.17.0.2
      🐳 Docker  redis-cache        ● running   172.17.0.3
      📦 LXC     app-backend        ● running   10.10.10.5
  🖥️ <strong>db-server-01</strong> (10.10.10.2)
      📦 LXC     mariadb-primary    ● running   10.10.10.6
      🖥️ VM      windows-dev        ● stopped   10.10.10.10</pre>
                    <p>Cluster headers only appear when you have multiple clusters. Single-cluster setups show server headers directly.</p>

                    <h3>Live Stats Bars</h3>
                    <p>Each container row has a stats sub-row showing CPU, memory, and disk usage as coloured bar charts:</p>
                    <table class="data-table" style="margin:16px 0;">
                        <thead><tr><th>Colour</th><th>CPU</th><th>Memory / Disk</th></tr></thead>
                        <tbody>
                            <tr><td><span style="color:#10b981;font-weight:bold;">Green</span></td><td>&lt; 50%</td><td>&lt; 70%</td></tr>
                            <tr><td><span style="color:#f59e0b;font-weight:bold;">Amber</span></td><td>50 &ndash; 80%</td><td>70 &ndash; 90%</td></tr>
                            <tr><td><span style="color:#ef4444;font-weight:bold;">Red</span></td><td>&gt; 80%</td><td>&gt; 90%</td></tr>
                        </tbody>
                    </table>
                    <p>VMs display static resource info (vCPU count, memory, disk size) rather than live percentages.</p>
                </div>

                <div class="content-section">
                    <h2>Container &amp; VM Controls</h2>
                    <p>Global View includes the <strong>full set of action buttons</strong> for every workload &mdash; exactly the same controls available on individual node pages.</p>

                    <h3>Docker Containers</h3>
                    <table class="data-table" style="margin:16px 0;">
                        <thead><tr><th>Button</th><th>Action</th><th>Available When</th></tr></thead>
                        <tbody>
                            <tr><td>▶️</td><td>Start / Unpause</td><td>Stopped or paused</td></tr>
                            <tr><td>⏹️</td><td>Stop</td><td>Running</td></tr>
                            <tr><td>🔄</td><td>Restart</td><td>Running</td></tr>
                            <tr><td>⏸️</td><td>Pause</td><td>Running</td></tr>
                            <tr><td>💻</td><td>Open terminal console</td><td>Running</td></tr>
                            <tr><td>🗑️</td><td>Remove container</td><td>Stopped</td></tr>
                            <tr><td>📜</td><td>View logs</td><td>Always</td></tr>
                            <tr><td>📁</td><td>Browse volumes</td><td>Always</td></tr>
                            <tr><td>📂</td><td>Browse container files</td><td>Always</td></tr>
                            <tr><td>⚙️</td><td>Container settings</td><td>Always</td></tr>
                            <tr><td>📋</td><td>Clone container</td><td>Always</td></tr>
                            <tr><td>🚀</td><td>Migrate to another node</td><td>Always</td></tr>
                        </tbody>
                    </table>

                    <h3>LXC Containers</h3>
                    <table class="data-table" style="margin:16px 0;">
                        <thead><tr><th>Button</th><th>Action</th><th>Available When</th></tr></thead>
                        <tbody>
                            <tr><td>▶️</td><td>Start</td><td>Stopped</td></tr>
                            <tr><td>⏹️</td><td>Stop</td><td>Running</td></tr>
                            <tr><td>🔄</td><td>Restart</td><td>Running</td></tr>
                            <tr><td>⏸️</td><td>Freeze</td><td>Running</td></tr>
                            <tr><td>💻</td><td>Open terminal console</td><td>Running</td></tr>
                            <tr><td>🗑️</td><td>Destroy container</td><td>Stopped</td></tr>
                            <tr><td>📜</td><td>View logs</td><td>Always</td></tr>
                            <tr><td>📂</td><td>Browse container files</td><td>Always</td></tr>
                            <tr><td>⚙️</td><td>Container settings</td><td>Always</td></tr>
                            <tr><td>📋</td><td>Clone container</td><td>Always</td></tr>
                            <tr><td>🚀</td><td>Migrate to another node</td><td>Always</td></tr>
                            <tr><td>📦</td><td>Export container</td><td>Always</td></tr>
                        </tbody>
                    </table>

                    <h3>Virtual Machines</h3>
                    <table class="data-table" style="margin:16px 0;">
                        <thead><tr><th>Button</th><th>Action</th><th>Available When</th></tr></thead>
                        <tbody>
                            <tr><td>▶️</td><td>Start VM</td><td>Stopped</td></tr>
                            <tr><td>⏹️</td><td>Stop VM</td><td>Running</td></tr>
                            <tr><td>🖥️</td><td>Open VNC console</td><td>Running (with VNC)</td></tr>
                            <tr><td>⚙️</td><td>VM settings</td><td>Stopped</td></tr>
                            <tr><td>🗑️</td><td>Delete VM</td><td>Stopped</td></tr>
                            <tr><td>📋</td><td>View logs</td><td>Always</td></tr>
                        </tbody>
                    </table>
                </div>

                <div class="content-section">
                    <h2>Cross-Node Operations</h2>
                    <p>All actions work transparently across remote nodes. WolfStack automatically routes commands through its node proxy system:</p>
                    <ul>
                        <li><strong>Local node</strong> &mdash; API calls go directly to <code>/api/...</code></li>
                        <li><strong>Remote nodes</strong> &mdash; API calls are proxied through <code>/api/nodes/{id}/proxy/...</code></li>
                    </ul>
                    <p>Console and VNC sessions connect directly to the correct host for the best experience. Logs, volumes, file browsing, settings, clone, migrate, and export operations all route through the proxy automatically.</p>
                </div>

                <div class="content-section">
                    <h2>WolfNet IP Scanning</h2>
                    <p>Global View also scans the WolfNet overlay network across all nodes, giving you a table of:</p>
                    <ul>
                        <li>Every WolfNet peer and its assigned IP address</li>
                        <li>Connection status per peer</li>
                        <li>Cluster membership</li>
                        <li>Summary stats &mdash; total nodes, WolfNet peers, LXC containers, Docker containers, and VMs</li>
                    </ul>
                </div>

                <div class="content-section">
                    <h2>Lazy Loading &amp; Performance</h2>
                    <p>Fetching container stats from every node can be slow, especially on large clusters. Global View handles this with:</p>
                    <ul>
                        <li><strong>Per-node loading placeholders</strong> &mdash; each server shows a spinner while its data is fetched, and results appear as they arrive</li>
                        <li><strong>Parallel fetching</strong> &mdash; all nodes are queried simultaneously, not one at a time</li>
                        <li><strong>Activity indicator</strong> &mdash; a pulsing red dot appears in the bottom-left corner whenever an action is in-flight, so you always know something is working</li>
                        <li><strong>Button loading states</strong> &mdash; when you click an action button, it shows a spinner and all buttons in that row are disabled until the operation completes</li>
                    </ul>
                </div>

                <div class="content-section">
                    <h2>Requirements</h2>
                    <ul>
                        <li>WolfStack v11.18 or later</li>
                        <li>Multi-node features require <a href="wolfstack-clustering.php">clustering</a> to be configured</li>
                        <li>WolfNet IP scanning requires <a href="wolfnet.php">WolfNet</a> to be installed on your nodes</li>
                        <li>Single-node setups work too &mdash; Global View will show all local containers and VMs</li>
                    </ul>
                </div>

                <div class="page-nav"><a href="wolfnet.php" class="prev">&larr; WolfNet</a><a href="wolfnet-vpn.php"
                        class="next">Remote Access VPN &rarr;</a></div>
            
    </main>
<?php include 'includes/footer.php'; ?>
