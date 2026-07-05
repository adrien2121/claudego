#!/bin/sh

set -e

REPO="adrien2121/claudego"
BIN_NAME="claudego"
LOGS_BIN_NAME="claudego-logs"
INSTALL_DIR="$HOME/.local/bin"

echo "Installing $BIN_NAME and $LOGS_BIN_NAME..."

# Create install directory if it doesn't exist
mkdir -p "$INSTALL_DIR"

# Check if cargo is installed, if so we can just install from git as a fallback/default
if command -v cargo >/dev/null 2>&1; then
    echo "Cargo detected. Building and installing via cargo..."
    cargo install --git "https://github.com/$REPO.git" --root "$HOME/.local"
    
    # Ensure binary is in the requested location (cargo install --root puts it in bin/)
    if [ -f "$HOME/.local/bin/$BIN_NAME" ]; then
        echo "$BIN_NAME and $LOGS_BIN_NAME installed successfully to $HOME/.local/bin!"
        echo "Make sure $HOME/.local/bin is in your PATH."
        exit 0
    fi
fi

# If they eventually add GitHub releases, they can add download logic here:
echo "Warning: GitHub releases download logic is not yet implemented."
echo "Please install Rust (https://rustup.rs/) to compile from source, or check back later for pre-built binaries."
exit 1
