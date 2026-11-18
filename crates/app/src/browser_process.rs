//! CEF application object and browser-process startup.

use std::cell::RefCell;

use cef::*;

use crate::config;
use crate::handler::{Handler, RustcordClient};
use crate::window::{RustcordBrowserViewDelegate, RustcordWindowDelegate};

wrap_app! {
    pub struct RustcordApp {
        switches: Vec<(String, Option<String>)>,
    }

    impl App {
        fn on_before_command_line_processing(
            &self,
            process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            // Only the browser process (empty process type) gets our switches; CEF
            // forwards the relevant ones to child processes itself.
            let is_browser = process_type.map(|t| t.to_string().is_empty()).unwrap_or(true);
            let Some(command_line) = command_line else { return };
            if !is_browser {
                return;
            }
            for (name, value) in &self.switches {
                let name = CefString::from(name.as_str());
                match value {
                    Some(v) => command_line.append_switch_with_value(Some(&name), Some(&CefString::from(v.as_str()))),
                    None => command_line.append_switch(Some(&name)),
                }
            }
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(RustcordBrowserProcessHandler::new(RefCell::new(None)))
        }
    }
}

wrap_browser_process_handler! {
    struct RustcordBrowserProcessHandler {
        client: RefCell<Option<Client>>,
    }

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            debug_assert_ne!(currently_on(ThreadId::UI), 0);
            let cfg = config::get();

            *self.client.borrow_mut() = Some(RustcordClient::new(Handler::new()));
            if cfg.settings.tray {
                crate::tray::start();
            }

            let url = CefString::from(cfg.start_url.as_str());
            let browser_settings = BrowserSettings::default();
            let mut client = self.client.borrow().clone();
            let mut view_delegate = RustcordBrowserViewDelegate::new(true);
            let browser_view = browser_view_create(
                client.as_mut(),
                Some(&url),
                Some(&browser_settings),
                None,
                None,
                Some(&mut view_delegate),
            );

            let show_state = if cfg.settings.start_minimized {
                ShowState::MINIMIZED
            } else {
                ShowState::NORMAL
            };
            let mut window_delegate =
                RustcordWindowDelegate::new(RefCell::new(browser_view), show_state, true);
            window_create_top_level(Some(&mut window_delegate));
        }
    }
}
