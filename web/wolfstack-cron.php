<?php
$page_title = '⏰ Cron Jobs — WolfStack Docs';
$page_desc = 'Schedule and manage cron tasks on any node from the web dashboard';
$active = 'wolfstack-cron.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Overview</h2>
                <p>WolfStack&rsquo;s Cron Manager lets you view, create, edit, and delete cron jobs on any node in your cluster from the web dashboard. No more SSH-ing into individual servers to manage scheduled tasks.</p>
                <h3>Features</h3>
                <ul>
                    <li>View all cron jobs for any user on any node</li>
                    <li>Create new cron jobs with a visual schedule builder</li>
                    <li>Edit existing cron entries in-place</li>
                    <li>Delete cron jobs from the dashboard</li>
                    <li>View cron job output and history</li>
                </ul>
            </div>

<div class="page-nav"><a href="wolfstack-certificates.php" class="prev">&larr; Certificates</a><a href="wolfstack-terminal.php" class="next">Terminal &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
