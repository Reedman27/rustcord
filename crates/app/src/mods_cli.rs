//! `rustcord mods install|update|status|enable|disable ...` (no CEF involved).

use std::io::Read;
use std::process::ExitCode;

use rustcord_core::mods::{self, ModClient};
use rustcord_core::paths;
use rustcord_core::settings::{self, Settings};

const USAGE: &str = "usage:
  rustcord mods status
  rustcord mods install <vencord|equicord>   download and record the official userscript build
  rustcord mods update  <vencord|equicord>   same as install (replaces the bundle)
  rustcord mods enable  <vencord|equicord>   load it at startup (off by default)
  rustcord mods disable
  rustcord mods path <file.js|none>          use your own build (userplugins) instead of the installed one";

fn download(url: &str) -> Result<Vec<u8>, String> {
    let resp = ureq::get(url).call().map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    resp.into_body()
        .into_reader()
        .take(mods::MAX_BUNDLE_BYTES as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    Ok(buf)
}

fn client_arg(args: &[String]) -> Option<ModClient> {
    args.get(1).and_then(|s| ModClient::parse(s))
}

pub fn run(args: &[String]) -> ExitCode {
    let data = paths::data_dir();
    let settings_path = paths::config_dir().join("settings.json");
    let mut s: Settings = settings::load_or_default(&settings_path);
    let save = |s: &Settings| {
        settings::save_atomic(&settings_path, s).map_err(|e| format!("cannot save settings: {e}"))
    };
    let result: Result<(), String> = match args.first().map(String::as_str) {
        Some("status") => {
            println!(
                "enabled: {}",
                s.mod_client.map(|c| c.name()).unwrap_or("no (off)")
            );
            if let Some(p) = &s.mod_script {
                println!("custom bundle: {p}");
            }
            for c in [ModClient::Vencord, ModClient::Equicord] {
                match mods::manifest(&data, c) {
                    Some(m) => println!(
                        "{}: installed, {} bytes, sha256 {}",
                        c.name(),
                        m.size,
                        m.sha256
                    ),
                    None => println!("{}: not installed", c.name()),
                }
            }
            Ok(())
        }
        Some("install") | Some("update") => match client_arg(args) {
            Some(c) => {
                println!("downloading {}", c.download_url());
                download(c.download_url())
                    .and_then(|b| mods::install(&data, c, &b, c.download_url()))
                    .map(|m| {
                        println!(
                            "installed {} ({} bytes, sha256 {})",
                            c.name(),
                            m.size,
                            m.sha256
                        )
                    })
            }
            None => Err(USAGE.into()),
        },
        Some("enable") => match client_arg(args) {
            Some(c) => {
                s.mod_client = Some(c);
                save(&s).map(|_| {
                    println!("{} enabled; restart Rustcord", c.name());
                    if mods::manifest(&data, c).is_none() && s.mod_script.is_none() {
                        println!(
                            "note: not installed yet, run `rustcord mods install {}`",
                            c.name()
                        );
                    }
                })
            }
            None => Err(USAGE.into()),
        },
        Some("disable") => {
            s.mod_client = None;
            save(&s).map(|_| println!("mods disabled"))
        }
        Some("path") => match args.get(1) {
            Some(p) if p == "none" => {
                s.mod_script = None;
                save(&s)
            }
            Some(p) => {
                s.mod_script = Some(p.clone());
                save(&s)
            }
            None => Err(USAGE.into()),
        },
        _ => Err(USAGE.into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
