<?php
$page_title = '💾 Storage Manager — WolfStack Docs';
$page_desc = 'Manage S3/R2, NFS, and WolfDisk storage mounts from the dashboard';
$active = 'wolfstack-storage.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Overview</h2>
                <p>WolfStack&rsquo;s Storage Manager lets you mount and manage remote storage directly from the dashboard. Supports S3-compatible object storage (AWS S3, Cloudflare R2, MinIO), NFS shares, and WolfDisk distributed filesystems.</p>
                <h3>Supported Storage Types</h3>
                <ul>
                    <li><strong>S3/R2 Object Storage</strong> &mdash; Mount buckets from AWS S3, Cloudflare R2, MinIO, or any S3-compatible provider</li>
                    <li><strong>NFS Shares</strong> &mdash; Mount NFS exports from your network</li>
                    <li><strong>WolfDisk</strong> &mdash; Mount WolfDisk distributed filesystem drives</li>
                </ul>
                <h3>Features</h3>
                <ul>
                    <li>View storage usage and capacity at a glance</li>
                    <li>Browse files in mounted storage via the File Manager</li>
                    <li>Configure credentials and mount points from the web UI</li>
                    <li>Auto-mount on boot</li>
                </ul>
            </div>

<div class="page-nav"><a href="wolfstack-containers.php" class="prev">&larr; Containers</a><a href="wolfstack-files.php" class="next">File Manager &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
