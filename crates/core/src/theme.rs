// SPDX-License-Identifier: LGPL-3.0-or-later
//! Theme engine: user CSS files plus an accent colour, turned into one script that
//! installs a `<style>` element in the page. Pure logic, no CEF.
//!
//! Layout in the config directory:
//!   themes/<name>.css   theme files, enabled by name in settings.json ("themes": ["name"])
//!   quick.css           always applied last (quick CSS)

use std::fs;
use std::path::Path;

pub const STYLE_ELEMENT_ID: &str = "rustcord-theme";
/// Refuse absurdly large CSS so a bad file cannot stall page load.
pub const MAX_CSS_BYTES: usize = 2 * 1024 * 1024;

/// `#rgb`, `#rrggbb` or `#rrggbbaa` only. Anything else is rejected so the value can be
/// placed into CSS without escaping concerns.
pub fn valid_hex_color(s: &str) -> bool {
    let Some(hex) = s.strip_prefix('#') else {
        return false;
    };
    matches!(hex.len(), 3 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit())
}

/// Theme names are file stems: letters, digits, `-`, `_`, `.` (not leading), no separators.
pub fn valid_theme_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Names of `*.css` files in `dir`, sorted. Missing directory means no themes.
pub fn discover(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension()? != "css" {
                return None;
            }
            let stem = path.file_stem()?.to_str()?.to_string();
            valid_theme_name(&stem).then_some(stem)
        })
        .collect();
    names.sort();
    names
}

fn read_css(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() as usize > MAX_CSS_BYTES {
        return None;
    }
    fs::read_to_string(path).ok()
}

/// Concatenate the accent variable, enabled themes (in the order given) and quick CSS.
/// Unknown or invalid theme names are skipped.
pub fn build_css(
    config_dir: &Path,
    enabled: &[String],
    accent: Option<&str>,
    mode: Option<&str>,
) -> String {
    let mut css = String::new();
    if let Some(a) = accent.filter(|a| valid_hex_color(a)) {
        css.push_str(&format!(":root{{--rustcord-accent:{a};}}\n"));
    }
    if let Some(m) = mode.filter(|m| matches!(*m, "dark" | "light")) {
        css.push_str(&format!(":root{{color-scheme:{m};}}\n"));
    }
    let themes_dir = config_dir.join("themes");
    for name in enabled.iter().filter(|n| valid_theme_name(n)) {
        if let Some(body) = read_css(&themes_dir.join(format!("{name}.css"))) {
            css.push_str(&format!("/* theme: {name} */\n{body}\n"));
        }
    }
    if let Some(body) = read_css(&config_dir.join("quick.css")) {
        css.push_str("/* quick.css */\n");
        css.push_str(&body);
        css.push('\n');
    }
    css
}

/// JavaScript that creates or replaces the theme `<style>` element. The CSS is embedded
/// as a JSON string literal, so quotes, newlines and `</style>` cannot break out.
pub fn injection_script(css: &str) -> String {
    // serde_json escapes control characters and quotes; `<` is additionally escaped so the
    // literal can never contain a closing tag if this script is ever inlined in HTML.
    let literal = serde_json::to_string(css)
        .unwrap_or_else(|_| "\"\"".into())
        .replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    format!(
        "(function(){{var id={id};var el=document.getElementById(id);\
if(!el){{el=document.createElement('style');el.id=id;\
(document.head||document.documentElement).appendChild(el);}}\
el.textContent={literal};}})();",
        id = serde_json::to_string(STYLE_ELEMENT_ID).unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rustcord-theme-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("themes")).unwrap();
        d
    }

    #[test]
    fn hex_colours() {
        for ok in ["#fff", "#FFFFFF", "#12345678", "#5865f2"] {
            assert!(valid_hex_color(ok), "{ok}");
        }
        for bad in [
            "",
            "fff",
            "#ff",
            "#ggg",
            "#12345",
            "red",
            "#fff;}body{display:none",
            "#fff\n",
        ] {
            assert!(!valid_hex_color(bad), "{bad:?}");
        }
    }

    #[test]
    fn theme_names() {
        assert!(valid_theme_name("midnight-2"));
        assert!(valid_theme_name("a.b_c"));
        for bad in ["", ".hidden", "../x", "a/b", "a b", "a\\b"] {
            assert!(!valid_theme_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn discovers_only_valid_css() {
        let d = tmpdir("discover");
        fs::write(d.join("themes/zeta.css"), "a{}").unwrap();
        fs::write(d.join("themes/alpha.css"), "a{}").unwrap();
        fs::write(d.join("themes/notes.txt"), "x").unwrap();
        fs::write(d.join("themes/.hidden.css"), "x").unwrap();
        assert_eq!(discover(&d.join("themes")), vec!["alpha", "zeta"]);
        assert!(discover(&d.join("missing")).is_empty());
    }

    #[test]
    fn builds_in_order_and_skips_bad_names() {
        let d = tmpdir("build");
        fs::write(d.join("themes/one.css"), ".one{}").unwrap();
        fs::write(d.join("themes/two.css"), ".two{}").unwrap();
        fs::write(d.join("quick.css"), ".quick{}").unwrap();
        let css = build_css(
            &d,
            &[
                "two".into(),
                "one".into(),
                "../etc".into(),
                "missing".into(),
            ],
            Some("#abcdef"),
            Some("dark"),
        );
        let (a, t, o, q) = (
            css.find("--rustcord-accent:#abcdef").unwrap(),
            css.find(".two{}").unwrap(),
            css.find(".one{}").unwrap(),
            css.find(".quick{}").unwrap(),
        );
        assert!(a < t && t < o && o < q, "{css}");
        assert!(css.contains("color-scheme:dark"));
        assert!(!css.contains("etc"));
    }

    #[test]
    fn rejects_bad_accent_and_mode() {
        let d = tmpdir("accent");
        let css = build_css(&d, &[], Some("red;}x{"), Some("neon"));
        assert!(css.is_empty(), "{css}");
    }

    #[test]
    fn oversized_css_is_ignored() {
        let d = tmpdir("big");
        fs::write(d.join("quick.css"), vec![b'a'; MAX_CSS_BYTES + 1]).unwrap();
        assert!(build_css(&d, &[], None, None).is_empty());
    }

    #[test]
    fn script_cannot_break_out_of_the_literal() {
        let evil = "a{}\"; alert(1); //</style><script>x</script>\u{2028}\n";
        let js = injection_script(evil);
        assert!(!js.contains("</style>"));
        assert!(!js.contains('\u{2028}'));
        // the payload only appears inside the escaped literal
        assert!(js.contains("\\\"; alert(1); //\\u003c/style>"));
        assert!(js.starts_with("(function(){var id=\"rustcord-theme\""));
    }
}
