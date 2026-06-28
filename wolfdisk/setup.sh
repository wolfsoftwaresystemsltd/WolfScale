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
        # Piped execution (curl | bash) — re-download and run as root.
        # Pass our args through (`-s -- "$@"`) so non-interactive flags like
        # --node-id / --bind survive the sudo re-exec (WolfDisk install report
        # B4, 2026-06-08 — they were dropped here before).
        SETUP_URL="https://raw.githubusercontent.com/wolfsoftwaresystemsltd/WolfScale/main/wolfdisk/setup.sh"
        curl -sSL "$SETUP_URL" | sudo bash -s -- "$@"
        exit $?
    fi
fi

# ─── Argument parsing (B4: non-interactive / automated installs) ────────────
# Supports the WolfStack dashboard (pipes `curl | bash` with no terminal) and
# scripted remote installs. Example:
#   curl -sSL .../wolfdisk/setup.sh | bash -s -- \
#     --node-id pve02 --role auto --bind 10.10.1.22:8650 \
#     --peers 10.10.1.21:8650 --data-dir /appdata/wolfdisk --mount /mnt/wolfdata
BRANCH="main"
NODE_ID=""; NODE_ROLE=""; BIND_ARG=""; PEERS_INPUT=""
DATA_DIR=""; MOUNT_PATH=""; DISCOVERY_MODE=""; FORCE_NONINTERACTIVE=false
while [ $# -gt 0 ]; do
    case "$1" in
        --beta) BRANCH="beta" ;;
        --node-id) NODE_ID="$2"; shift ;;           --node-id=*) NODE_ID="${1#*=}" ;;
        --role) NODE_ROLE="$2"; shift ;;            --role=*) NODE_ROLE="${1#*=}" ;;
        --bind) BIND_ARG="$2"; shift ;;             --bind=*) BIND_ARG="${1#*=}" ;;
        --peers) PEERS_INPUT="$2"; shift ;;         --peers=*) PEERS_INPUT="${1#*=}" ;;
        --data-dir) DATA_DIR="$2"; shift ;;         --data-dir=*) DATA_DIR="${1#*=}" ;;
        --mount) MOUNT_PATH="$2"; shift ;;          --mount=*) MOUNT_PATH="${1#*=}" ;;
        --discovery) DISCOVERY_MODE="$2"; shift ;;  --discovery=*) DISCOVERY_MODE="${1#*=}" ;;
        -y|--yes|--non-interactive) FORCE_NONINTERACTIVE=true ;;
        *) ;;
    esac
    shift
done

# Non-interactive when told to be, when any config flag was supplied, or when no
# controlling terminal is attached (a piped `curl | bash` — the dashboard path).
# Otherwise prompt as before.
if [ "$FORCE_NONINTERACTIVE" = "true" ] \
   || [ -n "$NODE_ID$NODE_ROLE$BIND_ARG$PEERS_INPUT$DATA_DIR$MOUNT_PATH$DISCOVERY_MODE" ]; then
    INTERACTIVE=false
elif (exec < /dev/tty) 2>/dev/null; then
    # Test that /dev/tty can actually be OPENED, not just that the device node
    # is mode-readable: a systemd-spawned `curl | bash` (the dashboard) has no
    # controlling terminal, so the open fails (ENXIO) and we go non-interactive
    # instead of dying on `read < /dev/tty`. A user who pipes it in their own
    # shell still gets the prompts.
    INTERACTIVE=true
else
    INTERACTIVE=false
fi

# data_dir: if --data-dir was supplied we honour it; otherwise the interactive
# config block below prompts for it (defaulting to /var/lib/wolfdisk) and creates
# it once chosen. Leave it unset here so the prompt's `-z "$DATA_DIR"` guard can
# fire. mount defaults later so we can tell a --mount flag from the default.

# Allow git to operate on repos owned by other users
export GIT_CONFIG_COUNT=1
export GIT_CONFIG_KEY_0=safe.directory
export GIT_CONFIG_VALUE_0="*"

# ─── Architecture detection for prebuilt binaries ──────────────────────────
HOST_ARCH=$(uname -m)
case "$HOST_ARCH" in
    x86_64)  BINARY_ARCH="x86_64" ;;
    aarch64) BINARY_ARCH="aarch64" ;;
    *)       BINARY_ARCH="" ;;  # unsupported — will build from source
esac

# Download a prebuilt binary from GitHub Releases.
# Queries the API to find the correct release asset (monorepo-safe).
# Usage: download_prebuilt <binary_name> <dest_path>
# Returns 0 on success, 1 on failure (caller should fall back to source build)
download_prebuilt() {
    local binary="$1" dest="$2"
    if [ -z "$BINARY_ARCH" ]; then
        return 1
    fi
    local asset="${binary}-${BINARY_ARCH}"
    echo "  Downloading prebuilt ${binary} for ${BINARY_ARCH}..."
    # Find download URL from the latest WolfScale release containing this asset
    local url
    url=$(curl -sSL "https://api.github.com/repos/wolfsoftwaresystemsltd/WolfScale/releases" \
        | grep "browser_download_url.*${asset}" | head -1 | cut -d'"' -f4)
    if [ -z "$url" ]; then
        echo "  ⚠ Prebuilt binary not available — will build from source"
        return 1
    fi
    local tmpfile="${dest}.download"
    if curl -fSL --connect-timeout 15 --max-time 300 --retry 2 -o "$tmpfile" "$url" 2>&1; then
        mv "$tmpfile" "$dest"
        chmod +x "$dest"
        echo "  ✓ Downloaded prebuilt ${binary} (${BINARY_ARCH})"
        return 0
    else
        echo "  ⚠ Prebuilt binary download failed — will build from source"
        rm -f "$tmpfile"
        return 1
    fi
}

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
elif command -v pacman &> /dev/null; then
    PKG_MANAGER="pacman"
    echo "  ✓ Detected Arch (pacman)"
elif command -v zypper &> /dev/null; then
    PKG_MANAGER="zypper"
    echo "  ✓ Detected openSUSE/SLES (zypper)"
elif command -v apk &> /dev/null; then
    PKG_MANAGER="apk"
    echo "  ✓ Detected Alpine (apk)"
else
    echo "  ✗ Could not detect package manager (apt/dnf/yum/pacman/zypper/apk)"
    echo "    Please install dependencies manually and run install_service.sh"
    exit 1
fi

# Install runtime dependencies (fuse3 is always needed)
echo ""
echo "  Installing system dependencies..."

if [ "$PKG_MANAGER" = "apt" ]; then
    apt update
    apt install -y curl fuse3
elif [ "$PKG_MANAGER" = "dnf" ]; then
    dnf install -y curl fuse3
elif [ "$PKG_MANAGER" = "yum" ]; then
    yum install -y curl fuse3
elif [ "$PKG_MANAGER" = "pacman" ]; then
    pacman -Sy --noconfirm curl fuse3
elif [ "$PKG_MANAGER" = "zypper" ]; then
    zypper install -y curl fuse3
elif [ "$PKG_MANAGER" = "apk" ]; then
    apk add curl fuse3
fi

echo "  ✓ System dependencies installed"

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

# --- Try prebuilt binaries first, fall back to source build ---
echo ""
WOLFDISK_PREBUILT=false
if download_prebuilt "wolfdisk" "/usr/local/bin/wolfdisk"; then
    download_prebuilt "wolfdiskctl" "/usr/local/bin/wolfdiskctl" || true
    WOLFDISK_PREBUILT=true
fi

if [ "$WOLFDISK_PREBUILT" = "false" ]; then
    echo ""
    echo "  Building from source..."

    # Install build dependencies
    if [ "$PKG_MANAGER" = "apt" ]; then
        apt install -y git build-essential pkg-config libssl-dev libfuse3-dev
    elif [ "$PKG_MANAGER" = "dnf" ]; then
        dnf install -y git gcc gcc-c++ make openssl-devel pkg-config fuse3-devel
    elif [ "$PKG_MANAGER" = "yum" ]; then
        yum install -y git gcc gcc-c++ make openssl-devel pkgconfig fuse3-devel
    elif [ "$PKG_MANAGER" = "pacman" ]; then
        pacman -S --noconfirm git base-devel openssl pkgconf fuse3
    elif [ "$PKG_MANAGER" = "zypper" ]; then
        zypper install -y git gcc gcc-c++ make libopenssl-devel pkg-config fuse3-devel
    elif [ "$PKG_MANAGER" = "apk" ]; then
        apk add git build-base openssl-dev pkgconf fuse3-dev
    fi

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
        git reset --hard origin/$BRANCH
    else
        git clone -b "$BRANCH" https://github.com/wolfsoftwaresystemsltd/WolfScale.git "$INSTALL_DIR"
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

    # Install binaries
    echo ""
    echo "  Installing WolfDisk..."
    cp "$INSTALL_DIR/wolfdisk/target/release/wolfdisk" /usr/local/bin/wolfdisk
    chmod +x /usr/local/bin/wolfdisk
    echo "  ✓ wolfdisk installed to /usr/local/bin/wolfdisk"

    cp "$INSTALL_DIR/wolfdisk/target/release/wolfdiskctl" /usr/local/bin/wolfdiskctl
    chmod +x /usr/local/bin/wolfdiskctl
    echo "  ✓ wolfdiskctl installed to /usr/local/bin/wolfdiskctl"
fi

# Create base directories. The data_dir itself is created in the config block
# below, once its location is known (honours --data-dir or the interactive prompt).
echo ""
echo "  Creating base directories..."
mkdir -p /etc/wolfdisk
mkdir -p /mnt/wolfdisk
echo "  ✓ Directories created"

# Create config if not exists. In interactive mode we prompt (reading /dev/tty);
# otherwise (flags supplied, -y, or no terminal) we use flags + sensible
# defaults so an unattended install just works (B4).
if [ ! -f "/etc/wolfdisk/config.toml" ]; then
    echo ""
    echo "  ─────────────────────────────────────"
    echo "  WolfDisk Configuration"
    echo "  ─────────────────────────────────────"
    echo ""

    DEFAULT_HOSTNAME=$(hostname)
    DEFAULT_IP=$(hostname -I | awk '{print $1}')
    DEFAULT_IP=${DEFAULT_IP:-"0.0.0.0"}

    # ── Node ID ──
    if [ "$INTERACTIVE" = "true" ]; then
        echo -n "  Node ID [${NODE_ID:-$DEFAULT_HOSTNAME}]: "
        read _in < /dev/tty; [ -n "$_in" ] && NODE_ID="$_in"
    fi
    NODE_ID=${NODE_ID:-$DEFAULT_HOSTNAME}

    # ── Role ──
    if [ "$INTERACTIVE" = "true" ] && [ -z "$NODE_ROLE" ]; then
        echo ""
        echo "  Node Roles:"
        echo "    1) auto     - Automatic election (lowest ID becomes leader)"
        echo "    2) leader   - Force this node to be leader"
        echo "    3) follower - Force this node to be follower"
        echo "    4) client   - Mount-only (no local storage, access remote data)"
        echo ""
        echo -n "  Select role [1-4, default: 1]: "
        read _in < /dev/tty
        case "$_in" in
            2) NODE_ROLE="leader" ;;
            3) NODE_ROLE="follower" ;;
            4) NODE_ROLE="client" ;;
            *) NODE_ROLE="auto" ;;
        esac
    fi
    NODE_ROLE=${NODE_ROLE:-auto}

    # ── Bind address (accepts IP or IP:PORT; default port 8650, NOT 8550 —
    #    8550..8599 is WolfStack's status-page range) ──
    if [ "$INTERACTIVE" = "true" ] && [ -z "$BIND_ARG" ]; then
        echo ""
        echo -n "  Bind IP address [$DEFAULT_IP]: "
        read _in < /dev/tty; [ -n "$_in" ] && BIND_ARG="$_in"
    fi
    BIND_PORT=8650
    case "$BIND_ARG" in
        *:*) BIND_IP="${BIND_ARG%:*}"; BIND_PORT="${BIND_ARG##*:}" ;;
        "")  BIND_IP="$DEFAULT_IP" ;;
        *)   BIND_IP="$BIND_ARG" ;;
    esac

    # ── Discovery / peers ──
    if [ "$INTERACTIVE" = "true" ] && [ -z "$DISCOVERY_MODE" ] && [ -z "$PEERS_INPUT" ]; then
        echo ""
        echo "  Cluster Discovery:"
        echo "    1) Auto-discovery (UDP broadcast - recommended for LAN)"
        echo "    2) Manual peers (specify IP addresses)"
        echo "    3) Standalone (single node, no clustering)"
        echo ""
        echo -n "  Select discovery method [1-3, default: 1]: "
        read _in < /dev/tty
        case "$_in" in
            2) DISCOVERY_MODE="peers"
               echo -n "  Enter peer addresses (comma-separated, e.g. 192.168.1.10:8650,192.168.1.11:8650): "
               read PEERS_INPUT < /dev/tty ;;
            3) DISCOVERY_MODE="standalone" ;;
            *) DISCOVERY_MODE="auto" ;;
        esac
    fi
    # A --peers value always implies manual-peer mode.
    [ -n "$PEERS_INPUT" ] && DISCOVERY_MODE="peers"
    DISCOVERY_MODE=${DISCOVERY_MODE:-auto}

    DISCOVERY_CONFIG=""
    PEERS_CONFIG="peers = []"
    case "$DISCOVERY_MODE" in
        peers)
            if [ -n "$PEERS_INPUT" ]; then
                PEERS_FORMATTED=$(echo "$PEERS_INPUT" | sed 's/^[[:space:]]*//; s/[[:space:]]*$//; s/ *, */", "/g')
                PEERS_CONFIG="peers = [\"$PEERS_FORMATTED\"]"
            fi
            ;;
        standalone) ;;  # no discovery, no peers
        *) DISCOVERY_CONFIG="discovery = \"udp://$BIND_IP:8651\"" ;;
    esac

    # ── Data directory (chunks + index + WAL live here) ──
    if [ "$INTERACTIVE" = "true" ] && [ -z "$DATA_DIR" ]; then
        echo ""
        echo "  Data directory — chunks, the index and the WAL are stored here."
        echo "  Put it on your bulk/fast data disk. To move it later you must stop"
        echo "  wolfdisk and move the contents, not just re-point this."
        echo -n "  Data directory [/var/lib/wolfdisk]: "
        read _in < /dev/tty; [ -n "$_in" ] && DATA_DIR="$_in"
    fi
    DATA_DIR=${DATA_DIR:-/var/lib/wolfdisk}
    mkdir -p "$DATA_DIR"/{chunks,index,wal}

    # ── Mount path ──
    if [ "$INTERACTIVE" = "true" ] && [ -z "$MOUNT_PATH" ]; then
        echo ""
        echo -n "  Mount path [/mnt/wolfdisk]: "
        read _in < /dev/tty; [ -n "$_in" ] && MOUNT_PATH="$_in"
    fi
    MOUNT_PATH=${MOUNT_PATH:-/mnt/wolfdisk}
    mkdir -p "$MOUNT_PATH"

    # ── Write config ──
    echo ""
    echo "  Creating configuration..."
    cat <<EOF > /etc/wolfdisk/config.toml
[node]
id = "$NODE_ID"
role = "$NODE_ROLE"
bind = "$BIND_IP:$BIND_PORT"
data_dir = "$DATA_DIR"

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
    echo "    Bind:       $BIND_IP:$BIND_PORT"
    echo "    Data dir:   $DATA_DIR"
    echo "    Mount:      $MOUNT_PATH"
    case "$DISCOVERY_MODE" in
        auto)       echo "    Discovery:  UDP broadcast (auto) on :8651" ;;
        peers)      echo "    Peers:      $PEERS_INPUT" ;;
        standalone) echo "    Mode:       Standalone" ;;
    esac
else
    echo ""
    echo "  ✓ Config already exists at /etc/wolfdisk/config.toml"
    echo "    (Upgrade mode - skipping configuration prompts)"
fi

# Service setup — ALWAYS (re)write the unit, every run. Previously this was
# gated on the file not existing, so an update could leave a node with a
# missing or stale wolfdisk.service (and "systemctl enable/start wolfdisk" then
# fails with "Unit wolfdisk.service not found"). Re-running setup.sh now always
# repairs it.
echo ""
echo "  ─────────────────────────────────────"
echo "  Installing systemd service..."
echo "  ─────────────────────────────────────"

# Get mount point from config or use default
SVC_MOUNT="/mnt/wolfdisk"
SVC_CONFIG="/etc/wolfdisk/config.toml"
if [ -f "$SVC_CONFIG" ]; then
    SVC_MOUNT_CFG=$(grep -E "^path\s*=" "$SVC_CONFIG" | cut -d'"' -f2 | head -1)
    [ -n "$SVC_MOUNT_CFG" ] && SVC_MOUNT="$SVC_MOUNT_CFG"
fi

# Enable user_allow_other in /etc/fuse.conf
if ! grep -q "^user_allow_other" /etc/fuse.conf 2>/dev/null; then
    echo "user_allow_other" >> /etc/fuse.conf
    echo "  ✓ Enabled user_allow_other in /etc/fuse.conf"
fi

cat << SVCEOF > /etc/systemd/system/wolfdisk.service
[Unit]
Description=WolfDisk Distributed File System
After=network.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/wolfdisk --config $SVC_CONFIG mount --mountpoint $SVC_MOUNT
ExecStop=/usr/local/bin/wolfdisk unmount --mountpoint $SVC_MOUNT
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal
NoNewPrivileges=false
ProtectSystem=false
PrivateTmp=false

[Install]
WantedBy=multi-user.target
SVCEOF

systemctl daemon-reload
echo "  ✓ Installed wolfdisk.service"

# Start/restart service
echo ""
if [ "$RESTART_SERVICE" = "true" ]; then
    echo "  Restarting WolfDisk service..."
    systemctl start wolfdisk
    echo "  ✓ Service restarted"
else
    echo "  Starting WolfDisk service..."
    systemctl enable wolfdisk
    systemctl start wolfdisk
    echo "  ✓ Service started and enabled"
fi

echo ""
echo "  🐺 Installation Complete!"
echo "  ─────────────────────────────────────"
echo "  Status:   sudo systemctl status wolfdisk"
echo "  Logs:     sudo journalctl -u wolfdisk -f"
echo "  Config:   /etc/wolfdisk/config.toml"
echo ""

# Container detection — warn about required LXC/Proxmox features
if [ -f /run/container_type ] || grep -qa 'lxc\|container' /proc/1/environ 2>/dev/null || [ -f /.dockerenv ] || grep -q 'lxc' /proc/1/cgroup 2>/dev/null; then
    echo ""
    echo "  ==========================================="
    echo "  ⚠️  IMPORTANT — RUNNING INSIDE A CONTAINER"
    echo "  ==========================================="
    echo ""
    echo "  WolfDisk requires /dev/tun and /dev/fuse"
    echo "  to be enabled in your container settings."
    echo ""
    echo "  If you are using WolfStack or Proxmox:"
    echo "    1. Stop this container"
    echo "    2. Go to the container's Settings page"
    echo "    3. Enable:  ✅ TUN/TAP Device"
    echo "    4. Enable:  ✅ FUSE"
    echo "    5. Save settings and start the container"
    echo ""
    echo "  Without these, WolfDisk will FAIL to start."
    echo "  ==========================================="
    echo ""
fi
