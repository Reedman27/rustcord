//! Rustcord on Qt WebEngine: the frontend for systems where Chromium Embedded Framework
//! has no build (musl, e.g. Chimera Linux). Same settings, themes, mods and Rich Presence
//! as the CEF frontend; only the window and web engine differ.

use std::ffi::{c_char, c_int, c_ulong, c_void, CStr, CString};
use std::process::ExitCode;
use std::sync::OnceLock;

use rustcord_core::settings::{self as rs, Settings, WindowState};
use rustcord_core::single_instance::{self, Message, Outcome};
use rustcord_core::{icon, mods, nav, paths, theme};

mod cdp;
#[path = "../../app/src/mods_cli.rs"]
mod mods_cli;
mod notify;

#[repr(C)]
struct RcConfig {
    start_url: *const c_char,
    profile_dir: *const c_char,
    chromium_flags: *const c_char,
    mod_script: *const c_char,
    theme_script: *const c_char,
    icon_png: *const u8,
    icon_len: c_ulong,
    x: c_int,
    y: c_int,
    w: c_int,
    h: c_int,
    has_pos: c_int,
    maximized: c_int,
    start_minimized: c_int,
    tray: c_int,
    close_to_tray: c_int,
    sandbox: c_int,
}

extern "C" {
    fn rc_qt_run(argc: c_int, argv: *mut *mut c_char, cfg: *const RcConfig) -> c_int;
    fn rc_qt_show();
    fn rc_qt_open(url: *const c_char);
}

static STATE_PATH: OnceLock<std::path::PathBuf> = OnceLock::new();
static BRANCH: OnceLock<nav::Branch> = OnceLock::new();

fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: the shim passes NUL-terminated strings.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

#[no_mangle]
pub extern "C" fn rc_nav_allowed(url: *const c_char) -> c_int {
    nav::in_app_navigation_allowed(&cstr(url)) as c_int
}

#[no_mangle]
pub extern "C" fn rc_open_external(url: *const c_char) {
    let url = cstr(url);
    if nav::external_open_allowed(&url) {
        let _ = std::process::Command::new("xdg-open")
            .arg(&url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

#[no_mangle]
pub extern "C" fn rc_save_window(x: c_int, y: c_int, w: c_int, h: c_int, maximized: c_int) {
    let Some(path) = STATE_PATH.get() else { return };
    let state = WindowState {
        x: Some(x),
        y: Some(y),
        width: w,
        height: h,
        maximized: maximized != 0,
    };
    if let Err(e) = rs::save_atomic(path, &state) {
        eprintln!("rustcord: could not save window state: {e}");
    }
}

fn on_ipc(msg: Message) {
    match msg {
        Message::Focus => unsafe { rc_qt_show() },
        Message::Open(uri) => {
            let branch = BRANCH.get().copied().unwrap_or(nav::Branch::Stable);
            if let Ok(url) = CString::new(nav::deep_link_url(branch, Some(&uri))) {
                unsafe { rc_qt_open(url.as_ptr()) }
            }
        }
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = mods_cli::dispatch(&argv) {
        return code;
    }

    let config_dir = paths::config_dir();
    let settings: Settings = rs::load_or_default(&config_dir.join("settings.json"));
    let state_path = config_dir.join("state.json");
    let state = rs::load_or_default::<WindowState>(&state_path).sanitized();
    let _ = STATE_PATH.set(state_path);
    let _ = BRANCH.set(settings.branch);

    let deep_link = argv.iter().find(|a| a.starts_with("discord://")).cloned();
    let message = match &deep_link {
        Some(uri) => Message::Open(uri.clone()),
        None => Message::Focus,
    };
    let _guard = match single_instance::acquire(&paths::runtime_socket(), &message, on_ipc) {
        Ok(Outcome::Forwarded) => return ExitCode::SUCCESS,
        Ok(Outcome::Primary(g)) => Some(g),
        Err(e) => {
            eprintln!("rustcord: single-instance disabled: {e}");
            None
        }
    };

    if settings.rpc {
        rustcord_core::rpc::start(&settings, &paths::data_dir());
    }
    notify::init(settings.notifications.clone());

    let mod_script = settings
        .mod_client
        .and_then(|c| {
            let custom = settings.mod_script.as_deref().map(std::path::Path::new);
            match mods::load_script(&paths::data_dir(), c, custom) {
                Ok(s) => Some(mods::wrap_script(&s)),
                Err(e) => {
                    eprintln!("rustcord: {} not loaded: {e}", c.name());
                    None
                }
            }
        })
        .unwrap_or_default();
    let css = theme::build_css(
        &config_dir,
        &settings.themes,
        settings.accent_color.as_deref(),
        settings.color_scheme.as_deref(),
    );
    let theme_script = if css.is_empty() {
        String::new()
    } else {
        theme::injection_script(&css)
    };

    let mut switches = settings.chromium_switches();
    if !mod_script.is_empty() && settings.mod_csp_bypass {
        match cdp::free_port() {
            Some(port) => {
                switches.push(("remote-debugging-port".into(), Some(port.to_string())));
                cdp::spawn(port);
            }
            None => eprintln!("rustcord: no free port, CSP bypass unavailable"),
        }
    }
    let flags = switches
        .into_iter()
        .map(|(k, v)| match v {
            // QTWEBENGINE_CHROMIUM_FLAGS is split on spaces; quote values that contain any.
            Some(v) if v.contains(char::is_whitespace) => {
                format!("--{k}=\"{}\"", v.replace('"', ""))
            }
            Some(v) => format!("--{k}={v}"),
            None => format!("--{k}"),
        })
        .collect::<Vec<_>>()
        .join(" ");
    let profile = paths::profile_dir();
    let png = icon::circle_png(256, [0x58, 0x65, 0xf2]);

    let c = |s: &str| CString::new(s.replace('\0', "")).unwrap_or_default();
    let start_url = c(&nav::deep_link_url(settings.branch, deep_link.as_deref()));
    let profile_c = c(&profile.to_string_lossy());
    let flags_c = c(&flags);
    let mod_c = c(&mod_script);
    let theme_c = c(&theme_script);
    let cfg = RcConfig {
        start_url: start_url.as_ptr(),
        profile_dir: profile_c.as_ptr(),
        chromium_flags: flags_c.as_ptr(),
        mod_script: mod_c.as_ptr(),
        theme_script: theme_c.as_ptr(),
        icon_png: png.as_ptr(),
        icon_len: png.len() as c_ulong,
        x: state.x.unwrap_or(0),
        y: state.y.unwrap_or(0),
        w: state.width,
        h: state.height,
        has_pos: state.x.is_some() as c_int,
        maximized: state.maximized as c_int,
        start_minimized: settings.start_minimized as c_int,
        tray: settings.tray as c_int,
        close_to_tray: settings.close_to_tray as c_int,
        sandbox: settings.sandbox as c_int,
    };

    // Qt wants a mutable, NUL-terminated argv that outlives the call.
    let args: Vec<CString> = std::env::args().map(|a| c(&a)).collect();
    let mut ptrs: Vec<*mut c_char> = args.iter().map(|a| a.as_ptr() as *mut c_char).collect();
    ptrs.push(std::ptr::null_mut());
    let _ = std::ptr::null::<c_void>();
    // SAFETY: cfg and all strings it points to live until rc_qt_run returns.
    let code = unsafe { rc_qt_run(args.len() as c_int, ptrs.as_mut_ptr(), &cfg) };
    ExitCode::from(code.clamp(0, 255) as u8)
}
