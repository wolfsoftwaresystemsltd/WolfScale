#!/bin/bash
#
# WolfScale Quick Install Script
# Installs WolfScale on Ubuntu/Debian (apt) or Fedora/RHEL (dnf)
#
# Usage: curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/setup.sh | bash
#

set -e

echo ""
echo "  WolfScale Installer"
echo "  Distributed MariaDB Synchronization"
echo "  $(printf '%0.s─' {1..50})"
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

# Ensure cargo is in PATH
export PATH="$HOME/.cargo/bin:$PATH"

# Clone or update repository
INSTALL_DIR="/opt/wolfscale-src"
echo ""
echo "Cloning WolfScale repository..."

if [ -d "$INSTALL_DIR" ]; then
    echo "  Updating existing installation..."
    cd "$INSTALL_DIR"
    # Use fetch + reset instead of pull to handle force-pushes cleanly
    sudo git fetch origin
    sudo git reset --hard origin/main
    # Clear logs to prevent huge log files from accumulating
    if [ -f "/var/log/wolfscale/wolfscale.log" ]; then
        echo "  Clearing logs..."
        sudo truncate -s 0 /var/log/wolfscale/wolfscale.log
    fi
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

# Check if this is an upgrade (service already exists)
IS_UPGRADE=false
if systemctl list-unit-files | grep -q wolfscale.service; then
    IS_UPGRADE=true
    echo ""
    echo "✓ Detected existing WolfScale installation - performing upgrade"
fi

if [ "$IS_UPGRADE" = true ]; then
    # Upgrade mode: just copy binary and restart service
    echo ""
    echo "Upgrading WolfScale..."
    
    # Copy new binary
    sudo cp "$INSTALL_DIR/target/release/wolfscale" /usr/local/bin/wolfscale
    sudo chmod +x /usr/local/bin/wolfscale
    echo "✓ Binary updated"
    
    # Restart service
    sudo systemctl daemon-reload
    sudo systemctl restart wolfscale
    echo "✓ Service restarted"
    
    # Update wolfctl if present
    if [ -f "$INSTALL_DIR/target/release/wolfctl" ]; then
        sudo cp "$INSTALL_DIR/target/release/wolfctl" /usr/local/bin/wolfctl
        sudo chmod +x /usr/local/bin/wolfctl
        echo "✓ wolfctl updated"
    fi
    
    echo ""
    echo ""
    echo "  Upgrade Complete!"
    echo "  $(printf '%0.s─' {1..50})"
    echo "  Status:   sudo systemctl status wolfscale"
    echo "  Logs:     sudo journalctl -u wolfscale -f"
    echo ""
else
    # New install: run interactive installer
    echo ""
    echo "  $(printf '%0.s─' {1..50})"
    echo "  Build complete! Starting service installer..."
    echo "  $(printf '%0.s─' {1..50})"
    echo ""
    
    # Run installer with TTY for interactive input
    # (Needed because stdin is consumed when script is piped via curl)
    sudo ./install_service.sh < /dev/tty
    
    # Install wolfctl CLI tool to /usr/local/bin
    echo ""
    echo "Installing wolfctl CLI tool..."
    if [ -f "$INSTALL_DIR/target/release/wolfctl" ]; then
        sudo cp "$INSTALL_DIR/target/release/wolfctl" /usr/local/bin/wolfctl
        sudo chmod +x /usr/local/bin/wolfctl
        echo "✓ wolfctl installed to /usr/local/bin/wolfctl"
    else
        echo "⚠ wolfctl binary not found (may not have been built)"
    fi
    
    echo ""
    echo ""
    echo "  Installation Complete!"
    echo "  $(printf '%0.s─' {1..50})"
    echo "  Connect:  mariadb -h 127.0.0.1 -P 8007 -u USER -p"
    echo "  Status:   sudo systemctl status wolfscale"
    echo "  Logs:     sudo journalctl -u wolfscale -f"
    echo "  Cluster:  wolfctl list servers"
    echo ""
fi
