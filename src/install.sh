#!/bin/bash
set -e

# The GitHub repository in "username/repo" format
REPO="adrien2121/claudego"

# The name of the binary
BINARY="claudego"

# ---

echo "Preparing to install $BINARY..."

# Identify OS and architecture
if [[ "$(uname)" == "Darwin" ]]; then
  os="apple-darwin"
  if [[ "$(uname -m)" == "arm64" ]]; then
    arch="aarch64"
  else
    arch="x86_64"
  fi
else
  os="unknown-linux-gnu"
  arch="x86_64" # Assuming x86_64 for Linux for simplicity
fi

# Fetch the latest release version from GitHub API
latest_version=$(curl -s "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')
if [ -z "$latest_version" ]; then
  echo "Error: Could not fetch the latest release version."
  exit 1
fi

# Construct the download URL
file_name="${BINARY}-${latest_version}-${arch}-${os}.tar.gz"
download_url="https://github.com/$REPO/releases/download/${latest_version}/${file_name}"

echo "Downloading $BINARY ${latest_version}..."
curl -L -o "${BINARY}.tar.gz" "$download_url"
tar -xzf "${BINARY}.tar.gz" "$BINARY"

install_dir="$HOME/.local/bin"
mkdir -p "$install_dir"
mv "$BINARY" "$install_dir/$BINARY"
chmod +x "$install_dir/$BINARY"

rm "${BINARY}.tar.gz"

echo ""
echo "$BINARY has been installed to $install_dir/$BINARY"
echo "Please ensure '$install_dir' is in your PATH."

# Check if install_dir is in PATH
case ":$PATH:" in
  *":$install_dir:"*)
    # Already in PATH
    ;;
  *)
    echo ""
    echo "ACTION REQUIRED:"
    echo "To use the '$BINARY' command, please add the installation directory to your PATH."
    echo "You can do this by adding the following line to your shell's configuration file:"
    echo ""

    shell_name=$(basename "$SHELL")
    config_file=""
    [ "$shell_name" = "zsh" ] && config_file="$HOME/.zshrc"
    [ "$shell_name" = "bash" ] && config_file="$HOME/.bashrc"

    if [ -n "$config_file" ]; then
        path_line="export PATH=\"\$HOME/.local/bin:\$PATH\""
        echo "  $path_line"
        echo ""
        read -p "Would you like to add this to '$config_file' automatically? (y/N) " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            echo "$path_line" >> "$config_file"
            echo "Successfully updated '$config_file'. Please restart your terminal to apply the changes."
        fi
    fi
    ;;
esac