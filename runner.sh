#!/bin/bash
set -e

# Configuration (defaults)
USE_SU=${USE_SU:-0}
echo "Runner: USE_SU=$USE_SU"

# When called by cargo run, the first argument is the path to the binary
LOCAL_BIN="$1"

if [ -z "$LOCAL_BIN" ]; then
    echo "Usage: [USE_SU=1] $0 <path_to_binary>"
    exit 1
fi

BIN_NAME=$(basename "$LOCAL_BIN")
REMOTE_DIR="/data/local/tmp"
REMOTE_PATH="$REMOTE_DIR/$BIN_NAME"

# Cleanup trap
# cleanup() {
#     echo "Cleaning up remote binary..."
#     if [ "$USE_SU" = "1" ]; then
#         adb shell su -c "rm $REMOTE_PATH" || true
#     else
#         adb shell "rm $REMOTE_PATH" || true
#     fi
# }
# trap cleanup EXIT

# Push
echo "Pushing $LOCAL_BIN to $REMOTE_DIR..."
adb push "$LOCAL_BIN" "$REMOTE_PATH"

# Run
ENV_VARS=""
if [ -n "$RUST_BACKTRACE" ]; then
    ENV_VARS="$ENV_VARS RUST_BACKTRACE=$RUST_BACKTRACE"
fi
if [ -n "$RUST_LOG" ]; then
    ENV_VARS="$ENV_VARS RUST_LOG=$RUST_LOG"
fi

if [ "$USE_SU" = "1" ]; then
    echo "Running on device as root (su)..."
    adb shell "su -c 'chmod +x $REMOTE_PATH && $ENV_VARS $REMOTE_PATH'"
else
    echo "Running on device as shell user..."
    adb shell "chmod +x $REMOTE_PATH && $ENV_VARS $REMOTE_PATH"
fi
