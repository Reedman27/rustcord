// SPDX-License-Identifier: LGPL-3.0-or-later
//! XDG paths, no extra dependencies.

use std::ffi::OsString;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

const APP: &str = "rustcord";

fn pick(xdg: Option<OsString>, home: Option<OsString>, home_rel: &str) -> PathBuf {
    let base = match xdg.filter(|v| !v.is_empty()).map(PathBuf::from) {
        Some(p) if p.is_absolute() => p,
        _ => PathBuf::from(home.unwrap_or_default()).join(home_rel),
    };
    base.join(APP)
}

/// `$XDG_CONFIG_HOME/rustcord` or `~/.config/rustcord` (settings.json, state.json).
pub fn config_dir() -> PathBuf {
    pick(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
        ".config",
    )
}

/// `$XDG_DATA_HOME/rustcord` or `~/.local/share/rustcord`. Holds the browser profile
/// (cookies, login session), so it is data, not cache.
pub fn data_dir() -> PathBuf {
    pick(
        std::env::var_os("XDG_DATA_HOME"),
        std::env::var_os("HOME"),
        ".local/share",
    )
}

/// Browser profile directory.
pub fn profile_dir() -> PathBuf {
    data_dir().join("profile")
}

pub fn log_file() -> PathBuf {
    data_dir().join("rustcord.log")
}

fn current_uid() -> u32 {
    std::fs::metadata("/proc/self")
        .map(|m| m.uid())
        .unwrap_or(0)
}

/// `$XDG_RUNTIME_DIR/rustcord.sock`, falling back to a per-uid file in /tmp.
pub fn runtime_socket() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir).join(format!("{APP}.sock")),
        None => PathBuf::from("/tmp").join(format!("{APP}-{}.sock", current_uid())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_wins_when_absolute() {
        let p = pick(Some("/x/cfg".into()), Some("/home/u".into()), ".config");
        assert_eq!(p, PathBuf::from("/x/cfg/rustcord"));
    }

    #[test]
    fn falls_back_to_home() {
        assert_eq!(
            pick(None, Some("/home/u".into()), ".config"),
            PathBuf::from("/home/u/.config/rustcord")
        );
        // empty and relative XDG values are ignored, per the spec
        assert_eq!(
            pick(Some("".into()), Some("/home/u".into()), ".config"),
            PathBuf::from("/home/u/.config/rustcord")
        );
        assert_eq!(
            pick(Some("rel".into()), Some("/home/u".into()), ".config"),
            PathBuf::from("/home/u/.config/rustcord")
        );
    }
}
