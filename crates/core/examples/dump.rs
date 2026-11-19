//! Dev helper for the verification scripts in `scripts/`.
//!   cargo run -q -p rustcord-core --example dump -- png <size>     > icon.png
//!   cargo run -q -p rustcord-core --example dump -- theme-script   < theme.css   > script.js
//!   cargo run -q -p rustcord-core --example dump -- mod-script     < Vencord.user.js > wrapped.js
use std::io::{Read, Write};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = std::io::stdout().lock();
    match args.first().map(String::as_str) {
        Some("png") => {
            let size: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(64);
            out.write_all(&rustcord_core::icon::circle_png(size, [0x58, 0x65, 0xf2]))
                .unwrap();
        }
        Some("theme-script") => {
            let mut css = String::new();
            std::io::stdin().read_to_string(&mut css).unwrap();
            out.write_all(rustcord_core::theme::injection_script(&css).as_bytes())
                .unwrap();
        }
        Some("mod-script") => {
            let mut src = String::new();
            std::io::stdin().read_to_string(&mut src).unwrap();
            out.write_all(rustcord_core::mods::wrap_script(&src).as_bytes())
                .unwrap();
        }
        _ => {
            eprintln!(
                "usage: dump png <size> | dump theme-script < css | dump mod-script < bundle"
            );
            std::process::exit(2);
        }
    }
}
