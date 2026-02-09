#!/bin/bash
#
# WolfDisk Quick Install Script
# Installs WolfDisk on Ubuntu/Debian (apt) or Fedora/RHEL (dnf)
#
# Usage: curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/wolfdisk/setup.sh | bash
#

set -e

# --- Root check: re-exec with sudo if not root ---
if [ "$EUID" -ne 0 ]; then
    echo ""
    echo "  ⚠  WolfDisk installer requires root privileges."
    echo "  ─────────────────────────────────────"
    echo "  Re-running with sudo..."
    echo ""
    if [ -f "$0" ] && [ "$0" != "bash" ] && [ "$0" != "/bin/bash" ] && [ "$0" != "/usr/bin/bash" ]; then
        # Script is a real file — re-exec directly
        exec sudo bash "$0" "$@"
    else
        # Piped execution (curl | bash) — re-download and run as root
        SETUP_URL="https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/wolfdisk/setup.sh"
        curl -sSL "$SETUP_URL" | sudo bash
        exit $?
    fi
fi

echo ""
echo "  🐺 WolfDisk Installer"
echo "  ─────────────────────────────────────"
echo "  Distributed File System for Linux"
echo ""

# Detect package manager
if command -v apt &> /dev/null; then
    PKG_MANAGER="apt"
    echo "  ✓ Detected Debian/Ubuntu (apt)"
elif command -v dnf &> /dev/null; then
    PKG_MANAGER="dnf"
    echo "  ✓ Detected Fedora/RHEL (dnf)"
elif command -v yum &> /dev/null; then
    PKG_MANAGER="yum"
    echo "  ✓ Detected RHEL/CentOS (yum)"
else
    echo "  ✗ Could not detect package manager (apt/dnf/yum)"
    echo "    Please install dependencies manually and run install_service.sh"
    exit 1
fi

# Install dependencies
echo ""
echo "  Installing system dependencies..."

if [ "$PKG_MANAGER" = "apt" ]; then
    apt update
    apt install -y git curl build-essential pkg-config libssl-dev libfuse3-dev fuse3
elif [ "$PKG_MANAGER" = "dnf" ]; then
    dnf install -y git curl gcc gcc-c++ make openssl-devel pkg-config fuse3-devel fuse3
elif [ "$PKG_MANAGER" = "yum" ]; then
    yum install -y git curl gcc gcc-c++ make openssl-devel pkgconfig fuse3-devel fuse3
fi

echo "  ✓ System dependencies installed"

# Determine the real user (even when running under sudo)
REAL_USER="${SUDO_USER:-$USER}"
REAL_HOME=$(eval echo "~$REAL_USER")

# Install Rust if not present (for the real user)
if ! su - "$REAL_USER" -c 'command -v rustc' &> /dev/null; then
    echo ""
    echo "  Installing Rust for user $REAL_USER..."
    su - "$REAL_USER" -c 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y'
    echo "  ✓ Rust installed"
else
    RUST_VER=$(su - "$REAL_USER" -c 'rustc --version' 2>/dev/null)
    echo "  ✓ Rust already installed ($RUST_VER)"
fi

# Clone or update repository
INSTALL_DIR="/opt/wolfscale-src"
echo ""
echo "  Cloning WolfScale repository..."

if [ -d "$INSTALL_DIR" ]; then
    echo "    Updating existing installation..."
    cd "$INSTALL_DIR"
    git fetch origin
    git reset --hard origin/main
else
    git clone https://github.com/wolfsoftwaresystemsltd/WolfScale.git "$INSTALL_DIR"
    cd "$INSTALL_DIR"
fi

chown -R "$REAL_USER:$REAL_USER" "$INSTALL_DIR"
echo "  ✓ Repository cloned to $INSTALL_DIR"

# Build WolfDisk (as the real user so cargo/rustup are available)
echo ""
echo "  Building WolfDisk (this may take a few minutes)..."
cd "$INSTALL_DIR/wolfdisk"
su - "$REAL_USER" -c "cd $INSTALL_DIR/wolfdisk && source \$HOME/.cargo/env && cargo build --release"
echo "  ✓ Build complete"

# Stop service if running (for upgrades)
if systemctl is-active --quiet wolfdisk 2>/dev/null; then
    echo ""
    echo "  Stopping WolfDisk service for upgrade..."
    systemctl stop wolfdisk
    sleep 2  # Give time for process to fully terminate
    echo "  ✓ Service stopped"
    RESTART_SERVICE=true
else
    RESTART_SERVICE=false
fi

# Install binary
echo ""
if [ -f "/usr/local/bin/wolfdisk" ]; then
    echo "  Upgrading WolfDisk..."
    rm -f /usr/local/bin/wolfdisk
else
    echo "  Installing WolfDisk..."
fi
cp "$INSTALL_DIR/wolfdisk/target/release/wolfdisk" /usr/local/bin/wolfdisk
chmod +x /usr/local/bin/wolfdisk
echo "  ✓ wolfdisk installed to /usr/local/bin/wolfdisk"

# Install wolfdiskctl control utility
cp "$INSTALL_DIR/wolfdisk/target/release/wolfdiskctl" /usr/local/bin/wolfdiskctl
chmod +x /usr/local/bin/wolfdiskctl
echo "  ✓ wolfdiskctl installed to /usr/local/bin/wolfdiskctl"

# Create data directory
echo ""
echo "  Creating data directories..."
mkdir -p /var/lib/wolfdisk/{chunks,index,wal}
mkdir -p /etc/wolfdisk
mkdir -p /mnt/wolfdisk
echo "  ✓ Directories created"

# Create config if not exists - with interactive prompts
# Use /dev/tty to read from terminal even when script is piped
if [ ! -f "/etc/wolfdisk/config.toml" ]; then
    echo ""
    echo "  ─────────────────────────────────────"
    echo "  WolfDisk Configuration"
    echo "  ─────────────────────────────────────"
    echo ""
    
    # Get hostname as default
    DEFAULT_HOSTNAME=$(hostname)
    
    # Prompt for Node ID
    echo -n "  Node ID [$DEFAULT_HOSTNAME]: "
    read NODE_ID < /dev/tty
    NODE_ID=${NODE_ID:-$DEFAULT_HOSTNAME}
    
    # Prompt for Role
    echo ""
    echo "  Node Roles:"
    echo "    1) auto     - Automatic election (lowest ID becomes leader)"
    echo "    2) leader   - Force this node to be leader"
    echo "    3) follower - Force this node to be follower"
    echo "    4) client   - Mount-only (no local storage, access remote data)"
    echo ""
    echo -n "  Select role [1-4, default: 1]: "
    read ROLE_CHOICE < /dev/tty
    
    case $ROLE_CHOICE in
        2) NODE_ROLE="leader" ;;
        3) NODE_ROLE="follower" ;;
        4) NODE_ROLE="client" ;;
        *) NODE_ROLE="auto" ;;
    esac
    
    # Get default IP address
    DEFAULT_IP=$(hostname -I | awk '{print $1}')
    DEFAULT_IP=${DEFAULT_IP:-"0.0.0.0"}
    
    # Prompt for bind address
    echo ""
    echo -n "  Bind IP address [$DEFAULT_IP]: "
    read BIND_IP < /dev/tty
    BIND_IP=${BIND_IP:-$DEFAULT_IP}
    
    # Prompt for Discovery
    echo ""
    echo "  Cluster Discovery:"
    echo "    1) Auto-discovery (UDP multicast - recommended for LAN)"
    echo "    2) Manual peers (specify IP addresses)"
    echo "    3) Standalone (single node, no clustering)"
    echo ""
    echo -n "  Select discovery method [1-3, default: 1]: "
    read DISCOVERY_CHOICE < /dev/tty
    
    DISCOVERY_CONFIG=""
    PEERS_CONFIG="peers = []"
    
    case $DISCOVERY_CHOICE in
        2)
            echo ""
            echo -n "  Enter peer addresses (comma-separated, e.g. 192.168.1.10:9500,192.168.1.11:9500): "
            read PEERS_INPUT < /dev/tty
            if [ -n "$PEERS_INPUT" ]; then
                # Convert comma-separated to TOML array format
                PEERS_FORMATTED=$(echo "$PEERS_INPUT" | sed 's/,/", "/g')
                PEERS_CONFIG="peers = [\"$PEERS_FORMATTED\"]"
            fi
            ;;
        3)
            # Standalone - no discovery, no peers
            ;;
        *)
            DISCOVERY_CONFIG="discovery = \"udp://$BIND_IP:9501\""
            ;;
    esac
    
    # Prompt for mount path
    echo ""
    echo -n "  Mount path [/mnt/wolfdisk]: "
    read MOUNT_PATH < /dev/tty
    MOUNT_PATH=${MOUNT_PATH:-/mnt/wolfdisk}
    
    # Create the mount directory
    mkdir -p "$MOUNT_PATH"
    
    # Write config
    echo ""
    echo "  Creating configuration..."
    cat <<EOF > /etc/wolfdisk/config.toml
[node]
id = "$NODE_ID"
role = "$NODE_ROLE"
bind = "$BIND_IP:9500"
data_dir = "/var/lib/wolfdisk"

[cluster]
$PEERS_CONFIG
$DISCOVERY_CONFIG

[replication]
mode = "shared"
factor = 3
chunk_size = 4194304  # 4MB

[mount]
path = "$MOUNT_PATH"
allow_other = true
EOF
    echo "  ✓ Config created at /etc/wolfdisk/config.toml"
    echo ""
    echo "  Configuration Summary:"
    echo "    Node ID:    $NODE_ID"
    echo "    Role:       $NODE_ROLE"
    echo "    Bind:       $BIND_IP:9500"
    echo "    Mount:      $MOUNT_PATH"
    if [ -n "$DISCOVERY_CONFIG" ]; then
        echo "    Discovery:  UDP multicast (auto)"
    elif [ "$DISCOVERY_CHOICE" = "2" ]; then
        echo "    Peers:      $PEERS_INPUT"
    else
        echo "    Mode:       Standalone"
    fi
else
    echo ""
    echo "  ✓ Config already exists at /etc/wolfdisk/config.toml"
    echo "    (Upgrade mode - skipping configuration prompts)"
fi

# Run service installer only for new installations
if [ ! -f "/etc/systemd/system/wolfdisk.service" ]; then
    echo ""
    echo "  ─────────────────────────────────────"
    echo "  Running service setup..."
    echo "  ─────────────────────────────────────"
    echo ""
    
    bash "$INSTALL_DIR/wolfdisk/install_service.sh"
else
    echo ""
    echo "  ✓ Service already installed - reloading systemd"
    systemctl daemon-reload
fi

# Restart service if it was running before upgrade
if [ "$RESTART_SERVICE" = "true" ]; then
    echo ""
    echo "  Restarting WolfDisk service..."
    systemctl start wolfdisk
    echo "  ✓ Service restarted"
fi

echo ""
echo "  🐺 Installation Complete!"
echo "  ─────────────────────────────────────"
echo "  Status:   sudo systemctl status wolfdisk"
echo "  Logs:     sudo journalctl -u wolfdisk -f"
echo "  Config:   /etc/wolfdisk/config.toml"
echo ""
