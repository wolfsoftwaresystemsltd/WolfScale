<?php
$page_title = '📂 File Manager — WolfStack Docs';
$page_desc = 'Browse, edit, upload, and download files on any node via the web UI';
$active = 'wolfstack-files.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Overview</h2>
                <p>The built-in File Manager lets you browse the filesystem of any node in your cluster directly from the web dashboard. Navigate directories, view and edit files, upload new files, and download existing ones.</p>
                <h3>Features</h3>
                <ul>
                    <li>Browse any directory on any managed node</li>
                    <li>View file contents with syntax highlighting</li>
                    <li>Edit files in-place with a built-in code editor</li>
                    <li>Upload files via drag &amp; drop</li>
                    <li>Download files from remote nodes</li>
                    <li>Delete, rename, and create files and directories</li>
                    <li>View file permissions, sizes, and modification times</li>
                </ul>
            </div>

<div class="page-nav"><a href="wolfstack-storage.php" class="prev">&larr; Storage</a><a href="wolfstack-networking.php" class="next">Networking &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
