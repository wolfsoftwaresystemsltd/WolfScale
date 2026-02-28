<?php
$page_title = '🐺 WolfRun Orchestration — WolfStack Docs';
$page_desc = 'Container orchestration — schedule, scale, and manage Docker & LXC services across your cluster';
$active = 'wolfrun.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Overview</h2>
                <p>WolfRun is WolfStack&rsquo;s native container orchestration engine. It lets you define <strong>services</strong> that are automatically scheduled, scaled, and managed across your cluster nodes &mdash; similar to Docker Swarm or Kubernetes, but built into WolfStack and supporting both Docker <em>and</em> LXC containers.</p>
                <h3>Key Features</h3>
                <ul>
                    <li><strong>Service definitions</strong> &mdash; Define a service with an image, port mappings, volumes, and environment variables</li>
                    <li><strong>Docker &amp; LXC support</strong> &mdash; Orchestrate both Docker containers and LXC system containers</li>
                    <li><strong>Auto-scaling</strong> &mdash; Set min/max replicas and WolfRun scales automatically based on load</li>
                    <li><strong>Placement strategies</strong> &mdash; Schedule on any node, prefer a specific node, or require a specific node</li>
                    <li><strong>Restart policies</strong> &mdash; Always, OnFailure, or Never</li>
                    <li><strong>Rolling updates</strong> &mdash; Update service images with zero-downtime rolling deployments</li>
                    <li><strong>Load balancing</strong> &mdash; Built-in load balancing across service instances</li>
                    <li><strong>Cross-node scheduling</strong> &mdash; Containers spread across cluster nodes for high availability</li>
                    <li><strong>Cloning</strong> &mdash; Clone services across nodes with automatic WolfNet IP assignment</li>
                </ul>
            </div>
            <div class="content-section">
                <h2>Creating a Service</h2>
                <p>From the WolfStack dashboard, go to the <strong>WolfRun</strong> section in the sidebar. Click <strong>Create Service</strong> and configure:</p>
                <ul>
                    <li><strong>Name</strong> &mdash; A friendly name for your service</li>
                    <li><strong>Image</strong> &mdash; Docker image (e.g. <code>nginx:latest</code>) or LXC template</li>
                    <li><strong>Runtime</strong> &mdash; Docker or LXC</li>
                    <li><strong>Replicas</strong> &mdash; Number of instances to run</li>
                    <li><strong>Ports</strong> &mdash; Port mappings (e.g. <code>80:80</code>)</li>
                    <li><strong>Volumes</strong> &mdash; Volume mounts</li>
                    <li><strong>Environment</strong> &mdash; Environment variables</li>
                    <li><strong>Placement</strong> &mdash; Where to schedule (any node, preferred node, required node)</li>
                </ul>
                <h3>Scaling</h3>
                <p>Scale services up or down from the dashboard. Set minimum and maximum replica counts, and WolfRun will manage instance creation and destruction across your cluster.</p>
                <h3>LXC Services</h3>
                <p>When using the LXC runtime, WolfRun creates system containers from distribution templates (Debian, Ubuntu, AlmaLinux, Alpine). Each LXC instance gets a WolfNet IP for cross-node communication.</p>
            </div>

<div class="page-nav"><a href="wolfstack-backups.php" class="prev">&larr; Backup &amp; Restore</a><a href="proxmox.php" class="next">Proxmox &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
