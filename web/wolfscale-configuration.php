<?php
$page_title = '🔧 WolfScale Configuration — WolfStack Docs';
$page_desc = 'Complete configuration reference for WolfScale';
$active = 'configuration.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Configuration File</h2>
                <p>WolfScale is configured via <code>/etc/wolfscale/config.toml</code>.</p>
                <h3>Database Section</h3>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">toml</span><button class="copy-btn" onclick="copyCode(this)">Copy</button></div>
                    <pre><code>[database]
host = "127.0.0.1"
port = 3306
user = "replicator"
password = "secure-password"
database = "mydb"</code></pre>
                </div>
                <h3>Cluster Section</h3>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">toml</span><button class="copy-btn" onclick="copyCode(this)">Copy</button></div>
                    <pre><code>[cluster]
name = "production"
node_id = "node-1"</code></pre>
                </div>
                <h3>Load Balancer Section</h3>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">toml</span><button class="copy-btn" onclick="copyCode(this)">Copy</button></div>
                    <pre><code>[loadbalancer]
listen = "0.0.0.0:3307"
algorithm = "round_robin"</code></pre>
                </div>
            </div>

<div class="page-nav"><a href="load-balancer.php" class="prev">&larr; Load Balancer</a><a href="performance.php" class="next">Performance &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
