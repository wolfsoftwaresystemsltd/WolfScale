<?php
$page_title = '⚖️ Load Balancer — WolfStack Docs';
$page_desc = 'Built-in load balancer with read/write splitting';
$active = 'load-balancer.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Overview</h2>
                <p>WolfScale includes a built-in load balancer that distributes database queries across healthy nodes. It supports read/write splitting, health checking, and multiple routing algorithms.</p>
                <h3>Features</h3>
                <ul>
                    <li><strong>Read/Write Splitting</strong> &mdash; Writes go to the primary, reads distribute across all nodes</li>
                    <li><strong>Health Checking</strong> &mdash; Automatically removes unhealthy nodes from the pool</li>
                    <li><strong>Connection Pooling</strong> &mdash; Efficient connection management</li>
                    <li><strong>Weighted Routing</strong> &mdash; Assign weights to prefer certain nodes</li>
                </ul>
                <h3>Usage</h3>
                <p>Connect your application to WolfScale&rsquo;s load balancer port (default: 3307) instead of directly to MySQL. WolfScale handles the routing.</p>
            </div>

<div class="page-nav"><a href="binlog.php" class="prev">&larr; Binlog Mode</a><a href="configuration.php" class="next">Configuration &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
