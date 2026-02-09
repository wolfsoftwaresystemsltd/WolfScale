#!/bin/bash
# WolfNet Setup Script
# Installs WolfNet as a systemd service with interactive configuration
#
# Usage: curl -sSL https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/wolfnet/setup.sh | bash

set -e

REPO="wolfsoftwaresystemsltd/WolfScale"
INSTALL_DIR="/usr/local/bin"
CONFIG_DIR="/etc/wolfnet"
CONFIG_FILE="$CONFIG_DIR/config.toml"
KEY_FILE="$CONFIG_DIR/private.key"
SERVICE_FILE="/etc/systemd/system/wolfnet.service"
STATUS_DIR="/var/run/wolfnet"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${CYAN}"
echo "  ╔═══════════════════════════════════════╗"
echo "  ║     🐺 WolfNet Installer              ║"
echo "  ║     Secure Private Mesh Networking     ║"
echo "  ╚═══════════════════════════════════════╝"
echo -e "${NC}"

# Ensure running as root
if [ "$EUID" -ne 0 ]; then
    echo -e "${RED}Error: Please run as root (sudo)${NC}"
    echo "  curl -sSL ... | sudo bash"
    exit 1
fi

# Reopen stdin for interactive prompts (needed when piped via curl | bash)
exec 3</dev/tty || exec 3<&0

prompt() {
    local var_name="$1" prompt_text="$2" default="$3"
    echo -ne "${CYAN}$prompt_text ${YELLOW}[$default]${NC}: "
    read -r input <&3
    eval "$var_name=\"${input:-$default}\""
}

# ─── Check for /dev/net/tun ─────────────────────────────────────────────────
echo -e "${CYAN}Checking system requirements...${NC}"

if [ ! -e /dev/net/tun ]; then
    echo ""
    echo -e "${RED}╔══════════════════════════════════════════════════════════╗${NC}"
    echo -e "${RED}║  ⚠️  /dev/net/tun is NOT available!                     ║${NC}"
    echo -e "${RED}╚══════════════════════════════════════════════════════════╝${NC}"
    echo ""
    echo -e "${YELLOW}This is common in Proxmox LXC containers.${NC}"
    echo ""
    echo "To fix this, run the following on the Proxmox HOST (not inside the container):"
    echo ""
    echo -e "  ${GREEN}1. Edit the container config:${NC}"
    echo -e "     nano /etc/pve/lxc/<CTID>.conf"
    echo ""
    echo -e "  ${GREEN}2. Add these lines:${NC}"
    echo -e "     lxc.cgroup2.devices.allow: c 10:200 rwm"
    echo -e "     lxc.mount.entry: /dev/net dev/net none bind,create=dir"
    echo ""
    echo -e "  ${GREEN}3. Restart the container:${NC}"
    echo -e "     pct restart <CTID>"
    echo ""
    echo -e "  ${GREEN}4. Inside the container, create the device if needed:${NC}"
    echo -e "     mkdir -p /dev/net"
    echo -e "     mknod /dev/net/tun c 10 200"
    echo -e "     chmod 666 /dev/net/tun"
    echo ""
    echo -ne "Continue anyway? (y/N): "
    read -r cont <&3
    if [ "$cont" != "y" ] && [ "$cont" != "Y" ]; then
        echo "Aborted. Fix /dev/net/tun and try again."
        exit 1
    fi
else
    echo -e "  ${GREEN}✓${NC} /dev/net/tun available"
fi

echo -e "  ${GREEN}✓${NC} Running as root"

# ─── Detect architecture and download ───────────────────────────────────────
ARCH=$(uname -m)
case "$ARCH" in
    x86_64)  ARCH_LABEL="x86_64" ;;
    aarch64) ARCH_LABEL="aarch64" ;;
    *)       echo -e "${RED}Unsupported architecture: $ARCH${NC}"; exit 1 ;;
esac
echo -e "  ${GREEN}✓${NC} Architecture: $ARCH_LABEL"

# Download latest release
echo ""
echo -e "${CYAN}Downloading WolfNet...${NC}"
LATEST_URL="https://api.github.com/repos/$REPO/releases/latest"
RELEASE_INFO=$(curl -s "$LATEST_URL" 2>/dev/null || echo "")

if echo "$RELEASE_INFO" | grep -q "wolfnet"; then
    DOWNLOAD_URL=$(echo "$RELEASE_INFO" | grep -o "https://.*wolfnet.*linux.*$ARCH_LABEL[^\"]*" | head -1)
    if [ -n "$DOWNLOAD_URL" ]; then
        curl -sSL "$DOWNLOAD_URL" -o /tmp/wolfnet
        curl -sSL "${DOWNLOAD_URL/wolfnet/wolfnetctl}" -o /tmp/wolfnetctl 2>/dev/null || true
    fi
fi

# If no release found, try building from source or use local binary
if [ ! -f /tmp/wolfnet ]; then
    echo -e "${YELLOW}No pre-built binary found. Checking for local build...${NC}"
    # Try to find a local build
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)" || SCRIPT_DIR=""
    if [ -f "$SCRIPT_DIR/target/release/wolfnet" ]; then
        cp "$SCRIPT_DIR/target/release/wolfnet" /tmp/wolfnet
        cp "$SCRIPT_DIR/target/release/wolfnetctl" /tmp/wolfnetctl 2>/dev/null || true
    else
        echo -e "${RED}Could not find WolfNet binary.${NC}"
        echo "Please build from source: cd wolfnet && cargo build --release"
        exit 1
    fi
fi

# Install binaries
echo -e "${CYAN}Installing binaries...${NC}"
install -m 0755 /tmp/wolfnet "$INSTALL_DIR/wolfnet"
echo -e "  ${GREEN}✓${NC} Installed wolfnet to $INSTALL_DIR/wolfnet"

if [ -f /tmp/wolfnetctl ]; then
    install -m 0755 /tmp/wolfnetctl "$INSTALL_DIR/wolfnetctl"
    echo -e "  ${GREEN}✓${NC} Installed wolfnetctl to $INSTALL_DIR/wolfnetctl"
fi

rm -f /tmp/wolfnet /tmp/wolfnetctl

# ─── Interactive Configuration ──────────────────────────────────────────────
echo ""
echo -e "${CYAN}Configuration${NC}"
echo "─────────────────────────────────────────"

# Detect hostname and IP
DEFAULT_HOSTNAME=$(hostname -s)
DEFAULT_IP=$(ip -4 route get 8.8.8.8 2>/dev/null | grep -oP 'src \K\S+' || echo "10.0.10.1")

prompt NODE_ADDRESS "WolfNet IP address for this node" "10.0.10.1"
prompt SUBNET "Subnet mask (CIDR)" "24"
prompt LISTEN_PORT "UDP listen port" "9600"
prompt GATEWAY_MODE "Enable gateway mode (NAT internet for network)" "no"
prompt DISCOVERY "Enable LAN auto-discovery" "yes"

IS_GATEWAY="false"
[ "$GATEWAY_MODE" = "yes" ] || [ "$GATEWAY_MODE" = "y" ] && IS_GATEWAY="true"

DISC_ENABLED="true"
[ "$DISCOVERY" = "no" ] || [ "$DISCOVERY" = "n" ] && DISC_ENABLED="false"

# ─── Generate Keys ──────────────────────────────────────────────────────────
echo ""
echo -e "${CYAN}Setting up encryption keys...${NC}"
mkdir -p "$CONFIG_DIR"

if [ ! -f "$KEY_FILE" ]; then
    "$INSTALL_DIR/wolfnet" genkey --output "$KEY_FILE"
    echo -e "  ${GREEN}✓${NC} Generated new keypair"
else
    echo -e "  ${GREEN}✓${NC} Using existing private key"
fi

PUBLIC_KEY=$("$INSTALL_DIR/wolfnet" pubkey --config "$CONFIG_FILE" 2>/dev/null || "$INSTALL_DIR/wolfnet" pubkey 2>/dev/null || echo "unknown")

# ─── Write Config ───────────────────────────────────────────────────────────
echo ""
echo -e "${CYAN}Writing configuration...${NC}"
mkdir -p "$CONFIG_DIR"

cat > "$CONFIG_FILE" << EOF
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

echo -e "  ${GREEN}✓${NC} Config written to $CONFIG_FILE"

# ─── Create Systemd Service ────────────────────────────────────────────────
echo ""
echo -e "${CYAN}Creating systemd service...${NC}"

cat > "$SERVICE_FILE" << EOF
[Unit]
Description=WolfNet - Secure Private Mesh Networking
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=$INSTALL_DIR/wolfnet --config $CONFIG_FILE
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
echo -e "  ${GREEN}✓${NC} Systemd service created"

# Create status directory
mkdir -p "$STATUS_DIR"

# ─── Enable and Start ──────────────────────────────────────────────────────
echo ""
echo -ne "${CYAN}Start WolfNet now? ${YELLOW}[Y/n]${NC}: "
read -r start_now <&3
if [ "$start_now" != "n" ] && [ "$start_now" != "N" ]; then
    systemctl enable wolfnet
    systemctl start wolfnet
    sleep 1
    if systemctl is-active --quiet wolfnet; then
        echo -e "  ${GREEN}✓${NC} WolfNet is running!"
    else
        echo -e "  ${YELLOW}⚠${NC} WolfNet may have failed to start. Check: journalctl -u wolfnet -n 20"
    fi
else
    systemctl enable wolfnet
    echo -e "  ${GREEN}✓${NC} WolfNet enabled (will start on boot)"
fi

# ─── Summary ────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}╔══════════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║  ✅  WolfNet Installation Complete!                      ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "  WolfNet IP:    ${CYAN}$NODE_ADDRESS/$SUBNET${NC}"
echo -e "  Listen Port:   ${CYAN}$LISTEN_PORT${NC}"
echo -e "  Gateway:       ${CYAN}$IS_GATEWAY${NC}"
echo -e "  Config:        ${CYAN}$CONFIG_FILE${NC}"
echo -e "  Public Key:    ${CYAN}$PUBLIC_KEY${NC}"
echo ""
echo "  Useful commands:"
echo -e "    ${YELLOW}wolfnetctl status${NC}      — Show node status"
echo -e "    ${YELLOW}wolfnetctl peers${NC}       — List network peers"
echo -e "    ${YELLOW}systemctl status wolfnet${NC} — Service status"
echo -e "    ${YELLOW}journalctl -u wolfnet -f${NC} — Live logs"
echo ""
echo -e "  To add a peer on another machine, share your public key:"
echo -e "    ${CYAN}$PUBLIC_KEY${NC}"
echo ""

exec 3<&-
