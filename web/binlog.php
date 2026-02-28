<?php
$page_title = '📋 Binlog Mode — WolfStack Docs';
$page_desc = 'Binary log replication for efficient database synchronisation';
$active = 'binlog.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Binary Log Replication</h2>
                <p>WolfScale supports MySQL/MariaDB binary log replication for efficient, low-latency synchronisation. This is the primary replication method and is recommended for production use.</p>
                <h3>Requirements</h3>
                <ul>
                    <li>Binary logging must be enabled: <code>log_bin = ON</code></li>
                    <li>Row-based replication format: <code>binlog_format = ROW</code></li>
                    <li>A replication user with appropriate privileges</li>
                </ul>
                <h3>Configuration</h3>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">ini</span><button class="copy-btn" onclick="copyCode(this)">Copy</button></div>
                    <pre><code># my.cnf
[mysqld]
log_bin = ON
binlog_format = ROW
server_id = 1
gtid_mode = ON
enforce_gtid_consistency = ON</code></pre>
                </div>
            </div>

<div class="page-nav"><a href="how-it-works.php" class="prev">&larr; How It Works</a><a href="load-balancer.php" class="next">Load Balancer &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
