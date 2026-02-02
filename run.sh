#!/bin/bash
# WolfScale Run Script
# Usage: ./run.sh [mode] [options]
#   Modes: start, proxy
#   Options: --bootstrap (for start mode), --listen ADDRESS (for proxy mode)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="$SCRIPT_DIR/target/release/wolfscale"
CONFIG="$SCRIPT_DIR/wolfscale.toml"

# Build if binary doesn't exist
if [ ! -f "$BINARY" ]; then
    echo "Building WolfScale..."
    cd "$SCRIPT_DIR"
    cargo build --release
fi

# Check for config file
if [ ! -f "$CONFIG" ]; then
    echo "Error: Configuration file not found at $CONFIG"
    echo "Run: $BINARY init --output $CONFIG"
    exit 1
fi

MODE="${1:-start}"

case "$MODE" in
    start)
        shift || true
        echo "Starting WolfScale node..."
        "$BINARY" --config "$CONFIG" start "$@"
        ;;
    proxy)
        shift || true
        LISTEN="${1:---listen 0.0.0.0:8007}"
        echo "Starting WolfScale MySQL Proxy..."
        "$BINARY" --config "$CONFIG" proxy $LISTEN
        ;;
    status)
        "$BINARY" --config "$CONFIG" status
        ;;
    info)
        "$BINARY" --config "$CONFIG" info
        ;;
    *)
        echo "Usage: $0 {start|proxy|status|info} [options]"
        echo ""
        echo "Modes:"
        echo "  start [--bootstrap]  Start as cluster node (use --bootstrap for first node)"
        echo "  proxy [--listen ADDR] Start MySQL proxy (default: 0.0.0.0:8007)"
        echo "  status               Check cluster status"
        echo "  info                 Show node configuration"
        exit 1
        ;;
esac
