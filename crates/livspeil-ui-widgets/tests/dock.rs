// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Panel and DockView: splitter drags, limits, reset, collapse.

mod common;

use common::{center, find, host_sized, wait};
use livspeil_ui_core::{CursorShape, Element, TestHost, View, div, text};
use livspeil_ui_layout::{DockEdge, Point, Size};
use livspeil_ui_theme::MotionConfig;
use livspeil_ui_widgets::{DockView, Panel};

fn fixture() -> (TestHost, View<DockView>) {
    let mut out = None;
    let (h, _) = host_sized(Size::new(800.0, 500.0), |app, _| {
        let dock = app.new_view(
            DockView::new(|_| text("Canvas"))
                .panel(DockEdge::Left, 200.0, 120.0, |app| {
                    Panel::new("Layers").frost().render(app.theme())
                })
                .panel(DockEdge::Right, 240.0, 160.0, |app| {
                    Panel::new("Inspector").render(app.theme())
                })
                .panel(DockEdge::Bottom, 120.0, 60.0, |app| {
                    Panel::new("Timeline").render(app.theme())
                })
                .center_min(200.0, 100.0),
        );
        out = Some(dock);
        Box::new(move |_| div().size(800.0, 500.0).child(Element::view(&dock)))
    });
    (h, out.unwrap())
}

fn width(h: &TestHost, name: &str) -> f32 {
    h.app.bounds(find(h, name)).unwrap().width
}

#[test]
fn layout_matches_the_dock() {
    let (h, _) = fixture();
    assert_eq!(width(&h, "panel:Layers"), 200.0);
    assert_eq!(width(&h, "panel:Inspector"), 240.0);
    let tl = h.app.bounds(find(&h, "panel:Timeline")).unwrap();
    assert_eq!((tl.x, tl.width, tl.height), (200.0, 360.0, 120.0));
    let split = h.app.bounds(find(&h, "splitter:left")).unwrap();
    assert_eq!(split.x, 199.0 - 4.0);
    assert_eq!(split.width, 9.0);
}

#[test]
fn splitter_drags_with_limits() {
    let (mut h, dock) = fixture();
    let s = center(&h, "splitter:left");
    h.pointer_move(s.x, s.y);
    assert_eq!(h.app.take_cursor_request(), Some(CursorShape::ResizeEw));
    h.drag(s, Point::new(s.x + 50.0, s.y), 5);
    h.run_until_idle(100);
    assert_eq!(dock.read(&h.app).size(DockEdge::Left), Some(250.0));
    assert_eq!(width(&h, "panel:Layers"), 250.0);
    // Below the minimum: clamped.
    let s = center(&h, "splitter:left");
    h.drag(s, Point::new(s.x - 400.0, s.y), 5);
    h.run_until_idle(100);
    assert_eq!(dock.read(&h.app).size(DockEdge::Left), Some(120.0));
    // Past what leaves the center its minimum: 800 - 240 - 200.
    let s = center(&h, "splitter:left");
    h.drag(s, Point::new(s.x + 700.0, s.y), 5);
    h.run_until_idle(100);
    assert_eq!(dock.read(&h.app).size(DockEdge::Left), Some(360.0));
    // The right splitter grows to the left; the bottom one upward.
    let s = center(&h, "splitter:bottom");
    let _ = h.app.take_cursor_request();
    h.pointer_move(s.x, s.y);
    assert_eq!(h.app.take_cursor_request(), Some(CursorShape::ResizeNs));
    h.drag(s, Point::new(s.x, s.y - 30.0), 3);
    h.run_until_idle(100);
    assert_eq!(dock.read(&h.app).size(DockEdge::Bottom), Some(150.0));
}

#[test]
fn double_press_resets() {
    let (mut h, dock) = fixture();
    let s = center(&h, "splitter:right");
    h.drag(s, Point::new(s.x - 60.0, s.y), 4);
    h.run_until_idle(100);
    assert_eq!(dock.read(&h.app).size(DockEdge::Right), Some(300.0));
    wait(&mut h, 600);
    let s = center(&h, "splitter:right");
    h.click(s.x, s.y);
    wait(&mut h, 100);
    h.click(s.x, s.y);
    h.run_until_idle(200);
    assert_eq!(dock.read(&h.app).size(DockEdge::Right), Some(240.0));
    assert_eq!(width(&h, "panel:Inspector"), 240.0);
}

#[test]
fn collapse_springs_and_reduce_motion_jumps() {
    let (mut h, dock) = fixture();
    DockView::toggle(dock, &mut h.app, DockEdge::Left);
    h.step();
    h.step();
    let mid = dock.read(&h.app).shown_size(DockEdge::Left);
    assert!(mid > 0.0 && mid < 200.0, "animating: {mid}");
    h.run_until_idle(200);
    assert_eq!(dock.read(&h.app).shown_size(DockEdge::Left), 0.0);
    assert!(h.app.find("panel:Layers").is_none());
    assert!(h.app.find("splitter:left").is_none());
    // The center took the space.
    assert_eq!(h.app.bounds(find(&h, "panel:Timeline")).unwrap().x, 0.0);
    h.app.set_motion(MotionConfig {
        reduce_motion: true,
        ..MotionConfig::default()
    });
    DockView::toggle(dock, &mut h.app, DockEdge::Left);
    h.step();
    assert_eq!(dock.read(&h.app).shown_size(DockEdge::Left), 200.0);
    h.run_until_idle(100);
    assert_eq!(width(&h, "panel:Layers"), 200.0);
}
