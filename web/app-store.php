<?php
$page_title = '🛍️ App Store — WolfStack Docs';
$page_desc = 'Deploy containers and applications to any node with one click';
$active = 'app-store.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Overview</h2>
                <p>WolfStack&rsquo;s built-in App Store lets you deploy preconfigured containers and applications to any node in your cluster with a single click.</p>
                <img src="images/app-store.png" alt="WolfStack App Store" style="width:100%;border-radius:10px;margin:1.5rem 0;border:1px solid var(--border-color);box-shadow:0 8px 32px rgba(0,0,0,0.3);">
                <h3>Features</h3>
                <ul>
                    <li>Browse a catalogue of preconfigured applications</li>
                    <li>Deploy to any node with one click</li>
                    <li>Automatic container creation and configuration</li>
                    <li>Choose target node and resource allocation</li>
                    <li>Popular apps: WordPress, Nginx, MariaDB, Redis, and more</li>
                </ul>
            </div>

            <div class="content-section">
                <h2>LXC Container Requirements</h2>
                <p>Some applications require special LXC features to be enabled in the container. When installing via the App Store, WolfStack enables these <strong>automatically</strong>. If you are installing manually or the container was created outside the App Store, you may need to enable them yourself.</p>
                <table>
                    <thead>
                        <tr><th>Application</th><th>TUN/TAP</th><th>FUSE</th><th>Nesting</th></tr>
                    </thead>
                    <tbody>
                        <tr><td><strong>WolfDisk</strong></td><td>✅ Required</td><td>✅ Required</td><td>—</td></tr>
                        <tr><td><strong>WireGuard</strong></td><td>✅ Required</td><td>—</td><td>—</td></tr>
                        <tr><td><strong>Tailscale</strong></td><td>✅ Required</td><td>—</td><td>—</td></tr>
                        <tr><td><strong>Supabase</strong></td><td>—</td><td>—</td><td>✅ Required (Docker)</td></tr>
                    </tbody>
                </table>
                <p>To enable features manually: go to the container → <strong>Settings</strong> → toggle the required features → save → restart the container.</p>
            </div>

<div class="page-nav"><a href="proxmox.php" class="prev">&larr; Proxmox</a><a href="wolfstack-ai.php" class="next">AI Agent &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
