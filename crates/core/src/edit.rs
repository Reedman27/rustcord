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
}
