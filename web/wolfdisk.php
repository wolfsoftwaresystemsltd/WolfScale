<?php
$page_title = '💾 WolfDisk — WolfStack Docs';
$page_desc = 'WolfDisk — Distributed filesystem for sharing and replicating files across your Linux network. FUSE-based, S3-compatible, with automatic leader election and failover.';
$active = 'wolfdisk.php';
include 'includes/head.php';
?>

<body>
    <div class="wiki-layout">
        <?php include 'includes/sidebar.php'; ?>
        <main class="wiki-content">

            <!-- ===== Overview ===== -->
            <div class="content-section">
                <h2>Overview</h2>
                <p>WolfDisk is a distributed file system that provides easy-to-use shared and replicated storage across
                    Linux machines. Mount a shared directory on any number of servers and have your data automatically
                    synchronised. Built on the same proven consensus mechanisms as <a
                        href="quickstart.php">WolfScale</a>.</p>

                <p>🎬 <strong>Watch the overview video:</strong> <a href="https://www.youtube.com/watch?v=qjGhqldvhp4"
                        target="_blank">WolfDisk on YouTube</a></p>

                <h3>Key Features</h3>
                <ul>
                    <li><strong>FUSE-based</strong> &mdash; Mount as a regular Linux directory using FUSE3</li>
                    <li><strong>Content-addressed deduplication</strong> &mdash; Automatic deduplication via SHA256
                        hashing</li>
                    <li><strong>Chunk-based storage</strong> &mdash; Large files split into 4 MB chunks for efficient
                        transfer and sync</li>
                    <li><strong>Leader / Follower / Client modes</strong> &mdash; Flexible node roles for any deployment
                    </li>
                    <li><strong>Auto-discovery</strong> &mdash; UDP multicast for automatic peer discovery on LAN</li>
                    <li><strong>Automatic leader election &amp; failover</strong> &mdash; Deterministic election with
                        2-second failover</li>
                    <li><strong>Delta sync</strong> &mdash; Incremental catchup — only missed changes are transferred
                    </li>
                    <li><strong>S3-compatible REST API</strong> &mdash; Optional S3 gateway for any S3 client</li>
                    <li><strong>LZ4 compression</strong> &mdash; Compressed network replication</li>
                    <li><strong>Symlink support</strong> &mdash; Full POSIX symlink support</li>
                    <li><strong>IBM Power ready</strong> &mdash; Pure Rust dependencies, builds natively on ppc64le</li>
                </ul>
            </div>

            <!-- ===== Node Roles ===== -->
            <div class="content-section">
                <h2>Node Roles</h2>
                <p>Every WolfDisk node operates in one of four roles:</p>
                <table>
                    <thead>
                        <tr>
                            <th>Role</th>
                            <th>Storage</th>
                            <th>Replication</th>
                            <th>Use Case</th>
                        </tr>
                    </thead>
                    <tbody>
                        <tr>
                            <td><strong>Leader</strong></td>
                            <td>✅ Yes</td>
                            <td>Broadcasts to followers</td>
                            <td>Primary write node</td>
                        </tr>
                        <tr>
                            <td><strong>Follower</strong></td>
                            <td>✅ Yes</td>
                            <td>Receives from leader</td>
                            <td>Read replicas, failover candidates</td>
                        </tr>
                        <tr>
                            <td><strong>Client</strong></td>
                            <td>❌ No</td>
                            <td>None (mount-only)</td>
                            <td>Access the drive remotely without local storage</td>
                        </tr>
                        <tr>
                            <td><strong>Auto</strong></td>
                            <td>✅ Yes</td>
                            <td>Dynamic election</td>
                            <td>Default — lowest node ID automatically becomes leader</td>
                        </tr>
                    </tbody>
                </table>
                <div class="info-box">
                    <p>💡 <strong>Client Mode</strong> is perfect for workstations or containers that just need to
                        access the shared filesystem without storing data locally.</p>
                </div>
            </div>

            <!-- ===== Quick Install ===== -->
            <div class="content-section">
                <h2>Quick Install</h2>
                <p>The interactive installer handles dependencies, compilation, configuration, and systemd service
                    setup:</p>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                            onclick="copyCode(this)">Copy</button></div>
                    <pre><code>curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/wolfdisk/setup.sh | bash</code></pre>
                </div>
                <p>The installer will prompt you for:</p>
                <ul>
                    <li><strong>Node ID</strong> — Unique identifier (defaults to hostname)</li>
                    <li><strong>Role</strong> — auto, leader, follower, or client</li>
                    <li><strong>Bind IP address</strong> — IP to listen on (auto-detected)</li>
                    <li><strong>Discovery method</strong> — Auto-discovery (UDP multicast), manual peers, or standalone
                    </li>
                    <li><strong>Mount path</strong> — Where to mount the filesystem (default:
                        <code>/mnt/wolfdisk</code>)</li>
                </ul>
                <div class="info-box">
                    <p>⚠️ <strong>Compilation note:</strong> The installer compiles WolfDisk from source using Rust.
                        This is CPU-intensive and may take several minutes. Please wait for it to complete.</p>
                </div>
            </div>

            <!-- ===== Manual Installation ===== -->
            <div class="content-section">
                <h2>Manual Installation</h2>
                <h3>Prerequisites</h3>
                <ul>
                    <li>Linux with FUSE3 support</li>
                    <li>Rust toolchain (<code>rustup</code>)</li>
                </ul>

                <div class="info-box" style="border-left: 4px solid #e74c3c; background: rgba(231, 76, 60, 0.1);">
                    <p>⚠️ <strong>Running in an LXC Container?</strong></p>
                    <p>WolfDisk <strong>requires</strong> the following features to be enabled in your container settings. Without these, WolfDisk <strong>will not start</strong>:</p>
                    <ul>
                        <li>✅ <strong>TUN/TAP Device</strong> — Required for WolfDisk networking (<code>/dev/net/tun</code>)</li>
                        <li>✅ <strong>FUSE</strong> — Required for the FUSE filesystem mount (<code>/dev/fuse</code>)</li>
                    </ul>
                    <p><strong>WolfStack:</strong> Go to the container → Settings → enable TUN/TAP Device and FUSE → save → reboot the container.</p>
                    <p><strong>Proxmox:</strong> Go to the container → Options → Features → enable <code>fuse</code> and <code>tun</code> → reboot the container.</p>
                    <p>If installing via the WolfStack App Store, these settings are applied automatically.</p>
                </div>

                <h3>Install Dependencies</h3>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                            onclick="copyCode(this)">Copy</button></div>
                    <pre><code># Ubuntu / Debian
sudo apt install libfuse3-dev fuse3

# Fedora / RHEL
sudo dnf install fuse3-devel fuse3</code></pre>
                </div>

                <h3>Build &amp; Install</h3>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                            onclick="copyCode(this)">Copy</button></div>
                    <pre><code>git clone https://github.com/wolfsoftwaresystemsltd/WolfScale.git
cd WolfScale/wolfdisk
cargo build --release
sudo cp target/release/wolfdisk /usr/local/bin/
sudo cp target/release/wolfdiskctl /usr/local/bin/</code></pre>
                </div>
            </div>

            <!-- ===== Usage ===== -->
            <div class="content-section">
                <h2>Usage</h2>
                <h3>1. Initialize Data Directory</h3>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                            onclick="copyCode(this)">Copy</button></div>
                    <pre><code>wolfdisk init -d /var/lib/wolfdisk</code></pre>
                </div>

                <h3>2. Mount the Filesystem</h3>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                            onclick="copyCode(this)">Copy</button></div>
                    <pre><code># Foreground (for testing)
sudo wolfdisk mount -m /mnt/wolfdisk

# As a systemd service (recommended)
sudo systemctl start wolfdisk</code></pre>
                </div>

                <h3>3. Check Status</h3>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                            onclick="copyCode(this)">Copy</button></div>
                    <pre><code>wolfdiskctl status</code></pre>
                </div>

                <h3>4. Use It Like Any Directory</h3>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                            onclick="copyCode(this)">Copy</button></div>
                    <pre><code># Write files
echo "Hello, WolfDisk!" > /mnt/wolfdisk/hello.txt
cp /var/log/syslog /mnt/wolfdisk/

# Read files — automatically available on all nodes
cat /mnt/wolfdisk/hello.txt
ls -la /mnt/wolfdisk/</code></pre>
                </div>
            </div>

            <!-- ===== Configuration ===== -->
            <div class="content-section">
                <h2>Configuration</h2>
                <p>WolfDisk is configured via <code>/etc/wolfdisk/config.toml</code>. The installer creates this file
                    for you, or you can edit it manually:</p>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">toml</span><button class="copy-btn"
                            onclick="copyCode(this)">Copy</button></div>
                    <pre><code>[node]
id = "node1"                    # Unique node identifier
role = "auto"                   # auto, leader, follower, or client
bind = "0.0.0.0:9500"           # IP and port for cluster communication
data_dir = "/var/lib/wolfdisk"  # Where chunks and index are stored

[cluster]
# Auto-discovery (recommended for LAN)
discovery = "udp://239.255.0.1:9501"

# Or manual peers (for cross-subnet / WAN)
# peers = ["192.168.1.10:9500", "192.168.1.11:9500"]

[replication]
mode = "shared"       # "shared" or "replicated"
factor = 3            # Number of copies (replicated mode)
chunk_size = 4194304  # 4 MB chunks

[mount]
path = "/mnt/wolfdisk"
allow_other = true    # Allow other users to access the mount

# Optional: S3-compatible API
[s3]
enabled = true
bind = "0.0.0.0:9878"
# access_key = "your-access-key"   # optional auth
# secret_key = "your-secret-key"   # optional auth</code></pre>
                </div>

                <h3>Configuration Reference</h3>
                <table>
                    <thead>
                        <tr>
                            <th>Section</th>
                            <th>Key</th>
                            <th>Default</th>
                            <th>Description</th>
                        </tr>
                    </thead>
                    <tbody>
                        <tr>
                            <td><code>[node]</code></td>
                            <td><code>id</code></td>
                            <td>hostname</td>
                            <td>Unique identifier — used in leader election (lowest ID wins)</td>
                        </tr>
                        <tr>
                            <td></td>
                            <td><code>role</code></td>
                            <td><code>auto</code></td>
                            <td><code>auto</code>, <code>leader</code>, <code>follower</code>, or <code>client</code>
                            </td>
                        </tr>
                        <tr>
                            <td></td>
                            <td><code>bind</code></td>
                            <td><code>0.0.0.0:9500</code></td>
                            <td>Listen address for cluster communication</td>
                        </tr>
                        <tr>
                            <td></td>
                            <td><code>data_dir</code></td>
                            <td><code>/var/lib/wolfdisk</code></td>
                            <td>Directory for chunks, index, and WAL</td>
                        </tr>
                        <tr>
                            <td><code>[cluster]</code></td>
                            <td><code>discovery</code></td>
                            <td>—</td>
                            <td>UDP multicast address for auto-discovery</td>
                        </tr>
                        <tr>
                            <td></td>
                            <td><code>peers</code></td>
                            <td><code>[]</code></td>
                            <td>Manual list of peer addresses</td>
                        </tr>
                        <tr>
                            <td><code>[replication]</code></td>
                            <td><code>mode</code></td>
                            <td><code>shared</code></td>
                            <td><code>shared</code> (single leader) or <code>replicated</code> (N copies)</td>
                        </tr>
                        <tr>
                            <td></td>
                            <td><code>factor</code></td>
                            <td><code>3</code></td>
                            <td>Replication factor for replicated mode</td>
                        </tr>
                        <tr>
                            <td></td>
                            <td><code>chunk_size</code></td>
                            <td><code>4194304</code></td>
                            <td>Chunk size in bytes (4 MB)</td>
                        </tr>
                        <tr>
                            <td><code>[mount]</code></td>
                            <td><code>path</code></td>
                            <td><code>/mnt/wolfdisk</code></td>
                            <td>FUSE mount point</td>
                        </tr>
                        <tr>
                            <td></td>
                            <td><code>allow_other</code></td>
                            <td><code>true</code></td>
                            <td>Allow other system users to access the mount</td>
                        </tr>
                        <tr>
                            <td><code>[s3]</code></td>
                            <td><code>enabled</code></td>
                            <td><code>false</code></td>
                            <td>Enable S3-compatible REST API</td>
                        </tr>
                        <tr>
                            <td></td>
                            <td><code>bind</code></td>
                            <td><code>0.0.0.0:9878</code></td>
                            <td>S3 API listen address</td>
                        </tr>
                        <tr>
                            <td></td>
                            <td><code>access_key</code></td>
                            <td>—</td>
                            <td>Optional S3 access key</td>
                        </tr>
                        <tr>
                            <td></td>
                            <td><code>secret_key</code></td>
                            <td>—</td>
                            <td>Optional S3 secret key</td>
                        </tr>
                    </tbody>
                </table>
            </div>

            <!-- ===== Architecture ===== -->
            <div class="content-section">
                <h2>Architecture</h2>
                <p>WolfDisk is composed of several layers that work together:</p>

                <div class="code-block">
                    <div class="code-header"><span class="code-lang">text</span></div>
                    <pre><code>┌───────────────────────────────────────────────────────────────┐
│                      Linux Applications                       │
├───────────────────────────────────────────────────────────────┤
│                    mount /mnt/wolfdisk                         │
├───────────────────────────────────────────────────────────────┤
│                       FUSE (fuser)                             │
├───────────────────────────────────────────────────────────────┤
│                     WolfDisk Core                              │
│  ┌─────────────┐  ┌───────────────┐  ┌──────────────────────┐ │
│  │  File Index  │  │  Chunk Store  │  │ Replication Engine   │ │
│  │  (metadata)  │  │   (SHA256)    │  │ (leader election)    │ │
│  └─────────────┘  └───────────────┘  └──────────────────────┘ │
│  ┌───────────────────────────────────────────────────────────┐ │
│  │              S3-Compatible API (optional)                  │ │
│  │         ListBuckets / Get / Put / Delete Objects           │ │
│  └───────────────────────────────────────────────────────────┘ │
├───────────────────────────────────────────────────────────────┤
│       Network Layer: Discovery + Peer Manager + Protocol       │
└───────────────────────────────────────────────────────────────┘</code></pre>
                </div>

                <h3>How It Works</h3>
                <ol>
                    <li><strong>FUSE Layer</strong> — Applications read/write to <code>/mnt/wolfdisk</code> like a
                        normal directory. The FUSE driver intercepts all filesystem calls.</li>
                    <li><strong>Chunk Store</strong> — Files are split into 4 MB chunks, each identified by its SHA256
                        hash. Identical data is automatically deduplicated.</li>
                    <li><strong>File Index</strong> — Maps file paths to their chunk references, permissions, size, and
                        modification times.</li>
                    <li><strong>Replication Engine</strong> — Synchronises chunks and index updates across nodes.
                        Handles leader election and failover.</li>
                    <li><strong>Network Layer</strong> — UDP multicast for peer discovery, TCP for data transfer. LZ4
                        compression for network efficiency.</li>
                </ol>
            </div>

            <!-- ===== Leader Failover ===== -->
            <div class="content-section">
                <h2>Leader Election &amp; Failover</h2>
                <p>WolfDisk uses deterministic leader election — <strong>no voting, no delays</strong>. The node with
                    the lowest ID is always the leader.</p>

                <h3>How Failover Works</h3>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">text</span></div>
                    <pre><code>Initial State:
  node-a (leader) ←→ node-b (follower) ←→ node-c (follower)

node-a goes down:
  ❌ node-a         node-b detects timeout (2s)
                    node-b becomes leader (next lowest ID)

node-a returns:
  node-a syncs from node-b (gets missed changes)
  node-a becomes leader again (lowest ID)</code></pre>
                </div>

                <ul>
                    <li><strong>Heartbeat timeout</strong> — Nodes monitor the leader with a 2-second timeout</li>
                    <li><strong>Fast election</strong> — No voting or consensus delay; lowest node ID always wins</li>
                    <li><strong>Seamless reads</strong> — Followers continue serving reads during failover</li>
                    <li><strong>Explicit override</strong> — Set <code>role = "leader"</code> to force a specific node
                        as leader</li>
                </ul>
            </div>

            <!-- ===== Sync & Replication ===== -->
            <div class="content-section">
                <h2>Sync &amp; Replication</h2>

                <h3>Write Replication</h3>
                <p>When the leader writes a file:</p>
                <ol>
                    <li><strong>Local write</strong> — Leader stores chunks and updates the file index locally</li>
                    <li><strong>Broadcast</strong> — Leader sends the index update and chunk data to all followers</li>
                    <li><strong>Apply</strong> — Followers update their local index and store the chunks</li>
                </ol>

                <h3>Delta Sync (Catchup)</h3>
                <p>When a node starts or recovers from downtime, it performs an <strong>incremental sync</strong>:</p>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">text</span></div>
                    <pre><code>Follower (version 45) → Leader: "SyncRequest(from_version=45)"
Leader (version 50)   → Follower: "SyncResponse(entries=[5 changes])"
Follower applies 5 changes, now at version 50</code></pre>
                </div>
                <p>Only missed changes are transferred — a node that was down briefly doesn't need to re-download
                    everything.</p>

                <h3>Read Caching</h3>
                <p>Followers cache chunks locally for fast reads:</p>
                <ul>
                    <li><strong>Cache hit</strong> — Chunk exists locally → return immediately</li>
                    <li><strong>Cache miss</strong> — Fetch chunk from leader → cache locally → return</li>
                </ul>
            </div>

            <!-- ===== Client Mode ===== -->
            <div class="content-section">
                <h2>Client Mode (Thin Client)</h2>
                <p>Client mode mounts the filesystem <strong>without storing any data locally</strong>. All reads and
                    writes are forwarded to the leader over the network.</p>
                <table>
                    <thead>
                        <tr>
                            <th>Aspect</th>
                            <th>Leader / Follower</th>
                            <th>Client</th>
                        </tr>
                    </thead>
                    <tbody>
                        <tr>
                            <td>Local Storage</td>
                            <td>✅ Stores data on disk</td>
                            <td>❌ No local storage</td>
                        </tr>
                        <tr>
                            <td>Reads</td>
                            <td>Served locally</td>
                            <td>Forwarded to leader</td>
                        </tr>
                        <tr>
                            <td>Writes</td>
                            <td>Local (leader) or forwarded</td>
                            <td>Forwarded to leader</td>
                        </tr>
                        <tr>
                            <td>Use Case</td>
                            <td>Data nodes, replicas</td>
                            <td>Workstations, containers</td>
                        </tr>
                    </tbody>
                </table>
                <p>Client mode is ideal for:</p>
                <ul>
                    <li>Workstations accessing shared files</li>
                    <li>Containers that need cluster storage access</li>
                    <li>Read-heavy applications where network latency is acceptable</li>
                </ul>
            </div>

            <!-- ===== S3 API ===== -->
            <div class="content-section">
                <h2>S3-Compatible API</h2>
                <p>WolfDisk can optionally expose an S3-compatible REST API, allowing any S3 client (AWS CLI, rclone,
                    MinIO Client, etc.) to read and write files. Both FUSE and S3 access the <strong>same underlying
                        data</strong> — files written via FUSE are instantly visible through S3, and vice versa.</p>

                <h3>Enable S3</h3>
                <p>Add to <code>/etc/wolfdisk/config.toml</code>:</p>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">toml</span><button class="copy-btn"
                            onclick="copyCode(this)">Copy</button></div>
                    <pre><code>[s3]
enabled = true
bind = "0.0.0.0:9878"
# Optional authentication:
# access_key = "your-access-key"
# secret_key = "your-secret-key"</code></pre>
                </div>

                <h3>How It Maps</h3>
                <table>
                    <thead>
                        <tr>
                            <th>WolfDisk</th>
                            <th>S3 Equivalent</th>
                        </tr>
                    </thead>
                    <tbody>
                        <tr>
                            <td>Top-level directory</td>
                            <td>Bucket</td>
                        </tr>
                        <tr>
                            <td>File in directory</td>
                            <td>Object</td>
                        </tr>
                        <tr>
                            <td>Nested directory</td>
                            <td>Object key prefix</td>
                        </tr>
                    </tbody>
                </table>

                <h3>Supported Operations</h3>
                <table>
                    <thead>
                        <tr>
                            <th>Operation</th>
                            <th>Method</th>
                            <th>Path</th>
                        </tr>
                    </thead>
                    <tbody>
                        <tr>
                            <td>ListBuckets</td>
                            <td><code>GET</code></td>
                            <td><code>/</code></td>
                        </tr>
                        <tr>
                            <td>CreateBucket</td>
                            <td><code>PUT</code></td>
                            <td><code>/bucket</code></td>
                        </tr>
                        <tr>
                            <td>DeleteBucket</td>
                            <td><code>DELETE</code></td>
                            <td><code>/bucket</code></td>
                        </tr>
                        <tr>
                            <td>HeadBucket</td>
                            <td><code>HEAD</code></td>
                            <td><code>/bucket</code></td>
                        </tr>
                        <tr>
                            <td>ListObjectsV2</td>
                            <td><code>GET</code></td>
                            <td><code>/bucket?prefix=...</code></td>
                        </tr>
                        <tr>
                            <td>GetObject</td>
                            <td><code>GET</code></td>
                            <td><code>/bucket/key</code></td>
                        </tr>
                        <tr>
                            <td>PutObject</td>
                            <td><code>PUT</code></td>
                            <td><code>/bucket/key</code></td>
                        </tr>
                        <tr>
                            <td>DeleteObject</td>
                            <td><code>DELETE</code></td>
                            <td><code>/bucket/key</code></td>
                        </tr>
                        <tr>
                            <td>HeadObject</td>
                            <td><code>HEAD</code></td>
                            <td><code>/bucket/key</code></td>
                        </tr>
                    </tbody>
                </table>

                <h3>Examples</h3>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                            onclick="copyCode(this)">Copy</button></div>
                    <pre><code># Using AWS CLI
aws --endpoint-url http://localhost:9878 s3 ls
aws --endpoint-url http://localhost:9878 s3 cp file.txt s3://mybucket/file.txt
aws --endpoint-url http://localhost:9878 s3 ls s3://mybucket/

# Using curl
curl http://localhost:9878/mybucket/myfile.txt
curl -X PUT --data-binary @file.txt http://localhost:9878/mybucket/file.txt</code></pre>
                </div>
            </div>

            <!-- ===== CLI Reference ===== -->
            <div class="content-section">
                <h2>CLI Reference</h2>

                <h3><code>wolfdisk</code> (Service Daemon)</h3>
                <table>
                    <thead>
                        <tr>
                            <th>Command</th>
                            <th>Description</th>
                        </tr>
                    </thead>
                    <tbody>
                        <tr>
                            <td><code>wolfdisk init -d PATH</code></td>
                            <td>Initialize a new WolfDisk data directory</td>
                        </tr>
                        <tr>
                            <td><code>wolfdisk mount -m PATH</code></td>
                            <td>Mount the filesystem at the given path</td>
                        </tr>
                        <tr>
                            <td><code>wolfdisk unmount -m PATH</code></td>
                            <td>Unmount the filesystem</td>
                        </tr>
                        <tr>
                            <td><code>wolfdisk status</code></td>
                            <td>Show node configuration</td>
                        </tr>
                    </tbody>
                </table>
                <p>Options:</p>
                <ul>
                    <li><code>--config PATH</code> — Path to config file (default:
                        <code>/etc/wolfdisk/config.toml</code>)</li>
                </ul>

                <h3><code>wolfdiskctl</code> (Control Utility)</h3>
                <table>
                    <thead>
                        <tr>
                            <th>Command</th>
                            <th>Description</th>
                        </tr>
                    </thead>
                    <tbody>
                        <tr>
                            <td><code>wolfdiskctl status</code></td>
                            <td>Show live status from the running service (role, state, version, file count, size,
                                peers)</td>
                        </tr>
                        <tr>
                            <td><code>wolfdiskctl list servers</code></td>
                            <td>List all discovered servers in the cluster with roles and status</td>
                        </tr>
                        <tr>
                            <td><code>wolfdiskctl stats</code></td>
                            <td>Live cluster statistics dashboard (refreshes every second, Ctrl+C to exit)</td>
                        </tr>
                    </tbody>
                </table>
                <p>Options:</p>
                <ul>
                    <li><code>-s, --status-file PATH</code> — Path to status file (default:
                        <code>/var/lib/wolfdisk/cluster_status.json</code>)</li>
                </ul>
            </div>

            <!-- ===== Systemd Service ===== -->
            <div class="content-section">
                <h2>Systemd Service</h2>
                <p>The installer creates a systemd service for you automatically. Common commands:</p>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                            onclick="copyCode(this)">Copy</button></div>
                    <pre><code># Start WolfDisk
sudo systemctl start wolfdisk

# Check status
sudo systemctl status wolfdisk

# View logs
sudo journalctl -u wolfdisk -f

# Enable on boot
sudo systemctl enable wolfdisk

# Restart after config change
sudo systemctl restart wolfdisk</code></pre>
                </div>
            </div>

            <!-- ===== Multi-Node Examples ===== -->
            <div class="content-section">
                <h2>Multi-Node Setup</h2>
                <p>A typical deployment has multiple nodes with different roles. All nodes with
                    <code>role = "auto"</code> will automatically elect a leader.</p>

                <h3>Server 1 (will become leader — lowest ID)</h3>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">toml</span><button class="copy-btn"
                            onclick="copyCode(this)">Copy</button></div>
                    <pre><code>[node]
id = "node-a"
role = "auto"
bind = "192.168.1.10:9500"
data_dir = "/var/lib/wolfdisk"

[cluster]
discovery = "udp://192.168.1.10:9501"

[mount]
path = "/mnt/shared"</code></pre>
                </div>

                <h3>Server 2–N (will become followers — higher IDs)</h3>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">toml</span><button class="copy-btn"
                            onclick="copyCode(this)">Copy</button></div>
                    <pre><code>[node]
id = "node-b"          # Higher ID → follower
role = "auto"
bind = "192.168.1.11:9500"
data_dir = "/var/lib/wolfdisk"

[cluster]
discovery = "udp://192.168.1.11:9501"

[mount]
path = "/mnt/shared"</code></pre>
                </div>

                <h3>Workstation (client only — no storage)</h3>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">toml</span><button class="copy-btn"
                            onclick="copyCode(this)">Copy</button></div>
                    <pre><code>[node]
id = "desktop"
role = "client"
bind = "192.168.1.50:9500"

[cluster]
peers = ["192.168.1.10:9500", "192.168.1.11:9500"]

[mount]
path = "/mnt/shared"</code></pre>
                </div>
            </div>

            <!-- ===== Troubleshooting ===== -->
            <div class="content-section">
                <h2>Troubleshooting</h2>

                <h3>WolfDisk fails to start in an LXC container</h3>
                <p>If WolfDisk fails immediately or you see errors about <code>/dev/fuse</code> or <code>/dev/net/tun</code> not being available, the container is missing required device access:</p>
                <ol>
                    <li><strong>Stop</strong> the container</li>
                    <li>Go to the container's <strong>Settings</strong> page</li>
                    <li>Enable <strong>TUN/TAP Device</strong> and <strong>FUSE</strong></li>
                    <li>Save and <strong>start</strong> the container</li>
                </ol>
                <p>On Proxmox: <code>Options → Features → fuse, tun</code>. On WolfStack: <code>Settings → TUN/TAP Device ✅, FUSE ✅</code>.</p>

                <h3>Mount fails with "Transport endpoint not connected"</h3>
                <p>The FUSE mount is stale. Unmount and remount:</p>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                            onclick="copyCode(this)">Copy</button></div>
                    <pre><code>sudo fusermount -u -z /mnt/wolfdisk
sudo systemctl restart wolfdisk</code></pre>
                </div>

                <h3>Permission denied on mount</h3>
                <p>Ensure <code>user_allow_other</code> is enabled:</p>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                            onclick="copyCode(this)">Copy</button></div>
                    <pre><code>echo "user_allow_other" | sudo tee -a /etc/fuse.conf</code></pre>
                </div>

                <h3>Peers not discovering each other</h3>
                <ul>
                    <li>Check that port <strong>9500</strong> (TCP) and <strong>9501</strong> (UDP) are open in your
                        firewall</li>
                    <li>For cross-subnet setups, use <code>peers = [...]</code> instead of UDP discovery</li>
                    <li>Verify <code>bind</code> is set to an accessible IP, not <code>127.0.0.1</code></li>
                </ul>

                <h3>Status file not found</h3>
                <p>If <code>wolfdiskctl status</code> reports "Status file not found", the service is not running:</p>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                            onclick="copyCode(this)">Copy</button></div>
                    <pre><code>sudo systemctl start wolfdisk
sudo journalctl -u wolfdisk -f    # Check for errors</code></pre>
                </div>

                <h3>Client Reset</h3>
                <p>If a <strong>client node</strong> (not leader/follower!) has a stuck mount or corrupted cache, use
                    the reset script:</p>
                <div class="code-block">
                    <div class="code-header"><span class="code-lang">bash</span><button class="copy-btn"
                            onclick="copyCode(this)">Copy</button></div>
                    <pre><code>sudo bash /opt/wolfscale-src/wolfdisk/reset_client.sh</code></pre>
                </div>
                <div class="info-box">
                    <p>⚠️ <strong>Warning:</strong> The reset script wipes <code>/var/lib/wolfdisk</code>. Only use it
                        on <strong>client</strong> nodes. Running it on a leader or follower will cause <strong>data
                            loss</strong>.</p>
                </div>
            </div>

            <!-- ===== Network Ports ===== -->
            <div class="content-section">
                <h2>Network Ports</h2>
                <table>
                    <thead>
                        <tr>
                            <th>Port</th>
                            <th>Protocol</th>
                            <th>Purpose</th>
                        </tr>
                    </thead>
                    <tbody>
                        <tr>
                            <td>9500</td>
                            <td>TCP</td>
                            <td>Cluster communication (data, replication, sync)</td>
                        </tr>
                        <tr>
                            <td>9501</td>
                            <td>UDP</td>
                            <td>Auto-discovery (multicast)</td>
                        </tr>
                        <tr>
                            <td>9878</td>
                            <td>TCP</td>
                            <td>S3-compatible API (when enabled)</td>
                        </tr>
                    </tbody>
                </table>
            </div>

            <div class="page-nav"><a href="wolfstack-ai.php" class="prev">&larr; AI Agent</a><a href="wolfnet.php"
                    class="next">WolfNet &rarr;</a></div>

        </main>
        <?php include 'includes/footer.php'; ?>