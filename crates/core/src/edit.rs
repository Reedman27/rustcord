// SPDX-License-Identifier: LGPL-3.0-or-later
//! Editing settings from the command line (there is no settings window), plus the XDG
//! autostart entry. Pure logic, no CEF or Qt.

use std::fs;
use std::path::{Path, PathBuf};

use crate::settings::Settings;
use crate::theme;

pub const KEYS: &[&str] = &[
    "branch",
    "start_minimized",
    "disable_background_throttling",
    "sandbox",
    "tray",
    "close_to_tray",
    "rpc",
    "rpc_detect_games",
    "mod_csp_bypass",
    "accent_color",
    "color_scheme",
];

pub fn parse_bool(v: &str) -> Result<bool, String> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "1" => Ok(true),
        "false" | "off" | "no" | "0" => Ok(false),
        _ => Err(format!("expected on/off, got `{v}`")),
    }
}

/// Set one known key. Values are validated, so a typo cannot corrupt the settings file.
pub fn set_key(s: &mut Settings, key: &str, value: &str) -> Result<(), String> {
    match key {
        "branch" => {
            s.branch = serde_json::from_value(serde_json::json!(value.to_ascii_lowercase()))
                .map_err(|_| "branch must be stable, ptb or canary".to_string())?
        }
        "start_minimized" => s.start_minimized = parse_bool(value)?,
        "disable_background_throttling" => s.disable_background_throttling = parse_bool(value)?,
        "sandbox" => s.sandbox = parse_bool(value)?,
        "tray" => s.tray = parse_bool(value)?,
        "close_to_tray" => s.close_to_tray = parse_bool(value)?,
        "rpc" => s.rpc = parse_bool(value)?,
        "rpc_detect_games" => s.rpc_detect_games = parse_bool(value)?,
        "mod_csp_bypass" => s.mod_csp_bypass = parse_bool(value)?,
        "accent_color" => {
            s.accent_color = if value == "none" {
                None
            } else if theme::valid_hex_color(value) {
                Some(value.to_string())
            } else {
                return Err("accent_color must be #rgb, #rrggbb, #rrggbbaa or `none`".into());
            }
        }
        "color_scheme" => {
            s.color_scheme = match value {
                "none" => None,
                "dark" | "light" => Some(value.to_string()),
                _ => return Err("color_scheme must be dark, light or none".into()),
            }
        }
        _ => return Err(format!("unknown key `{key}`; known: {}", KEYS.join(", "))),
    }
    Ok(())
}

/// Enable or disable a theme by name, keeping the order of already enabled ones.
/// Enabling requires the file `themes/<name>.css` to exist.
pub fn set_theme(
    s: &mut Settings,
    config_dir: &Path,
    name: &str,
    enable: bool,
) -> Result<(), String> {
    if !theme::valid_theme_name(name) {
        return Err(format!("invalid theme name `{name}`"));
    }
    if enable {
        if !theme::discover(&config_dir.join("themes"))
            .iter()
            .any(|t| t == name)
        {
            return Err(format!(
                "no such theme: put it at {}/themes/{name}.css",
                config_dir.display()
            ));
        }
        if !s.themes.iter().any(|t| t == name) {
            s.themes.push(name.to_string());
        }
    } else {
        s.themes.retain(|t| t != name);
    }
    Ok(())
}

pub fn autostart_path(xdg_config_home: &Path) -> PathBuf {
    xdg_config_home.join("autostart").join("rustcord.desktop")
}

/// `exec` is the command to run at login; whitespace-free paths only, so no quoting is needed.
pub fn autostart_entry(exec: &str) -> Result<String, String> {
    if exec.is_empty()
        || exec
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '%')
    {
        return Err("executable path must not contain spaces or special characters".into());
    }
    Ok(format!(
        "[Desktop Entry]\nType=Application\nName=Rustcord\nExec={exec}\nIcon=rustcord\n\
         X-GNOME-Autostart-enabled=true\nTerminal=false\n"
    ))
}

pub fn set_autostart(xdg_config_home: &Path, exec: &str, enable: bool) -> Result<(), String> {
    let path = autostart_path(xdg_config_home);
    if enable {
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(&path, autostart_entry(exec)?).map_err(|e| e.to_string())
    } else {
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// One row of the settings window: a switch, a choice or a short text value. The window is
/// generated from this list, so the CLI (`config set`) and the window share one validation path.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Field {
    pub key: String,
    pub group: &'static str,
    pub label: String,
    pub help: &'static str,
    /// `bool` (value `on`/`off`), `choice` (one of `options`) or `text`.
    pub kind: &'static str,
    pub options: Vec<String>,
    pub value: String,
}

fn onoff(b: bool) -> String {
    if b { "on" } else { "off" }.to_string()
}

fn boolean(key: &str, group: &'static str, label: &str, help: &'static str, v: bool) -> Field {
    Field {
        key: key.into(),
        group,
        label: label.into(),
        help,
        kind: "bool",
        options: vec![],
        value: onoff(v),
    }
}

/// The fields of the settings window for the current settings. `autostart_on` is whether the
/// XDG autostart entry exists; themes are the `.css` files found in `<config>/themes`.
pub fn fields(s: &Settings, config_dir: &Path, autostart_on: bool) -> Vec<Field> {
    let branch = serde_json::to_value(s.branch)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "stable".into());
    let mut out = vec![
        Field {
            key: "branch".into(),
            group: "General",
            label: "Discord branch".into(),
            help: "Which Discord web app to load.",
            kind: "choice",
            options: vec!["stable".into(), "ptb".into(), "canary".into()],
            value: branch,
        },
        boolean(
            "start_minimized",
            "General",
            "Start minimized",
            "",
            s.start_minimized,
        ),
        boolean(
            "autostart",
            "General",
            "Start at login",
            "Adds an XDG autostart entry.",
            autostart_on,
        ),
        boolean(
            "tray",
            "Tray",
            "Show tray icon",
            "Needs a tray host (a dock tray works).",
            s.tray,
        ),
        boolean(
            "close_to_tray",
            "Tray",
            "Close button hides to tray",
            "Only applies while the tray icon is registered.",
            s.close_to_tray,
        ),
        Field {
            key: "color_scheme".into(),
            group: "Appearance",
            label: "Color scheme".into(),
            help: "Force dark or light; none follows Discord.",
            kind: "choice",
            options: vec!["none".into(), "dark".into(), "light".into()],
            value: s.color_scheme.clone().unwrap_or_else(|| "none".into()),
        },
        Field {
            key: "accent_color".into(),
            group: "Appearance",
            label: "Accent color".into(),
            help: "#rgb or #rrggbb, or none. Available to themes as --rustcord-accent.",
            kind: "text",
            options: vec![],
            value: s.accent_color.clone().unwrap_or_else(|| "none".into()),
        },
        boolean(
            "rpc",
            "Rich Presence",
            "Accept games over Discord IPC",
            "Needs the Vencord/Equicord arRPC plugin.",
            s.rpc,
        ),
        boolean(
            "rpc_detect_games",
            "Rich Presence",
            "Detect running games",
            "Run `rpc update-games` once to download the game list.",
            s.rpc_detect_games,
        ),
        boolean(
            "disable_background_throttling",
            "Advanced",
            "Keep running in the background",
            "Stops Chromium throttling voice and notifications while hidden.",
            s.disable_background_throttling,
        ),
        boolean(
            "sandbox",
            "Advanced",
            "Chromium sandbox",
            "Turn off only if the sandbox cannot start on your system.",
            s.sandbox,
        ),
        boolean(
            "mod_csp_bypass",
            "Advanced",
            "Let online themes load (Qt)",
            "Opens a loopback DevTools port while a mod client is enabled.",
            s.mod_csp_bypass,
        ),
    ];
    for name in theme::discover(&config_dir.join("themes")) {
        out.push(boolean(
            &format!("theme:{name}"),
            "Themes",
            &name,
            "",
            s.themes.contains(&name),
        ));
    }
    out
}

/// Apply one settings-window change. Handles `autostart`, `theme:<name>` and every key
/// `set_key` knows; nothing is changed if the value is rejected.
pub fn apply_field(
    s: &mut Settings,
    config_dir: &Path,
    exe: &str,
    key: &str,
    value: &str,
) -> Result<(), String> {
    if key == "autostart" {
        let xdg = config_dir.parent().ok_or("no config directory")?;
        return set_autostart(xdg, exe, parse_bool(value)?);
    }
    if let Some(name) = key.strip_prefix("theme:") {
        return set_theme(s, config_dir, name, parse_bool(value)?);
    }
    set_key(s, key, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rustcord-edit-{n}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("themes")).unwrap();
        d
    }

    #[test]
    fn sets_validated_keys() {
        let mut s = Settings::default();
        set_key(&mut s, "branch", "Canary").unwrap();
        set_key(&mut s, "tray", "off").unwrap();
        set_key(&mut s, "accent_color", "#5865f2").unwrap();
        set_key(&mut s, "color_scheme", "dark").unwrap();
        assert_eq!(s.branch, crate::nav::Branch::Canary);
        assert!(!s.tray);
        assert_eq!(s.accent_color.as_deref(), Some("#5865f2"));
        set_key(&mut s, "accent_color", "none").unwrap();
        assert!(s.accent_color.is_none());
    }

    #[test]
    fn rejects_bad_input_without_changing_anything() {
        let mut s = Settings::default();
        let before = s.clone();
        for (k, v) in [
            ("branch", "beta"),
            ("tray", "maybe"),
            ("accent_color", "red;}"),
            ("color_scheme", "neon"),
            ("nope", "1"),
        ] {
            assert!(set_key(&mut s, k, v).is_err(), "{k}={v}");
        }
        assert_eq!(s, before);
    }

    #[test]
    fn themes_enable_disable() {
        let d = tmp("themes");
        fs::write(d.join("themes/midnight.css"), "a{}").unwrap();
        let mut s = Settings::default();
        assert!(set_theme(&mut s, &d, "missing", true).is_err());
        assert!(set_theme(&mut s, &d, "../x", true).is_err());
        set_theme(&mut s, &d, "midnight", true).unwrap();
        set_theme(&mut s, &d, "midnight", true).unwrap();
        assert_eq!(s.themes, vec!["midnight"]);
        set_theme(&mut s, &d, "midnight", false).unwrap();
        assert!(s.themes.is_empty());
    }

    #[test]
    fn autostart_roundtrip() {
        let d = tmp("auto");
        set_autostart(&d, "/opt/rustcord/rustcord", true).unwrap();
        let text = fs::read_to_string(autostart_path(&d)).unwrap();
        assert!(text.contains("Exec=/opt/rustcord/rustcord\n"));
        set_autostart(&d, "/x", false).unwrap();
        assert!(!autostart_path(&d).exists());
        set_autostart(&d, "/x", false).unwrap(); // already gone: fine
        assert!(autostart_entry("/a b").is_err());
        assert!(autostart_entry("/a%f").is_err());
    }

    #[test]
    fn window_fields_round_trip_through_apply() {
        let d = tmp("fields");
        let cfg = d.join("rustcord");
        fs::create_dir_all(cfg.join("themes")).unwrap();
        fs::write(cfg.join("themes/midnight.css"), "a{}").unwrap();
        let mut s = Settings::default();
        let f = fields(&s, &cfg, false);
        // Every non-theme, non-autostart field must be accepted by set_key as it is shown.
        for fld in &f {
            let mut c = s.clone();
            assert!(
                apply_field(&mut c, &cfg, "/bin/x", &fld.key, &fld.value).is_ok(),
                "{} = {}",
                fld.key,
                fld.value
            );
            if fld.kind == "choice" {
                assert!(fld.options.contains(&fld.value), "{}", fld.key);
            }
        }
        assert!(f
            .iter()
            .any(|x| x.key == "theme:midnight" && x.value == "off"));
        apply_field(&mut s, &cfg, "/bin/x", "theme:midnight", "on").unwrap();
        apply_field(&mut s, &cfg, "/bin/x", "branch", "ptb").unwrap();
        apply_field(&mut s, &cfg, "/bin/x", "autostart", "on").unwrap();
        assert!(autostart_path(&d).exists());
        let f = fields(&s, &cfg, true);
        let get = |k: &str| f.iter().find(|x| x.key == k).unwrap().value.clone();
        assert_eq!(get("theme:midnight"), "on");
        assert_eq!(get("branch"), "ptb");
        assert_eq!(get("autostart"), "on");
        assert!(apply_field(&mut s, &cfg, "/bin/x", "theme:../x", "on").is_err());
        assert!(apply_field(&mut s, &cfg, "/bin/x", "accent_color", "red;}").is_err());
    }
}
