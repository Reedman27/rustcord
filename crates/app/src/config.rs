//! Process-wide configuration, set once in `main` before CEF starts.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use rustcord_core::settings::{self, Settings, WindowState};

pub struct Config {
    pub settings: Settings,
    pub state_path: PathBuf,
    pub config_dir: PathBuf,
    /// First URL to load (landing page or a deep link).
    pub start_url: String,
}

static CONFIG: OnceLock<Config> = OnceLock::new();
static STATE: OnceLock<Mutex<WindowState>> = OnceLock::new();

pub fn init(config: Config, state: WindowState) {
    let _ = CONFIG.set(config);
    let _ = STATE.set(Mutex::new(state));
}

pub fn get() -> &'static Config {
    CONFIG
        .get()
        .expect("config::init must run before CEF starts")
}

fn state() -> &'static Mutex<WindowState> {
    STATE
        .get()
        .expect("config::init must run before CEF starts")
}

pub fn window_state() -> WindowState {
    *state().lock().unwrap_or_else(|e| e.into_inner())
}

pub fn update_window_state(f: impl FnOnce(&mut WindowState)) {
    f(&mut state().lock().unwrap_or_else(|e| e.into_inner()));
}

pub fn save_window_state() {
    let s = window_state();
    if let Err(e) = settings::save_atomic(&get().state_path, &s) {
        eprintln!("rustcord: could not save window state: {e}");
    }
}
