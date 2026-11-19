#!/bin/sh
# Build Rustcord and assemble a runnable directory in dist/rustcord.
#
# The cef crate downloads a matching CEF (Chromium) build the first time it compiles. We pin
# CEF_PATH so the download lands in a known place and can be found afterwards. Override
# CEF_PATH to reuse an existing download. First build needs network access and a few
# hundred MB of disk.
set -eu

cd "$(dirname "$0")/.."

export CEF_PATH="${CEF_PATH:-${XDG_CACHE_HOME:-$HOME/.cache}/rustcord/cef}"

cargo build --release -p rustcord

# The runtime files (libcef.so, .pak resources, ICU data, locales, GPU libraries) are the
# directory that holds libcef.so.
libcef=$(find "$CEF_PATH" -name libcef.so 2>/dev/null | head -n 1 || true)
if [ -z "$libcef" ]; then
    echo "error: libcef.so not found under $CEF_PATH after the build." >&2
    echo "       Set CEF_PATH to a CEF distribution directory (the one holding libcef.so)." >&2
    exit 1
fi
cef_dir=$(dirname "$libcef")

out=dist/rustcord
rm -rf "$out"
mkdir -p "$out"
cp target/release/rustcord "$out/"
# Everything CEF needs at run time sits next to the executable. Headers, CMake files and
# the wrapper sources from the distribution are not needed.
for f in "$cef_dir"/*; do
    case "$(basename "$f")" in
        include|cmake|libcef_dll|CMakeLists.txt|*.a) ;;
        *) cp -r "$f" "$out"/ ;;
    esac
done

# chrome-sandbox only works when setuid root. Without it Chromium falls back to user
# namespaces if the kernel allows them; if neither works, set "sandbox": false in
# ~/.config/rustcord/settings.json.
if [ -f "$out/chrome-sandbox" ]; then
    echo "note: to use the setuid sandbox: sudo chown root:root $out/chrome-sandbox && sudo chmod 4755 $out/chrome-sandbox"
fi

echo "built: $out/rustcord"
