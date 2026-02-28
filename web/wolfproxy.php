<?php
$page_title = '🔀 WolfProxy — WolfStack Docs';
$page_desc = 'NGINX-compatible reverse proxy with built-in firewall and auto-ban';
$active = 'wolfproxy.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Overview</h2>
                <p>WolfProxy is an NGINX-compatible reverse proxy with a built-in firewall. It reads your existing nginx configuration directly &mdash; just stop nginx, start WolfProxy.</p>
                <h3>Features</h3>
                <ul>
                    <li><strong>Drop-in nginx replacement</strong> &mdash; Reads <code>sites-enabled</code> configs</li>
                    <li><strong>Built-in firewall</strong> with auto-ban for malicious IPs</li>
                    <li><strong>5 load balancing algorithms</strong> &mdash; Round Robin, Weighted, Least Connections, IP Hash, Random</li>
                    <li><strong>Health checking</strong> for upstream servers</li>
                    <li><strong>Real-time monitoring dashboard</strong></li>
                    <li><strong>SSL/TLS termination</strong></li>
                    <li><strong>WebSocket proxying</strong></li>
                </ul>
                <h3>Installation</h3>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn" onclick="copyCode(this)">Copy</button></div>
                    <pre><code># Stop nginx first
sudo systemctl stop nginx
# Install and start WolfProxy
curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/master/wolfproxy/install.sh | sudo bash</code></pre>
                </div>
            </div>

<div class="page-nav"><a href="wolfnet-global.php" class="prev">&larr; Global WolfNet</a><a href="wolfserve.php" class="next">WolfServe &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
