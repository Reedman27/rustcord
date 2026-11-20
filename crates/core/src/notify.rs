//! Selective notifications: choose which servers (and whether DMs) may raise desktop
//! notifications and unread badges. Pure logic, shared by the frontends.
//!
//! Discord's web client titles guild notifications `Author (#channel, Server name)` and
//! DM notifications with just the author, so the server is read from the title. This is a
//! heuristic that depends on Discord keeping that format; a title that does not match is
//! treated as a DM / other and follows the `dms` switch.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Every notification is shown (default).
    #[default]
    Off,
    /// Only the listed servers (and DMs when `dms` is on) are shown.
    Allow,
    /// Everything except the listed servers is shown.
    Block,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifySettings {
    pub mode: Mode,
    /// Server names as Discord displays them; compared case-insensitively.
    pub servers: Vec<String>,
    /// Whether DMs and anything without a server get through when mode is not `off`.
    pub dms: bool,
    /// `auto` (portal inside Flatpak, otherwise the notification daemon), `daemon` or `portal`.
    pub backend: String,
    /// Count notifications that arrive while the window is unfocused and show the number
    /// on the tray icon and the launcher entry.
    pub badge: bool,
}

impl Default for NotifySettings {
    fn default() -> Self {
        Self {
            mode: Mode::Off,
            servers: Vec::new(),
            dms: true,
            backend: "auto".into(),
            badge: true,
        }
    }
}

/// Where a notification came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Dm,
    Server(String),
}

impl Source {
    pub fn label(&self) -> &str {
        match self {
            Source::Dm => "Direct messages",
            Source::Server(s) => s,
        }
    }
}

/// `Author (#channel, Server name)` gives the server; anything else is `Dm`.
pub fn source_of(title: &str) -> Source {
    let t = title.trim_end();
    if let Some(inner) = t.strip_suffix(')') {
        if let Some(open) = inner.rfind(" (#") {
            let rest = &inner[open + 3..];
            if let Some((_channel, server)) = rest.split_once(", ") {
                let server = server.trim();
                if !server.is_empty() {
                    return Source::Server(server.to_string());
                }
            }
        }
    }
    Source::Dm
}

impl NotifySettings {
    fn listed(&self, server: &str) -> bool {
        self.servers.iter().any(|s| s.eq_ignore_ascii_case(server))
    }

    pub fn allows(&self, src: &Source) -> bool {
        match (self.mode, src) {
            (Mode::Off, _) => true,
            (_, Source::Dm) => self.dms,
            (Mode::Allow, Source::Server(s)) => self.listed(s),
            (Mode::Block, Source::Server(s)) => !self.listed(s),
        }
    }

    pub fn add_server(&mut self, name: &str) {
        let name = name.trim();
        if !name.is_empty() && !self.listed(name) {
            self.servers.push(name.to_string());
        }
    }

    pub fn remove_server(&mut self, name: &str) {
        self.servers
            .retain(|s| !s.eq_ignore_ascii_case(name.trim()));
    }
}

/// Unread notification counts per source since the window was last focused.
#[derive(Debug, Default)]
pub struct Badges {
    counts: Vec<(Source, u32)>,
}

impl Badges {
    pub fn add(&mut self, src: Source) {
        match self.counts.iter_mut().find(|(s, _)| *s == src) {
            Some((_, n)) => *n += 1,
            None => self.counts.push((src, 1)),
        }
    }
    pub fn clear(&mut self) {
        self.counts.clear();
    }
    pub fn total(&self) -> u32 {
        self.counts.iter().map(|(_, n)| n).sum()
    }
    /// Tooltip text such as `3 unread: Rust (2), Direct messages (1)`.
    pub fn summary(&self) -> String {
        if self.counts.is_empty() {
            return "Rustcord".into();
        }
        let parts: Vec<String> = self
            .counts
            .iter()
            .map(|(s, n)| format!("{} ({n})", s.label()))
            .collect();
        format!("{} unread: {}", self.total(), parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(mode: Mode, servers: &[&str], dms: bool) -> NotifySettings {
        NotifySettings {
            mode,
            servers: servers.iter().map(|x| x.to_string()).collect(),
            dms,
            ..Default::default()
        }
    }

    #[test]
    fn parses_titles() {
        assert_eq!(
            source_of("Alice (#general, Rust Lang)"),
            Source::Server("Rust Lang".into())
        );
        // commas and parentheses in the server name
        assert_eq!(
            source_of("Bob (#off-topic, Cats, Dogs & (Friends))"),
            Source::Server("Cats, Dogs & (Friends)".into())
        );
        assert_eq!(source_of("Alice"), Source::Dm);
        assert_eq!(source_of("Alice (she/her)"), Source::Dm);
        assert_eq!(source_of("Alice (#general)"), Source::Dm);
        assert_eq!(source_of(""), Source::Dm);
    }

    #[test]
    fn allow_and_block_modes() {
        let srv = |n: &str| Source::Server(n.into());
        let off = s(Mode::Off, &["x"], false);
        assert!(off.allows(&srv("anything")) && off.allows(&Source::Dm));

        let allow = s(Mode::Allow, &["Rust Lang"], true);
        assert!(allow.allows(&srv("rust lang")));
        assert!(!allow.allows(&srv("Other")));
        assert!(allow.allows(&Source::Dm));
        assert!(!s(Mode::Allow, &["Rust Lang"], false).allows(&Source::Dm));

        let block = s(Mode::Block, &["Spam"], true);
        assert!(!block.allows(&srv("SPAM")));
        assert!(block.allows(&srv("Fine")));
        assert!(!s(Mode::Block, &[], false).allows(&Source::Dm));
    }

    #[test]
    fn list_editing() {
        let mut n = NotifySettings::default();
        n.add_server("Rust Lang");
        n.add_server("rust lang");
        n.add_server("  ");
        assert_eq!(n.servers, vec!["Rust Lang"]);
        n.remove_server("RUST LANG");
        assert!(n.servers.is_empty());
    }

    #[test]
    fn badge_counts() {
        let mut b = Badges::default();
        assert_eq!(b.summary(), "Rustcord");
        b.add(Source::Server("Rust".into()));
        b.add(Source::Server("Rust".into()));
        b.add(Source::Dm);
        assert_eq!(b.total(), 3);
        assert_eq!(b.summary(), "3 unread: Rust (2), Direct messages (1)");
        b.clear();
        assert_eq!(b.total(), 0);
    }
}
