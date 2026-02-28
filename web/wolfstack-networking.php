<?php
$page_title = '🌐 Networking — WolfStack Docs';
$page_desc = 'IP management, WolfNet mesh, port forwarding, and firewall rules';
$active = 'wolfstack-networking.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Overview</h2>
                <p>WolfStack includes comprehensive networking tools for managing IPs, WolfNet mesh connections, and container networking across your fleet.</p>
                <h3>Global WolfNet</h3>
                <p>The Global WolfNet view shows all WolfNet IPs and peer connections across your entire infrastructure. See which IPs belong to nodes, LXC containers, Docker containers, and VMs at a glance.</p>
                <h3>Container Networking</h3>
                <ul>
                    <li>Each LXC container gets a bridge IP on <code>lxcbr0</code> for local communication</li>
                    <li>WolfNet IPs (10.10.10.x) enable encrypted cross-node communication</li>
                    <li>Automatic route management &mdash; containers can ping each other across nodes</li>
                    <li>VIP (Virtual IP) support for load balancing</li>
                </ul>
                <h3>WolfNet Mesh</h3>
                <p>WolfNet creates an encrypted mesh network between all your nodes automatically. Uses X25519 + ChaCha20-Poly1305 encryption (WireGuard-class). Works even if machines are on different networks or behind NAT.</p>
            </div>

<div class="page-nav"><a href="wolfstack-files.php" class="prev">&larr; File Manager</a><a href="wolfstack-clustering.php" class="next">Clustering &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
