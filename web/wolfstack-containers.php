<?php
$page_title = '📦 Container Management — WolfStack Docs';
$page_desc = 'Create, manage, clone, and migrate Docker & LXC containers across your fleet';
$active = 'wolfstack-containers.php';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


            <div class="content-section">
                <h2>Overview</h2>
                <p>WolfStack provides comprehensive container management for both <strong>Docker</strong> and <strong>LXC</strong> containers. Create, start, stop, restart, clone, and migrate containers across your entire fleet from a single dashboard.</p>
                <h3>Docker Containers</h3>
                <ul>
                    <li>View all running Docker containers with live CPU, memory, and network stats</li>
                    <li>Start, stop, restart, and remove containers</li>
                    <li>View real-time container logs</li>
                    <li>Open a web terminal (shell) into any container</li>
                    <li>Pull images and create containers from the App Store</li>
                    <li>Manage Docker volumes and networks</li>
                </ul>
                <h3>LXC Containers</h3>
                <ul>
                    <li>Create system containers from downloadable templates</li>
                    <li>Full lifecycle management: start, stop, freeze, unfreeze, destroy</li>
                    <li>Clone containers locally or across nodes</li>
                    <li>Migrate containers between servers in one click</li>
                    <li>Edit container configuration directly</li>
                    <li>Assign WolfNet IPs for cross-node communication</li>
                    <li>Set CPU, memory, and disk resource limits</li>
                    <li>Autostart containers on boot</li>
                </ul>
            </div>
            <div class="content-section">
                <h2>Creating an LXC Container</h2>
                <p>Click the <strong>Create Container</strong> button on any node page. Choose a distribution template (Debian, Ubuntu, AlmaLinux, Alpine, etc.), set the container name, and optionally configure resources. WolfStack handles the rest.</p>
                <h3>Container Networking</h3>
                <p>Each container automatically gets a bridge IP on the <code>lxcbr0</code> bridge. When WolfNet is enabled, containers also receive a WolfNet IP (10.10.10.x) for encrypted cross-node communication. The bridge IP matches the WolfNet last octet for easy identification.</p>
            </div>
            <div class="content-section">
                <h2>LXC Container Features</h2>
                <p>WolfStack lets you toggle advanced LXC features from the container's <strong>Settings</strong> page. These are applied to the container configuration and take effect on the next start.</p>
                <table>
                    <thead>
                        <tr><th>Feature</th><th>Description</th><th>Required By</th></tr>
                    </thead>
                    <tbody>
                        <tr>
                            <td><strong>TUN/TAP Device</strong></td>
                            <td>Enables <code>/dev/net/tun</code> inside the container for VPN and tunnel support</td>
                            <td>WolfDisk, Tailscale, WireGuard, OpenVPN</td>
                        </tr>
                        <tr>
                            <td><strong>FUSE</strong></td>
                            <td>Enables <code>/dev/fuse</code> for user-space filesystems</td>
                            <td>WolfDisk, AppImage, sshfs, rclone mount</td>
                        </tr>
                        <tr>
                            <td><strong>Nesting</strong></td>
                            <td>Run LXC or Docker inside the container</td>
                            <td>Docker-in-LXC, nested containers</td>
                        </tr>
                        <tr>
                            <td><strong>NFS</strong></td>
                            <td>NFS server/client support inside the container</td>
                            <td>NFS shares</td>
                        </tr>
                        <tr>
                            <td><strong>Keyctl</strong></td>
                            <td>Kernel key management for systemd support</td>
                            <td>Some systemd services</td>
                        </tr>
                    </tbody>
                </table>
                <div class="info-box" style="border-left: 4px solid #e74c3c; background: rgba(231, 76, 60, 0.1);">
                    <p>⚠️ <strong>Installing WolfDisk in a container?</strong> You <strong>must</strong> enable both <strong>TUN/TAP Device</strong> and <strong>FUSE</strong> in the container settings before WolfDisk will work. If installing via the App Store, these are enabled automatically. After changing settings, stop and start the container for them to take effect.</p>
                </div>
            </div>
            <div class="content-section">
                <h2>Cloning &amp; Migration</h2>
                <p><strong>Clone</strong> creates a copy of a container on the same node. <strong>Migrate</strong> moves a container to a different node in the cluster — WolfStack handles the file transfer, IP reassignment, and route configuration automatically.</p>
            </div>

<div class="page-nav"><a href="wolfstack.php" class="prev">&larr; Overview</a><a href="wolfstack-storage.php" class="next">Storage Manager &rarr;</a></div>
        
    </main>
<?php include 'includes/footer.php'; ?>
