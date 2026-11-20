#!/bin/sh
# Build and install Rustcord as a Flatpak (for musl systems such as Chimera Linux, where the
# glibc-only Chromium/CEF binaries cannot run natively).
#
# Chimera prerequisites:  apk add flatpak xdg-desktop-portal-gtk
# Run:                    ./scripts/flatpak.sh
set -eu
cd "$(dirname "$0")/.."

flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install --user -y flathub org.gnome.Platform//49 org.gnome.Sdk//49 \
    org.freedesktop.Sdk.Extension.rust-stable//25.08

if command -v flatpak-builder >/dev/null 2>&1; then
    builder="flatpak-builder"
else
    flatpak install --user -y flathub org.flatpak.Builder
    builder="flatpak run org.flatpak.Builder"
fi

$builder --user --install --force-clean --state-dir=.flatpak-builder \
    build-flatpak packaging/flatpak/dev.rustcord.Rustcord.yml

echo "installed. Run:  flatpak run dev.rustcord.Rustcord"
echo "Mods:            flatpak run --command=rustcord dev.rustcord.Rustcord mods install equicord"
