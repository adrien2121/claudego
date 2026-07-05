$ErrorActionPreference = "Stop"

$Repo = "adrien2121/claudego"
$BinName = "claudego"

Write-Host "Installing $BinName..."

# Check if cargo is installed
if (Get-Command cargo -ErrorAction SilentlyContinue) {
    Write-Host "Cargo detected. Building and installing via cargo..."
    cargo install --git "https://github.com/$Repo.git"
    
    Write-Host "$BinName installed successfully!"
    Write-Host "Make sure your cargo bin directory (usually ~/.cargo/bin) is in your PATH."
    exit 0
}

Write-Host "Warning: GitHub releases download logic is not yet implemented." -ForegroundColor Yellow
Write-Host "Please install Rust (https://rustup.rs/) to compile from source, or check back later for pre-built binaries." -ForegroundColor Yellow
exit 1
