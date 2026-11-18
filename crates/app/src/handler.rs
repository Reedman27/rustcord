//! Browser-level handlers: lifecycle, titles, navigation policy, media permissions.

use std::sync::{Arc, Mutex, OnceLock, Weak};

use cef::*;
use rustcord_core::single_instance::Message;
use rustcord_core::{nav, theme};

use crate::{config, external};

static INSTANCE: OnceLock<Weak<Mutex<Handler>>> = OnceLock::new();

pub struct Handler {
    browser_list: Vec<Browser>,
}

impl Handler {
    pub fn instance() -> Option<Arc<Mutex<Self>>> {
        INSTANCE.get().and_then(|w| w.upgrade())
    }

    pub fn new() -> Arc<Mutex<Self>> {
        Arc::new_cyclic(|weak| {
            let _ = INSTANCE.set(weak.clone());
            Mutex::new(Self {
                browser_list: Vec::new(),
            })
        })
    }

    fn on_title_change(&mut self, browser: Option<&mut Browser>, title: Option<&CefString>) {
        let mut browser = browser.cloned();
        if let Some(browser_view) = browser_view_get_for_browser(browser.as_mut()) {
            if let Some(window) = browser_view.window() {
                window.set_title(title);
            }
        }
    }

    fn on_after_created(&mut self, browser: Option<&mut Browser>) {
        if let Some(browser) = browser.cloned() {
            self.browser_list.push(browser);
        }
    }

    fn on_before_close(&mut self, browser: Option<&mut Browser>) {
        let Some(mut browser) = browser.cloned() else {
            return;
        };
        if let Some(i) = self
            .browser_list
            .iter()
            .position(|b| b.is_same(Some(&mut browser)) != 0)
        {
            self.browser_list.remove(i);
        }
        if self.browser_list.is_empty() {
            quit_message_loop();
        }
    }

    /// UI thread only.
    fn show_main_window(&mut self) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);
        let Some(mut main_browser) = self.browser_list.first().cloned() else {
            return;
        };
        if let Some(browser_view) = browser_view_get_for_browser(Some(&mut main_browser)) {
            if let Some(window) = browser_view.window() {
                if window.is_minimized() == 1 {
                    window.restore();
                }
                window.show();
                window.activate();
                window.bring_to_top();
            }
        }
    }

    /// UI thread only. Close every browser; the last `on_before_close` ends the message loop.
    fn quit_all(&mut self) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);
        for browser in &self.browser_list {
            if let Some(host) = browser.host() {
                host.close_browser(1);
            }
        }
    }

    /// UI thread only. Loads `url` (always a Discord URL built by `nav`) and shows the window.
    fn open_url(&mut self, url: &str) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);
        if let Some(frame) = self.browser_list.first().and_then(|b| b.main_frame()) {
            frame.load_url(Some(&CefString::from(url)));
        }
        self.show_main_window();
    }
}

/// Ask the UI thread to show the main window. Safe to call from any thread.
pub fn request_show() {
    if let Some(inner) = Handler::instance() {
        post_task(ThreadId::UI, Some(&mut ShowMainWindow::new(inner)));
    }
}

/// Ask the UI thread to close everything and exit. Safe to call from any thread.
pub fn request_quit() {
    if let Some(inner) = Handler::instance() {
        post_task(ThreadId::UI, Some(&mut QuitApp::new(inner)));
    }
}

/// Called on the single-instance listener thread when a second launch forwards a request.
pub fn handle_ipc(msg: Message) {
    let Some(inner) = Handler::instance() else {
        return;
    };
    let mut task = match msg {
        Message::Focus => ShowMainWindow::new(inner),
        Message::Open(uri) => {
            let url = nav::deep_link_url(config::get().settings.branch, Some(&uri));
            OpenUrl::new(inner, url)
        }
    };
    post_task(ThreadId::UI, Some(&mut task));
}

wrap_task! {
    struct ShowMainWindow {
        inner: Arc<Mutex<Handler>>,
    }

    impl Task {
        fn execute(&self) {
            self.inner.lock().unwrap_or_else(|e| e.into_inner()).show_main_window();
        }
    }
}

wrap_task! {
    struct QuitApp {
        inner: Arc<Mutex<Handler>>,
    }

    impl Task {
        fn execute(&self) {
            self.inner.lock().unwrap_or_else(|e| e.into_inner()).quit_all();
        }
    }
}

wrap_task! {
    struct OpenUrl {
        inner: Arc<Mutex<Handler>>,
        url: String,
    }

    impl Task {
        fn execute(&self) {
            self.inner.lock().unwrap_or_else(|e| e.into_inner()).open_url(&self.url);
        }
    }
}

wrap_client! {
    pub struct RustcordClient {
        inner: Arc<Mutex<Handler>>,
    }

    impl Client {
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(RustcordDisplayHandler::new(self.inner.clone()))
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(RustcordLoadHandler::new())
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(RustcordLifeSpanHandler::new(self.inner.clone()))
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            Some(RustcordRequestHandler::new())
        }

        fn permission_handler(&self) -> Option<PermissionHandler> {
            Some(RustcordPermissionHandler::new())
        }
    }
}

wrap_display_handler! {
    struct RustcordDisplayHandler {
        inner: Arc<Mutex<Handler>>,
    }

    impl DisplayHandler {
        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            self.inner.lock().unwrap_or_else(|e| e.into_inner()).on_title_change(browser, title);
        }
    }
}

wrap_life_span_handler! {
    struct RustcordLifeSpanHandler {
        inner: Arc<Mutex<Handler>>,
    }

    impl LifeSpanHandler {
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: i32,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            _user_gesture: i32,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            let url = target_url.map(|u| u.to_string()).unwrap_or_default();
            // Discord pop-outs open about:blank or a Discord URL and stay in the app.
            if url.is_empty() || url == "about:blank" || nav::is_discord_url(&url) {
                return 0;
            }
            // Everything else goes to the system browser and the popup is cancelled.
            external::open(&url);
            1
        }

        fn on_after_created(&self, browser: Option<&mut Browser>) {
            self.inner.lock().unwrap_or_else(|e| e.into_inner()).on_after_created(browser);
        }

        // Let the close proceed; the window delegate's can_close drives the browser close.
        fn do_close(&self, _browser: Option<&mut Browser>) -> i32 {
            0
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            self.inner.lock().unwrap_or_else(|e| e.into_inner()).on_before_close(browser);
        }
    }
}

wrap_request_handler! {
    struct RustcordRequestHandler;

    impl RequestHandler {
        /// Navigation guard: the main frame may only go to Discord hosts.
        fn on_before_browse(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _user_gesture: i32,
            _is_redirect: i32,
        ) -> i32 {
            let (Some(frame), Some(request)) = (frame, request) else { return 0 };
            if frame.is_main() != 1 {
                return 0;
            }
            let url = CefString::from(&request.url()).to_string();
            if nav::is_discord_url(&url) {
                return 0;
            }
            external::open(&url);
            1
        }
    }
}

wrap_permission_handler! {
    struct RustcordPermissionHandler;

    impl PermissionHandler {
        /// Microphone and camera for Discord origins only (voice and video calls).
        fn on_request_media_access_permission(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut MediaAccessCallback>,
        ) -> i32 {
            let Some(callback) = callback else { return 0 };
            let origin = requesting_origin.map(|o| o.to_string()).unwrap_or_default();
            if nav::is_discord_url(&origin) {
                callback.cont(requested_permissions);
            } else {
                callback.cancel();
            }
            1
        }
    }
}

wrap_load_handler! {
    struct RustcordLoadHandler;

    impl LoadHandler {
        /// Install the user's theme CSS once each Discord main-frame page has loaded.
        fn on_load_end(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _http_status_code: i32,
        ) {
            let Some(frame) = frame else { return };
            if frame.is_main() != 1 {
                return;
            }
            let url = CefString::from(&frame.url()).to_string();
            if !nav::is_discord_url(&url) {
                return;
            }
            let cfg = config::get();
            let css = theme::build_css(
                &cfg.config_dir,
                &cfg.settings.themes,
                cfg.settings.accent_color.as_deref(),
                cfg.settings.color_scheme.as_deref(),
            );
            if css.is_empty() {
                return;
            }
            let script = CefString::from(theme::injection_script(&css).as_str());
            frame.execute_java_script(Some(&script), Some(&CefString::from("rustcord://theme")), 0);
        }
    }
}
