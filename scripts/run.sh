#!/bin/bash
#
# WolfScale Run Script
# 
# Usage:
#   ./run.sh [options]
#
# Options:
#   --bootstrap       Start as the initial cluster leader
#   --config PATH     Path to configuration file (default: ../wolfscale.toml)
#   --log-level LEVEL Log level: trace, debug, info, warn, error (default: info)
#   --help            Show this help message
#

set -e

# Default values
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
CONFIG_FILE="${PROJECT_DIR}/wolfscale.toml"
LOG_LEVEL="info"
BOOTSTRAP=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --bootstrap)
            BOOTSTRAP="--bootstrap"
            shift
            ;;
        --config)
            CONFIG_FILE="$2"
            shift 2
            ;;
        --log-level)
            LOG_LEVEL="$2"
            shift 2
            ;;
        --help)
            echo "WolfScale Run Script"
            echo ""
            echo "Usage:"
            echo "  $0 [options]"
            echo ""
            echo "Options:"
            echo "  --bootstrap       Start as the initial cluster leader"
            echo "  --config PATH     Path to configuration file (default: ../wolfscale.toml)"
            echo "  --log-level LEVEL Log level: trace, debug, info, warn, error (default: info)"
            echo "  --help            Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

# Check if configuration file exists
if [[ ! -f "$CONFIG_FILE" ]]; then
    echo "Error: Configuration file not found: $CONFIG_FILE"
    echo ""
    echo "To create a configuration file, run:"
    echo "  wolfscale init --output $CONFIG_FILE"
    echo ""
    echo "Or copy the example configuration:"
    echo "  cp ${PROJECT_DIR}/wolfscale.toml.example $CONFIG_FILE"
    exit 1
fi

# Determine the binary location
if [[ -f "${PROJECT_DIR}/target/release/wolfscale" ]]; then
    BINARY="${PROJECT_DIR}/target/release/wolfscale"
elif [[ -f "${PROJECT_DIR}/target/debug/wolfscale" ]]; then
    BINARY="${PROJECT_DIR}/target/debug/wolfscale"
elif command -v wolfscale &> /dev/null; then
    BINARY="wolfscale"
else
    echo "Error: WolfScale binary not found."
    echo ""
    echo "Build it with:"
    echo "  cd ${PROJECT_DIR} && cargo build --release"
    exit 1
fi

echo "=========================================="
echo "  WolfScale - Distributed MariaDB Sync"
echo "=========================================="
echo ""
echo "Binary:     $BINARY"
echo "Config:     $CONFIG_FILE"
echo "Log Level:  $LOG_LEVEL"
if [[ -n "$BOOTSTRAP" ]]; then
    echo "Mode:       Leader (bootstrap)"
else
    echo "Mode:       Follower"
fi
echo ""
echo "Starting WolfScale..."
echo "Press Ctrl+C to stop"
echo ""

# Run WolfScale
exec "$BINARY" --config "$CONFIG_FILE" --log-level "$LOG_LEVEL" start $BOOTSTRAP
