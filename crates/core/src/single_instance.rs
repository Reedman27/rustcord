//! Single instance via a Unix socket (same idea as Vesktop's `vesktop.sock`).
//!
//! The first process binds the socket and listens. Later processes connect, send one
//! line (`focus` or `open discord://...`) and exit. A stale socket left by a crash is
//! removed, but only if it is a socket owned by the current user.

use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Bring the existing window to the front.
    Focus,
    /// Navigate to a `discord://` deep link and bring the window to the front.
    Open(String),
}

impl Message {
    fn to_line(&self) -> String {
        match self {
            Message::Focus => "focus\n".to_string(),
            // newlines in a URI would let a caller smuggle a second command
            Message::Open(u) => format!("open {}\n", u.replace(['\n', '\r'], "")),
        }
    }

    fn parse(line: &str) -> Option<Message> {
        let line = line.trim_end_matches(['\n', '\r']);
        if line == "focus" {
            return Some(Message::Focus);
        }
        let uri = line.strip_prefix("open ")?;
        // Only deep links are accepted, never arbitrary URLs.
        uri.starts_with("discord://")
            .then(|| Message::Open(uri.to_string()))
    }
}

pub enum Outcome {
    /// This process is the primary instance. Keep the guard alive for the process lifetime.
    Primary(Guard),
    /// An existing instance received our message; the caller should exit.
    Forwarded,
}

/// Removes the socket file on drop.
pub struct Guard {
    path: PathBuf,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn current_uid() -> u32 {
    fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(0)
}

fn try_forward(path: &Path, msg: &Message) -> io::Result<()> {
    let mut s = UnixStream::connect(path)?;
    s.set_write_timeout(Some(Duration::from_secs(2)))?;
    s.write_all(msg.to_line().as_bytes())
}

/// Become the primary instance or forward `msg` to the running one.
/// `on_message` runs on a background thread for each message a later instance sends.
pub fn acquire<F>(path: &Path, msg: &Message, on_message: F) -> io::Result<Outcome>
where
    F: Fn(Message) + Send + 'static,
{
    let mut on_message = Some(on_message);
    for _ in 0..5 {
        if try_forward(path, msg).is_ok() {
            return Ok(Outcome::Forwarded);
        }

        // Nobody answered. Remove a stale socket, but only one that is ours.
        if let Ok(meta) = fs::symlink_metadata(path) {
            if !meta.file_type().is_socket() || meta.uid() != current_uid() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} exists and is not our socket", path.display()),
                ));
            }
            let _ = fs::remove_file(path);
        }

        match UnixListener::bind(path) {
            Ok(listener) => {
                fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
                let cb = on_message.take().expect("callback used once");
                thread::Builder::new()
                    .name("rustcord-ipc".into())
                    .spawn(move || serve(listener, cb))?;
                return Ok(Outcome::Primary(Guard {
                    path: path.to_path_buf(),
                }));
            }
            // Another instance won the race between our connect and bind; retry as a client.
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
                thread::sleep(Duration::from_millis(50))
            }
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "could not acquire single-instance socket",
    ))
}

fn serve<F: Fn(Message)>(listener: UnixListener, on_message: F) {
    for conn in listener.incoming() {
        let Ok(conn) = conn else { continue };
        let _ = conn.set_read_timeout(Some(Duration::from_secs(2)));
        // Cap the line length so a misbehaving local client cannot make us buffer forever.
        let mut line = String::new();
        let mut reader = BufReader::new(conn.take(4096));
        if reader.read_line(&mut line).is_ok() {
            if let Some(m) = Message::parse(&line) {
                on_message(m);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn sock(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rustcord-si-{name}-{}.sock", std::process::id()))
    }

    #[test]
    fn message_parsing() {
        assert_eq!(Message::parse("focus\n"), Some(Message::Focus));
        assert_eq!(
            Message::parse("open discord://-/channels/1\n"),
            Some(Message::Open("discord://-/channels/1".into()))
        );
        assert_eq!(Message::parse("open https://evil.example/\n"), None);
        assert_eq!(Message::parse("open file:///etc/passwd\n"), None);
        assert_eq!(Message::parse("quit\n"), None);
        assert_eq!(Message::parse(""), None);
        assert_eq!(
            Message::Open("discord://a\nfocus".into()).to_line(),
            "open discord://afocus\n"
        );
    }

    #[test]
    fn second_instance_forwards() {
        let p = sock("fwd");
        let _ = fs::remove_file(&p);
        let (tx, rx) = mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        let primary = acquire(&p, &Message::Focus, move |m| {
            let _ = tx.lock().unwrap().send(m);
        })
        .unwrap();
        let guard = match primary {
            Outcome::Primary(g) => g,
            Outcome::Forwarded => panic!("first instance must be primary"),
        };

        let second = acquire(&p, &Message::Open("discord://-/channels/9".into()), |_| {}).unwrap();
        assert!(matches!(second, Outcome::Forwarded));
        let got = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(got, Message::Open("discord://-/channels/9".into()));

        let third = acquire(&p, &Message::Focus, |_| {}).unwrap();
        assert!(matches!(third, Outcome::Forwarded));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(3)).unwrap(),
            Message::Focus
        );

        drop(guard);
        assert!(!p.exists(), "socket removed on drop");
    }

    #[test]
    fn stale_socket_is_replaced() {
        let p = sock("stale");
        let _ = fs::remove_file(&p);
        // Leave a dead socket file behind, like a crash would.
        drop(UnixListener::bind(&p).unwrap());
        assert!(p.exists());
        let out = acquire(&p, &Message::Focus, |_| {}).unwrap();
        assert!(matches!(out, Outcome::Primary(_)));
    }

    #[test]
    fn refuses_to_replace_a_regular_file() {
        let p = sock("regular");
        fs::write(&p, b"not a socket").unwrap();
        let err = acquire(&p, &Message::Focus, |_| {})
            .err()
            .expect("must refuse");
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&p).unwrap(), b"not a socket");
        let _ = fs::remove_file(&p);
    }
}
