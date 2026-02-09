#!/bin/bash
#
# WolfNet Quick Install Script
# Installs WolfNet on Ubuntu/Debian (apt) or Fedora/RHEL (dnf)
#
# Usage: curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/wolfnet/setup.sh | sudo bash
#

set -e

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║              🐺  WolfNet Installer                          ║"
echo "║          Secure Private Mesh Networking                     ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""

# ─── Must run as root ────────────────────────────────────────────────────────
if [ "$(id -u)" -ne 0 ]; then
    echo "✗ This script must be run as root."
    echo "  Usage: sudo bash setup.sh"
    echo "     or: curl -sSL <url> | sudo bash"
    exit 1
fi

# Detect the real user (for Rust install) when running under sudo
REAL_USER="${SUDO_USER:-root}"
REAL_HOME=$(eval echo "~$REAL_USER")

# ─── Check for /dev/net/tun ─────────────────────────────────────────────────
echo "Checking system requirements..."

if [ ! -e /dev/net/tun ]; then
    echo ""
    echo "╔══════════════════════════════════════════════════════════════╗"
    echo "║  ⚠️   /dev/net/tun is NOT available!                        ║"
    echo "╚══════════════════════════════════════════════════════════════╝"
    echo ""
    echo "This is common in Proxmox LXC containers."
    echo ""
    echo "To fix this, run the following on the Proxmox HOST (not inside the container):"
    echo ""
    echo "  1. Edit the container config:"
    echo "     nano /etc/pve/lxc/<CTID>.conf"
    echo ""
    echo "  2. Add these lines:"
    echo "     lxc.cgroup2.devices.allow: c 10:200 rwm"
    echo "     lxc.mount.entry: /dev/net dev/net none bind,create=dir"
    echo ""
    echo "  3. Restart the container:"
    echo "     pct restart <CTID>"
    echo ""
    echo "  4. Inside the container, create the device if needed:"
    echo "     mkdir -p /dev/net"
    echo "     mknod /dev/net/tun c 10 200"
    echo "     chmod 666 /dev/net/tun"
    echo ""
    echo -n "Continue anyway? (y/N): "
    read cont < /dev/tty
    if [ "$cont" != "y" ] && [ "$cont" != "Y" ]; then
        echo "Aborted. Fix /dev/net/tun and try again."
        exit 1
    fi
else
    echo "✓ /dev/net/tun available"
fi

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
    echo "  Please install dependencies manually."
    exit 1
fi

# Install dependencies
echo ""
echo "Installing system dependencies..."

if [ "$PKG_MANAGER" = "apt" ]; then
    apt update
    apt install -y git curl build-essential pkg-config libssl-dev
elif [ "$PKG_MANAGER" = "dnf" ]; then
    dnf install -y git curl gcc gcc-c++ make openssl-devel pkg-config
elif [ "$PKG_MANAGER" = "yum" ]; then
    yum install -y git curl gcc gcc-c++ make openssl-devel pkgconfig
fi

echo "✓ System dependencies installed"

# Install Rust if not present (install as the real user, not root)
CARGO_BIN="$REAL_HOME/.cargo/bin/cargo"

if [ -f "$CARGO_BIN" ]; then
    echo "✓ Rust already installed"
elif command -v cargo &> /dev/null; then
    CARGO_BIN="$(command -v cargo)"
    echo "✓ Rust already installed (system-wide)"
else
    echo ""
    echo "Installing Rust for user '$REAL_USER'..."
    if [ "$REAL_USER" = "root" ]; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    else
        su - "$REAL_USER" -c "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"
    fi
    echo "✓ Rust installed"
fi

# Ensure cargo is found
export PATH="$REAL_HOME/.cargo/bin:/usr/local/bin:/usr/bin:$PATH"

if ! command -v cargo &> /dev/null; then
    echo "✗ cargo not found after installation. Check Rust install."
    exit 1
fi

echo "✓ Using cargo: $(command -v cargo)"

# Clone or update repository
INSTALL_DIR="/opt/wolfscale-src"
echo ""
echo "Cloning WolfScale repository..."

if [ -d "$INSTALL_DIR" ]; then
    echo "  Updating existing installation..."
    cd "$INSTALL_DIR"
    git fetch origin
    git reset --hard origin/main
else
    git clone https://github.com/wolfsoftwaresystemsltd/WolfScale.git "$INSTALL_DIR"
    cd "$INSTALL_DIR"
fi

echo "✓ Repository cloned to $INSTALL_DIR"

# Build WolfNet (as the real user if possible, to use their cargo)
echo ""
echo "Building WolfNet (this may take a few minutes)..."
cd "$INSTALL_DIR/wolfnet"

if [ "$REAL_USER" != "root" ] && [ -f "$REAL_HOME/.cargo/bin/cargo" ]; then
    # Build as the real user so cargo uses their toolchain
    chown -R "$REAL_USER:$REAL_USER" "$INSTALL_DIR"
    su - "$REAL_USER" -c "cd $INSTALL_DIR/wolfnet && $REAL_HOME/.cargo/bin/cargo build --release"
else
    cargo build --release
fi

echo "✓ Build complete"

# Stop service if running (for upgrades)
if systemctl is-active --quiet wolfnet 2>/dev/null; then
    echo ""
    echo "Stopping WolfNet service for upgrade..."
    systemctl stop wolfnet
    sleep 2
    echo "✓ Service stopped"
    RESTART_SERVICE=true
else
    RESTART_SERVICE=false
fi

# Install binary
echo ""
if [ -f "/usr/local/bin/wolfnet" ]; then
    echo "Upgrading WolfNet..."
    rm -f /usr/local/bin/wolfnet
else
    echo "Installing WolfNet..."
fi
cp "$INSTALL_DIR/wolfnet/target/release/wolfnet" /usr/local/bin/wolfnet
chmod +x /usr/local/bin/wolfnet
echo "✓ wolfnet installed to /usr/local/bin/wolfnet"

# Install wolfnetctl if it exists
if [ -f "$INSTALL_DIR/wolfnet/target/release/wolfnetctl" ]; then
    cp "$INSTALL_DIR/wolfnet/target/release/wolfnetctl" /usr/local/bin/wolfnetctl
    chmod +x /usr/local/bin/wolfnetctl
    echo "✓ wolfnetctl installed to /usr/local/bin/wolfnetctl"
fi

# Create directories
echo ""
echo "Creating directories..."
mkdir -p /etc/wolfnet
mkdir -p /var/run/wolfnet
echo "✓ Directories created"

# Create config if not exists - with interactive prompts
# Use /dev/tty to read from terminal even when script is piped
if [ ! -f "/etc/wolfnet/config.toml" ]; then
    echo ""
    echo "═══════════════════════════════════════════════════════════════"
    echo "                   WolfNet Configuration"
    echo "═══════════════════════════════════════════════════════════════"
    echo ""

    # Detect default IP
    DEFAULT_IP=$(hostname -I | awk '{print $1}')
    DEFAULT_IP=${DEFAULT_IP:-"0.0.0.0"}

    # Prompt for WolfNet IP address
    echo -n "WolfNet IP address for this node [10.0.10.1]: "
    read NODE_ADDRESS < /dev/tty
    NODE_ADDRESS=${NODE_ADDRESS:-10.0.10.1}

    # Prompt for subnet
    echo -n "Subnet mask (CIDR) [24]: "
    read SUBNET < /dev/tty
    SUBNET=${SUBNET:-24}

    # Prompt for listen port
    echo -n "UDP listen port [9600]: "
    read LISTEN_PORT < /dev/tty
    LISTEN_PORT=${LISTEN_PORT:-9600}

    # Prompt for gateway mode
    echo ""
    echo "Gateway mode enables NAT so other WolfNet nodes can access"
    echo "the internet through this node."
    echo -n "Enable gateway mode? [y/N]: "
    read GATEWAY_MODE < /dev/tty
    IS_GATEWAY="false"
    if [ "$GATEWAY_MODE" = "y" ] || [ "$GATEWAY_MODE" = "Y" ] || [ "$GATEWAY_MODE" = "yes" ]; then
        IS_GATEWAY="true"
    fi

    # Prompt for discovery
    echo ""
    echo -n "Enable LAN auto-discovery? [Y/n]: "
    read DISCOVERY < /dev/tty
    DISC_ENABLED="true"
    if [ "$DISCOVERY" = "n" ] || [ "$DISCOVERY" = "N" ] || [ "$DISCOVERY" = "no" ]; then
        DISC_ENABLED="false"
    fi

    # Generate keys
    echo ""
    echo "Generating encryption keys..."
    KEY_FILE="/etc/wolfnet/private.key"
    if [ ! -f "$KEY_FILE" ]; then
        /usr/local/bin/wolfnet genkey --output "$KEY_FILE" 2>/dev/null || {
            # If genkey fails, create a placeholder
            echo "Note: Key generation via CLI not available."
            echo "  You may need to generate keys manually."
            KEY_FILE="/etc/wolfnet/private.key"
        }
        if [ -f "$KEY_FILE" ]; then
            echo "✓ Generated new keypair"
        fi
    else
        echo "✓ Using existing private key"
    fi

    # Get public key
    PUBLIC_KEY=$(/usr/local/bin/wolfnet pubkey --config /etc/wolfnet/config.toml 2>/dev/null || \
                 /usr/local/bin/wolfnet pubkey 2>/dev/null || echo "unknown")

    # Write config
    echo ""
    echo "Creating configuration..."
    cat <<EOF > /etc/wolfnet/config.toml
# WolfNet Configuration
# Generated by setup.sh

[network]
interface = "wolfnet0"
address = "$NODE_ADDRESS"
subnet = $SUBNET
listen_port = $LISTEN_PORT
gateway = $IS_GATEWAY
discovery = $DISC_ENABLED
mtu = 1400

[security]
private_key_file = "$KEY_FILE"

# Add peers here:
# [[peers]]
# public_key = "base64_encoded_public_key"
# endpoint = "1.2.3.4:9600"
# allowed_ip = "10.0.10.2"
# name = "server2"
EOF
    echo "✓ Config created at /etc/wolfnet/config.toml"
    echo ""
    echo "Configuration Summary:"
    echo "  WolfNet IP: $NODE_ADDRESS/$SUBNET"
    echo "  Listen:     $LISTEN_PORT/udp"
    echo "  Gateway:    $IS_GATEWAY"
    echo "  Discovery:  $DISC_ENABLED"
else
    echo ""
    echo "✓ Config already exists at /etc/wolfnet/config.toml"
    echo "  (Upgrade mode - skipping configuration prompts)"
fi

# Create systemd service if not exists
if [ ! -f "/etc/systemd/system/wolfnet.service" ]; then
    echo ""
    echo "═══════════════════════════════════════════════════════════════"
    echo "Creating systemd service..."
    echo "═══════════════════════════════════════════════════════════════"
    echo ""

    cat > /etc/systemd/system/wolfnet.service <<EOF
[Unit]
Description=WolfNet - Secure Private Mesh Networking
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/wolfnet --config /etc/wolfnet/config.toml
Restart=on-failure
RestartSec=5
LimitNOFILE=65535

# Security hardening
ProtectSystem=false
ProtectHome=false
NoNewPrivileges=false

# Ensure /dev/net/tun access
DeviceAllow=/dev/net/tun rw

# Status directory
RuntimeDirectory=wolfnet
RuntimeDirectoryMode=0755

[Install]
WantedBy=multi-user.target
EOF

    systemctl daemon-reload
    echo "✓ Systemd service created"

    # Enable and optionally start
    echo ""
    echo -n "Start WolfNet now? [Y/n]: "
    read start_now < /dev/tty
    if [ "$start_now" != "n" ] && [ "$start_now" != "N" ]; then
        systemctl enable wolfnet
        systemctl start wolfnet
        sleep 2
        if systemctl is-active --quiet wolfnet; then
            echo "✓ WolfNet is running!"
        else
            echo "⚠ WolfNet may have failed to start. Check: journalctl -u wolfnet -n 20"
        fi
    else
        systemctl enable wolfnet
        echo "✓ WolfNet enabled (will start on boot)"
    fi
else
    echo ""
    echo "✓ Service already installed - reloading systemd"
    systemctl daemon-reload
fi

# Restart service if it was running before upgrade
if [ "$RESTART_SERVICE" = "true" ]; then
    echo ""
    echo "Restarting WolfNet service..."
    systemctl start wolfnet
    echo "✓ Service restarted"
fi

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║              Installation Complete!                         ║"
echo "╠══════════════════════════════════════════════════════════════╣"
echo "║  Status:   sudo systemctl status wolfnet                    ║"
echo "║  Logs:     sudo journalctl -u wolfnet -f                    ║"
echo "║  Config:   /etc/wolfnet/config.toml                         ║"
echo "╚══════════════════════════════════════════════════════════════╝"
