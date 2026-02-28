<?php
$page_title = '🟠 Proxmox Integration — WolfStack Docs';
$page_desc = 'Install WolfStack on Proxmox to manage VE clusters from the WolfStack dashboard';
$active = 'proxmox.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Overview</h2>
                <p>WolfStack installs directly on top of your Proxmox VE servers. Once installed, WolfStack automatically detects that it&rsquo;s running on Proxmox and manages your entire Proxmox cluster &mdash; VMs, LXC containers, storage, and networking &mdash; from the WolfStack dashboard.</p>
                <h3>How It Works</h3>
                <p>Simply install WolfStack on each Proxmox node using the standard installer. WolfStack detects Proxmox automatically and integrates with it, giving you a unified dashboard for all your infrastructure.</p>
                <h3>Features</h3>
                <ul>
                    <li>Install WolfStack directly on Proxmox VE nodes</li>
                    <li>Automatic Proxmox detection &mdash; no extra configuration</li>
                    <li>View and manage all VMs and LXC containers</li>
                    <li>Start, stop, and restart VMs and containers</li>
                    <li>View real-time resource metrics for each VM and container</li>
                    <li>Automatic cluster grouping and discovery</li>
                    <li>Manage Proxmox and non-Proxmox nodes together in one dashboard</li>
                </ul>
                <h3>LXC Container Features</h3>
                <p>WolfStack manages Proxmox LXC container features via <code>pct set</code>. When installing applications like WolfDisk through the App Store, required features (TUN, FUSE) are enabled automatically. You can also toggle them manually from the container's Settings page in WolfStack:</p>
                <ul>
                    <li><strong>FUSE</strong> — Mapped to Proxmox <code>--features fuse=1</code></li>
                    <li><strong>TUN/TAP</strong> — Mapped to Proxmox <code>/dev/net/tun</code> device passthrough</li>
                    <li><strong>Nesting</strong> — Mapped to Proxmox <code>--features nesting=1</code></li>
                    <li><strong>Keyctl</strong> — Mapped to Proxmox <code>--features keyctl=1</code></li>
                </ul>
                <h3>Getting Started</h3>
                <ol>
                    <li>Install WolfStack on each Proxmox node:
                        <div class="code-block" style="margin:0.5rem 0;">
                            <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn" onclick="copyCode(this)">Copy</button></div>
                            <pre><code>curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfStack/master/setup.sh | sudo bash</code></pre>
                        </div>
                    </li>
                    <li>WolfStack automatically detects Proxmox and begins managing the cluster</li>
                    <li>Open the WolfStack dashboard and your Proxmox VMs and containers appear alongside your other nodes</li>
                    <li>Add additional Proxmox or WolfStack nodes using the <strong>+</strong> button</li>
                </ol>
            </div>

<div class="page-nav"><a href="wolfrun.php" class="prev">&larr; WolfRun</a><a href="app-store.php" class="next">App Store &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
