<?php
$page_title = '💻 Terminal — WolfStack Docs';
$page_desc = 'Full web-based SSH terminal for any node or container';
$active = 'wolfstack-terminal.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Overview</h2>
                <p>WolfStack includes a full web-based terminal that lets you open SSH sessions to any node or container directly from the dashboard. No separate SSH client needed.</p>
                <h3>Features</h3>
                <ul>
                    <li>Open a shell on any managed node</li>
                    <li>Connect to LXC containers (via <code>lxc-attach</code>)</li>
                    <li>Connect to Docker containers (attempts <code>bash</code>, <code>sh</code>, <code>ash</code> in sequence)</li>
                    <li>Full terminal emulation with colour support</li>
                    <li>Copy &amp; paste support</li>
                    <li>Multiple terminal tabs</li>
                    <li>Resizable terminal window</li>
                </ul>
            </div>

<div class="page-nav"><a href="wolfstack-cron.php" class="prev">&larr; Cron Jobs</a><a href="wolfstack-issues.php" class="next">Issues Scanner &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
