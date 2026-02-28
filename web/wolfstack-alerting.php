<?php
$page_title = '🔔 Alerting &amp; Notifications — WolfStack Docs';
$page_desc = 'Discord, Slack, and Telegram notifications for infrastructure events';
$active = 'wolfstack-alerting.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Overview</h2>
                <p>WolfStack can send alerts to Discord, Slack, or Telegram when resource thresholds are exceeded. Configure alert rules from the Settings page.</p>
                <h3>Supported Channels</h3>
                <ul>
                    <li><strong>Discord</strong> &mdash; Webhook integration</li>
                    <li><strong>Slack</strong> &mdash; Webhook integration</li>
                    <li><strong>Telegram</strong> &mdash; Bot API integration</li>
                </ul>
                <h3>Alert Types</h3>
                <ul>
                    <li>CPU usage exceeds threshold</li>
                    <li>Memory usage exceeds threshold</li>
                    <li>Disk usage exceeds threshold</li>
                    <li>Node goes offline</li>
                    <li>Container stops unexpectedly</li>
                </ul>
                <h3>Configuration</h3>
                <p>Go to <strong>Settings &rarr; Alerting</strong> to configure webhook URLs and thresholds. You can test your alerting configuration before enabling it.</p>
            </div>

<div class="page-nav"><a href="wolfstack-issues.php" class="prev">&larr; Issues Scanner</a><a href="wolfstack-statuspage.php" class="next">Status Pages &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
