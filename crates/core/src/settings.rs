//! User settings and window state, stored as JSON with atomic writes.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::mods::ModClient;
use crate::nav::Branch;

pub const MIN_WIDTH: i32 = 940;
pub const MIN_HEIGHT: i32 = 500;
pub const DEFAULT_WIDTH: i32 = 1280;
pub const DEFAULT_HEIGHT: i32 = 720;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Which Discord web app to load.
    pub branch: Branch,
    /// Start with the window minimized.
    pub start_minimized: bool,
    /// Keep Chromium timers and renderers running while the window is hidden or
    /// covered, so voice and notifications do not stall (same as Vesktop).
    pub disable_background_throttling: bool,
    /// Chromium sandbox. Turn off only if the sandbox cannot start on your system.
    pub sandbox: bool,
    /// Extra Chromium switches, `name` or `name=value`, without leading dashes.
    pub extra_switches: Vec<String>,
    /// Show a system tray icon (StatusNotifierItem) when a tray host is available.
    pub tray: bool,
    /// Closing the window hides it to the tray instead of quitting. Only takes effect
    /// while the tray is actually registered, so the app can never become unreachable.
    pub close_to_tray: bool,
    /// Enabled theme names (files in `<config>/themes/<name>.css`), applied in order.
    pub themes: Vec<String>,
    /// Accent colour as `#rgb`, `#rrggbb` or `#rrggbbaa`, exposed as `--rustcord-accent`.
    pub accent_color: Option<String>,
    /// Force `dark` or `light` color-scheme; `None` follows Discord.
    pub color_scheme: Option<String>,
    /// Opt-in: load Vencord or Equicord (installed with `rustcord mods install`).
    /// Off by default; modifying the Discord client is against Discord's terms.
    pub mod_client: Option<ModClient>,
    /// Path to your own build of the selected client (e.g. with userplugins), used
    /// instead of the installed bundle.
    pub mod_script: Option<String>,
    /// Rich Presence: accept games on `$XDG_RUNTIME_DIR/discord-ipc-N` and feed the
    /// Vencord/Equicord "WebRichPresence (arRPC)" plugin on 127.0.0.1:1337.
    pub rpc: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            branch: Branch::Stable,
            start_minimized: false,
            disable_background_throttling: true,
            sandbox: true,
            extra_switches: Vec::new(),
            tray: true,
            close_to_tray: false,
            themes: Vec::new(),
            accent_color: None,
            color_scheme: None,
            mod_client: None,
            mod_script: None,
            rpc: true,
        }
    }
}

impl Settings {
    /// Chromium switches implied by these settings, as `(name, optional value)`.
    pub fn chromium_switches(&self) -> Vec<(String, Option<String>)> {
        let mut out: Vec<(String, Option<String>)> = Vec::new();
        if self.disable_background_throttling {
            for s in [
                "disable-background-timer-throttling",
                "disable-renderer-backgrounding",
                "disable-backgrounding-occluded-windows",
            ] {
                out.push((s.to_string(), None));
            }
        }
        for raw in &self.extra_switches {
            let raw = raw.trim().trim_start_matches('-');
            if raw.is_empty() {
                continue;
            }
            match raw.split_once('=') {
                Some((k, v)) if valid_switch_name(k) => {
                    out.push((k.to_string(), Some(v.to_string())))
                }
                None if valid_switch_name(raw) => out.push((raw.to_string(), None)),
                _ => {}
            }
        }
        out
    }
}

fn valid_switch_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            maximized: false,
        }
    }
}

impl WindowState {
    /// True if enough of this window (at least 100x50 px) overlaps the screen rectangle, so a
    /// position saved on a monitor that is gone is not restored off-screen.
    pub fn overlaps_screen(&self, sx: i32, sy: i32, sw: i32, sh: i32) -> bool {
        let (Some(x), Some(y)) = (self.x, self.y) else {
            return false;
        };
        let w = (x.saturating_add(self.width)).min(sx.saturating_add(sw)) - x.max(sx);
        let h = (y.saturating_add(self.height)).min(sy.saturating_add(sh)) - y.max(sy);
        w >= 100 && h >= 50
    }

    /// Clamp sizes to sane values so a corrupt state file cannot create a 1x1 window.
    pub fn sanitized(mut self) -> Self {
        self.width = self.width.clamp(MIN_WIDTH, 16384);
        self.height = self.height.clamp(MIN_HEIGHT, 16384);
        if self.x.is_some() != self.y.is_some() {
            self.x = None;
            self.y = None;
        }
        self
    }
}

/// Load JSON from `path`. A missing file or malformed JSON yields `T::default()`;
/// a malformed file is kept as `<name>.bad` so the user's data is not silently lost.
pub fn load_or_default<T: Default + for<'de> Deserialize<'de>>(path: &Path) -> T {
    let Ok(bytes) = fs::read(path) else {
        return T::default();
    };
    match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => {
            let mut bad = path.as_os_str().to_owned();
            bad.push(".bad");
            let _ = fs::rename(path, bad);
            T::default()
        }
    }
}

/// Write JSON atomically: temp file in the same directory, fsync, rename.
pub fn save_atomic<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp{}", std::process::id()));
    let tmp = std::path::PathBuf::from(tmp);
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(value).map_err(io::Error::other)?)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rustcord-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn roundtrip_and_defaults() {
        let d = tmpdir("settings");
        let p = d.join("settings.json");
        assert_eq!(load_or_default::<Settings>(&p), Settings::default());
        let s = Settings {
            branch: Branch::Canary,
            start_minimized: true,
            ..Default::default()
        };
        save_atomic(&p, &s).unwrap();
        assert_eq!(load_or_default::<Settings>(&p), s);
        // no temp files left behind
        let leftovers: Vec<_> = fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(leftovers.len(), 1, "{leftovers:?}");
    }

    #[test]
    fn partial_json_fills_defaults() {
        let d = tmpdir("partial");
        let p = d.join("settings.json");
        fs::write(&p, br#"{"branch":"ptb"}"#).unwrap();
        let s: Settings = load_or_default(&p);
        assert_eq!(s.branch, Branch::Ptb);
        assert!(s.sandbox && s.disable_background_throttling);
    }

    #[test]
    fn corrupt_file_is_preserved() {
        let d = tmpdir("corrupt");
        let p = d.join("settings.json");
        fs::write(&p, b"{ not json").unwrap();
        assert_eq!(load_or_default::<Settings>(&p), Settings::default());
        assert!(d.join("settings.json.bad").exists());
    }

    #[test]
    fn saved_position_must_be_on_a_screen() {
        let at = |x, y| WindowState {
            x: Some(x),
            y: Some(y),
            width: 1280,
            height: 720,
            maximized: false,
        };
        // 1920x1080 screen at the origin
        assert!(at(0, 0).overlaps_screen(0, 0, 1920, 1080));
        assert!(at(600, 300).overlaps_screen(0, 0, 1920, 1080));
        // 120x80 px still on screen: enough to grab and drag
        assert!(at(1800, 1000).overlaps_screen(0, 0, 1920, 1080));
        // only 80 px wide on screen: too little
        assert!(!at(-1200, 10).overlaps_screen(0, 0, 1920, 1080));
        // fully on another monitor that is no longer connected
        assert!(!at(3000, 0).overlaps_screen(0, 0, 1920, 1080));
        // no saved position: nothing to restore
        assert!(!WindowState::default().overlaps_screen(0, 0, 1920, 1080));
    }

    #[test]
    fn window_state_is_clamped() {
        let w = WindowState {
            x: Some(5),
            y: None,
            width: 1,
            height: 99999,
            maximized: false,
        }
        .sanitized();
        assert_eq!((w.width, w.height), (MIN_WIDTH, 16384));
        assert_eq!((w.x, w.y), (None, None));
    }

    #[test]
    fn switches_are_filtered() {
        let s = Settings {
            extra_switches: vec![
                "--enable-gpu-rasterization".into(),
                "ozone-platform=x11".into(),
                "bad name".into(),
                "".into(),
                "x;rm -rf=1".into(),
            ],
            ..Default::default()
        };
        let sw = s.chromium_switches();
        assert!(sw.contains(&("enable-gpu-rasterization".into(), None)));
        assert!(sw.contains(&("ozone-platform".into(), Some("x11".into()))));
        assert!(!sw.iter().any(|(k, _)| k.contains(' ') || k.contains(';')));
        assert!(sw
            .iter()
            .any(|(k, _)| k == "disable-renderer-backgrounding"));
    }
}
