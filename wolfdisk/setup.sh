#!/bin/bash
#
# WolfDisk Quick Install Script
# Installs WolfDisk on Ubuntu/Debian (apt) or Fedora/RHEL (dnf)
#
# Usage: curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfDisk/main/setup.sh | bash
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

# Create default config if not exists
if [ ! -f "/etc/wolfdisk/config.toml" ]; then
    echo ""
    echo "Creating default configuration..."
    HOSTNAME=$(hostname)
    cat << EOF | sudo tee /etc/wolfdisk/config.toml > /dev/null
[node]
id = "$HOSTNAME"
bind = "0.0.0.0:9500"
data_dir = "/var/lib/wolfdisk"

[cluster]
peers = []
# discovery = "udp://239.0.0.1:9501"

[replication]
mode = "shared"
factor = 3
chunk_size = 4194304  # 4MB

[mount]
path = "/mnt/wolfdisk"
allow_other = true
EOF
    echo "✓ Default config created at /etc/wolfdisk/config.toml"
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
