#!/bin/bash
#
# WolfScale Load Balancer Quick Install Script
# Installs WolfScale in Load Balancer mode
#
# Usage: curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/setup_lb.sh | bash
#

set -e

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║               WolfScale Load Balancer Installer              ║"
echo "║            Distribute MySQL across your cluster              ║"
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
    exit 1
fi

# Install dependencies
echo ""
echo "Installing system dependencies..."

if [ "$PKG_MANAGER" = "apt" ]; then
    sudo apt update -qq
    sudo apt install -y git curl build-essential pkg-config libssl-dev
elif [ "$PKG_MANAGER" = "dnf" ]; then
    sudo dnf install -y git curl gcc gcc-c++ make openssl-devel pkg-config
elif [ "$PKG_MANAGER" = "yum" ]; then
    sudo yum install -y git curl gcc gcc-c++ make openssl-devel pkgconfig
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

# Build
echo ""
echo "Building WolfScale (this may take a few minutes)..."
cd "$INSTALL_DIR"
cargo build --release
echo "✓ Build complete"

# Copy binary
sudo mkdir -p /opt/wolfscale
sudo cp target/release/wolfscale /opt/wolfscale/wolfscale
sudo chmod +x /opt/wolfscale/wolfscale

# Install wolfctl
if [ -f "target/release/wolfctl" ]; then
    sudo cp target/release/wolfctl /usr/local/bin/wolfctl
    sudo chmod +x /usr/local/bin/wolfctl
    echo "✓ wolfctl installed"
fi

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "             Load Balancer Configuration"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Check for existing config
CONFIG_FILE="/opt/wolfscale/wolfscale.toml"
PEERS=""

if [ -f "$CONFIG_FILE" ]; then
    echo "Found existing config: $CONFIG_FILE"
    
    # Extract peers from config
    DETECTED_PEERS=$(grep -E "^\s*peers\s*=" "$CONFIG_FILE" 2>/dev/null | sed 's/.*\[//' | sed 's/\].*//' | tr -d '"' | tr -d ' ' | tr ',' '\n' | paste -sd',' -)
    ADVERTISE=$(grep -E "^\s*advertise_address\s*=" "$CONFIG_FILE" 2>/dev/null | head -1 | sed 's/.*=\s*//' | tr -d '"' | tr -d ' ')
    
    if [ -n "$DETECTED_PEERS" ] || [ -n "$ADVERTISE" ]; then
        # Combine peers and advertise address
        ALL_PEERS=""
        [ -n "$DETECTED_PEERS" ] && ALL_PEERS="$DETECTED_PEERS"
        if [ -n "$ADVERTISE" ]; then
            [ -n "$ALL_PEERS" ] && ALL_PEERS="$ALL_PEERS,$ADVERTISE" || ALL_PEERS="$ADVERTISE"
        fi
        
        echo ""
        echo "Detected cluster peers from config:"
        echo "  $ALL_PEERS"
        echo ""
        read -p "Use these peers? (y/n) [y]: " USE_DETECTED < /dev/tty
        USE_DETECTED=${USE_DETECTED:-y}
        
        if [[ "$USE_DETECTED" =~ ^[Yy] ]]; then
            PEERS="$ALL_PEERS"
            echo "✓ Using peers from config"
        fi
    fi
else
    echo "No existing config found."
    echo ""
    echo "TIP: Copy wolfscale.toml from a cluster node:"
    echo "  scp user@cluster-node:/opt/wolfscale/wolfscale.toml /opt/wolfscale/"
    echo ""
fi

# If no peers detected, ask for them
if [ -z "$PEERS" ]; then
    echo "Enter WolfScale cluster node addresses."
    echo "(These are the WolfScale nodes, NOT MariaDB addresses)"
    echo ""
    
    PEER_LIST=()
    while true; do
        read -p "Cluster node address (e.g., 10.0.10.115:7654) [done]: " PEER < /dev/tty
        [ -z "$PEER" ] && break
        PEER_LIST+=("$PEER")
        echo "  Added: $PEER"
    done
    
    if [ ${#PEER_LIST[@]} -eq 0 ]; then
        echo "✗ Error: At least one peer address is required"
        exit 1
    fi
    
    PEERS=$(IFS=,; echo "${PEER_LIST[*]}")
fi

# Get listen address
echo ""
read -p "MySQL proxy listen address [0.0.0.0:3306]: " LISTEN_ADDR < /dev/tty
LISTEN_ADDR=${LISTEN_ADDR:-0.0.0.0:3306}

echo ""
echo "Load balancer configuration:"
echo "  Listen:  $LISTEN_ADDR (MySQL clients connect here)"
echo "  Peers:   $PEERS"
echo ""

# Create systemd service
echo "Creating systemd service..."

sudo tee /etc/systemd/system/wolfscale-lb.service > /dev/null << EOF
[Unit]
Description=WolfScale Load Balancer
After=network.target

[Service]
Type=simple
ExecStart=/opt/wolfscale/wolfscale load-balancer --peers $PEERS --listen $LISTEN_ADDR
Restart=always
RestartSec=5
User=root

# Security
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
ReadWritePaths=/var/log/wolfscale

[Install]
WantedBy=multi-user.target
EOF

# Create log directory
sudo mkdir -p /var/log/wolfscale

# Enable and start service
sudo systemctl daemon-reload
sudo systemctl enable wolfscale-lb
sudo systemctl start wolfscale-lb

echo "✓ wolfscale-lb service installed and started"

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║           Load Balancer Installation Complete!               ║"
echo "╠══════════════════════════════════════════════════════════════╣"
echo "║  Connect:  mysql -h 127.0.0.1 -P 3306 -u USER -p            ║"
echo "║  Status:   sudo systemctl status wolfscale-lb                ║"
echo "║  Logs:     sudo journalctl -u wolfscale-lb -f                ║"
echo "╚══════════════════════════════════════════════════════════════╝"
