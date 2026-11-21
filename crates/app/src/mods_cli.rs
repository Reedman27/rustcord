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

const CONFIG_USAGE: &str = "usage:
  rustcord config show                 print settings.json
  rustcord config set <key> <value>    keys: branch, start_minimized, disable_background_throttling,
                                       sandbox, tray, close_to_tray, rpc, mod_csp_bypass,
                                       accent_color, color_scheme
  rustcord themes list                 themes found in <config>/themes (* = enabled)
  rustcord themes enable|disable <name>
  rustcord autostart enable|disable    XDG autostart entry for this executable
  rustcord rpc update-games            download Discord's list of known games (for detection)
  rustcord config set rpc_detect_games on   then show running games as playing
  rustcord notify show                 selective notifications (Qt build): which servers may notify
  rustcord notify mode off|allow|block
  rustcord notify add|remove <server name>
  rustcord notify dms on|off           whether DMs pass when mode is allow or block
  rustcord notify badge on|off         unread count on tray icon and launcher
  rustcord notify backend auto|daemon|portal";

/// Handle the non-GUI subcommands (`mods`, `config`, `themes`, `autostart`).
/// Returns `None` when `argv` is a normal launch.
pub fn dispatch(argv: &[String]) -> Option<ExitCode> {
    let rest = &argv[1.min(argv.len())..];
    match argv.first().map(String::as_str)? {
        "mods" => Some(run(rest)),
        "config" | "themes" | "autostart" | "notify" | "rpc" => {
            let result = settings_command(argv[0].as_str(), rest);
            Some(match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            })
        }
        _ => None,
    }
}

fn settings_command(cmd: &str, args: &[String]) -> Result<(), String> {
    use rustcord_core::edit;
    let dir = paths::config_dir();
    let path = dir.join("settings.json");
    let mut s: Settings = settings::load_or_default(&path);
    let save = |s: &Settings| {
        settings::save_atomic(&path, s).map_err(|e| format!("cannot save settings: {e}"))
    };
    let a = |i: usize| args.get(i).map(String::as_str);
    match (cmd, a(0)) {
        ("config", Some("show")) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?
            );
            Ok(())
        }
        ("config", Some("set")) => match (a(1), a(2)) {
            (Some(k), Some(v)) => {
                edit::set_key(&mut s, k, v)?;
                save(&s)?;
                println!("{k} = {v}  (restart Rustcord to apply)");
                Ok(())
            }
            _ => Err(CONFIG_USAGE.into()),
        },
        ("themes", Some("list")) => {
            let found = rustcord_core::theme::discover(&dir.join("themes"));
            if found.is_empty() {
                println!("no themes; put .css files in {}/themes", dir.display());
            }
            for t in found {
                println!("{} {t}", if s.themes.contains(&t) { "*" } else { " " });
            }
            Ok(())
        }
        ("themes", Some(op @ ("enable" | "disable"))) => match a(1) {
            Some(name) => {
                edit::set_theme(&mut s, &dir, name, op == "enable")?;
                save(&s)?;
                println!("theme {name} {op}d  (restart Rustcord to apply)");
                Ok(())
            }
            None => Err(CONFIG_USAGE.into()),
        },
        ("autostart", Some(op @ ("enable" | "disable"))) => {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let home = paths::config_dir();
            // config_dir is <xdg_config_home>/rustcord
            let xdg = home.parent().ok_or("no config directory")?;
            edit::set_autostart(xdg, &exe.to_string_lossy(), op == "enable")?;
            println!("autostart {op}d");
            Ok(())
        }
        ("rpc", Some("update-games")) => {
            let url = "https://discord.com/api/v10/applications/detectable";
            println!("downloading {url}");
            let bytes = download(url)?;
            let db = rustcord_core::games::Db::parse(&bytes)?;
            let path = rustcord_core::rpc::detectable_path(&paths::data_dir());
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
            println!(
                "saved {} known executables; enable with `config set rpc_detect_games on`",
                db.len()
            );
            Ok(())
        }
        ("notify", Some("show")) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&s.notifications).map_err(|e| e.to_string())?
            );
            Ok(())
        }
        ("notify", Some(op)) => {
            use rustcord_core::notify::Mode;
            let n = &mut s.notifications;
            let rest = args[1..].join(" ");
            match op {
                "mode" => {
                    n.mode = match rest.as_str() {
                        "off" => Mode::Off,
                        "allow" => Mode::Allow,
                        "block" => Mode::Block,
                        _ => return Err("mode must be off, allow or block".into()),
                    }
                }
                "add" if !rest.is_empty() => n.add_server(&rest),
                "remove" if !rest.is_empty() => n.remove_server(&rest),
                "dms" => n.dms = edit::parse_bool(&rest)?,
                "badge" => n.badge = edit::parse_bool(&rest)?,
                "backend" => match rest.as_str() {
                    "auto" | "daemon" | "portal" => n.backend = rest.clone(),
                    _ => return Err("backend must be auto, daemon or portal".into()),
                },
                _ => return Err(CONFIG_USAGE.into()),
            }
            save(&s)?;
            println!("ok  (restart Rustcord to apply)");
            Ok(())
        }
        _ => Err(CONFIG_USAGE.into()),
    }
}
