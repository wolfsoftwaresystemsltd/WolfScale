<?php
$page_title = '🏗️ WolfScale Architecture — WolfStack Docs';
$page_desc = 'Technical architecture of WolfScale database replication';
$active = 'architecture.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Architecture Overview</h2>
                <p>WolfScale uses a peer-to-peer architecture where each node connects to the database it manages and communicates with other WolfScale nodes to synchronise changes.</p>
                <h3>Components</h3>
                <ul>
                    <li><strong>Replication Engine</strong> &mdash; Reads binlog events and forwards to peers</li>
                    <li><strong>Peer Discovery</strong> &mdash; Auto-discovers nodes via WolfNet or manual configuration</li>
                    <li><strong>Load Balancer</strong> &mdash; Routes queries to healthy nodes</li>
                    <li><strong>Health Checker</strong> &mdash; Monitors node liveness and replication lag</li>
                    <li><strong>Conflict Resolver</strong> &mdash; Handles concurrent writes to the same rows</li>
                </ul>
            </div>

<div class="page-nav"><a href="features.php" class="prev">&larr; Features</a><a href="how-it-works.php" class="next">How It Works &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
