//! Open links in the user's default browser.

use std::process::{Command, Stdio};

use rustcord_core::nav;

/// Hand `url` to `xdg-open`. Only http, https and mailto are allowed.
pub fn open(url: &str) {
    if !nav::external_open_allowed(url) {
        eprintln!("rustcord: refusing to open external URL with disallowed scheme");
        return;
    }
    match Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        // Reap the child so it does not linger as a zombie.
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => eprintln!("rustcord: could not run xdg-open: {e}"),
    }
}
