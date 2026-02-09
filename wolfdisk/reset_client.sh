#!/bin/bash
# WolfDisk Client Reset Script
# Cleans up stuck FUSE mounts and wipes local cache data.
# WARNING: Only run this on CLIENT machines. Running on a Leader/Follower will cause DATA LOSS.

if [ "$EUID" -ne 0 ]; then
  echo "Please run as root (sudo)"
  exit 1
fi

echo "========================================================"
echo "WARNING: This script will WIPE all data in /var/lib/wolfdisk"
echo "If this is a LEADER or FOLLOWER node, you will LOSE DATA."
echo "Only proceed if this is a CLIENT node (cache only)."
echo "========================================================"
read -p "Are you sure you want to proceed? (y/N) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Aborted."
    exit 1
fi

echo "Stopping any WolfDisk processes..."
pkill -f wolfdisk || true
sleep 2
pkill -9 -f wolfdisk || true

echo "Unmounting WolfDisk mount point..."
umount -f /mnt/wolfdisk 2>/dev/null || fusermount -u -z /mnt/wolfdisk 2>/dev/null
if mount | grep -q "wolfdisk"; then
    echo "Error: Failed to unmount /mnt/wolfdisk. Please check open handles (lsof)."
    exit 1
fi

echo "Wiping local cache (/var/lib/wolfdisk)..."
if [ -d "/var/lib/wolfdisk" ]; then
    rm -rf /var/lib/wolfdisk/*
    echo "Cache cleared."
else
    echo "No cache found at /var/lib/wolfdisk (already clean?)"
fi

echo "Reset complete. please restart wolfdisk now."
