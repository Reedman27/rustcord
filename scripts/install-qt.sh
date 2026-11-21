#!/bin/sh
# SPDX-License-Identifier: LGPL-3.0-or-later
# Build and install the Qt frontend (for musl systems such as Chimera Linux) under a prefix.
# Usage: ./scripts/install-qt.sh [PREFIX]    default PREFIX is ~/.local
set -eu
cd "$(dirname "$0")/.."
PREFIX=${1:-$HOME/.local}

cargo build --release -p rustcord-qt
# POSIX only (no `install -D`, which BSD userlands such as Chimera's may lack).
put() { mkdir -p "$(dirname "$3")" && install -m "$1" "$2" "$3"; }
put 755 target/release/rustcord-qt "$PREFIX/bin/rustcord-qt"
put 644 packaging/rustcord-qt.desktop "$PREFIX/share/applications/rustcord-qt.desktop"
put 644 packaging/icons/rustcord-256.png "$PREFIX/share/icons/hicolor/256x256/apps/rustcord.png"

echo "installed to $PREFIX (make sure $PREFIX/bin is in PATH)"
echo "discord:// links: xdg-mime default rustcord-qt.desktop x-scheme-handler/discord"
