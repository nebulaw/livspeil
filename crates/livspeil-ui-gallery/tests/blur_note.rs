// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The Materials page shows a note when blur is in fallback (D63). Own test
//! binary: the note is process-wide state.

use livspeil_ui_core::{App, TestHost};
use livspeil_ui_gallery::{Page, WINDOW_SIZE, mount, set_blur_note};
use livspeil_ui_layout::Size;
use livspeil_ui_text::FontSystem;

fn build() -> TestHost {
    let mut app = App::new();
    let _ = mount(&mut app, FontSystem::test(), Page::Materials);
    let mut h = TestHost::new(app, Size::new(WINDOW_SIZE.0, WINDOW_SIZE.1));
    h.run_until_idle(300);
    h
}

#[test]
fn note_only_in_fallback() {
    set_blur_note(None);
    assert!(build().app.find("blur-fallback-note").is_none());
    set_blur_note(Some("no TEXTURE_BINDING".to_owned()));
    assert!(build().app.find("blur-fallback-note").is_some());
    set_blur_note(None);
}
