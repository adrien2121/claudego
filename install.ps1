$ErrorActionPreference = "Stop"

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "Cargo is required. Install Rust from https://rustup.rs/."
}

cargo install `
    --git https://github.com/adrien2121/botsitter `
    --bin botsitter `
    --bin botsitter-logs

if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
