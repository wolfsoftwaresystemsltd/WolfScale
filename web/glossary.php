<?php
$page_title = '📖 Glossary — WolfStack Docs';
$page_desc = 'Common terms and concepts used in WolfStack documentation';
$active = 'glossary.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Terms &amp; Definitions</h2>
                <h3>A&ndash;C</h3>
                <ul>
                    <li><strong>Cluster</strong> &mdash; A group of servers managed together in WolfStack</li>
                    <li><strong>Container</strong> &mdash; An isolated environment (Docker or LXC) running on a node</li>
                </ul>
                <h3>D&ndash;L</h3>
                <ul>
                    <li><strong>Docker</strong> &mdash; Application container runtime</li>
                    <li><strong>LXC</strong> &mdash; Linux Containers &mdash; system-level containerisation</li>
                    <li><strong>lxcbr0</strong> &mdash; The default LXC bridge network interface</li>
                </ul>
                <h3>N&ndash;P</h3>
                <ul>
                    <li><strong>Node</strong> &mdash; A single server managed by WolfStack</li>
                    <li><strong>PAM</strong> &mdash; Pluggable Authentication Modules &mdash; Linux authentication system</li>
                    <li><strong>Proxmox VE</strong> &mdash; A virtualisation management platform that WolfStack can integrate with</li>
                </ul>
                <h3>V&ndash;W</h3>
                <ul>
                    <li><strong>VIP</strong> &mdash; Virtual IP &mdash; A floating IP address for load balancing</li>
                    <li><strong>WolfDisk</strong> &mdash; Distributed filesystem for file sharing and replication</li>
                    <li><strong>WolfNet</strong> &mdash; Encrypted mesh networking between nodes</li>
                    <li><strong>WolfProxy</strong> &mdash; NGINX-compatible reverse proxy with firewall</li>
                    <li><strong>WolfScale</strong> &mdash; Database replication and load balancing</li>
                    <li><strong>WolfServe</strong> &mdash; Apache2-compatible web server</li>
                    <li><strong>WolfStack</strong> &mdash; The central server management platform</li>
                    <li><strong>wolfnet0</strong> &mdash; The WolfNet TUN interface on each node</li>
                </ul>
            </div>

<div class="page-nav"><a href="contact.php" class="prev">&larr; Contact</a><a href="licensing.php" class="next">Licensing &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
