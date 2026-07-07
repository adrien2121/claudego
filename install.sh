#!/bin/sh

set -e

REPO="adrien2121/claudego"
BIN_NAME="claudego"
LOGS_BIN_NAME="claudego-logs"
INSTALL_DIR="$HOME/.local/bin"

echo "Installing $BIN_NAME and $LOGS_BIN_NAME..."

# Create install directory if it doesn't exist
mkdir -p "$INSTALL_DIR"

get_arch() {
    a=$(uname -m)
    case ${a} in
        "x86_64" | "amd64")
            echo "x86_64"
            ;;
        "aarch64" | "arm64")
            echo "aarch64"
            ;;
        *)
            echo "${a}"
            ;;
    esac
}

get_os() {
    os=$(uname -s | tr '[:upper:]' '[:lower:]')
    echo "$os"
}

download_and_install() {
    os=$(get_os)
    arch=$(get_arch)
    
    # Construct the release asset URL (e.g., claudego-aarch64-darwin.tar.gz)
    asset_name="${BIN_NAME}-${arch}-${os}.tar.gz"
    download_url="https://github.com/$REPO/releases/latest/download/$asset_name"

    echo "Attempting to download pre-built binary from $download_url"
    
    # Use curl to download and unpack directly into the install directory
    if curl -sSL "$download_url" | tar -xz -C "$INSTALL_DIR" "$BIN_NAME" "$LOGS_BIN_NAME" 2>/dev/null; then
        if [ -f "$INSTALL_DIR/$BIN_NAME" ]; then
            echo "Successfully installed pre-built binaries to $INSTALL_DIR"
            return 0
        fi
    fi
    echo "Failed to download pre-built binary. Will try to build from source."
    return 1
}

# Try to download pre-built binary first
if download_and_install; then
    echo "Make sure $INSTALL_DIR is in your PATH."
    exit 0
fi

# Fallback to building from source with cargo
if command -v cargo >/dev/null 2>&1; then
    echo "Cargo detected. Building and installing from source via cargo..."
    cargo install --git "https://github.com/$REPO.git" --root "$HOME/.local"
    echo "$BIN_NAME and $LOGS_BIN_NAME installed successfully to $HOME/.local/bin!"
    echo "Make sure $HOME/.local/bin is in your PATH."
else
    echo "Error: Could not find pre-built binary and Cargo is not installed."
    echo "Please install Rust (https://rustup.rs/) to build from source."
    exit 1
fi
