# Rustcord

A Linux-first desktop client written in Rust that hosts the **official Discord web app** inside
Chromium (CEF, through [`cef-rs`](https://github.com/tauri-apps/cef-rs)). No Electron, no Node.
Rust owns the process, windows, settings, themes and desktop integration.

It does not touch user tokens, automate accounts or call private endpoints. You sign in to
discord.com in the embedded browser like in any browser.

License: LGPL-3.0-or-later (see `COPYING.LESSER` and `COPYING`). No code was copied from
the reference projects (Vesktop, Vencord and others); behaviour was reimplemented in Rust.

## Status

This is an early build. Read "Not done" before relying on it.

### Verified here
- `rustcord-core` (URL policy, settings, XDG paths, single instance, theme engine, icon
  generation): 35 unit tests pass.
- The generated window icon PNG decodes correctly in Pillow, and the theme injection script
  applies, is idempotent and cannot inject markup in a real Chromium page
  (`python3 scripts/verify_assets.py`).
- The real Vencord and Equicord userscript builds, wrapped by the same Rust function the app
  uses, start in headless Chromium against a stub discord.com page: 179 / 373 plugins
  register and settings persist (`python3 scripts/verify_mods.py <bundle.user.js>...`).
  That is the page-side half only; the CEF injection path itself has never run.
- `rustcord` (the CEF application) **type-checks and passes clippy `-D warnings`** against the
  real cef-rs 156.6.0 bindings (`--features dox`).

### Not verified (never run)
- The app has **not been linked or launched**. The sandbox I built it in could not run a
  Chromium window, so nothing below has been seen working: login, text chat, voice, camera,
  navigation guard in a live page, window-state restore, theme injection, deep links.
  Treat the first run on your machine as the real Phase 0 test.

## What is implemented

| Feature | Where |
|---|---|
| CEF main window loading discord.com (stable, canary or ptb), no browser toolbar | `crates/app/src/{browser_process,window}.rs` |
| Navigation guard: main frame may only load exact Discord hosts over https; everything else opens with `xdg-open` (http, https, mailto only) | `core/nav.rs`, `app/handler.rs` |
| Pop-outs: Discord popups open in their own window, other popups go to the system browser | `app/handler.rs` |
| Mic and camera permission granted only to Discord origins | `app/handler.rs` |
| Window size, position and maximized state persisted (atomic JSON) | `core/settings.rs`, `app/window.rs` |
| Persistent login (browser profile in `~/.local/share/rustcord/profile`) | `app/main.rs` |
| Single instance over `$XDG_RUNTIME_DIR/rustcord.sock`; second launch focuses the window | `core/single_instance.rs` |
| `discord://` deep links (`rustcord discord://-/channels/...`, `.desktop` MimeType) | `core/nav.rs`, `packaging/` |
| Settings file with Chromium switch passthrough and background-throttling off | `core/settings.rs` |
| Theme engine: `themes/*.css`, `quick.css`, accent colour, forced dark or light, injected after page load | `core/theme.rs`, `app/handler.rs` |
| System tray (StatusNotifierItem via `ksni`): retries up to 30 s for a tray host, Show and Quit menu, left click shows the window, optional close-to-tray that only applies while the tray is registered | `app/tray.rs`, `core/tray_icon.rs` |
| Window class (`WM_CLASS` rustcord/Rustcord, role main/popup), window and app icon, minimum size | `app/window.rs`, `app/icon.rs` |
| XDG config and data directories, no hard-coded paths, no systemd | `core/paths.rs` |

## Build

Needs Rust (stable), a C++ toolchain, CMake, Ninja, and the usual Chromium runtime libraries
(GTK3, NSS, ALSA or PipeWire, X11 libraries). The cef crate downloads CEF on first build.

```sh
cargo test -p rustcord-core          # no CEF needed
cargo clippy --workspace --features rustcord/dox -- -D warnings   # type-check without CEF
./scripts/build.sh                   # full build into dist/rustcord (downloads CEF once)
dist/rustcord/rustcord
```

`build.sh` keeps the CEF download in `~/.cache/rustcord/cef` (override with `CEF_PATH`) and
copies the runtime next to the executable. The binary looks for `libcef.so` in its own
directory, so run it from `dist/rustcord`, not from `target/`.

If Chromium's sandbox cannot start (no setuid `chrome-sandbox`), either install it setuid root
or set `"sandbox": false` in `~/.config/rustcord/settings.json`.

## Configuration

`~/.config/rustcord/settings.json` (all keys optional):

```json
{
  "branch": "stable",
  "start_minimized": false,
  "disable_background_throttling": true,
  "sandbox": true,
  "tray": true,
  "close_to_tray": false,
  "extra_switches": ["force-device-scale-factor=1.25"],
  "themes": ["midnight"],
  "accent_color": "#5865f2",
  "color_scheme": "dark"
}
```

Put theme CSS in `~/.config/rustcord/themes/<name>.css` and list the name in `themes`.
`~/.config/rustcord/quick.css` is always applied last. The accent colour is exposed to CSS as
`var(--rustcord-accent)`. Changes apply on the next page load or restart. A malformed settings
file is kept as `settings.json.bad` and defaults are used.

## Chimera Linux and other musl systems (Flatpak)

Official CEF/Chromium builds are glibc-only and cannot run on musl, and building Chromium
for musl is not realistic. On Chimera, build the Flatpak instead, which brings its own glibc
runtime (needs network; first build downloads the runtime, crates and CEF):

```
apk add flatpak xdg-desktop-portal-gtk
./scripts/flatpak.sh
flatpak run dev.rustcord.Rustcord
flatpak run --command=rustcord dev.rustcord.Rustcord mods install equicord
```

Inside Flatpak, Chromium's own sandbox is replaced by Flatpak's. Settings live in
`~/.var/app/dev.rustcord.Rustcord/config/rustcord/`. **Untested**: I could not run
flatpak-builder here. If it reports a missing runtime branch, bump `49` / `25.08` in
`packaging/flatpak/dev.rustcord.Rustcord.yml` and `scripts/flatpak.sh` together.

## Vencord / Equicord (opt-in, off by default)

Rustcord runs the projects' own standalone `*.user.js` builds unmodified at document start,
so your plugins and themes work as they do in the browser extension (settings and themes are
stored in the profile). Discord's terms forbid client modifications; this is your choice and
your risk.

```
rustcord mods install equicord     # or vencord; records the SHA-256, nothing runs on its own
rustcord mods enable equicord
rustcord mods status
rustcord mods path ~/my-Equicord.user.js   # your own build (userplugins) instead
```

The bundle is verified against its recorded hash on every start. Updating is manual
(`mods update`), because the bundle runs with full page privileges. Plain CSS themes still
go in `~/.config/rustcord/themes/` and `quick.css`, with or without a mod client.

## Not done (honest list)

- **Never run**, see above.
- **Tray on Window Maker**: Window Maker has no StatusNotifier host. Run one (for example
  `snixembed` with a system tray dockapp) or the tray quietly stays off after 30 s.
- **Desktop notifications**: only whatever Chromium does by default; no custom handling.
- **Screenshare**: no code of mine. Discord origins are allowed to request screen capture
  (the media permission handler passes through the requested types), but what the user sees
  (Chromium's own picker or nothing) is untested. There is no per-app audio routing; CEF has
  no `desktopCapturer` equivalent, so that needs PipeWire work.
- **Vencord/Equicord**: implemented but untested in CEF, see below. Plugins that need
  Vesktop's native layer (`VencordNative`) are unavailable; cross-origin requests from the
  clients use a `fetch` shim, so hosts without CORS headers fail.
- **Global shortcuts, autostart (background portal), badge count**: not implemented.
- **Frameless window mode**, multiple main windows: not implemented.
- **Settings UI**: there is none. Settings are edited as JSON.
- **Slint interface**: not started. In a hybrid client the web page is the main UI, so I
  deferred native Slint windows until there is something native to put in them.
- **Plugin system (WASM)**: not started. No placeholder was added on purpose.
- **Audio device enumeration**, **mod loader (Vencord/Equicord)**, **Equicloud sync**,
  **RPC/presence**, **overlay**: not started.
- **Packaging**: a `.desktop` file and a build script only. No ebuild, Flatpak or AppImage yet.
- **Proprietary codecs**: CEF's official builds ship without H.264/AAC (and Widevine), so
  inline MP4 previews that use them may not play. Calls use VP8/VP9/Opus and are unaffected.

## Layout

```
crates/core   pure logic, fully unit-tested, no CEF
crates/app    the CEF application (binary: rustcord)
packaging/    desktop entry
scripts/      build script
```
