#!/bin/bash
set -e

# The GitHub repository in "username/repo" format
REPO="adrien2121/claudego"

# The name of the binary
PRIMARY_BINARY="claudego"
LOGS_BINARY="claudego-logs"

# ---

echo "Preparing to install $PRIMARY_BINARY and $LOGS_BINARY..."

# Identify OS and architecture
os_name=$(uname)
machine_arch=$(uname -m)

if [[ "$os_name" == "Darwin" ]]; then
  os="apple-darwin"
  if [[ "$machine_arch" == "arm64" ]]; then
    arch="aarch64"
  else
    arch="x86_64"
  fi
elif [[ "$os_name" == "Linux" ]]; then
  os="unknown-linux-gnu"
  if [[ "$machine_arch" == "aarch64" ]]; then
    arch="aarch64"
  else
    arch="x86_64"
  fi
else
  echo "Error: Unsupported OS '$os_name'. Only macOS and Linux are supported by this script."
  exit 1
fi

# Fetch the latest release version from GitHub API
latest_version=$(curl -s "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')
if [ -z "$latest_version" ]; then
  echo "Error: Could not fetch the latest release version."
  exit 1
fi

# Construct the download URL
file_name="${PRIMARY_BINARY}-${latest_version}-${arch}-${os}.tar.gz"
download_url="https://github.com/$REPO/releases/download/${latest_version}/${file_name}"

echo "Downloading binaries ${latest_version}..."
curl -L -o "claudego_release.tar.gz" "$download_url"
# Extract both binaries. We assume the tarball contains both.
tar -xzf "claudego_release.tar.gz" "$PRIMARY_BINARY" "$LOGS_BINARY"

install_dir="$HOME/.local/bin"
mkdir -p "$install_dir"

mv "$PRIMARY_BINARY" "$install_dir/$PRIMARY_BINARY"
mv "$LOGS_BINARY" "$install_dir/$LOGS_BINARY"
chmod +x "$install_dir/$PRIMARY_BINARY"
chmod +x "$install_dir/$LOGS_BINARY"

rm "claudego_release.tar.gz"

echo ""
echo "$PRIMARY_BINARY and $LOGS_BINARY have been installed to $install_dir"
echo "Please ensure '$install_dir' is in your PATH."

# Check if install_dir is in PATH
case ":$PATH:" in
  *":$install_dir:"*)
    # Already in PATH
    ;;
  *)
    echo ""
    echo "ACTION REQUIRED:"
    echo "To use the commands, please add the installation directory to your PATH."
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