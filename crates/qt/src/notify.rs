// SPDX-License-Identifier: LGPL-3.0-or-later
//! Desktop notifications with per-server filtering and unread badges.
//!
//! Qt hands each web notification to `rc_notify`. The policy in `rustcord_core::notify`
//! decides whether it may show; allowed ones go out over `org.freedesktop.Notifications`
//! (what dunst, mako, swaync and friends implement) or the XDG desktop portal inside
//! Flatpak. Clicking a notification focuses the window. Allowed notifications that arrive
//! while the window is unfocused are counted per server and shown on the tray icon and, via
//! the Unity LauncherEntry signal, on docks that support it.

use std::collections::HashMap;
use std::ffi::{c_char, c_int, CStr, CString};
use std::sync::{Mutex, OnceLock};
use std::thread;

use rustcord_core::notify::{source_of, Badges, NotifySettings};
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::Value;
use zbus::MatchRule;

extern "C" {
    fn rc_qt_show();
    fn rc_qt_set_badge(total: c_int, tooltip: *const c_char);
}

struct State {
    cfg: NotifySettings,
    portal: bool,
    conn: Option<Connection>,
    badges: Badges,
    window_active: bool,
    next_portal_id: u64,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE
        .get_or_init(|| {
            Mutex::new(State {
                cfg: NotifySettings::default(),
                portal: false,
                conn: None,
                badges: Badges::default(),
                window_active: false,
                next_portal_id: 1,
            })
        })
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Call once at startup, before the Qt event loop.
pub fn init(cfg: NotifySettings) {
    let in_flatpak = std::path::Path::new("/.flatpak-info").exists();
    let portal = match cfg.backend.as_str() {
        "portal" => true,
        "daemon" => false,
        _ => in_flatpak,
    };
    let conn = Connection::session().ok();
    if conn.is_none() {
        eprintln!("rustcord: no D-Bus session bus, desktop notifications and badges disabled");
    }
    if let Some(c) = &conn {
        spawn_click_listener(c.clone(), portal);
    }
    let mut s = state();
    s.cfg = cfg;
    s.portal = portal;
    s.conn = conn;
}

/// Focus the window when a notification is clicked.
fn spawn_click_listener(conn: Connection, portal: bool) {
    let iface = if portal {
        "org.freedesktop.portal.Notification"
    } else {
        "org.freedesktop.Notifications"
    };
    thread::spawn(move || {
        let rule = (|| {
            Ok::<_, zbus::Error>(
                MatchRule::builder()
                    .msg_type(zbus::message::Type::Signal)
                    .interface(iface)?
                    .member("ActionInvoked")?
                    .build(),
            )
        })();
        let Ok(rule) = rule else { return };
        let Ok(iter) = MessageIterator::for_match_rule(rule, &conn, None) else {
            return;
        };
        for msg in iter.flatten() {
            // Only our own notifications: the daemon's ids are not ours, so ids we sent are
            // tracked implicitly by acting on the default action of any of them.
            let action_default = if portal {
                msg.body()
                    .deserialize::<(String, String, Vec<Value>)>()
                    .map(|(_, a, _)| a)
            } else {
                msg.body().deserialize::<(u32, String)>().map(|(_, a)| a)
            };
            if matches!(action_default.as_deref(), Ok("default")) {
                unsafe { rc_qt_show() };
            }
        }
    });
}

fn send(s: &mut State, title: &str, body: &str) -> Result<(), zbus::Error> {
    let Some(conn) = s.conn.clone() else {
        return Err(zbus::Error::Failure("no session bus".into()));
    };
    if s.portal {
        let mut props: HashMap<&str, Value> = HashMap::new();
        props.insert("title", Value::from(title));
        props.insert("body", Value::from(body));
        props.insert("default-action", Value::from("default"));
        let id = format!("rustcord-{}", s.next_portal_id);
        s.next_portal_id += 1;
        conn.call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.portal.Notification"),
            "AddNotification",
            &(id, props),
        )?;
    } else {
        // Daemons render a limited markup in the body, so plain text is escaped.
        let body = body
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        let hints: HashMap<&str, Value> = HashMap::new();
        conn.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "Rustcord",
                0u32,
                "rustcord",
                title,
                body,
                vec!["default", "Open"],
                hints,
                -1i32,
            ),
        )?;
    }
    Ok(())
}

fn publish_badge(s: &State) {
    if !s.cfg.badge {
        return;
    }
    let total = s.badges.total();
    if let Ok(tip) = CString::new(s.badges.summary()) {
        // SAFETY: the shim copies the string before returning.
        unsafe { rc_qt_set_badge(total as c_int, tip.as_ptr()) };
    }
    if let Some(conn) = &s.conn {
        let mut props: HashMap<&str, Value> = HashMap::new();
        props.insert("count", Value::from(total as i64));
        props.insert("count-visible", Value::from(total > 0));
        if let Err(e) = conn.emit_signal(
            None::<&str>,
            "/com/canonical/unity/launcherentry/1",
            "com.canonical.Unity.LauncherEntry",
            "Update",
            &("application://rustcord.desktop", props),
        ) {
            eprintln!("rustcord: launcher badge signal failed: {e}");
        }
    }
}

fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: the shim passes NUL-terminated strings.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// Returns 1 if shown, 0 if filtered out, -1 if it could not be delivered (the shim then
/// falls back to a tray balloon).
#[no_mangle]
pub extern "C" fn rc_notify(title: *const c_char, body: *const c_char) -> c_int {
    let (title, body) = (cstr(title), cstr(body));
    let src = source_of(&title);
    let mut s = state();
    if !s.cfg.allows(&src) {
        return 0;
    }
    // Count first: the badge is useful even when no notification daemon is running.
    if s.cfg.badge && !s.window_active {
        s.badges.add(src);
        publish_badge(&s);
    }
    if send(&mut s, &title, &body).is_err() {
        return -1;
    }
    1
}

/// Called by the shim when the application gains or loses focus.
#[no_mangle]
pub extern "C" fn rc_window_active(active: c_int) {
    let mut s = state();
    s.window_active = active != 0;
    if s.window_active && s.badges.total() > 0 {
        s.badges.clear();
        publish_badge(&s);
    }
}
