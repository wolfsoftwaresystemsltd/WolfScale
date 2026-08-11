#!/bin/bash
set -e

# ─── WolfDisk Docker Entrypoint ──────────────────────────────────────────────
# Writes /etc/wolfdisk/config.toml from env vars on first run, initialises
# the data directory, then mounts the filesystem.

CONFIG="/etc/wolfdisk/config.toml"
DATA_DIR="${WOLFDISK_DATA_DIR:-/var/lib/wolfdisk}"
MOUNT="${WOLFDISK_MOUNT:-/mnt/wolfdisk}"

mkdir -p "$(dirname "$CONFIG")" "$MOUNT" "$DATA_DIR"

# Chunk-store placement guard: if DATA_DIR sits on the container's own
# overlay filesystem, no volume/bind is mapped there — chunks would land
# inside the container image (lost on re-create; on Unraid this silently
# fills the size-limited docker image). Longest-prefix mount lookup from
# /proc/mounts, same rule the kernel uses.
data_fstype=$(awk -v dir="$DATA_DIR" '
    # "/" prefixes every absolute path but "//" prefixes none — handle root
    # explicitly or the overlay-root case (the one this guard exists for)
    # never matches.
    { if (dir == $2 || $2 == "/" || index(dir, $2 "/") == 1) { if (length($2) > best) { best = length($2); fs = $3 } } }
    END { print fs }' /proc/mounts)
if [ "$data_fstype" = "overlay" ]; then
    echo "⚠ WARNING: $DATA_DIR is on the container's overlay filesystem — no volume is mapped there."
    echo "  Chunk data will be LOST when this container is re-created."
    echo "  Map it to persistent storage, e.g.  -v wolfdisk-data:/var/lib/wolfdisk"
    echo "  (Unraid: -v /mnt/user/appdata/wolfdisk/data:/var/lib/wolfdisk — see the image docs)"
fi

if [ ! -c /dev/fuse ]; then
    echo "✗ /dev/fuse not found inside the container."
    echo "  Run with:  --device /dev/fuse:/dev/fuse  (and usually --privileged or --cap-add SYS_ADMIN)"
    exit 1
fi

wolfdisk --config "$CONFIG" init --data-dir "$DATA_DIR" >/dev/null 2>&1 || true

if [ ! -f "$CONFIG" ]; then
    NODE_ID="${WOLFDISK_NODE_ID:-$(hostname)}"
    ROLE="${WOLFDISK_ROLE:-client}"
    BIND="${WOLFDISK_BIND:-0.0.0.0:8650}"  # 8650, not 8550 — avoids WolfStack's 8550..8599 status-page range
    MODE="${WOLFDISK_MODE:-shared}"

    # Parse comma-separated WOLFDISK_PEERS into a TOML array
    peers_toml=""
    if [ -n "$WOLFDISK_PEERS" ]; then
        IFS=',' read -ra parts <<< "$WOLFDISK_PEERS"
        first=1
        for p in "${parts[@]}"; do
            p="${p// /}"
            [ -z "$p" ] && continue
            if [ $first -eq 1 ]; then
                peers_toml="\"$p\""
                first=0
            else
                peers_toml="$peers_toml, \"$p\""
            fi
        done
    fi

    cat > "$CONFIG" <<EOF
[node]
id = "$NODE_ID"
role = "$ROLE"
bind = "$BIND"
data_dir = "$DATA_DIR"

[cluster]
peers = [$peers_toml]

[replication]
mode = "$MODE"
factor = 3
chunk_size = 4194304

[mount]
path = "$MOUNT"
allow_other = true
EOF
    echo "→ Wrote $CONFIG (role=$ROLE, peers=[$peers_toml])"
fi

echo "→ Mounting WolfDisk at $MOUNT..."
exec wolfdisk --config "$CONFIG" mount --mountpoint "$MOUNT" "$@"
