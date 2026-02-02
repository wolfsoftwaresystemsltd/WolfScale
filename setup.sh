#!/bin/bash
#
# WolfScale Quick Install Script
# Installs WolfScale on Ubuntu/Debian (apt) or Fedora/RHEL (dnf)
#
# Usage: curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/setup.sh | bash
#

set -e

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║                    WolfScale Installer                        ║"
echo "║           Distributed MariaDB Synchronization                 ║"
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

# Clone repository
INSTALL_DIR="/opt/wolfscale-src"
echo ""
echo "Cloning WolfScale repository..."

if [ -d "$INSTALL_DIR" ]; then
    echo "  Updating existing installation..."
    cd "$INSTALL_DIR"
    sudo git pull
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

# Run installer
echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "Build complete! Starting service installer..."
echo "═══════════════════════════════════════════════════════════════"
echo ""

sudo ./install_service.sh

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║                Installation Complete!                        ║"
echo "╠══════════════════════════════════════════════════════════════╣"
echo "║  Connect:  mariadb -h 127.0.0.1 -P 3307 -u USER -p          ║"
echo "║  Status:   sudo systemctl status wolfscale                   ║"
echo "║  Logs:     sudo journalctl -u wolfscale -f                   ║"
echo "╚══════════════════════════════════════════════════════════════╝"
