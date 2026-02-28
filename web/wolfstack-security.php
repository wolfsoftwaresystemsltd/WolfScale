<?php
$page_title = '🔒 Security — WolfStack Docs';
$page_desc = 'Linux PAM authentication, API tokens, and security best practices';
$active = 'wolfstack-security.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Overview</h2>
                <p>WolfStack uses Linux PAM authentication &mdash; log in with your existing Linux user credentials. All inter-node communication is encrypted via WolfNet and authenticated with cluster secrets.</p>
                <h3>Authentication</h3>
                <ul>
                    <li><strong>Linux PAM</strong> &mdash; Log in with your Linux username and password</li>
                    <li><strong>Cluster secrets</strong> &mdash; Automatically generated for inter-node API authentication</li>
                    <li><strong>Join tokens</strong> &mdash; One-time tokens for adding new nodes</li>
                </ul>
                <h3>Encryption</h3>
                <ul>
                    <li>WolfNet encrypts all inter-node traffic with X25519 + ChaCha20-Poly1305</li>
                    <li>HTTPS support for the dashboard</li>
                    <li>API tokens for programmatic access</li>
                </ul>
                <h3>Best Practices</h3>
                <ul>
                    <li>Use strong Linux passwords or SSH key-only auth</li>
                    <li>Keep WolfStack behind a firewall or VPN</li>
                    <li>Regularly update to the latest version</li>
                    <li>Use WolfNet to keep management traffic off the public internet</li>
                </ul>
            </div>

<div class="page-nav"><a href="wolfstack-mysql.php" class="prev">&larr; MySQL Editor</a><a href="wolfstack-certificates.php" class="next">Certificates &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
