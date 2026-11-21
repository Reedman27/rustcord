#!/bin/sh
# Build and install the Qt frontend (for musl systems such as Chimera Linux) under a prefix.
# Usage: ./scripts/install-qt.sh [PREFIX]    default PREFIX is ~/.local
set -eu
cd "$(dirname "$0")/.."
PREFIX=${1:-$HOME/.local}

cargo build --release -p rustcord-qt
install -Dm755 target/release/rustcord-qt "$PREFIX/bin/rustcord-qt"
install -Dm644 packaging/rustcord-qt.desktop "$PREFIX/share/applications/rustcord-qt.desktop"
mkdir -p "$PREFIX/share/icons/hicolor/256x256/apps"
cargo run --release -q -p rustcord-core --example dump -- png 256 \
    > "$PREFIX/share/icons/hicolor/256x256/apps/rustcord.png"

echo "installed to $PREFIX (make sure $PREFIX/bin is in PATH)"
echo "discord:// links: xdg-mime default rustcord-qt.desktop x-scheme-handler/discord"
