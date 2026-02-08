#!/bin/bash
#
# WolfDisk Service Installer
# Sets up systemd service for WolfDisk
#

set -e

echo "WolfDisk Service Installer"
echo "=========================="
echo ""

# Check for root
if [ "$EUID" -ne 0 ]; then
    echo "Please run as root (sudo)"
    exit 1
fi

# Get mount point from config or use default
MOUNT_POINT="/mnt/wolfdisk"
CONFIG_FILE="/etc/wolfdisk/config.toml"

if [ -f "$CONFIG_FILE" ]; then
    # Try to extract mount path from config
    MOUNT_FROM_CONFIG=$(grep -E "^path\s*=" "$CONFIG_FILE" | cut -d'"' -f2 | head -1)
    if [ -n "$MOUNT_FROM_CONFIG" ]; then
        MOUNT_POINT="$MOUNT_FROM_CONFIG"
    fi
fi

echo "Mount point: $MOUNT_POINT"

# Enable user_allow_other in /etc/fuse.conf
echo ""
echo "Configuring FUSE..."
if grep -q "^user_allow_other" /etc/fuse.conf 2>/dev/null; then
    echo "✓ user_allow_other already enabled"
else
    echo "user_allow_other" >> /etc/fuse.conf
    echo "✓ Enabled user_allow_other in /etc/fuse.conf"
fi

# Create systemd service
echo ""
echo "Creating systemd service..."

cat << EOF > /etc/systemd/system/wolfdisk.service
[Unit]
Description=WolfDisk Distributed File System
After=network.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/wolfdisk --config $CONFIG_FILE mount --mountpoint $MOUNT_POINT
ExecStop=/usr/local/bin/wolfdisk unmount --mountpoint $MOUNT_POINT
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal

# Security hardening
NoNewPrivileges=false
ProtectSystem=false
PrivateTmp=false

[Install]
WantedBy=multi-user.target
EOF

echo "✓ Created /etc/systemd/system/wolfdisk.service"

# Reload systemd
systemctl daemon-reload
echo "✓ Reloaded systemd"

# Enable service - use /dev/tty for interactive input when piped
echo -n "Enable WolfDisk to start on boot? [Y/n] "
read -n 1 -r REPLY < /dev/tty || REPLY="y"
echo
if [[ ! $REPLY =~ ^[Nn]$ ]]; then
    systemctl enable wolfdisk
    echo "✓ Enabled WolfDisk service"
fi

# Start service
echo -n "Start WolfDisk now? [Y/n] "
read -n 1 -r REPLY < /dev/tty || REPLY="y"
echo
if [[ ! $REPLY =~ ^[Nn]$ ]]; then
    systemctl start wolfdisk
    echo "✓ Started WolfDisk service"
    sleep 2
    
    # Check status
    if systemctl is-active --quiet wolfdisk; then
        echo ""
        echo "✓ WolfDisk is running!"
        echo ""
        echo "Mount point: $MOUNT_POINT"
        echo "Try: ls $MOUNT_POINT"
    else
        echo ""
        echo "⚠ WolfDisk may not have started correctly."
        echo "Check logs with: journalctl -u wolfdisk -f"
    fi
fi

echo ""
echo "Done!"
