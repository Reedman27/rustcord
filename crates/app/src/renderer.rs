// SPDX-License-Identifier: LGPL-3.0-or-later
//! Renderer-process side: runs the Vencord/Equicord bundle at document start.
//!
//! The renderer is sandboxed and cannot read files, so the browser process reads and
//! verifies the bundle and hands it over in the browser's `extra_info`.

use std::collections::HashMap;
use std::sync::Mutex;

use cef::*;
use rustcord_core::nav;

pub const EXTRA_KEY: &str = "rustcord_mod_script";

static SCRIPTS: Mutex<Option<HashMap<i32, String>>> = Mutex::new(None);

fn with_scripts<R>(f: impl FnOnce(&mut HashMap<i32, String>) -> R) -> R {
    let mut g = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new))
}

// App object for child processes (renderer, GPU, utility).
wrap_app! {
    pub struct RustcordChildApp;

    impl App {
        fn render_process_handler(&self) -> Option<RenderProcessHandler> {
            Some(RustcordRenderHandler::new())
        }
    }
}

wrap_render_process_handler! {
    struct RustcordRenderHandler;

    impl RenderProcessHandler {
        fn on_browser_created(
            &self,
            browser: Option<&mut Browser>,
            extra_info: Option<&mut DictionaryValue>,
        ) {
            let (Some(browser), Some(info)) = (browser, extra_info) else { return };
            let key = CefString::from(EXTRA_KEY);
            if info.has_key(Some(&key)) != 1 {
                return;
            }
            let script = CefString::from(&info.string(Some(&key))).to_string();
            if !script.is_empty() {
                with_scripts(|m| m.insert(browser.identifier(), script));
            }
        }

        fn on_browser_destroyed(&self, browser: Option<&mut Browser>) {
            if let Some(browser) = browser {
                with_scripts(|m| m.remove(&browser.identifier()));
            }
        }

        // Runs before any page script, which the clients need to patch webpack.
        fn on_context_created(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            context: Option<&mut V8Context>,
        ) {
            let (Some(browser), Some(frame), Some(context)) = (browser, frame, context) else { return };
            if frame.is_main() != 1 {
                return;
            }
            let url = CefString::from(&frame.url()).to_string();
            if !nav::is_discord_url(&url) {
                return;
            }
            let Some(script) = with_scripts(|m| m.get(&browser.identifier()).cloned()) else {
                return;
            };
            // Synchronous eval inside the new context, so it is guaranteed to run before
            // any page script (execute_java_script gives no such ordering promise).
            let mut exception = None;
            let ok = context.eval(
                Some(&CefString::from(script.as_str())),
                Some(&CefString::from("rustcord://mod")),
                1,
                None,
                Some(&mut exception),
            );
            if ok != 1 {
                let msg = exception
                    .map(|e| CefString::from(&e.message()).to_string())
                    .unwrap_or_default();
                eprintln!("rustcord: mod bundle failed to run: {msg}");
            }
        }
    }
}
