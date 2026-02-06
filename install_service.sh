#!/bin/bash
# WolfScale Service Installer
# Installs WolfScale as a systemd service
# Run with sudo: sudo ./install_service.sh [node|lb]

set -e

SERVICE_TYPE="${1:-node}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="$SCRIPT_DIR/target/release/wolfscale"
SOURCE_CONFIG="$SCRIPT_DIR/wolfscale.toml"
INSTALL_DIR="/opt/wolfscale"
CONFIG="$INSTALL_DIR/wolfscale.toml"
USER="wolfscale"

# Check if running as root
if [ "$EUID" -ne 0 ]; then
    echo "Please run as root: sudo $0 [node|lb]"
    exit 1
fi

# Validate service type
if [[ "$SERVICE_TYPE" != "node" && "$SERVICE_TYPE" != "lb" ]]; then
    echo "Usage: sudo $0 [node|lb]"
    echo "  node - Install WolfScale database node (requires MariaDB)"
    echo "  lb   - Install WolfScale load balancer (no database needed)"
    exit 1
fi

# Check for binary
if [ ! -f "$BINARY" ]; then
    echo "ERROR: WolfScale binary not found at $BINARY"
    echo "Please build first with: cargo build --release"
    exit 1
fi

echo "Installing WolfScale as systemd service ($SERVICE_TYPE mode)..."

# Stop existing service if running (for upgrades)
if systemctl is-active --quiet wolfscale 2>/dev/null; then
    echo "Stopping existing WolfScale service..."
    systemctl stop wolfscale
fi

# Create user if doesn't exist
if ! id "$USER" &>/dev/null; then
    useradd --system --no-create-home --shell /bin/false "$USER"
    echo "Created user: $USER"
fi

# Create install directory
mkdir -p "$INSTALL_DIR"
mkdir -p /var/lib/wolfscale
mkdir -p /var/log/wolfscale

# Copy binary
cp "$BINARY" "$INSTALL_DIR/wolfscale"
chmod +x "$INSTALL_DIR/wolfscale"

# ============ Configuration Wizard ============
# Different wizards for node vs load balancer mode

if [ "$SERVICE_TYPE" == "lb" ]; then
    # ========== LOAD BALANCER WIZARD ==========
    echo ""
    echo "Load Balancer Configuration"
    echo "============================"
    echo ""
    echo "The load balancer proxies MySQL connections to your WolfScale cluster."
    echo "It does NOT require a local MariaDB installation."
    echo ""
    
    # Check for existing config file that we can extract peers from
    LB_PEERS=""
    if [ -f "$SOURCE_CONFIG" ] || [ -f "$CONFIG" ]; then
        CONFIG_TO_READ="${SOURCE_CONFIG}"
        [ ! -f "$SOURCE_CONFIG" ] && CONFIG_TO_READ="$CONFIG"
        
        echo "Found existing config: $CONFIG_TO_READ"
        echo ""
        
        # Extract peers from the config file
        # Look for peers = ["addr1", "addr2"] or peers = ["addr1:7654"]
        EXTRACTED_PEERS=$(grep -E '^\s*peers\s*=' "$CONFIG_TO_READ" | sed 's/.*=\s*\[//' | sed 's/\]//' | tr -d '"' | tr -d ' ' | tr ',' '\n' | grep -v '^$' | tr '\n' ',' | sed 's/,$//')
        
        if [ -n "$EXTRACTED_PEERS" ]; then
            echo "Detected WolfScale cluster peers from config:"
            echo "  $EXTRACTED_PEERS"
            echo ""
            read -p "Use these peers? (y/n) [y]: " USE_EXTRACTED
            USE_EXTRACTED="${USE_EXTRACTED:-y}"
            
            if [[ "$USE_EXTRACTED" == "y" || "$USE_EXTRACTED" == "Y" ]]; then
                LB_PEERS="$EXTRACTED_PEERS"
                echo "Using peers from config file."
            fi
        fi
    fi
    
    # If no peers extracted or user wants manual entry
    if [ -z "$LB_PEERS" ]; then
        echo ""
        echo "Enter at least one WolfScale cluster node address."
        echo "These are the nodes in your WolfScale cluster (port 7654, not MariaDB 3306)."
        echo "Format: ip:port (one per line, press Enter after each)"
        echo "Example: 10.0.10.115:7654"
        echo "Press Enter on an empty line when done."
        echo ""
        while true; do
            read -p "Cluster node address: " PEER
            [ -z "$PEER" ] && break
            if [ -z "$LB_PEERS" ]; then
                LB_PEERS="$PEER"
            else
                LB_PEERS="$LB_PEERS,$PEER"
            fi
        done
    fi
    
    if [ -z "$LB_PEERS" ]; then
        echo "ERROR: At least one cluster node address is required!"
        exit 1
    fi
    
    # Listen address
    echo ""
    read -p "MySQL proxy listen address [0.0.0.0:3306]: " LB_LISTEN
    LB_LISTEN="${LB_LISTEN:-0.0.0.0:3306}"
    
    # Max lag
    read -p "Maximum replication lag (entries) [100]: " MAX_LAG
    MAX_LAG="${MAX_LAG:-100}"
    
    echo ""
    echo "Load balancer configuration:"
    echo "  Listen:  $LB_LISTEN (MySQL clients connect here)"
    echo "  Peers:   $LB_PEERS"
    echo ""
else
    # ========== NODE WIZARD ==========
    if [ -f "$SOURCE_CONFIG" ]; then
        cp "$SOURCE_CONFIG" "$CONFIG"
        echo "Using existing configuration from $SOURCE_CONFIG"
    elif [ -f "$CONFIG" ]; then
        echo "Using existing configuration at $CONFIG"
    else
        echo ""
        echo "No configuration file found. Let's create one!"
        echo "=============================================="
        echo ""
        
        # Node ID
        HOSTNAME=$(hostname)
        read -p "Node ID [$HOSTNAME]: " NODE_ID
        NODE_ID="${NODE_ID:-$HOSTNAME}"
        
        # Bind address
        read -p "Bind address [0.0.0.0:7654]: " BIND_ADDR
        BIND_ADDR="${BIND_ADDR:-0.0.0.0:7654}"
        
        # Advertise address (external IP:port for cluster communication)
        echo ""
        echo "The advertise address is the IP:port other nodes use to reach this node."
        echo "Use this node's external/internal IP, e.g., 10.0.10.115:7654"
        read -p "Advertise address: " ADVERTISE_ADDR
        
        # Is this the bootstrap (first) node?
        read -p "Is this the FIRST node in the cluster? (y/n) [n]: " IS_BOOTSTRAP
        IS_BOOTSTRAP="${IS_BOOTSTRAP:-n}"
        
        # Convert to boolean for config file
        if [[ "$IS_BOOTSTRAP" == "y" || "$IS_BOOTSTRAP" == "Y" ]]; then
            IS_BOOTSTRAP_BOOL="true"
        else
            IS_BOOTSTRAP_BOOL="false"
        fi
        
        # Peer addresses (for all nodes - needed for cluster membership)
        PEERS=""
        echo ""
        echo "Enter ALL cluster peer addresses (including this node)."
        echo "Format: ip:port (one per line, press Enter after each)"
        echo "Example: 10.0.10.115:7654"
        echo "Press Enter on an empty line when done."
        echo ""
        while true; do
            read -p "Peer address: " PEER
            [ -z "$PEER" ] && break
            if [ -z "$PEERS" ]; then
                PEERS="\"$PEER\""
            else
                PEERS="$PEERS, \"$PEER\""
            fi
        done
        
        echo ""
        echo "Database Configuration"
        echo "======================"
        echo ""
        echo "WolfScale connects to the local MariaDB server and replicates ALL"
        echo "operations to other nodes. Enter the connection details below."
        echo "(The database name is just for the connection - all databases are synced)"
        echo ""
        # Database host
        read -p "MariaDB host [127.0.0.1]: " DB_HOST
        DB_HOST="${DB_HOST:-127.0.0.1}"
        
        # Database port
        read -p "MariaDB port [3306]: " DB_PORT
        DB_PORT="${DB_PORT:-3306}"
        
        # Database user
        read -p "Database user [wolfscale]: " DB_USER
        DB_USER="${DB_USER:-wolfscale}"
        
        # Database password
        read -sp "Database password: " DB_PASS
        echo ""
        
        # API port
        read -p "HTTP API port [8080]: " API_PORT
        API_PORT="${API_PORT:-8080}"
        
        # Proxy port
        read -p "MySQL proxy port [8007]: " PROXY_PORT
        PROXY_PORT="${PROXY_PORT:-8007}"
        
        # Generate config file
        cat > "$CONFIG" <<EOF
# WolfScale Configuration
# Generated by install_service.sh

[node]
id = "$NODE_ID"
bind_address = "$BIND_ADDR"
data_dir = "/var/lib/wolfscale/$NODE_ID"
advertise_address = "$ADVERTISE_ADDR"

[database]
host = "$DB_HOST"
port = $DB_PORT
user = "$DB_USER"
password = "$DB_PASS"
pool_size = 10

[wal]
batch_size = 1000
compression = true
segment_size_mb = 64
fsync = true

[cluster]
bootstrap = $IS_BOOTSTRAP_BOOL
peers = [$PEERS]
heartbeat_interval_ms = 500
election_timeout_ms = 2000
disable_auto_election = false

[api]
enabled = true
bind_address = "0.0.0.0:$API_PORT"

[proxy]
enabled = true
bind_address = "0.0.0.0:$PROXY_PORT"
EOF

        echo ""
        echo "Configuration saved to $CONFIG"
    fi
fi

# Set permissions
chown -R "$USER:$USER" "$INSTALL_DIR"
chown -R "$USER:$USER" /var/lib/wolfscale
chown -R "$USER:$USER" /var/log/wolfscale

# Create systemd service based on mode
if [ "$SERVICE_TYPE" == "lb" ]; then
    # Load Balancer Service
    SERVICE_NAME="wolfscale-lb"
    EXEC_START="$INSTALL_DIR/wolfscale load-balancer --peers $LB_PEERS --listen $LB_LISTEN --max-lag $MAX_LAG"
    DESCRIPTION="WolfScale Load Balancer"
    AFTER_DEPS="network.target"
    WANTS_LINE=""
else
    # Node Service - determine if bootstrap mode is needed
    BOOTSTRAP_FLAG=""
    if [[ "$IS_BOOTSTRAP" == "y" || "$IS_BOOTSTRAP" == "Y" ]]; then
        BOOTSTRAP_FLAG="--bootstrap"
    fi
    
    SERVICE_NAME="wolfscale"
    EXEC_START="$INSTALL_DIR/wolfscale --config $CONFIG start $BOOTSTRAP_FLAG"
    DESCRIPTION="WolfScale Distributed Database Manager"
    AFTER_DEPS="network.target mariadb.service"
    WANTS_LINE="Wants=mariadb.service"
fi

cat > "/etc/systemd/system/${SERVICE_NAME}.service" << EOF
[Unit]
Description=$DESCRIPTION
After=$AFTER_DEPS
$WANTS_LINE

[Service]
Type=simple
User=$USER
Group=$USER
WorkingDirectory=$INSTALL_DIR
ExecStart=$EXEC_START
Restart=always
RestartSec=5
StandardOutput=append:/var/log/wolfscale/${SERVICE_NAME}.log
StandardError=append:/var/log/wolfscale/${SERVICE_NAME}.error.log

# Security
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/wolfscale /var/log/wolfscale

[Install]
WantedBy=multi-user.target
EOF

# Install logrotate configuration
LOGROTATE_SRC="$SCRIPT_DIR/wolfscale.logrotate"
if [ -f "$LOGROTATE_SRC" ]; then
    cp "$LOGROTATE_SRC" /etc/logrotate.d/wolfscale
    echo "Installed log rotation configuration"
fi

# Reload systemd
systemctl daemon-reload

# Enable and start the service
echo "Enabling and starting service..."
systemctl enable "$SERVICE_NAME"
systemctl start "$SERVICE_NAME"

# Wait a moment for startup
sleep 2

# Check if service started successfully
if systemctl is-active --quiet "$SERVICE_NAME"; then
    echo ""
    echo "=============================================="
    echo "WolfScale installed and running!"
    echo "=============================================="
else
    echo ""
    echo "=============================================="
    echo "WolfScale installed (service may need attention)"
    echo "=============================================="
    echo ""
    echo "Check logs: sudo journalctl -u $SERVICE_NAME -n 50"
fi

echo ""
echo "Configuration: $CONFIG"
echo "Data directory: /var/lib/wolfscale"
echo "Logs: /var/log/wolfscale/"
echo ""
echo "Commands:"
echo "  sudo systemctl status $SERVICE_NAME    # Check status"
echo "  sudo systemctl restart $SERVICE_NAME   # Restart service"
echo "  sudo journalctl -u $SERVICE_NAME -f    # View logs"
echo ""
echo "To edit configuration:"
echo "  sudo nano $CONFIG"
echo "  sudo systemctl restart $SERVICE_NAME"
