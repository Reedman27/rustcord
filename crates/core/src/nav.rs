// SPDX-License-Identifier: LGPL-3.0-or-later
//! URL policy. Only Discord's own hosts may load inside the app window;
//! everything else is handed to the system browser. Parsing uses the `url`
//! crate (WHATWG rules) so userinfo, port, backslash and lookalike-host tricks
//! are handled the same way Chromium handles them.

use serde::{Deserialize, Serialize};
use url::Url;

pub const DISCORD_HOSTS: [&str; 3] = ["discord.com", "canary.discord.com", "ptb.discord.com"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Branch {
    #[default]
    Stable,
    Canary,
    Ptb,
}

impl Branch {
    fn subdomain(self) -> &'static str {
        match self {
            Branch::Stable => "",
            Branch::Canary => "canary.",
            Branch::Ptb => "ptb.",
        }
    }
}

/// True only for `https://` URLs whose host is exactly a Discord host.
pub fn is_discord_url(url: &str) -> bool {
    let Ok(u) = Url::parse(url) else { return false };
    if u.scheme() != "https" {
        return false;
    }
    // Userinfo (`https://discord.com@evil.com`) is never legitimate here.
    if !u.username().is_empty() || u.password().is_some() {
        return false;
    }
    match u.host_str() {
        Some(host) => DISCORD_HOSTS.contains(&host),
        None => false,
    }
}

/// Whether the main frame may navigate to `url` inside the app window. Discord's own hosts,
/// plus the two internal pages Chromium and Discord use themselves: `about:blank` (the
/// initial document of pop-out windows) and Chromium's offline error page.
pub fn in_app_navigation_allowed(url: &str) -> bool {
    is_discord_url(url) || url == "about:blank" || url.starts_with("chrome-error://chromewebdata/")
}

/// Schemes we are willing to pass to `xdg-open`. Anything else (file:, javascript:,
/// custom app schemes) is dropped.
pub fn external_open_allowed(url: &str) -> bool {
    match Url::parse(url) {
        Ok(u) => matches!(u.scheme(), "http" | "https" | "mailto"),
        Err(_) => false,
    }
}

/// `https://discord.com/<path>` for the chosen branch. `path` may start with `/`.
pub fn discord_url(branch: Branch, path: &str) -> String {
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() {
        "channels/@me"
    } else {
        path
    };
    format!("https://{}discord.com/{}", branch.subdomain(), path)
}

/// Turn `discord://-/channels/123/456` into `Some("channels/123/456")`.
/// Returns `None` for anything that is not a `discord://` URI.
pub fn deep_link_path(uri: &str) -> Option<String> {
    let u = Url::parse(uri).ok()?;
    if u.scheme() != "discord" {
        return None;
    }
    let mut path = u.path().trim_start_matches('/').to_string();
    if let Some(q) = u.query() {
        path.push('?');
        path.push_str(q);
    }
    Some(path)
}

/// Full https URL for a `discord://` deep link, or the default landing page.
pub fn deep_link_url(branch: Branch, uri: Option<&str>) -> String {
    let path = uri.and_then(deep_link_path).unwrap_or_default();
    discord_url(branch, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_discord_hosts() {
        assert!(is_discord_url("https://discord.com/channels/@me"));
        assert!(is_discord_url("https://canary.discord.com/app"));
        assert!(is_discord_url("https://ptb.discord.com/"));
        assert!(is_discord_url("https://discord.com:443/x"));
        assert!(is_discord_url("HTTPS://DISCORD.COM/x"));
    }

    #[test]
    fn rejects_lookalikes_and_tricks() {
        for bad in [
            "http://discord.com/",
            "https://discord.com.evil.com/",
            "https://evil.com/discord.com",
            "https://evil.com/?u=https://discord.com/",
            "https://discord.com@evil.com/",
            "https://evil.com@discord.com/",
            "https://user:pw@discord.com/",
            "https://notdiscord.com/",
            "https://discord.com./",
            "https://sub.discord.com/",
            "https://cdn.discordapp.com/x",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "discord://-/channels/@me",
            "about:blank",
            "",
            "not a url",
        ] {
            assert!(!is_discord_url(bad), "must reject {bad:?}");
        }
        // WHATWG treats backslash as a path separator for https, so this host is evil.com.
        assert!(!is_discord_url("https://evil.com\\@discord.com/"));
    }

    #[test]
    fn in_app_navigation_policy() {
        assert!(in_app_navigation_allowed("https://discord.com/app"));
        assert!(in_app_navigation_allowed("about:blank"));
        assert!(in_app_navigation_allowed("chrome-error://chromewebdata/"));
        for bad in [
            "https://example.org/",
            "about:srcdoc",
            "about:blank#x\nhttps://evil",
            "data:text/html,<script>1</script>",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "chrome://settings",
            "chrome-error://other/",
            "",
        ] {
            assert!(!in_app_navigation_allowed(bad), "must reject {bad:?}");
        }
    }

    #[test]
    fn external_scheme_allowlist() {
        assert!(external_open_allowed("https://example.org/"));
        assert!(external_open_allowed("http://example.org/"));
        assert!(external_open_allowed("mailto:a@b.c"));
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ssh://host",
            "steam://run/1",
            "",
        ] {
            assert!(!external_open_allowed(bad), "must reject {bad:?}");
        }
    }

    #[test]
    fn builds_urls_per_branch() {
        assert_eq!(
            discord_url(Branch::Stable, ""),
            "https://discord.com/channels/@me"
        );
        assert_eq!(
            discord_url(Branch::Canary, "/app"),
            "https://canary.discord.com/app"
        );
        assert_eq!(
            discord_url(Branch::Ptb, "channels/1/2"),
            "https://ptb.discord.com/channels/1/2"
        );
    }

    #[test]
    fn parses_deep_links() {
        assert_eq!(
            deep_link_path("discord://-/channels/@me").as_deref(),
            Some("channels/@me")
        );
        assert_eq!(
            deep_link_path("discord://-/channels/1/2?x=y").as_deref(),
            Some("channels/1/2?x=y")
        );
        assert_eq!(deep_link_path("https://discord.com/channels/@me"), None);
        assert_eq!(deep_link_path("garbage"), None);
        assert_eq!(
            deep_link_url(Branch::Stable, Some("discord://-/channels/1/2")),
            "https://discord.com/channels/1/2"
        );
        assert_eq!(
            deep_link_url(Branch::Stable, None),
            "https://discord.com/channels/@me"
        );
        assert_eq!(
            deep_link_url(Branch::Stable, Some("junk")),
            "https://discord.com/channels/@me"
        );
    }

    #[test]
    fn deep_link_cannot_escape_host() {
        // The path is appended to a fixed host, so even odd paths stay on discord.com.
        let url = deep_link_url(Branch::Stable, Some("discord://evil.com/..//evil.com"));
        assert!(is_discord_url(&url), "{url}");
    }
}
