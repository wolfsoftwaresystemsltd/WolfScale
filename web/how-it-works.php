<?php
$page_title = '⚙️ How WolfScale Works — WolfStack Docs';
$page_desc = 'Understanding WolfScale\'s replication mechanism';
$active = 'how-it-works.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Replication Flow</h2>
                <ol>
                    <li>A write occurs on any node&rsquo;s local database</li>
                    <li>WolfScale captures the change via the binary log</li>
                    <li>The change is forwarded to all peer nodes</li>
                    <li>Each peer applies the change to its local database</li>
                    <li>The load balancer directs reads to any healthy node</li>
                </ol>
                <h3>Consistency Model</h3>
                <p>WolfScale provides <strong>eventual consistency</strong> with configurable conflict resolution. For most workloads, replication lag is under 100ms.</p>
            </div>

<div class="page-nav"><a href="architecture.php" class="prev">&larr; Architecture</a><a href="binlog.php" class="next">Binlog Mode &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
