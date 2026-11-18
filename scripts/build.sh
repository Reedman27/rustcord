#!/bin/sh
# Build Rustcord and assemble a runnable directory in dist/rustcord.
#
# CEF (Chromium) must sit next to the executable. The cef crate downloads the matching
# CEF build the first time it compiles, so the first build needs network access and
# several hundred MB of disk.
set -eu

cd "$(dirname "$0")/.."

cargo build --release -p rustcord

out=dist/rustcord
rm -rf "$out"
mkdir -p "$out"
cp target/release/rustcord "$out/"

# CEF runtime files: libcef.so, snapshots, ICU data, .pak resources, locales, ANGLE/Vulkan libs.
cef_dir=$(find target/release/build -type d -name 'cef_linux_*' 2>/dev/null | head -n 1 || true)
if [ -z "$cef_dir" ] && [ -n "${CEF_PATH:-}" ]; then
    cef_dir=$CEF_PATH
fi
if [ -z "$cef_dir" ] || [ ! -d "$cef_dir" ]; then
    echo "error: could not find the CEF runtime directory." >&2
    echo "       Set CEF_PATH to the extracted CEF distribution (the folder holding libcef.so)." >&2
    exit 1
fi
cp -r "$cef_dir"/. "$out"/

# chrome-sandbox only works when setuid root; without it, set "sandbox": false in settings.json
# or install it as root:  chown root:root chrome-sandbox && chmod 4755 chrome-sandbox
if [ -f "$out/chrome-sandbox" ]; then
    echo "note: $out/chrome-sandbox is not setuid. Either install it setuid root or set"
    echo '      "sandbox": false in ~/.config/rustcord/settings.json'
fi

echo "built: $out/rustcord"
