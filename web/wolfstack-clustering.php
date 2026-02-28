<?php
$page_title = '🔗 Multi-Server Clustering — WolfStack Docs';
$page_desc = 'Join servers into managed clusters with automatic discovery and fleet-wide management';
$active = 'wolfstack-clustering.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Overview</h2>
                <p>WolfStack&rsquo;s clustering feature lets you group servers into logical clusters and manage them as a single unit. Add WolfStack nodes, Proxmox servers, or a mix of both.</p>
                <h3>How It Works</h3>
                <ol>
                    <li>Install WolfStack on each server you want to manage</li>
                    <li>Log in to any one server&rsquo;s web UI</li>
                    <li>Click <strong>+</strong> to add nodes by entering their join token</li>
                    <li>Click <strong>🔗 Update WolfNet Connections</strong> to set up encrypted networking</li>
                </ol>
                <h3>Features</h3>
                <ul>
                    <li>Multiple clusters in one dashboard</li>
                    <li>Fleet-wide metrics and health monitoring</li>
                    <li>Cross-cluster container migration</li>
                    <li>Centralised settings and configuration</li>
                    <li>Node health checks with automatic offline detection</li>
                    <li>Per-cluster WolfNet configuration</li>
                </ul>
            </div>

<div class="page-nav"><a href="wolfstack-networking.php" class="prev">&larr; Networking</a><a href="wolfstack-mysql.php" class="next">MySQL Editor &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
