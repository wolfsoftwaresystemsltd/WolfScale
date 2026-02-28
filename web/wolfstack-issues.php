<?php
$page_title = '⚠️ Issues Scanner — WolfStack Docs';
$page_desc = 'AI-powered proactive server health monitoring and diagnostics';
$active = 'wolfstack-issues.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Overview</h2>
                <p>WolfStack&rsquo;s Issues Scanner proactively scans your fleet for hardware, software, and configuration problems. Issues are categorised by severity (Critical, Warning, Info) and grouped by cluster.</p>
                <h3>What Gets Scanned</h3>
                <ul>
                    <li><strong>CPU</strong> &mdash; High usage, thermal throttling</li>
                    <li><strong>Memory</strong> &mdash; Low available memory, swap usage</li>
                    <li><strong>Disk</strong> &mdash; Low disk space, failing drives, I/O errors</li>
                    <li><strong>Services</strong> &mdash; Failed systemd units, stopped critical services</li>
                    <li><strong>Network</strong> &mdash; Interface errors, unreachable peers</li>
                    <li><strong>Security</strong> &mdash; Outdated packages, open ports</li>
                </ul>
                <h3>Features</h3>
                <ul>
                    <li>Per-node scanning with real-time progress indicators</li>
                    <li>Results grouped by cluster</li>
                    <li>AI-enhanced issue categorisation</li>
                    <li>Suggested remediation steps</li>
                    <li>Fleet-wide summary dashboard</li>
                </ul>
            </div>

<div class="page-nav"><a href="wolfstack-terminal.php" class="prev">&larr; Terminal</a><a href="wolfstack-alerting.php" class="next">Alerting &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
