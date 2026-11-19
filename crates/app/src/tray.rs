//! System tray through the StatusNotifierItem protocol (ksni).
//!
//! Chromium (and Vesktop) fail if the tray is created before a StatusNotifierHost exists,
//! which happens when the app autostarts before the panel. We retry for up to 30 seconds,
//! and give up quietly if no host ever appears (for example a plain Window Maker session).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ksni::blocking::TrayMethods;
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem};
use rustcord_core::icon;

use crate::handler;

/// True once the tray icon is registered with a host.
pub static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Set when the user chose Quit, so close-to-tray does not intercept the close.
pub static QUITTING: AtomicBool = AtomicBool::new(false);

const WAIT: Duration = Duration::from_secs(30);
const RETRY: Duration = Duration::from_secs(1);
const ICON_SIZE: i32 = 32;
const ACCENT: [u8; 3] = [0x58, 0x65, 0xf2];

struct RustcordTray;

impl ksni::Tray for RustcordTray {
    fn id(&self) -> String {
        "rustcord".into()
    }

    fn title(&self) -> String {
        "Rustcord".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![Icon {
            width: ICON_SIZE,
            height: ICON_SIZE,
            data: icon::circle_argb(ICON_SIZE as u32, ACCENT),
        }]
    }

    // Left click.
    fn activate(&mut self, _x: i32, _y: i32) {
        handler::request_show();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Show Rustcord".into(),
                activate: Box::new(|_| handler::request_show()),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|_| {
                    QUITTING.store(true, Ordering::SeqCst);
                    handler::request_quit();
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Start the tray on a background thread. Returns immediately.
pub fn start() {
    let spawned = std::thread::Builder::new()
        .name("rustcord-tray".into())
        .spawn(|| {
            let deadline = Instant::now() + WAIT;
            loop {
                match RustcordTray.spawn() {
                    Ok(handle) => {
                        ACTIVE.store(true, Ordering::SeqCst);
                        // The tray lives as long as the handle; keep it for the process lifetime.
                        let _keep = handle;
                        loop {
                            std::thread::park();
                        }
                    }
                    Err(_) if Instant::now() < deadline => std::thread::sleep(RETRY),
                    Err(e) => {
                        eprintln!("rustcord: no tray host, running without a tray icon ({e})");
                        return;
                    }
                }
            }
        });
    if let Err(e) = spawned {
        eprintln!("rustcord: could not start tray thread: {e}");
    }
}
