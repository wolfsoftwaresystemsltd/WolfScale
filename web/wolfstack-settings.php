<?php
$page_title = '⚙️ Settings — WolfStack Docs';
$page_desc = 'Configure themes, alerting, Docker registries, node settings, and more';
$active = 'wolfstack-settings.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Overview</h2>
                <p>The Settings page is your central configuration hub for WolfStack. Access it from the gear icon in the bottom-left of the sidebar.</p>
                <img src="images/settings-themes.png" alt="WolfStack Settings" style="width:100%;border-radius:10px;margin:1.5rem 0;border:1px solid var(--border-color);box-shadow:0 8px 32px rgba(0,0,0,0.3);">
            </div>
            <div class="content-section">
                <h2>Themes</h2>
                <p>WolfStack includes multiple beautiful themes. Switch between them on the Themes tab:</p>
                <ul>
                    <li><strong>WolfStack Dark</strong> &mdash; The default dark theme with red accents</li>
                    <li><strong>Midnight</strong> &mdash; Deep blue/black theme</li>
                    <li><strong>Glass</strong> &mdash; Glassmorphism with frosted effects and deep shadows</li>
                    <li><strong>Amber Terminal</strong> &mdash; Retro amber-on-black terminal aesthetic</li>
                    <li><strong>Light</strong> &mdash; Clean light theme</li>
                </ul>
            </div>
            <div class="content-section">
                <h2>Alerting &amp; Notifications</h2>
                <p>Configure alert thresholds and notification channels from the <strong>Alerting</strong> tab in Settings.</p>
                <h3>Notification Channels</h3>
                <ul>
                    <li><strong>Discord</strong> &mdash; Send alerts to a Discord channel via webhook URL</li>
                    <li><strong>Slack</strong> &mdash; Send alerts to a Slack channel via webhook URL</li>
                    <li><strong>Telegram</strong> &mdash; Send alerts via Telegram Bot API</li>
                </ul>
                <h3>Alert Thresholds</h3>
                <ul>
                    <li><strong>CPU</strong> &mdash; Alert when CPU usage exceeds a percentage</li>
                    <li><strong>Memory</strong> &mdash; Alert when memory usage exceeds a percentage</li>
                    <li><strong>Disk</strong> &mdash; Alert when disk usage exceeds a percentage</li>
                </ul>
                <p>Alerts are checked on a configurable interval (default: 60 seconds). One node in the cluster is automatically elected as the primary alerter to avoid duplicate notifications.</p>
                <p>You can <strong>test</strong> your configuration before enabling it to verify webhooks are working correctly.</p>
            </div>
            <div class="content-section">
                <h2>Docker Registry</h2>
                <p>Configure private Docker registry credentials so WolfStack can pull images from your private registries. Add registry URLs, usernames, and passwords/tokens from the <strong>Docker</strong> tab.</p>
            </div>
            <div class="content-section">
                <h2>Node Settings</h2>
                <p>Per-node configuration is available by clicking the gear icon on any node in the sidebar. Node settings include:</p>
                <ul>
                    <li><strong>Hostname &amp; Address</strong> &mdash; View and modify node connection details</li>
                    <li><strong>Cluster assignment</strong> &mdash; Move nodes between clusters</li>
                    <li><strong>WolfStack version</strong> &mdash; View installed and latest versions, upgrade from the dashboard</li>
                    <li><strong>WolfNet configuration</strong> &mdash; View and manage WolfNet peer details</li>
                    <li><strong>Delete node</strong> &mdash; Remove a node from the cluster</li>
                </ul>
            </div>
            <div class="content-section">
                <h2>Cluster Settings</h2>
                <ul>
                    <li><strong>Cluster name</strong> &mdash; Rename your cluster</li>
                    <li><strong>WolfNet connections</strong> &mdash; Click <strong>🔗 Update WolfNet Connections</strong> to automatically configure encrypted mesh networking between all nodes in the cluster</li>
                    <li><strong>Cluster secret</strong> &mdash; View or regenerate the cluster authentication secret</li>
                </ul>
            </div>

<div class="page-nav"><a href="wolfstack-ai.php" class="prev">&larr; AI Agent</a><a href="index.php" class="next">Home &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
