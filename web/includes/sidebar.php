<?php
// $active should be set before including this file, e.g. $active = 'wolfstack.php';
$active = $active ?? '';
function nav_link($href, $label, $active) {
    $cls = ($href === $active) ? ' class="active"' : '';
    echo '<a href="' . $href . '"' . $cls . '>' . $label . '</a>' . "\n";
}
?>
<div class="sidebar-overlay" id="sidebar-overlay"></div>
<aside class="sidebar" id="sidebar">
    <a href="index.php" class="sidebar-logo"><img src="images/wolfstack-logo.png" alt="WolfStack"></a>
    <nav class="sidebar-nav">
        <div class="nav-section">
            <div class="nav-section-title">WolfStack<span class="nav-desc">Server Management Dashboard</span></div>
            <?php
            nav_link('wolfstack.php', '&#x1F4CA; Overview &amp; Quick Start', $active);
            nav_link('wolfstack-containers.php', '&#x1F4E6; Container Management', $active);
            nav_link('wolfstack-storage.php', '&#x1F4BE; Storage Manager', $active);
            nav_link('wolfstack-files.php', '&#x1F4C1; File Manager', $active);
            nav_link('wolfstack-networking.php', '&#x1F310; Networking', $active);
            nav_link('wolfstack-clustering.php', '&#x1F517; Multi-Server Clustering', $active);
            nav_link('wolfstack-mysql.php', '&#x1F5C4;&#xFE0F; MariaDB/MySQL Editor', $active);
            nav_link('wolfstack-security.php', '&#x1F512; Security', $active);
            nav_link('wolfstack-certificates.php', '&#x1F4DC; Certificates', $active);
            nav_link('wolfstack-cron.php', '&#x23F0; Cron Jobs', $active);
            nav_link('wolfstack-terminal.php', '&#x1F4BB; Terminal', $active);
            nav_link('wolfstack-issues.php', '&#x1F50D; Issues Scanner', $active);
            nav_link('wolfstack-alerting.php', '&#x1F514; Alerting &amp; Notifications', $active);
            nav_link('wolfstack-statuspage.php', '&#x1F4DF; Status Pages', $active);
            nav_link('wolfstack-backups.php', '&#x1F5C3;&#xFE0F; Backup &amp; Restore', $active);
            nav_link('wolfrun.php', '&#x1F43A; WolfRun Orchestration', $active);
            nav_link('proxmox.php', '&#x1F7E0; Proxmox Integration', $active);
            nav_link('app-store.php', '&#x1F6CD;&#xFE0F; App Store', $active);
            nav_link('wolfstack-ai.php', '&#x1F916; AI Agent', $active);
            nav_link('wolfnet-vpn.php', '&#x1F510; Remote Access VPN', $active);
            nav_link('wolfstack-settings.php', '&#x2699;&#xFE0F; Settings', $active);
            nav_link('wolfnet-global.php', '&#x1F30D; Global View', $active);
            ?>
        </div>
        <div class="nav-section">
            <div class="nav-section-title">WolfScale<span class="nav-desc">Database Replication &amp; Load Balancer</span></div>
            <?php
            nav_link('quickstart.php', '&#x26A1; Quick Start', $active);
            nav_link('features.php', '&#x2728; Features', $active);
            nav_link('architecture.php', '&#x1F3D7;&#xFE0F; Architecture', $active);
            nav_link('how-it-works.php', '&#x2699;&#xFE0F; How It Works', $active);
            nav_link('binlog.php', '&#x1F4CB; Binlog Mode', $active);
            nav_link('load-balancer.php', '&#x2696;&#xFE0F; Load Balancer', $active);
            nav_link('configuration.php', '&#x1F527; Configuration', $active);
            nav_link('performance.php', '&#x1F4C8; Performance', $active);
            nav_link('cli.php', '&#x1F5A5;&#xFE0F; CLI Reference', $active);
            nav_link('troubleshooting.php', '&#x1F527; Troubleshooting', $active);
            ?>
        </div>
        <div class="nav-section">
            <div class="nav-section-title">WolfDisk<span class="nav-desc">Disk Replication &amp; Sharing</span></div>
            <?php nav_link('wolfdisk.php', '&#x1F4BF; Overview &amp; Quick Start', $active); ?>
        </div>
        <div class="nav-section">
            <div class="nav-section-title">WolfNet<span class="nav-desc">Easy Private Network</span></div>
            <?php
            nav_link('wolfnet.php', '&#x1F310; Overview &amp; Quick Start', $active);
            nav_link('wolfnet-global.php', '&#x1F30D; Global View', $active);
            nav_link('wolfnet-vpn.php', '&#x1F510; Remote Access VPN', $active);
            ?>
        </div>
        <div class="nav-section">
            <div class="nav-section-title">WolfProxy<span class="nav-desc">NGINX Compatible Proxy with Firewall</span></div>
            <?php nav_link('wolfproxy.php', '&#x1F6E1;&#xFE0F; Overview &amp; Quick Start', $active); ?>
        </div>
        <div class="nav-section">
            <div class="nav-section-title">WolfServe<span class="nav-desc">Apache2 Compatible Web Server</span></div>
            <?php nav_link('wolfserve.php', '&#x1F30E; Overview &amp; Quick Start', $active); ?>
        </div>
        <div class="nav-section">
            <div class="nav-section-title">Company</div>
            <a href="https://wolf.uk.com" target="_blank">&#x1F43A; Wolf Software</a>
            <?php
            nav_link('about.php', '&#x2139;&#xFE0F; About', $active);
            nav_link('roadmap.php', '&#x1F5FA;&#xFE0F; Roadmap', $active);
            nav_link('contact.php', '&#x1F4E7; Contact', $active);
            nav_link('glossary.php', '&#x1F4D6; Glossary', $active);
            nav_link('licensing.php', '&#x1F4C4; Licensing', $active);
            nav_link('enterprise.php', '&#x1F3E2; Enterprise', $active);
            ?>
            <a href="support.php"<?php echo ($active === 'support.php') ? ' class="active"' : ''; ?> style="color:#ff424d;font-weight:700;">&#x2764;&#xFE0F; Support Us</a>
        </div>
    </nav>
    <div class="theme-toggle-wrapper">
        <span class="theme-toggle-label">Theme</span>
        <button class="theme-toggle" id="theme-toggle" aria-label="Toggle theme"><span class="theme-toggle-slider"></span></button>
    </div>
    <div class="sidebar-links">
        <a href="https://github.com/wolfsoftwaresystemsltd/WolfScale" target="_blank">GitHub</a>
        <a href="https://www.patreon.com/15362110/join" target="_blank">Patreon</a>
        <a href="https://discord.gg/q9qMjHjUQY" target="_blank">Discord</a>
        <a href="https://www.reddit.com/r/WolfStack/" target="_blank">Reddit</a>
        <a href="https://wolf.uk.com" target="_blank">wolf.uk.com</a>
    </div>
</aside>
