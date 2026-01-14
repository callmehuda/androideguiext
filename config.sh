#!/bin/bash
set -e

# Detect NDK
ANDROID_HOME=${ANDROID_HOME:-$HOME/Android/Sdk}
NDK_BASE="$ANDROID_HOME/ndk"

if [ ! -d "$NDK_BASE" ]; then
    echo "Error: NDK directory not found at $NDK_BASE"
    exit 1
fi

# Get latest NDK version
NDK_VERSION=$(ls -1 "$NDK_BASE" | sort -V | tail -n 1)
NDK_PATH="$NDK_BASE/$NDK_VERSION"
NDK_PATH=$(realpath "$NDK_PATH")

echo "Using NDK: $NDK_PATH"

# Defaults
PROFILE="debug"
ARCH="arm64-v8a"

# Argument Parsing
if [[ "$1" == "dev" || "$1" == "debug" ]]; then
    PROFILE="$1"
    shift
fi

# Check if the next arg is a valid arch, or if it was the first arg (handled by shift above? no)
# If $1 is still set, it must be the arch
if [[ -n "$1" ]]; then
    ARCH="$1"
fi

# Architecture Selection
case "$ARCH" in
    "arm64-v8a")
        TARGET="aarch64-linux-android"
        ;;
    "armeabi-v7a")
        TARGET="armv7-linux-androideabi"
        ;;
    "x86")
        TARGET="i686-linux-android"
        ;;
    "x86-64"|"x86_64")
        TARGET="x86_64-linux-android"
        ;;
    *)
        echo "Error: Unknown architecture '$ARCH'"
        echo "Supported architectures: arm64-v8a, armeabi-v7a, x86, x86-64"
        exit 1
        ;;
esac

echo "Configuration: Profile=$PROFILE, Arch=$ARCH ($TARGET)"

# NDK Toolchain Paths
HOST_TAG="linux-x86_64"
API_LEVEL=35
TOOLCHAIN="$NDK_PATH/toolchains/llvm/prebuilt/$HOST_TAG"
BIN="$TOOLCHAIN/bin"

# Linker selection logic
if [ "$TARGET" == "armv7-linux-androideabi" ]; then
    LINKER_TARGET="armv7a-linux-androideabi"
else
    LINKER_TARGET="$TARGET"
fi

LINKER="$BIN/${LINKER_TARGET}${API_LEVEL}-clang"
AR="$BIN/llvm-ar"
CC="$BIN/${LINKER_TARGET}${API_LEVEL}-clang"
CXX="$BIN/${LINKER_TARGET}${API_LEVEL}-clang++"
PLATFORM="android-$API_LEVEL"

if [ ! -x "$LINKER" ]; then
    echo "Error: Linker not found at $LINKER"
    exit 1
fi

# Generate .cargo/config.toml
mkdir -p .cargo
CONFIG_FILE=".cargo/config.toml"

cat > "$CONFIG_FILE" <<EOF
[build]
target = "$TARGET"

[env]
ANDROID_NDK_HOME = { value = "$NDK_PATH", force = true }
ANDROID_PLATFORM = { value = "$PLATFORM", force = true }
CC = { value = "$CC", force = true }
CXX = { value = "$CXX", force = true }
PATH = { value = "$BIN", pre = true }

[target.$TARGET]
linker = "$LINKER"
ar = "$AR"
runner = "./runner.sh"
EOF

# Append Profile Settings
if [ "$PROFILE" == "dev" ]; then
    echo "Configuring [profile.dev] for balanced speed/size..."
    # optimized for compile speed, but stripping symbols for transfer speed
    cat >> "$CONFIG_FILE" <<EOF

[profile.dev]
opt-level = 0
strip = true
lto = false
panic = "abort"
EOF
else
    echo "Configuring [profile.dev] for standard debugging..."
    cat >> "$CONFIG_FILE" <<EOF

[profile.dev]
opt-level = 0
debug = true
strip = false
EOF
fi

echo "Created $CONFIG_FILE"
