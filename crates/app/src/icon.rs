// SPDX-License-Identifier: LGPL-3.0-or-later
//! Window icon, generated at startup. Window managers read it for the title bar,
//! task list and icon (Window Maker uses it for the app icon).

use cef::*;
use rustcord_core::icon;

const ACCENT: [u8; 3] = [0x58, 0x65, 0xf2];

pub fn window_icon() -> Option<Image> {
    let image = image_create()?;
    // 1x and 2x representations; CEF picks the one matching the display scale.
    let ok1 = image.add_png(1.0, Some(&icon::circle_png(64, ACCENT)));
    let ok2 = image.add_png(2.0, Some(&icon::circle_png(128, ACCENT)));
    (ok1 == 1 || ok2 == 1).then_some(image)
}
