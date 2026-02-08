#!/bin/bash
#
# WolfDisk Quick Install Script
# Installs WolfDisk on Ubuntu/Debian (apt) or Fedora/RHEL (dnf)
#
# Usage: curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/wolfdisk/setup.sh | bash
#

set -e

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║                    WolfDisk Installer                        ║"
echo "║            Distributed File System for Linux                 ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""

# Detect package manager
if command -v apt &> /dev/null; then
    PKG_MANAGER="apt"
    echo "✓ Detected Debian/Ubuntu (apt)"
elif command -v dnf &> /dev/null; then
    PKG_MANAGER="dnf"
    echo "✓ Detected Fedora/RHEL (dnf)"
elif command -v yum &> /dev/null; then
    PKG_MANAGER="yum"
    echo "✓ Detected RHEL/CentOS (yum)"
else
    echo "✗ Could not detect package manager (apt/dnf/yum)"
    echo "  Please install dependencies manually and run install_service.sh"
    exit 1
fi

# Install dependencies
echo ""
echo "Installing system dependencies..."

if [ "$PKG_MANAGER" = "apt" ]; then
    sudo apt update
    sudo apt install -y git curl build-essential pkg-config libssl-dev libfuse3-dev fuse3
elif [ "$PKG_MANAGER" = "dnf" ]; then
    sudo dnf install -y git curl gcc gcc-c++ make openssl-devel pkg-config fuse3-devel fuse3
elif [ "$PKG_MANAGER" = "yum" ]; then
    sudo yum install -y git curl gcc gcc-c++ make openssl-devel pkgconfig fuse3-devel fuse3
fi

echo "✓ System dependencies installed"

# Install Rust if not present
if ! command -v rustc &> /dev/null; then
    echo ""
    echo "Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
    echo "✓ Rust installed"
else
    echo "✓ Rust already installed ($(rustc --version))"
fi

# Ensure cargo is in PATH
export PATH="$HOME/.cargo/bin:$PATH"

# Clone or update repository
INSTALL_DIR="/opt/wolfscale-src"
echo ""
echo "Cloning WolfScale repository..."

if [ -d "$INSTALL_DIR" ]; then
    echo "  Updating existing installation..."
    cd "$INSTALL_DIR"
    sudo git fetch origin
    sudo git reset --hard origin/main
else
    sudo git clone https://github.com/wolfsoftwaresystemsltd/WolfScale.git "$INSTALL_DIR"
    cd "$INSTALL_DIR"
fi

sudo chown -R "$USER:$USER" "$INSTALL_DIR"
echo "✓ Repository cloned to $INSTALL_DIR"

# Build WolfDisk
echo ""
echo "Building WolfDisk (this may take a few minutes)..."
cd "$INSTALL_DIR/wolfdisk"
cargo build --release
echo "✓ Build complete"

# Install binary
echo ""
echo "Installing WolfDisk..."
sudo cp "$INSTALL_DIR/wolfdisk/target/release/wolfdisk" /usr/local/bin/wolfdisk
sudo chmod +x /usr/local/bin/wolfdisk
echo "✓ wolfdisk installed to /usr/local/bin/wolfdisk"

# Create data directory
echo ""
echo "Creating data directories..."
sudo mkdir -p /var/lib/wolfdisk/{chunks,index,wal}
sudo mkdir -p /etc/wolfdisk
sudo mkdir -p /mnt/wolfdisk
echo "✓ Directories created"

# Create config if not exists - with interactive prompts
# Use /dev/tty to read from terminal even when script is piped
if [ ! -f "/etc/wolfdisk/config.toml" ]; then
    echo ""
    echo "═══════════════════════════════════════════════════════════════"
    echo "                   WolfDisk Configuration"
    echo "═══════════════════════════════════════════════════════════════"
    echo ""
    
    # Get hostname as default
    DEFAULT_HOSTNAME=$(hostname)
    
    # Prompt for Node ID
    echo -n "Node ID [$DEFAULT_HOSTNAME]: "
    read NODE_ID < /dev/tty
    NODE_ID=${NODE_ID:-$DEFAULT_HOSTNAME}
    
    # Prompt for Role
    echo ""
    echo "Node Roles:"
    echo "  1) auto     - Automatic election (lowest ID becomes leader)"
    echo "  2) leader   - Force this node to be leader"
    echo "  3) follower - Force this node to be follower"
    echo "  4) client   - Mount-only (no local storage, access remote data)"
    echo ""
    echo -n "Select role [1-4, default: 1]: "
    read ROLE_CHOICE < /dev/tty
    
    case $ROLE_CHOICE in
        2) NODE_ROLE="leader" ;;
        3) NODE_ROLE="follower" ;;
        4) NODE_ROLE="client" ;;
        *) NODE_ROLE="auto" ;;
    esac
    
    # Prompt for Discovery
    echo ""
    echo "Cluster Discovery:"
    echo "  1) Auto-discovery (UDP multicast - recommended for LAN)"
    echo "  2) Manual peers (specify IP addresses)"
    echo "  3) Standalone (single node, no clustering)"
    echo ""
    echo -n "Select discovery method [1-3, default: 1]: "
    read DISCOVERY_CHOICE < /dev/tty
    
    DISCOVERY_CONFIG=""
    PEERS_CONFIG="peers = []"
    
    case $DISCOVERY_CHOICE in
        2)
            echo ""
            echo -n "Enter peer addresses (comma-separated, e.g. 192.168.1.10:9500,192.168.1.11:9500): "
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
            DISCOVERY_CONFIG='discovery = "udp://239.255.0.1:9501"'
            ;;
    esac
    
    # Prompt for mount path
    echo ""
    echo -n "Mount path [/mnt/wolfdisk]: "
    read MOUNT_PATH < /dev/tty
    MOUNT_PATH=${MOUNT_PATH:-/mnt/wolfdisk}
    
    # Create the mount directory
    sudo mkdir -p "$MOUNT_PATH"
    
    # Write config
    echo ""
    echo "Creating configuration..."
    cat <<EOF | sudo tee /etc/wolfdisk/config.toml > /dev/null
[node]
id = "$NODE_ID"
role = "$NODE_ROLE"
bind = "0.0.0.0:9500"
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
    echo "✓ Config created at /etc/wolfdisk/config.toml"
    echo ""
    echo "Configuration Summary:"
    echo "  Node ID:    $NODE_ID"
    echo "  Role:       $NODE_ROLE"
    echo "  Mount:      $MOUNT_PATH"
    if [ -n "$DISCOVERY_CONFIG" ]; then
        echo "  Discovery:  UDP multicast (auto)"
    elif [ "$DISCOVERY_CHOICE" = "2" ]; then
        echo "  Peers:      $PEERS_INPUT"
    else
        echo "  Mode:       Standalone"
    fi
else
    echo ""
    echo "✓ Config already exists at /etc/wolfdisk/config.toml"
fi

# Run service installer
echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "Installation complete! Running service setup..."
echo "═══════════════════════════════════════════════════════════════"
echo ""

bash "$INSTALL_DIR/wolfdisk/install_service.sh"

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║                Installation Complete!                        ║"
echo "╠══════════════════════════════════════════════════════════════╣"
echo "║  Mount:    sudo wolfdisk mount -m /mnt/wolfdisk              ║"
echo "║  Status:   sudo systemctl status wolfdisk                    ║"
echo "║  Logs:     sudo journalctl -u wolfdisk -f                    ║"
echo "║  Config:   /etc/wolfdisk/config.toml                         ║"
echo "╚══════════════════════════════════════════════════════════════╝"
