//! Window and view delegates (CEF Views framework).

use std::cell::RefCell;

use cef::*;
use rustcord_core::settings::{MIN_HEIGHT, MIN_WIDTH};

use crate::config;

wrap_window_delegate! {
    pub struct RustcordWindowDelegate {
        browser_view: RefCell<Option<BrowserView>>,
        initial_show_state: ShowState,
        is_main: bool,
    }

    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            if self.is_main {
                let s = config::window_state();
                Size { width: s.width, height: s.height }
            } else {
                Size { width: 800, height: 600 }
            }
        }

        fn minimum_size(&self, _view: Option<&mut View>) -> Size {
            if self.is_main {
                Size { width: MIN_WIDTH, height: MIN_HEIGHT }
            } else {
                Size { width: 320, height: 240 }
            }
        }
    }

    impl PanelDelegate {}

    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let browser_view = self.browser_view.borrow();
            let (Some(window), Some(browser_view)) = (window, browser_view.as_ref()) else {
                return;
            };
            let mut view = View::from(browser_view);
            window.add_child_view(Some(&mut view));
            window.set_title(Some(&CefString::from("Rustcord")));

            if self.is_main {
                let s = config::window_state();
                if let (Some(x), Some(y)) = (s.x, s.y) {
                    window.set_bounds(Some(&Rect { x, y, width: s.width, height: s.height }));
                }
                if s.maximized {
                    window.maximize();
                }
            }

            if self.initial_show_state != ShowState::HIDDEN {
                window.show();
            }
        }

        fn on_window_bounds_changed(&self, window: Option<&mut Window>, new_bounds: Option<&Rect>) {
            if !self.is_main {
                return;
            }
            let (Some(window), Some(b)) = (window, new_bounds) else {
                return;
            };
            let maximized = window.is_maximized() == 1;
            config::update_window_state(|s| {
                s.maximized = maximized;
                // Keep the last normal bounds while maximized so restore goes back to them.
                if !maximized {
                    s.x = Some(b.x);
                    s.y = Some(b.y);
                    s.width = b.width;
                    s.height = b.height;
                }
            });
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            if self.is_main {
                config::save_window_state();
            }
            *self.browser_view.borrow_mut() = None;
        }

        fn can_close(&self, _window: Option<&mut Window>) -> i32 {
            let browser_view = self.browser_view.borrow();
            let Some(browser_view) = browser_view.as_ref() else {
                return 1;
            };
            match browser_view.browser().and_then(|b| b.host()) {
                Some(host) => host.try_close_browser(),
                None => 1,
            }
        }

        fn initial_show_state(&self, _window: Option<&mut Window>) -> ShowState {
            self.initial_show_state
        }
    }
}

wrap_browser_view_delegate! {
    pub struct RustcordBrowserViewDelegate {
        // The macro's unit-struct form is broken in cef 156 (it drops `impl ViewDelegate`),
        // so keep one field, like the cefsimple example does.
        main: bool,
    }

    impl ViewDelegate {}

    impl BrowserViewDelegate {
        fn on_popup_browser_view_created(
            &self,
            _browser_view: Option<&mut BrowserView>,
            popup_browser_view: Option<&mut BrowserView>,
            _is_devtools: i32,
        ) -> i32 {
            let _ = self.main;
            // Allowed popups (Discord pop-outs) get their own top-level window.
            let mut delegate = RustcordWindowDelegate::new(
                RefCell::new(popup_browser_view.cloned()),
                ShowState::NORMAL,
                false,
            );
            window_create_top_level(Some(&mut delegate));
            1
        }

        // App-style window: no browser toolbar or address bar.
        fn chrome_toolbar_type(&self, _browser_view: Option<&mut BrowserView>) -> ChromeToolbarType {
            ChromeToolbarType::NONE
        }
    }
}
