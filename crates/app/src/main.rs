//! Rustcord entry point.
//!
//! One binary plays both roles: the browser process (this file's main path) and CEF's
//! child processes (renderer, GPU, utility), which re-execute this same executable.

mod browser_process;
mod config;
mod external;
mod handler;
mod icon;
mod mods_cli;
mod renderer;
mod tray;
mod window;

use std::process::ExitCode;

use cef::*;
use rustcord_core::settings::{self as rs, WindowState};
use rustcord_core::single_instance::{self, Message, Outcome};
use rustcord_core::{nav, paths};

use crate::browser_process::RustcordApp;

fn main() -> ExitCode {
    // `rustcord mods ...` is a plain CLI and never starts CEF.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().map(String::as_str) == Some("mods") {
        return mods_cli::run(&argv[1..]);
    }

    // Initialize the CEF API version.
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);

    let args = cef::args::Args::new();
    let Some(cmd_line) = args.as_cmd_line() else {
        eprintln!("rustcord: cannot parse command line");
        return ExitCode::FAILURE;
    };

    let type_switch = CefString::from("type");
    let is_browser_process = cmd_line.has_switch(Some(&type_switch)) != 1;

    // Child processes run to completion inside execute_process.
    let mut child_app = renderer::RustcordChildApp::new();
    let ret = execute_process(
        Some(args.as_main_args()),
        Some(&mut child_app),
        std::ptr::null_mut(),
    );
    if !is_browser_process {
        return if ret >= 0 {
            ExitCode::from(ret.min(255) as u8)
        } else {
            ExitCode::FAILURE
        };
    }
    if ret != -1 {
        eprintln!("rustcord: unexpected execute_process result {ret}");
        return ExitCode::FAILURE;
    }

    // --- browser process only from here ---

    let config_dir = paths::config_dir();
    let settings: rs::Settings = rs::load_or_default(&config_dir.join("settings.json"));
    let state_path = config_dir.join("state.json");
    let state = rs::load_or_default::<WindowState>(&state_path).sanitized();

    // Deep link passed by the desktop environment (`rustcord discord://-/channels/...`).
    let deep_link = std::env::args()
        .skip(1)
        .find(|a| a.starts_with("discord://"));
    let message = match &deep_link {
        Some(uri) => Message::Open(uri.clone()),
        None => Message::Focus,
    };

    // Second launch: hand the request to the running instance and exit.
    let _instance_guard =
        match single_instance::acquire(&paths::runtime_socket(), &message, handler::handle_ipc) {
            Ok(Outcome::Forwarded) => return ExitCode::SUCCESS,
            Ok(Outcome::Primary(guard)) => Some(guard),
            Err(e) => {
                eprintln!("rustcord: single-instance disabled: {e}");
                None
            }
        };

    let profile_dir = paths::profile_dir();
    if let Err(e) = std::fs::create_dir_all(&profile_dir) {
        eprintln!("rustcord: cannot create {}: {e}", profile_dir.display());
        return ExitCode::FAILURE;
    }

    config::init(
        config::Config {
            start_url: nav::deep_link_url(settings.branch, deep_link.as_deref()),
            settings: settings.clone(),
            state_path,
            config_dir,
        },
        state,
    );

    let profile = CefString::from(profile_dir.to_string_lossy().as_ref());
    let cef_settings = Settings {
        // Inside Flatpak the bubblewrap sandbox replaces Chromium's own (which cannot start there).
        no_sandbox: (!settings.sandbox || std::path::Path::new("/.flatpak-info").exists()) as _,
        root_cache_path: profile.clone(),
        cache_path: profile,
        persist_session_cookies: 1,
        log_file: CefString::from(paths::log_file().to_string_lossy().as_ref()),
        log_severity: LogSeverity::WARNING,
        ..Default::default()
    };

    let mut app = RustcordApp::new(settings.chromium_switches());
    let ok = initialize(
        Some(args.as_main_args()),
        Some(&cef_settings),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    if ok != 1 {
        eprintln!(
            "rustcord: CEF failed to initialize (is another instance using the same profile?)"
        );
        return ExitCode::FAILURE;
    }

    run_message_loop();
    shutdown();
    ExitCode::SUCCESS
}
