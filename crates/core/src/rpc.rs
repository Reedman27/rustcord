// SPDX-License-Identifier: LGPL-3.0-or-later
//! Rich Presence for the web app, compatible with games that speak Discord's local IPC
//! protocol (`$XDG_RUNTIME_DIR/discord-ipc-N`) and with the arRPC bridge plugin that Vencord
//! and Equicord ship ("WebRichPresence (arRPC)"), which listens on `ws://127.0.0.1:1337`.
//!
//! Games connect to the unix socket and send `SET_ACTIVITY`; each update is forwarded to
//! connected bridge clients as `{"activity":{..},"pid":N,"socketId":".."}`. Web pages and
//! tools that use Discord's browser transport connect to `ws://127.0.0.1:6463..6472` instead
//! (see [`serve_browser`]). Automatic game detection lives in `games.rs`.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

pub const BRIDGE_PORT: u16 = 1337;
/// Discord's browser RPC transport tries these ports in order (same range as the real client).
pub const BROWSER_PORTS: std::ops::RangeInclusive<u16> = 6463..=6472;
const MAX_BROWSER_CONNECTIONS: usize = 32;
const MAX_FRAME: usize = 64 * 1024;

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;
const OP_PING: u32 = 3;
const OP_PONG: u32 = 4;

/// Fan-out of activity updates to bridge clients, remembering the latest per socket so a
/// client that connects later still sees what is already running.
#[derive(Default)]
pub struct Hub {
    inner: Mutex<HubInner>,
}

#[derive(Default)]
struct HubInner {
    clients: Vec<Sender<String>>,
    current: HashMap<String, String>,
}

impl Hub {
    pub fn publish(&self, socket_id: &str, message: String, clear: bool) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if clear {
            g.current.remove(socket_id);
        } else {
            g.current.insert(socket_id.to_string(), message.clone());
        }
        g.clients.retain(|c| c.send(message.clone()).is_ok());
    }

    pub fn subscribe(&self) -> Receiver<String> {
        let (tx, rx) = mpsc::channel();
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        for m in g.current.values() {
            let _ = tx.send(m.clone());
        }
        g.clients.push(tx);
        rx
    }
}

fn frame(op: u32, payload: &Value) -> Vec<u8> {
    let body = payload.to_string().into_bytes();
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

fn read_frame(s: &mut impl Read) -> io::Result<(u32, Value)> {
    let mut head = [0u8; 8];
    s.read_exact(&mut head)?;
    let op = u32::from_le_bytes(head[..4].try_into().unwrap());
    let len = u32::from_le_bytes(head[4..].try_into().unwrap()) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut body = vec![0u8; len];
    s.read_exact(&mut body)?;
    let v = serde_json::from_slice(&body).unwrap_or(Value::Null);
    Ok((op, v))
}

/// Shape an activity the way the web client expects (same rules as arRPC).
pub fn normalize_activity(mut a: Value, client_id: &str) -> Value {
    let Some(obj) = a.as_object_mut() else {
        return a;
    };
    obj.insert("application_id".into(), json!(client_id));
    obj.entry("type").or_insert(json!(0));
    let instance = obj
        .get("instance")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    obj.insert("flags".into(), json!(if instance { 1 } else { 0 }));
    if let Some(ts) = obj.get_mut("timestamps").and_then(Value::as_object_mut) {
        for key in ["start", "end"] {
            if let Some(n) = ts.get(key).and_then(Value::as_f64) {
                // Games send seconds; the client wants milliseconds.
                let ms = if n < 1.0e10 { n * 1000.0 } else { n };
                ts.insert(key.into(), json!(ms as u64));
            }
        }
    }
    if let Some(buttons) = obj.get("buttons").and_then(Value::as_array).cloned() {
        let labels: Vec<Value> = buttons
            .iter()
            .filter_map(|b| b.get("label").cloned())
            .collect();
        let urls: Vec<Value> = buttons
            .iter()
            .filter_map(|b| b.get("url").cloned())
            .collect();
        obj.insert("buttons".into(), Value::Array(labels));
        obj.insert("metadata".into(), json!({ "button_urls": urls }));
    }
    a
}

fn bridge_message(activity: Value, pid: u64, socket_id: &str) -> String {
    json!({ "activity": activity, "pid": pid, "socketId": socket_id }).to_string()
}

/// The READY event both transports send after a valid handshake.
fn ready_event() -> Value {
    json!({"cmd":"DISPATCH","evt":"READY","nonce":null,"data":{
        "v":1,
        "config":{"cdn_host":"cdn.discordapp.com","api_endpoint":"//discord.com/api","environment":"production"},
        "user":{"id":"1045800378228281345","username":"rustcord","discriminator":"0",
                "global_name":"Rustcord","avatar":null,"bot":false,"flags":0,"premium_type":0}}})
}

/// Handle one command frame from a game or web page and return the reply. Only
/// `SET_ACTIVITY` does anything; every other command is acknowledged with `data: null`.
fn handle_command(
    hub: &Hub,
    sid: &str,
    client_id: &str,
    msg: &Value,
    pid: &mut u64,
    active: &mut bool,
) -> Value {
    let cmd = msg.get("cmd").and_then(Value::as_str).unwrap_or("");
    let nonce = msg.get("nonce").cloned().unwrap_or(Value::Null);
    if cmd != "SET_ACTIVITY" {
        return json!({"cmd":cmd,"data":null,"evt":null,"nonce":nonce});
    }
    let args = msg.get("args").cloned().unwrap_or(Value::Null);
    *pid = args.get("pid").and_then(Value::as_u64).unwrap_or(*pid);
    let act = args.get("activity").cloned().unwrap_or(Value::Null);
    if act.is_object() {
        let norm = normalize_activity(act, client_id);
        hub.publish(sid, bridge_message(norm.clone(), *pid, sid), false);
        *active = true;
        json!({"cmd":cmd,"data":norm,"evt":null,"nonce":nonce})
    } else {
        hub.publish(sid, bridge_message(Value::Null, *pid, sid), true);
        *active = false;
        json!({"cmd":cmd,"data":null,"evt":null,"nonce":nonce})
    }
}

#[allow(clippy::while_let_loop)]
fn handle_ipc(mut s: UnixStream, hub: &Hub, sid: String) {
    let mut client_id = String::new();
    let mut pid = 0u64;
    let mut active = false;
    loop {
        let Ok((op, msg)) = read_frame(&mut s) else {
            break;
        };
        match op {
            OP_HANDSHAKE => {
                client_id = msg
                    .get("client_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if client_id.is_empty() || msg.get("v") != Some(&json!(1)) {
                    let _ = s.write_all(&frame(
                        OP_CLOSE,
                        &json!({"code": 4000, "message": "Invalid handshake"}),
                    ));
                    break;
                }
                let ready = ready_event();
                if s.write_all(&frame(OP_FRAME, &ready)).is_err() {
                    break;
                }
            }
            OP_FRAME if !client_id.is_empty() => {
                let reply = handle_command(hub, &sid, &client_id, &msg, &mut pid, &mut active);
                if s.write_all(&frame(OP_FRAME, &reply)).is_err() {
                    break;
                }
            }
            OP_PING => {
                if s.write_all(&frame(OP_PONG, &msg)).is_err() {
                    break;
                }
            }
            OP_CLOSE => break,
            OP_PONG => {}
            _ => break,
        }
    }
    if active {
        hub.publish(&sid, bridge_message(Value::Null, pid, &sid), true);
    }
}

/// Bind the first free `discord-ipc-N` (N = 0..9) in `dir` and serve it on a thread.
/// A leftover socket file from a crashed run is replaced; a live one is skipped.
pub fn serve_ipc(dir: &Path, hub: Arc<Hub>) -> io::Result<PathBuf> {
    static CONN: AtomicU64 = AtomicU64::new(0);
    for n in 0..10 {
        let path = dir.join(format!("discord-ipc-{n}"));
        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                continue;
            }
            let _ = std::fs::remove_file(&path);
        }
        let Ok(listener) = UnixListener::bind(&path) else {
            continue;
        };
        let ret = path.clone();
        thread::spawn(move || {
            for conn in listener.incoming().flatten() {
                let hub = hub.clone();
                let sid = format!("{n}-{}", CONN.fetch_add(1, Ordering::Relaxed));
                thread::spawn(move || handle_ipc(conn, &hub, sid));
            }
        });
        return Ok(ret);
    }
    Err(io::Error::new(
        io::ErrorKind::AddrInUse,
        "no free discord-ipc-N socket",
    ))
}

fn origin_allowed(origin: Option<&str>) -> bool {
    match origin {
        None => true, // non-browser clients send no Origin
        Some(o) => crate::nav::is_discord_url(o),
    }
}

fn handle_ws(stream: TcpStream, hub: &Hub) {
    use tungstenite::handshake::server::{ErrorResponse, Request, Response};
    #[allow(clippy::result_large_err)]
    let callback = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        let origin = req.headers().get("origin").and_then(|v| v.to_str().ok());
        if origin_allowed(origin) {
            Ok(resp)
        } else {
            Err(ErrorResponse::new(Some("forbidden origin".into())))
        }
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let Ok(mut ws) = tungstenite::accept_hdr(stream, callback) else {
        return;
    };
    let _ = ws
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(100)));
    let rx = hub.subscribe();
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(m) => {
                if ws.send(tungstenite::Message::text(m)).is_err() {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        match ws.read() {
            Ok(tungstenite::Message::Close(_)) => return,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return,
        }
    }
}

/// Serve the bridge on 127.0.0.1:`port` (loopback only, Discord origins only).
pub fn serve_bridge(port: u16, hub: Arc<Hub>) -> io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let bound = listener.local_addr()?.port();
    thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let hub = hub.clone();
            thread::spawn(move || handle_ws(conn, &hub));
        }
    });
    Ok(bound)
}

/// Close codes of Discord's RPC protocol.
const CLOSE_INVALID_CLIENT: u16 = 4000;
const CLOSE_INVALID_ORIGIN: u16 = 4001;
const CLOSE_INVALID_VERSION: u16 = 4004;
const CLOSE_INVALID_ENCODING: u16 = 4005;

fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
}

fn browser_origin_allowed(origin: Option<&str>, extra: &[String]) -> bool {
    match origin {
        None => true, // non-browser clients send no Origin
        Some(o) => crate::nav::is_discord_url(o) || extra.iter().any(|e| e == o),
    }
}

/// Why a browser RPC connection must be closed, if it must.
fn browser_reject(
    query: &str,
    origin: Option<&str>,
    extra: &[String],
) -> Option<(u16, &'static str)> {
    if !browser_origin_allowed(origin, extra) {
        return Some((CLOSE_INVALID_ORIGIN, "Invalid Origin"));
    }
    if query_param(query, "v") != Some("1") {
        return Some((CLOSE_INVALID_VERSION, "Invalid Version"));
    }
    if query_param(query, "encoding").is_some_and(|e| e != "json") {
        return Some((CLOSE_INVALID_ENCODING, "Invalid Encoding"));
    }
    match query_param(query, "client_id") {
        Some(id) if (1..=32).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit()) => None,
        _ => Some((CLOSE_INVALID_CLIENT, "Invalid Client ID")),
    }
}

fn handle_browser(stream: TcpStream, hub: &Hub, extra: &[String], sid: String) {
    use tungstenite::handshake::server::{ErrorResponse, Request, Response};
    use tungstenite::protocol::frame::coding::CloseCode;
    use tungstenite::protocol::{CloseFrame, WebSocketConfig};

    let mut query = String::new();
    let mut origin: Option<String> = None;
    #[allow(clippy::result_large_err)]
    let callback = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        query = req.uri().query().unwrap_or("").to_string();
        origin = req
            .headers()
            .get("origin")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        Ok(resp)
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME));
    let Ok(mut ws) = tungstenite::accept_hdr_with_config(stream, callback, Some(config)) else {
        return;
    };
    // Past the handshake an idle page is fine; a dead connection ends with a read error.
    let _ = ws.get_mut().set_read_timeout(None);

    // Close codes can only be seen by the page after the upgrade, so reject here.
    if let Some((code, reason)) = browser_reject(&query, origin.as_deref(), extra) {
        let _ = ws.close(Some(CloseFrame {
            code: CloseCode::from(code),
            reason: reason.into(),
        }));
        let _ = ws.flush();
        return;
    }
    let client_id = query_param(&query, "client_id").unwrap_or("").to_string();
    if ws
        .send(tungstenite::Message::text(ready_event().to_string()))
        .is_err()
    {
        return;
    }
    let (mut pid, mut active) = (0u64, false);
    while let Ok(m) = ws.read() {
        match m {
            tungstenite::Message::Text(text) => {
                let Ok(v) = serde_json::from_str::<Value>(text.as_str()) else {
                    continue;
                };
                let reply = handle_command(hub, &sid, &client_id, &v, &mut pid, &mut active);
                if ws
                    .send(tungstenite::Message::text(reply.to_string()))
                    .is_err()
                {
                    break;
                }
            }
            tungstenite::Message::Close(_) => break,
            _ => {}
        }
    }
    if active {
        hub.publish(&sid, bridge_message(Value::Null, pid, &sid), true);
    }
}

/// Serve Discord's browser RPC transport (`ws://127.0.0.1:<port>/?v=1&client_id=..`) on the
/// first free port of `ports`. Loopback only. Connections without an Origin (native tools)
/// are accepted; browser pages only from Discord's own origins or the exact origins in
/// `extra_origins`, so a random website cannot set your presence.
pub fn serve_browser(
    ports: std::ops::RangeInclusive<u16>,
    hub: Arc<Hub>,
    extra_origins: Vec<String>,
) -> io::Result<u16> {
    static CONN: AtomicU64 = AtomicU64::new(0);
    static OPEN: AtomicUsize = AtomicUsize::new(0);
    let mut last = io::Error::new(io::ErrorKind::AddrInUse, "no free RPC port");
    for port in ports {
        let listener = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => l,
            Err(e) => {
                last = e;
                continue;
            }
        };
        let bound = listener.local_addr()?.port();
        let extra = Arc::new(extra_origins);
        thread::spawn(move || {
            for conn in listener.incoming().flatten() {
                if OPEN.fetch_add(1, Ordering::SeqCst) >= MAX_BROWSER_CONNECTIONS {
                    OPEN.fetch_sub(1, Ordering::SeqCst);
                    continue; // dropped: too many open connections
                }
                let hub = hub.clone();
                let extra = extra.clone();
                let sid = format!("web-{}", CONN.fetch_add(1, Ordering::Relaxed));
                thread::spawn(move || {
                    handle_browser(conn, &hub, &extra, sid);
                    OPEN.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        return Ok(bound);
    }
    Err(last)
}

/// Where the detectable-games list lives.
pub fn detectable_path(data_dir: &Path) -> PathBuf {
    data_dir.join("rpc").join("detectable.json")
}

/// Start the IPC socket, the bridge and (if enabled) game detection. Failures only disable
/// presence, never the app.
pub fn start(settings: &crate::settings::Settings, data_dir: &Path) {
    let hub = Arc::new(Hub::default());
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => match serve_ipc(Path::new(&dir), hub.clone()) {
            Ok(p) => eprintln!("rustcord: rich presence socket {}", p.display()),
            Err(e) => eprintln!("rustcord: rich presence socket unavailable: {e}"),
        },
        None => eprintln!("rustcord: XDG_RUNTIME_DIR not set, rich presence socket disabled"),
    }
    if let Err(e) = serve_bridge(BRIDGE_PORT, hub.clone()) {
        eprintln!("rustcord: rich presence bridge port {BRIDGE_PORT} unavailable: {e}");
    }
    match serve_browser(
        BROWSER_PORTS,
        hub.clone(),
        settings.rpc_browser_origins.clone(),
    ) {
        Ok(p) => eprintln!("rustcord: browser RPC on 127.0.0.1:{p}"),
        Err(e) => eprintln!("rustcord: browser RPC unavailable: {e}"),
    }
    if settings.rpc_detect_games {
        match crate::games::Db::load(&detectable_path(data_dir)) {
            Some(db) if !db.is_empty() => {
                crate::games::spawn_detector(
                    hub,
                    db,
                    PathBuf::from("/proc"),
                    Duration::from_secs(5),
                );
            }
            _ => eprintln!(
                "rustcord: game detection on but no list; run `rustcord rpc update-games`"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rustcord-rpc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn activity_is_normalized() {
        let a = normalize_activity(
            json!({"details":"d","instance":true,"timestamps":{"start":1700000000},
                   "buttons":[{"label":"Go","url":"https://x.y"}]}),
            "42",
        );
        assert_eq!(a["application_id"], "42");
        assert_eq!(a["flags"], 1);
        assert_eq!(a["type"], 0);
        assert_eq!(a["timestamps"]["start"], 1_700_000_000_000u64);
        assert_eq!(a["buttons"], json!(["Go"]));
        assert_eq!(a["metadata"]["button_urls"], json!(["https://x.y"]));
    }

    #[test]
    fn origins() {
        assert!(origin_allowed(None));
        assert!(origin_allowed(Some("https://discord.com")));
        assert!(!origin_allowed(Some("https://evil.example")));
        assert!(!origin_allowed(Some("null")));
    }

    #[test]
    fn game_to_bridge_end_to_end() {
        let dir = tmpdir("e2e");
        let hub = Arc::new(Hub::default());
        let sock = serve_ipc(&dir, hub.clone()).unwrap();
        let port = serve_bridge(0, hub).unwrap();

        // Bridge client (what the Vencord plugin does), connecting first.
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let (mut ws, _) = tungstenite::client(format!("ws://127.0.0.1:{port}"), tcp).unwrap();

        // Rogue browser origin is refused.
        let tcp2 = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let req = tungstenite::client::IntoClientRequest::into_client_request(format!(
            "ws://127.0.0.1:{port}"
        ))
        .map(|mut r| {
            r.headers_mut()
                .insert("origin", "https://evil.example".parse().unwrap());
            r
        })
        .unwrap();
        assert!(tungstenite::client(req, tcp2).is_err());

        // A "game" over the IPC socket.
        let mut game = UnixStream::connect(&sock).unwrap();
        game.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        game.write_all(&frame(OP_HANDSHAKE, &json!({"v":1,"client_id":"1234"})))
            .unwrap();
        let (op, ready) = read_frame(&mut game).unwrap();
        assert_eq!(op, OP_FRAME);
        assert_eq!(ready["evt"], "READY");

        game.write_all(&frame(
            OP_FRAME,
            &json!({"cmd":"SET_ACTIVITY","nonce":"n1","args":{"pid":777,
                "activity":{"details":"In menu","state":"Idle"}}}),
        ))
        .unwrap();
        let (_, reply) = read_frame(&mut game).unwrap();
        assert_eq!(reply["nonce"], "n1");
        assert_eq!(reply["data"]["application_id"], "1234");

        let msg = ws.read().unwrap().into_text().unwrap();
        let v: Value = serde_json::from_str(msg.as_str()).unwrap();
        assert_eq!(v["pid"], 777);
        assert_eq!(v["activity"]["details"], "In menu");
        assert_eq!(v["activity"]["application_id"], "1234");

        // Game exits: the activity is cleared.
        drop(game);
        let msg = ws.read().unwrap().into_text().unwrap();
        let v: Value = serde_json::from_str(msg.as_str()).unwrap();
        assert!(v["activity"].is_null());
    }

    #[test]
    fn late_bridge_client_sees_running_game() {
        let dir = tmpdir("late");
        let hub = Arc::new(Hub::default());
        let sock = serve_ipc(&dir, hub.clone()).unwrap();
        let port = serve_bridge(0, hub).unwrap();
        let mut game = UnixStream::connect(&sock).unwrap();
        game.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        game.write_all(&frame(OP_HANDSHAKE, &json!({"v":1,"client_id":"9"})))
            .unwrap();
        read_frame(&mut game).unwrap();
        game.write_all(&frame(
            OP_FRAME,
            &json!({"cmd":"SET_ACTIVITY","nonce":"x","args":{"pid":1,"activity":{"state":"s"}}}),
        ))
        .unwrap();
        read_frame(&mut game).unwrap();
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let (mut ws, _) = tungstenite::client(format!("ws://127.0.0.1:{port}"), tcp).unwrap();
        let v: Value =
            serde_json::from_str(ws.read().unwrap().into_text().unwrap().as_str()).unwrap();
        assert_eq!(v["activity"]["state"], "s");
    }

    #[test]
    fn bad_handshake_is_closed_and_stale_socket_replaced() {
        let dir = tmpdir("bad");
        std::fs::write(dir.join("discord-ipc-0"), b"stale").unwrap();
        let hub = Arc::new(Hub::default());
        let sock = serve_ipc(&dir, hub).unwrap();
        assert_eq!(sock.file_name().unwrap(), "discord-ipc-0");
        let mut c = UnixStream::connect(&sock).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        c.write_all(&frame(OP_HANDSHAKE, &json!({"v":2}))).unwrap();
        let (op, _) = read_frame(&mut c).unwrap();
        assert_eq!(op, OP_CLOSE);
    }

    fn ws_open(
        port: u16,
        query: &str,
        origin: Option<&str>,
    ) -> Result<tungstenite::WebSocket<TcpStream>, tungstenite::Error> {
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut req = tungstenite::client::IntoClientRequest::into_client_request(format!(
            "ws://127.0.0.1:{port}/?{query}"
        ))
        .unwrap();
        if let Some(o) = origin {
            req.headers_mut().insert("origin", o.parse().unwrap());
        }
        tungstenite::client(req, tcp)
            .map(|(ws, _)| ws)
            .map_err(|e| match e {
                tungstenite::HandshakeError::Failure(e) => e,
                tungstenite::HandshakeError::Interrupted(_) => panic!("interrupted"),
            })
    }

    fn close_code(ws: &mut tungstenite::WebSocket<TcpStream>) -> u16 {
        match ws.read().unwrap() {
            tungstenite::Message::Close(Some(f)) => u16::from(f.code),
            other => panic!("expected close, got {other:?}"),
        }
    }

    #[test]
    fn browser_rejections() {
        let none: Vec<String> = vec![];
        let q = "v=1&client_id=123&encoding=json";
        assert_eq!(browser_reject(q, None, &none), None);
        assert_eq!(browser_reject(q, Some("https://discord.com"), &none), None);
        assert_eq!(browser_reject("v=1&client_id=123", None, &none), None);
        assert_eq!(
            browser_reject(q, Some("https://evil.example"), &none)
                .unwrap()
                .0,
            4001
        );
        assert_eq!(browser_reject(q, Some("null"), &none).unwrap().0, 4001);
        assert_eq!(
            browser_reject("v=2&client_id=123", None, &none).unwrap().0,
            4004
        );
        assert_eq!(
            browser_reject("client_id=123", None, &none).unwrap().0,
            4004
        );
        assert_eq!(
            browser_reject("v=1&client_id=1&encoding=etf", None, &none)
                .unwrap()
                .0,
            4005
        );
        assert_eq!(browser_reject("v=1", None, &none).unwrap().0, 4000);
        assert_eq!(
            browser_reject("v=1&client_id=abc", None, &none).unwrap().0,
            4000
        );
        assert_eq!(
            browser_reject("v=1&client_id=", None, &none).unwrap().0,
            4000
        );
        let extra = vec!["https://game.example".to_string()];
        assert_eq!(
            browser_reject(q, Some("https://game.example"), &extra),
            None
        );
        assert!(browser_reject(q, Some("https://game.example.evil"), &extra).is_some());
    }

    #[test]
    fn browser_transport_end_to_end() {
        let hub = Arc::new(Hub::default());
        let bridge = serve_bridge(0, hub.clone()).unwrap();
        let port = serve_browser(0..=0, hub, vec!["https://game.example".into()]).unwrap();
        assert!(port != 0);

        let tcp = TcpStream::connect(("127.0.0.1", bridge)).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let (mut bws, _) = tungstenite::client(format!("ws://127.0.0.1:{bridge}"), tcp).unwrap();

        // A web page on an allowed origin sets an activity.
        let mut ws = ws_open(
            port,
            "v=1&client_id=555&encoding=json",
            Some("https://game.example"),
        )
        .unwrap();
        let ready: Value =
            serde_json::from_str(ws.read().unwrap().into_text().unwrap().as_str()).unwrap();
        assert_eq!(ready["evt"], "READY");
        ws.send(tungstenite::Message::text(
            json!({"cmd":"SET_ACTIVITY","nonce":"a","args":{"pid":9,"activity":{"details":"Level 3"}}}).to_string(),
        ))
        .unwrap();
        let reply: Value =
            serde_json::from_str(ws.read().unwrap().into_text().unwrap().as_str()).unwrap();
        assert_eq!(reply["nonce"], "a");
        assert_eq!(reply["data"]["application_id"], "555");
        let v: Value =
            serde_json::from_str(bws.read().unwrap().into_text().unwrap().as_str()).unwrap();
        assert_eq!(v["activity"]["details"], "Level 3");
        assert_eq!(v["pid"], 9);

        // Unknown commands are acknowledged, garbage is ignored.
        ws.send(tungstenite::Message::text("not json")).unwrap();
        ws.send(tungstenite::Message::text(
            json!({"cmd":"PING","nonce":"z"}).to_string(),
        ))
        .unwrap();
        let reply: Value =
            serde_json::from_str(ws.read().unwrap().into_text().unwrap().as_str()).unwrap();
        assert_eq!(reply["nonce"], "z");

        // Closing the page clears the activity.
        drop(ws);
        let v: Value =
            serde_json::from_str(bws.read().unwrap().into_text().unwrap().as_str()).unwrap();
        assert!(v["activity"].is_null());

        // Bad origin, version and client id are closed with Discord's codes and set nothing.
        let mut bad = ws_open(port, "v=1&client_id=555", Some("https://evil.example")).unwrap();
        assert_eq!(close_code(&mut bad), 4001);
        let mut bad = ws_open(port, "v=9&client_id=555", None).unwrap();
        assert_eq!(close_code(&mut bad), 4004);
        let mut bad = ws_open(port, "v=1&client_id=x", Some("https://discord.com")).unwrap();
        assert_eq!(close_code(&mut bad), 4000);
    }
}
