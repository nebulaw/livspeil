// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Titlebar: layout, drag region and zoom requests.

mod common;

use std::time::Duration;

use common::{center, find, host_sized};
use livspeil_ui_core::{WindowRequest, div};
use livspeil_ui_layout::Size;
use livspeil_ui_widgets::{Button, TOOLBAR_HEIGHT, TRAFFIC_LIGHT_INSET, Titlebar};

#[test]
fn titlebar_layout_drag_and_zoom() {
    let (mut h, _) = host_sized(Size::new(600.0, 200.0), |_, _| {
        Box::new(|cx| {
            let theme = cx.theme().clone();
            div().w_full().flex_col().child(
                Titlebar::new("Untitled Scene")
                    .leading(Button::new("Back").ghost().render(&theme))
                    .trailing(Button::new("Share").render(&theme))
                    .render(&theme),
            )
        })
    });
    let bar = h.app.bounds(find(&h, "titlebar")).unwrap();
    assert_eq!(bar.height, TOOLBAR_HEIGHT);
    let back = h.app.bounds(find(&h, "button:Back")).unwrap();
    assert!(back.x >= TRAFFIC_LIGHT_INSET, "{back:?}");
    let share = h.app.bounds(find(&h, "button:Share")).unwrap();
    assert!(share.right() <= 600.0 && share.right() > 580.0);
    // The title is centered in the window.
    let dump = h.app.dump_tree();
    assert!(dump.contains("\"Untitled Scene\""));
    let title = h.app.bounds(find(&h, "title")).unwrap();
    assert_eq!(title.center().x, 300.0);
    // Empty space and the title drag; a double press zooms.
    h.click(200.0, 10.0);
    assert_eq!(h.app.take_window_requests(), vec![WindowRequest::Drag]);
    h.advance(Duration::from_millis(600));
    let t = center(&h, "title");
    h.click(t.x, t.y);
    assert_eq!(h.app.take_window_requests(), vec![WindowRequest::Drag]);
    h.advance(Duration::from_millis(100));
    h.click(t.x, t.y);
    assert_eq!(h.app.take_window_requests(), vec![WindowRequest::Zoom]);
    // Buttons do not.
    h.advance(Duration::from_millis(600));
    let s = center(&h, "button:Share");
    h.click(s.x, s.y);
    assert!(h.app.take_window_requests().is_empty());
    let node = h.app.semantic_node(find(&h, "titlebar")).unwrap();
    assert_eq!(node.role(), Some(livspeil_ui_core::Role::Toolbar));
}
