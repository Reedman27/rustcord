# Rustcord

A Linux-first desktop client written in Rust that hosts the **official Discord web app** inside
Chromium (CEF, through [`cef-rs`](https://github.com/tauri-apps/cef-rs)). No Electron, no Node.
Rust owns the process, windows, settings, themes and desktop integration.

It does not touch user tokens, automate accounts or call private endpoints. You sign in to
discord.com in the embedded browser like in any browser.

License: GNU LGPL, version 3 or later (`LGPL-3.0-or-later`); every source file carries an
SPDX header. `COPYING.LESSER` is the license; `COPYING` is the GPLv3 text, which the LGPLv3
incorporates by reference and which must ship with it. It is not a second, dual license.
No code was copied from the reference projects (Vesktop, Vencord and others); behaviour was
reimplemented in Rust.

## Status

This is an early build. Read "Not done" before relying on it.

### Verified here
- `rustcord-core` (URL policy, settings, XDG paths, single instance, theme engine, icon
  generation): 54 unit tests pass (including Rich Presence over a real socket and both websocket transports).
- The generated window icon PNG decodes correctly in Pillow, and the theme injection script
  applies, is idempotent and cannot inject markup in a real Chromium page
  (`python3 scripts/verify_assets.py`).
- The real Vencord and Equicord userscript builds, wrapped by the same Rust function the app
  uses, start in headless Chromium against a stub discord.com page: 179 / 373 plugins
  register and settings persist (`python3 scripts/verify_mods.py <bundle.user.js>...`).
  That is the page-side half only; the CEF injection path itself has never run.
- `rustcord` (the CEF application) **type-checks and passes clippy `-D warnings`** against the
  real cef-rs 156.6.0 bindings (`--features dox`).

- `rustcord-qt` (Qt WebEngine frontend) **runs** in a virtual display against a local stand-in
  for discord.com (`scripts/qt_e2e/run.sh <Equicord.user.js>`): real Equicord bundle with 373
  plugins at document start, `quick.css`, a CSP-blocked external stylesheet made to load by the
  DevTools bypass, WebRTC APIs, a desktop notification arriving on D-Bus, and a simulated game's
  Rich Presence reaching the page and being cleared again.

- The Qt **settings window** (`rustcord-qt --settings`, tray menu, Ctrl+,) opens in a virtual
  display; toggling checkboxes with the keyboard changed `settings.json` and created the autostart
  entry, and a second `--settings` launch reused the running window. Looks and layout checked in a
  screenshot only on Qt 6.4 / xcb.
- `scripts/install-qt.sh` installs the binary, desktop entry and icon into a temp prefix.

### Not verified (never run)
- The **CEF** application (`rustcord`) has not been linked or launched; its sandbox could not run
  Chromium. Login, text chat, voice, camera, navigation guard in a live page, window-state
  restore, deep links: nothing seen working. The first run on your machine is the real test.
- Neither frontend has been run against the real discord.com (no account or access here), so
  login, voice, video and screenshare are unconfirmed on both.
- Qt: tray, pop-out windows, the screen-share picker (compiled only on Qt 6.7+; I had 6.4) and
  your distro's Qt build are unconfirmed.

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
| `rustcord config show/set`, `themes list/enable/disable`, `autostart enable/disable` (CLI, since there is no settings window) | `core/edit.rs`, `app/mods_cli.rs` |
| Qt frontend: window, tray (SNI or XEmbed), downloads, D-Bus notifications, CSP bypass for online themes, pop-outs | `crates/qt/` |
| Selective notifications and unread badges (Qt build): per-server allow/block list, DMs switch, freedesktop notification daemon or XDG portal, tray-icon count, launcher badge | `core/notify.rs`, `crates/qt/src/notify.rs` |
| Settings window (Qt build): switches, choices, accent colour, themes, autostart; each change is validated and saved by the same code as `config set`. Open with the tray menu, Ctrl+, or `rustcord-qt --settings` | `core/edit.rs`, `qt/src/shim.cpp` |
| Browser RPC websocket on `127.0.0.1:6463..6472` (Discord's own origins plus `rpc_browser_origins`; native tools without an Origin) | `core/rpc.rs` |
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
`var(--rustcord-accent)`. Changes apply on the next page load or restart.

Themes that pull from other hosts (`@import url(https://...)`, remote `url()` images or fonts) are
detected automatically and Discord's CSP is bypassed for them, with or without a mod client
(Qt: through the loopback DevTools port, which `mod_csp_bypass: false` turns off; CEF: in-process).
The theme `<style>` element is re-created if Discord removes it. A malformed settings
file is kept as `settings.json.bad` and defaults are used.

## Chimera Linux and other musl systems (native Qt WebEngine build)

CEF/Chromium binaries are glibc-only, so on musl Rustcord uses **Qt WebEngine** (Chromium, built
by your distro) through the `rustcord-qt` frontend. It shares everything else with the CEF
build: settings, themes, Vencord/Equicord, Rich Presence, single instance, deep links.

```
apk add qt6-qtwebengine-devel rust cargo clang pkgconf     # package names: check `apk search`
./scripts/install-qt.sh                                    # builds and installs to ~/.local
rustcord-qt
target/release/rustcord-qt mods install equicord && target/release/rustcord-qt mods enable equicord
```

Verified in a virtual display (Qt 6.4, Ubuntu, `scripts/qt_e2e/run.sh <Equicord.user.js>`): the real
Equicord bundle runs at document start with 373 plugins registered, `quick.css` applies, WebRTC
APIs exist, and a simulated game's presence reaches the page through the bridge and is cleared when
the game exits. **Not verified:** real Discord login, voice/video/screenshare, the tray, pop-out
windows, and your Qt version (Chimera's is newer; I used only long-stable APIs). Voice with
Qt WebEngine depends on how your distro built it; Qt 6.7+ adds a proper screen-share picker that
this build does not use yet. Qt's tray falls back to XEmbed, so a Window Maker dock tray works.

A Flatpak recipe also exists (`scripts/flatpak.sh`, CEF inside Flatpak) as a fallback; it is untested.

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

## Selective notifications (Qt build)

Choose which servers may raise desktop notifications and unread badges:

```
rustcord-qt notify mode allow          # off (default) | allow | block
rustcord-qt notify add Rust Lang       # server name as Discord shows it, case-insensitive
rustcord-qt notify dms off             # DMs follow this switch when mode is allow/block
rustcord-qt notify show
rustcord-qt notify backend auto        # auto: portal inside Flatpak, else the notification daemon
```

Notifications go to `org.freedesktop.Notifications` (dunst, mako, swaync, ...) or the XDG desktop
portal, and clicking one focuses Rustcord. Allowed ones that arrive while the window is unfocused
are counted per server: the number is drawn on the tray icon (tooltip lists servers) and sent as a
Unity LauncherEntry signal for docks that support it. Verified in the virtual-display test: an
allowed server notifies and badges, a blocked server and a blocked DM do nothing. **Heuristic**:
the server is read from Discord's notification title `Author (#channel, Server)`; if Discord
changes that format, those notifications are treated as DMs. Not verified against real Discord,
and click-to-focus was not exercised. The CEF build has no filtering.

## Rich Presence (RPC)

Rustcord has a built-in, pure-Rust Discord IPC server (`$XDG_RUNTIME_DIR/discord-ipc-N`) so
games and tools that set a presence work without arRPC or Node. It feeds the
**WebRichPresence (arRPC)** plugin that Vencord and Equicord include, which connects to
`ws://127.0.0.1:1337` (loopback only, Discord origins only). Enable that plugin in Discord's
Vencord/Equicord settings. Tested with a simulated game and bridge client (6 tests); not tested with a
real game. Game detection is opt-in: `rustcord-qt rpc update-games` downloads Discord's list of known games,
then `config set rpc_detect_games on` shows running ones as playing (process matching, including
Wine paths, unit-tested against a fake /proc; the real list was not downloadable here).

The browser transport (`ws://127.0.0.1:6463/?v=1&client_id=<id>&encoding=json`, ports 6463 to
6472) lets web pages and tools that use Discord's browser RPC set a presence. To keep random
websites from changing your status, a browser page is accepted only from Discord's own origins
or an exact origin you list in `rpc_browser_origins` in `settings.json`; clients that send no
Origin (native tools) are always accepted. Only `SET_ACTIVITY` is implemented: invites, deep
links and the rest of the RPC command set are acknowledged and ignored. Tested with a
simulated page (connect, set, clear on close, rejected origins, bad version and client id);
not with a real web game. Not implemented: Flatpak socket sharing. Set `"rpc": false` to turn
all of it off.

## Not done (honest list)

- **CEF build never run**, see above. The Qt build ran only against a stand-in page.
- **Tray on Window Maker with the CEF build**: needs a StatusNotifier host (`snixembed`). The Qt
  build falls back to XEmbed, which a dock tray provides.
- **Notifications**: filtering and badges exist for the Qt build only; CEF uses Chromium's default.
- **Screenshare**: no per-app audio routing (needs PipeWire work). CEF shows Chromium's own
  picker or nothing; Qt 6.7+ gets a simple list dialog (uncompiled here); Qt below 6.7 cannot
  select a source.
- **Vencord/Equicord**: plugins that need Vesktop's native layer (`VencordNative`) are
  unavailable; cross-origin requests use a `fetch` shim, so hosts without CORS headers fail.
  Equicloud sync is not implemented.
- **Rich Presence**: the browser RPC handles `SET_ACTIVITY` only. Game detection exists but was only
  tested against a fake process table and a hand-written list.
- **Global shortcuts and badge count**: not implemented (no portable way on plain X11 without
  grabbing keys; I did not want to guess).
- **Frameless window mode**, multiple main windows, **audio device enumeration**: not implemented.
- **Settings window**: Qt build only (the CEF build still uses the CLI). It covers the common
  switches; `mod_client`, notification rules and `extra_switches` are still CLI or JSON. No Slint
  interface.
- **Plugin system (WASM)**: not started, no placeholder on purpose.
- **Overlay**: not started.
- **Packaging**: `.desktop` file, `scripts/install-qt.sh`, an untested Flatpak recipe and a
  draft Chimera `cports` template (`packaging/chimera/template.py`, never built; needs a release
  tag, the repo URL and a checksum). No ebuild, apk or AppImage.
- **Proprietary codecs**: official CEF builds lack H.264/AAC/Widevine (calls use VP8/VP9/Opus and
  are unaffected). Qt WebEngine depends on how your distro built it.

## Layout

```
crates/core   pure logic, fully unit-tested, no CEF
crates/app    the CEF application (binary: rustcord)
crates/qt     the Qt WebEngine application (binary: rustcord-qt), C++ shim in src/shim.cpp
packaging/    desktop entry, icon, Flatpak recipe, draft Chimera template
scripts/      build, Flatpak and verification scripts (qt_e2e/ = virtual-display test)
```
