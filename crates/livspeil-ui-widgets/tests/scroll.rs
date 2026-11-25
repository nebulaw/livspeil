// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! ScrollView physics and behavior, VirtualList.

mod common;

use std::time::Duration;

use common::{find, host_sized, pointer, wait};
use livspeil_ui_core::{
    InputEvent, Modifiers, PointerKind, ScrollDelta, SemanticRequest, TestHost, View, div, text,
};
use livspeil_ui_layout::{Point, Size, Vec2};
use livspeil_ui_theme::{MotionClass, MotionConfig, MotionSpec, PropertyKind, SpringSpec};
use livspeil_ui_widgets::{
    Button, ScrollAxis, ScrollPhysics, ScrollView, Scroller, VirtualList, rubber_band,
    visible_range,
};

const DT: f32 = 1.0 / 60.0;

fn spring() -> MotionSpec {
    MotionSpec::Spring(SpringSpec::new(0.16, 0.86))
}

/// A scroller with a 200 px viewport over 1000 px of content.
fn scroller() -> Scroller {
    let mut s = Scroller::new(ScrollAxis::Vertical);
    s.set_sizes(Size::new(100.0, 200.0), Size::new(100.0, 1000.0));
    s
}

/// A trackpad flick: `n` deltas of `dy` px, one frame apart, starting at `t0`.
fn flick(s: &mut Scroller, p: &ScrollPhysics, dy: f32, n: u64, t0: u64) -> u64 {
    let mut t = t0;
    for _ in 0..n {
        s.pixel_scroll(Vec2::new(0.0, dy), Duration::from_millis(t), p);
        s.tick(DT, p, spring());
        t += 16;
    }
    t
}

fn run(s: &mut Scroller, p: &ScrollPhysics, spec: MotionSpec, max: usize) -> usize {
    for i in 0..max {
        if !s.tick(DT, p, spec) {
            return i;
        }
    }
    max
}

#[test]
fn momentum_continues_and_decays() {
    let p = ScrollPhysics::default();
    let mut s = scroller();
    flick(&mut s, &p, -10.0, 6, 0);
    assert!(s.is_tracking());
    let at_release = s.offset().y;
    assert!((at_release - 60.0).abs() < 1e-3, "{at_release}");
    // Release is detected after the release delay; then it glides.
    for _ in 0..5 {
        s.tick(DT, &p, spring());
    }
    assert!(s.is_gliding(), "{s:?}");
    let v0 = s.velocity().y;
    assert!(v0 > 400.0, "release velocity {v0}");
    s.tick(DT, &p, spring());
    let v1 = s.velocity().y;
    assert!(v1 < v0 && v1 > 0.0, "decays: {v0} -> {v1}");
    let frames = run(&mut s, &p, spring(), 600);
    assert!(frames < 300, "settles in {frames} frames");
    assert!(!s.is_moving());
    let end = s.offset().y;
    assert!(end > at_release + 100.0 && end <= 800.0, "glided to {end}");
}

#[test]
fn rubber_band_resists_and_returns() {
    let p = ScrollPhysics::default();
    let mut s = scroller();
    flick(&mut s, &p, 20.0, 5, 0); // pull 100 px past the top
    let o = s.offset().y;
    assert!(o < 0.0 && o > -100.0, "resisted overscroll {o}");
    assert!((o - rubber_band(-100.0, 200.0, 0.55)).abs() < 1e-3);
    let frames = run(&mut s, &p, spring(), 600);
    assert!(frames < 120, "returns in {frames} frames");
    assert_eq!(s.offset().y, 0.0);
    // Past the bottom too.
    s.jump_to(Vec2::new(0.0, 800.0));
    flick(&mut s, &p, -20.0, 5, 1000);
    assert!(s.offset().y > 800.0 && s.offset().y < 900.0);
    run(&mut s, &p, spring(), 600);
    assert_eq!(s.offset().y, 800.0);
}

#[test]
fn momentum_bounces_at_the_end() {
    let p = ScrollPhysics::default();
    let mut s = scroller();
    s.jump_to(Vec2::new(0.0, 700.0));
    flick(&mut s, &p, -15.0, 4, 0);
    let mut beyond = 0.0_f32;
    for _ in 0..600 {
        if !s.tick(DT, &p, spring()) {
            break;
        }
        beyond = beyond.max(s.offset().y - 800.0);
    }
    assert!(beyond > 1.0, "overshoots the end ({beyond})");
    assert_eq!(s.offset().y, 800.0);
}

#[test]
fn reduce_motion_has_no_overscroll_or_bounce() {
    let motion = MotionConfig {
        reduce_motion: true,
        ..MotionConfig::default()
    };
    let p = ScrollPhysics::from_motion(&motion);
    assert!(!p.rubber_band);
    let spec = motion.resolve(
        Some(livspeil_ui_widgets::SCROLL_SCOPE),
        MotionClass::Standard,
        PropertyKind::Movement,
    );
    assert_eq!(spec, MotionSpec::Instant);
    let mut s = scroller();
    flick(&mut s, &p, 20.0, 5, 0);
    assert_eq!(s.offset().y, 0.0, "no pull past the top");
    // Momentum into the end stops there.
    s.jump_to(Vec2::new(0.0, 700.0));
    flick(&mut s, &p, -15.0, 4, 1000);
    for _ in 0..600 {
        assert!(s.offset().y <= 800.0);
        if !s.tick(DT, &p, spec) {
            break;
        }
    }
    assert_eq!(s.offset().y, 800.0);
    // Animated scrolls jump.
    s.scroll_to(Vec2::new(0.0, 100.0));
    s.tick(DT, &p, spec);
    assert_eq!(s.offset().y, 100.0);
    assert!(!s.is_moving());
}

#[test]
fn reveal_moves_minimally() {
    let mut s = scroller();
    s.jump_to(Vec2::new(0.0, 100.0));
    let r = |y: f32| livspeil_ui_layout::Rect::new(0.0, y, 100.0, 20.0);
    assert_eq!(s.reveal(r(150.0), 8.0).y, 100.0, "already visible");
    assert_eq!(s.reveal(r(400.0), 8.0).y, 400.0 + 20.0 + 8.0 - 200.0);
    assert_eq!(s.reveal(r(50.0), 8.0).y, 42.0);
    assert_eq!(s.reveal(r(990.0), 8.0).y, 800.0, "clamped");
}

#[test]
fn virtual_range_with_100k_rows() {
    let n = 100_000;
    assert_eq!(visible_range(0.0, 400.0, 20.0, n, 4), 0..24);
    let r = visible_range(1_000_000.0, 400.0, 20.0, n, 4);
    assert_eq!(r, 49_996..50_024);
    let end = visible_range(n as f32 * 20.0 - 400.0, 400.0, 20.0, n, 4);
    assert_eq!(end, 99_976..100_000);
    assert_eq!(visible_range(5e9, 400.0, 20.0, n, 4), 100_000..100_000);
    assert_eq!(visible_range(0.0, 400.0, 20.0, 0, 4), 0..0);
}

fn list_host(rows: usize) -> (TestHost, View<ScrollView>) {
    let mut out = None;
    let (h, _) = host_sized(Size::new(300.0, 400.0), |app, env| {
        let list = app.new_view(
            VirtualList::build(env, rows, 20.0, |_, i| text(format!("Row {i}")))
                .name("list")
                .label("Rows"),
        );
        out = Some(list);
        Box::new(move |_| {
            div()
                .size(300.0, 400.0)
                .child(livspeil_ui_core::Element::view(&list))
        })
    });
    (h, out.unwrap())
}

fn rows_rendered(h: &TestHost) -> usize {
    let list = find(h, "virtual-list");
    h.app.children_of(list).len()
}

#[test]
fn virtual_list_renders_only_visible_rows() {
    let (mut h, list) = list_host(100_000);
    let n = rows_rendered(&h);
    assert!(n <= 24 + 4, "{n} rows mounted");
    assert!(h.app.find("row-0").is_some());
    // Keyboard End: to the last row.
    let el = find(&h, "list");
    h.app.focus(el);
    h.key("end");
    h.run_until_idle(300);
    let off = list.read(&h.app).offset().y;
    assert_eq!(off, 100_000.0 * 20.0 - 400.0);
    assert!(h.app.find("row-99999").is_some());
    assert!(h.app.find("row-0").is_none());
    assert!(rows_rendered(&h) <= 28);
    let stats = h.last_frame().unwrap().stats;
    assert!(stats.nodes < 200, "{} nodes", stats.nodes);
    // Page up by a page minus a line.
    h.key("pageup");
    h.run_until_idle(300);
    assert_eq!(list.read(&h.app).offset().y, off - (400.0 - 40.0));
}

#[test]
fn wheel_lines_and_trackpad_pixels() {
    let (mut h, list) = list_host(1000);
    let c = Point::new(150.0, 200.0);
    pointer(
        &mut h,
        PointerKind::Scroll(ScrollDelta::Lines(Vec2::new(0.0, -2.0))),
        c,
        Modifiers::NONE,
    );
    h.run_until_idle(300);
    assert_eq!(list.read(&h.app).offset().y, 80.0);
    for _ in 0..5 {
        h.scroll(c.x, c.y, Vec2::new(0.0, -12.0));
        h.step();
    }
    assert!(list.read(&h.app).scroller().is_tracking());
    assert!(list.read(&h.app).bars_visible());
    h.run_until_idle(600);
    let off = list.read(&h.app).offset().y;
    assert!(off > 80.0 + 60.0, "momentum carried it further: {off}");
    // Bars fade after the linger time once idle.
    wait(&mut h, 1200);
    h.run_until_idle(100);
    assert!(!list.read(&h.app).bars_visible());
    // Ctrl+scroll is not consumed.
    h.app
        .dispatch(InputEvent::Pointer(livspeil_ui_core::PointerEvent {
            kind: PointerKind::Scroll(ScrollDelta::Pixels(Vec2::new(0.0, -50.0))),
            position: c,
            modifiers: Modifiers {
                ctrl: true,
                ..Modifiers::NONE
            },
            time: h.now(),
        }));
    h.run_until_idle(300);
    assert_eq!(list.read(&h.app).offset().y, off);
}

#[test]
fn focus_and_a11y_scroll_into_view() {
    let mut out = None;
    let (mut h, _) = host_sized(Size::new(300.0, 300.0), |app, env| {
        let sv = app.new_view(
            ScrollView::new(env, |app, _| {
                let theme = app.theme().clone();
                let mut col = div().flex_col().gap(8.0).p(8.0);
                for i in 0..20 {
                    col = col.child(Button::new(format!("B{i}")).render(&theme));
                }
                col
            })
            .name("sv")
            .focusable(false),
        );
        out = Some(sv);
        Box::new(move |_| {
            div()
                .size(300.0, 200.0)
                .child(livspeil_ui_core::Element::view(&sv))
        })
    });
    let sv = out.unwrap();
    for _ in 0..10 {
        h.key("tab");
    }
    h.run_until_idle(300);
    let focused = h.app.focused().unwrap();
    assert_eq!(h.app.label(focused).as_deref(), Some("button:B9"));
    let off = sv.read(&h.app).offset().y;
    assert!(off > 100.0, "scrolled to the focused button: {off}");
    let b = h.app.bounds(focused).unwrap();
    let vp = h.app.bounds(find(&h, "sv")).unwrap();
    let shown = b.y - off;
    assert!(shown >= vp.y && shown + b.height <= vp.bottom(), "{shown}");
    // Assistive technology: scroll B0 into view.
    let b0 = find(&h, "button:B0");
    let handled = h
        .app
        .perform_semantic_action(b0, SemanticRequest::ScrollIntoView)
        .unwrap();
    assert!(handled);
    h.run_until_idle(300);
    assert_eq!(sv.read(&h.app).offset().y, 0.0);
}
