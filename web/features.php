<?php
$page_title = '✨ WolfScale Features — WolfStack Docs';
$page_desc = 'Complete feature list for WolfScale database replication and load balancing';
$active = 'features.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Core Features</h2>
                <ul>
                    <li><strong>Multi-Master Replication</strong> &mdash; Write to any node, changes propagate everywhere</li>
                    <li><strong>Automatic Failover</strong> &mdash; Nodes detect failures and reroute traffic</li>
                    <li><strong>Built-in Load Balancer</strong> &mdash; Distribute queries across nodes with read/write splitting</li>
                    <li><strong>Binary Log Replication</strong> &mdash; Uses MySQL binlog for efficient, low-latency sync</li>
                    <li><strong>Conflict Resolution</strong> &mdash; Automatic handling of write conflicts</li>
                    <li><strong>Geographic Distribution</strong> &mdash; Run nodes across data centres</li>
                    <li><strong>Zero Configuration</strong> &mdash; Auto-discovery of peers via WolfNet</li>
                    <li><strong>Health Monitoring</strong> &mdash; Real-time node health and lag monitoring</li>
                </ul>
                <h3>Supported Databases</h3>
                <ul>
                    <li>MariaDB 10.3+</li>
                    <li>MySQL 5.7+ / 8.0+</li>
                    <li>Percona Server</li>
                    <li>Amazon RDS for MySQL</li>
                </ul>
            </div>

<div class="page-nav"><a href="quickstart.php" class="prev">&larr; Quick Start</a><a href="architecture.php" class="next">Architecture &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
