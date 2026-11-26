// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Every icon the gallery draws is one the set has (no placeholders).

use livspeil_ui_core::PaintCmd;
use livspeil_ui_gallery::Page;
use livspeil_ui_icons::IconId;
use livspeil_ui_paint::{FontSystem, PaintBridge};
use livspeil_ui_shot::gallery::settle_page;
use livspeil_ui_theme::Theme;

#[test]
fn gallery_pages_only_use_icons_of_the_set() {
    let bridge = PaintBridge::new(FontSystem::test());
    let mut drawn = 0;
    for page in Page::ALL {
        let scene = settle_page(&bridge, page, Theme::default()).unwrap();
        for command in &scene.commands {
            if let PaintCmd::Icon(i) = command {
                drawn += 1;
                assert!(
                    IconId::from_name(&i.name).is_some(),
                    "{page:?}: unknown icon {}",
                    i.name
                );
            }
        }
    }
    assert!(drawn > 10, "only {drawn} icons found");
}
