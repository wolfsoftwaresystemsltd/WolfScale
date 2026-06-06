#!/bin/bash
#
# WolfScale Quick Install Script
# Installs WolfScale on Ubuntu/Debian (apt) or Fedora/RHEL (dnf).
#
# Downloads a prebuilt static binary from the release pipeline (the
# 'wolfscale-latest' GitHub release). Only falls back to a source build if no
# prebuilt binary is available for this CPU architecture or the download fails.
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

# Detect CPU architecture for the prebuilt binary
case "$(uname -m)" in
    x86_64|amd64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *) ARCH="" ;;
esac

RELEASE_BASE="https://github.com/wolfsoftwaresystemsltd/WolfScale/releases/download/wolfscale-latest"

# Root-only environments (e.g. a minimal Proxmox VE host) frequently ship no
# `sudo` binary. Use sudo only when we're NOT already root; if we're not root
# and sudo is missing, fail clearly up front instead of erroring on every line.
# (JJ 2026-06: the installer assumed sudo and broke on root-only Proxmox.)
if [ "$(id -u)" -eq 0 ]; then
    SUDO=""
elif command -v sudo >/dev/null 2>&1; then
    SUDO="sudo"
else
    echo "ERROR: not running as root and 'sudo' is not installed — re-run this script as root." >&2
    exit 1
fi

# Base dependencies: git + curl are enough to fetch the repo (for
# install_service.sh) and download the prebuilt binary. Build tools are only
# installed in the source-build fallback below.
echo ""
echo "Installing base dependencies..."
if [ "$PKG_MANAGER" = "apt" ]; then
    $SUDO apt update
    $SUDO apt install -y git curl ca-certificates
else
    $SUDO "$PKG_MANAGER" install -y git curl ca-certificates
fi
echo "✓ Base dependencies installed"

# Clone or update the repository — needed for install_service.sh + upgrade
# detection. (A shallow clone is cheap; we no longer build it here.)
INSTALL_DIR="/opt/wolfscale-src"
echo ""
echo "Fetching WolfScale..."
if [ -d "$INSTALL_DIR/.git" ]; then
    echo "  Updating existing checkout..."
    cd "$INSTALL_DIR"
    $SUDO git fetch origin
    $SUDO git reset --hard origin/main
    if [ -f "/var/log/wolfscale/wolfscale.log" ]; then
        $SUDO truncate -s 0 /var/log/wolfscale/wolfscale.log
    fi
else
    $SUDO git clone --depth 1 https://github.com/wolfsoftwaresystemsltd/WolfScale.git "$INSTALL_DIR"
    cd "$INSTALL_DIR"
fi
$SUDO chown -R "$USER:$USER" "$INSTALL_DIR"
mkdir -p "$INSTALL_DIR/target/release"
echo "✓ Repository ready at $INSTALL_DIR"

# Get the binary: prefer the prebuilt release artifact, build from source only
# if it's unavailable for this arch. The rest of the script copies from
# $INSTALL_DIR/target/release, so we drop the downloaded binary there.
echo ""
GOT_BINARY=false
if [ -n "$ARCH" ]; then
    echo "Downloading prebuilt WolfScale binary ($ARCH)..."
    if curl -fsSL "$RELEASE_BASE/wolfscale-$ARCH" -o "$INSTALL_DIR/target/release/wolfscale" \
        && chmod +x "$INSTALL_DIR/target/release/wolfscale" \
        && "$INSTALL_DIR/target/release/wolfscale" --version >/dev/null 2>&1; then
        curl -fsSL "$RELEASE_BASE/wolfctl-$ARCH" -o "$INSTALL_DIR/target/release/wolfctl" 2>/dev/null \
            && chmod +x "$INSTALL_DIR/target/release/wolfctl" || true
        GOT_BINARY=true
        echo "✓ Installed prebuilt binary ($("$INSTALL_DIR/target/release/wolfscale" --version 2>/dev/null))"
    else
        echo "⚠ Prebuilt binary not available for $ARCH — falling back to a source build"
    fi
else
    echo "⚠ Unknown CPU architecture ($(uname -m)) — falling back to a source build"
fi

if [ "$GOT_BINARY" = false ]; then
    echo ""
    echo "Building WolfScale from source (this may take a few minutes)..."
    if [ "$PKG_MANAGER" = "apt" ]; then
        $SUDO apt install -y build-essential pkg-config libssl-dev
    elif [ "$PKG_MANAGER" = "dnf" ]; then
        $SUDO dnf install -y gcc gcc-c++ make openssl-devel pkg-config
    else
        $SUDO yum install -y gcc gcc-c++ make openssl-devel pkgconfig
    fi
    if ! command -v rustc &> /dev/null; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source "$HOME/.cargo/env"
    fi
    export PATH="$HOME/.cargo/bin:$PATH"
    cd "$INSTALL_DIR"
    cargo build --release
    echo "✓ Build complete"
fi

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

    $SUDO cp "$INSTALL_DIR/target/release/wolfscale" /usr/local/bin/wolfscale
    $SUDO chmod +x /usr/local/bin/wolfscale
    echo "✓ Binary updated"

    $SUDO systemctl daemon-reload
    $SUDO systemctl restart wolfscale
    echo "✓ Service restarted"

    if [ -f "$INSTALL_DIR/target/release/wolfctl" ]; then
        $SUDO cp "$INSTALL_DIR/target/release/wolfctl" /usr/local/bin/wolfctl
        $SUDO chmod +x /usr/local/bin/wolfctl
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
    echo "  Binary ready! Starting service installer..."
    echo "  $(printf '%0.s─' {1..50})"
    echo ""

    # Run installer with TTY for interactive input
    # (Needed because stdin is consumed when script is piped via curl)
    $SUDO ./install_service.sh < /dev/tty

    echo ""
    echo "Installing wolfctl CLI tool..."
    if [ -f "$INSTALL_DIR/target/release/wolfctl" ]; then
        $SUDO cp "$INSTALL_DIR/target/release/wolfctl" /usr/local/bin/wolfctl
        $SUDO chmod +x /usr/local/bin/wolfctl
        echo "✓ wolfctl installed to /usr/local/bin/wolfctl"
    else
        echo "⚠ wolfctl binary not found"
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
