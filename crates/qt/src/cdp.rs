// SPDX-License-Identifier: LGPL-3.0-or-later
//! Content-Security-Policy bypass for the Discord page through Chromium's DevTools protocol.
//!
//! Discord's CSP blocks stylesheets and images from other hosts, which breaks online
//! Vencord/Equicord themes. The browser extensions strip the header; Qt WebEngine offers no
//! way to edit response headers, so this attaches to the page over the loopback debugging
//! port and calls `Page.setBypassCSP`, then reloads the page once so it applies.
//!
//! Only used while a mod client is enabled or an enabled theme / quick.css loads remote assets. The debugging port listens on 127.0.0.1 only,
//! but any local process of any user can connect to it, which is why it is limited to that case
//! and can be turned off with `mod_csp_bypass = false`.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rustcord_core::nav;
use serde_json::{json, Value};

/// A free loopback port. There is a small race between releasing and Chromium binding it,
/// acceptable for a local helper; if it loses, the bypass is simply unavailable.
pub fn free_port() -> Option<u16> {
    TcpListener::bind(("127.0.0.1", 0))
        .ok()?
        .local_addr()
        .ok()
        .map(|a| a.port())
}

fn http_get(port: u16, path: &str) -> Option<String> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    // Chromium keeps the connection open, so read by Content-Length, not to EOF.
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos]).to_ascii_lowercase();
            let len: usize = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())?;
            if buf.len() >= pos + 4 + len {
                return String::from_utf8(buf[pos + 4..pos + 4 + len].to_vec()).ok();
            }
        }
        let n = s.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn attach(port: u16, ws_url: &str) {
    let Ok(tcp) = TcpStream::connect(("127.0.0.1", port)) else {
        return;
    };
    let mut ws = match tungstenite::client(ws_url, tcp) {
        Ok((ws, _)) => ws,
        Err(e) => {
            eprintln!("rustcord: DevTools attach failed: {e}");
            return;
        }
    };
    let send = |ws: &mut tungstenite::WebSocket<TcpStream>, id: u32, m: &str, p: Value| {
        ws.send(tungstenite::Message::text(
            json!({"id": id, "method": m, "params": p}).to_string(),
        ))
        .is_ok()
    };
    if !send(&mut ws, 1, "Page.setBypassCSP", json!({"enabled": true})) {
        return;
    }
    // The reload makes the already-loaded page pick the bypass up.
    if !send(&mut ws, 2, "Page.reload", json!({})) {
        return;
    }
    // Keep the session open: the bypass only lasts while a client stays attached.
    while ws.read().is_ok() {}
}

/// Watch for Discord page targets and attach to each one once.
pub fn spawn(port: u16) {
    thread::spawn(move || {
        let seen: Arc<Mutex<HashSet<String>>> = Arc::default();
        loop {
            thread::sleep(Duration::from_millis(400));
            let Some(body) = http_get(port, "/json/list") else {
                continue;
            };
            let Ok(Value::Array(targets)) = serde_json::from_str::<Value>(&body) else {
                continue;
            };
            for t in targets {
                let (Some(id), Some(url), Some(ws)) = (
                    t["id"].as_str(),
                    t["url"].as_str(),
                    t["webSocketDebuggerUrl"].as_str(),
                ) else {
                    continue;
                };
                if t["type"] != "page" || !nav::is_discord_url(url) {
                    continue;
                }
                if !seen
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id.to_string())
                {
                    continue;
                }
                let (id, ws, seen) = (id.to_string(), ws.to_string(), seen.clone());
                thread::spawn(move || {
                    attach(port, &ws);
                    seen.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                });
            }
        }
    });
}
