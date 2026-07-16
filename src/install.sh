#!/bin/sh
set -eu

if ! command -v cargo >/dev/null 2>&1; then
    echo "Cargo is required. Install Rust from https://rustup.rs/." >&2
    exit 1
fi

cargo install \
    --git https://github.com/adrien2121/botsitter \
    --bin botsitter \
    --bin botsitter-logs
