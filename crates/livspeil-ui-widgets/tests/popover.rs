// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Popover: placement and flipping, opening, fading, dismissal, focus.

mod common;

use common::{center, find, golden_dir, host_sized, scene};
use livspeil_testkit::assert_golden;
use livspeil_ui_core::{Element, Role, TestHost, View, div, text};
use livspeil_ui_layout::{Point, Rect, Size, Vec2};
use livspeil_ui_theme::{MotionConfig, Scheme, Theme};
use livspeil_ui_widgets::{
    Button, IconButton, POPOVER_GAP, POPOVER_MARGIN, Placement, Popover, place_popover,
};

const VP: Vec2 = Vec2::new(400.0, 300.0);
const SIZE: Vec2 = Vec2::new(120.0, 80.0);

#[test]
fn placement_prefers_a_side_and_flips_when_it_does_not_fit() {
    let anchor = Rect::new(180.0, 130.0, 40.0, 24.0);
    let at = |a: Rect, p| place_popover(a, SIZE, VP, p);
    // Every side, centered on the anchor, with the gap.
    let (r, s) = at(anchor, Placement::Bottom);
    assert_eq!(
        (s, r.x, r.y),
        (Placement::Bottom, 140.0, 130.0 + 24.0 + POPOVER_GAP)
    );
    let (r, s) = at(anchor, Placement::Top);
    assert_eq!((s, r.y), (Placement::Top, 130.0 - POPOVER_GAP - 80.0));
    let (r, s) = at(anchor, Placement::Left);
    assert_eq!(
        (s, r.right(), r.y),
        (Placement::Left, 180.0 - POPOVER_GAP, 142.0 - 40.0)
    );
    let (r, s) = at(anchor, Placement::Right);
    assert_eq!((s, r.x), (Placement::Right, 220.0 + POPOVER_GAP));
    // Flip: no room below -> above; no room above -> below.
    let low = Rect::new(180.0, 270.0, 40.0, 24.0);
    assert_eq!(at(low, Placement::Bottom).1, Placement::Top);
    let high = Rect::new(180.0, 4.0, 40.0, 24.0);
    assert_eq!(at(high, Placement::Top).1, Placement::Bottom);
    // Flip horizontally.
    let right = Rect::new(370.0, 130.0, 24.0, 24.0);
    assert_eq!(at(right, Placement::Right).1, Placement::Left);
    let left = Rect::new(4.0, 130.0, 24.0, 24.0);
    assert_eq!(at(left, Placement::Left).1, Placement::Right);
    // Fits on the preferred side: no flip even though the other fits too.
    assert_eq!(at(anchor, Placement::Top).1, Placement::Top);
}

#[test]
fn placement_clamps_across_the_axis_and_picks_the_roomier_side() {
    // Anchor at the right edge: centered would overflow; clamped with margin.
    let (r, _) = place_popover(
        Rect::new(370.0, 100.0, 24.0, 24.0),
        SIZE,
        VP,
        Placement::Bottom,
    );
    assert_eq!(r.right(), VP.x - POPOVER_MARGIN);
    let (r, _) = place_popover(
        Rect::new(0.0, 100.0, 12.0, 24.0),
        SIZE,
        VP,
        Placement::Bottom,
    );
    assert_eq!(r.x, POPOVER_MARGIN);
    // Neither side fits (tall popover): use the side with more room.
    let tall = Vec2::new(100.0, 280.0);
    let (r, s) = place_popover(
        Rect::new(180.0, 200.0, 40.0, 24.0),
        tall,
        VP,
        Placement::Bottom,
    );
    assert_eq!(s, Placement::Top, "200 above beats 70 below");
    assert!(r.y >= POPOVER_MARGIN, "clamped into the window: {r:?}");
    // Everything stays inside the window for a sweep of anchors.
    for p in [
        Placement::Top,
        Placement::Bottom,
        Placement::Left,
        Placement::Right,
    ] {
        for x in (0..380).step_by(40) {
            for y in (0..280).step_by(40) {
                let a = Rect::new(x as f32, y as f32, 20.0, 20.0);
                let (r, _) = place_popover(a, SIZE, VP, p);
                assert!(
                    r.x >= POPOVER_MARGIN - 0.01
                        && r.y >= POPOVER_MARGIN - 0.01
                        && r.right() <= VP.x - POPOVER_MARGIN + 0.01
                        && r.bottom() <= VP.y - POPOVER_MARGIN + 0.01,
                    "{p:?} {a:?} -> {r:?}"
                );
            }
        }
    }
}

struct Fx {
    h: TestHost,
    pop: View<Popover>,
}

fn fixture(scheme: Scheme, at: (f32, f32), placement: Placement) -> Fx {
    let mut view = None;
    let (h, _) = host_sized(Size::new(400.0, 300.0), |app, env| {
        app.set_theme(Theme {
            scheme,
            ..Theme::default()
        });
        let pop = app.new_view(
            Popover::new(env, |app| {
                let theme = app.theme().clone();
                div()
                    .flex_col()
                    .gap(8.0)
                    .child(
                        text("Snap to grid")
                            .font_size(13.0)
                            .text_color(theme.colors().text_primary),
                    )
                    .child(Button::new("First").render(&theme))
                    .child(Button::new("Second").render(&theme))
            })
            .trigger(|theme| IconButton::new("bx-gear", "Settings").render(theme))
            .placement(placement)
            .size(120.0, 120.0)
            .label("Grid settings"),
        );
        view = Some(pop);
        Box::new(move |_| {
            div()
                .w_full()
                .h_full()
                .child(
                    div()
                        .absolute(Rect::new(at.0, at.1, 60.0, 40.0))
                        .child(Element::view(&pop)),
                )
                .child(
                    div()
                        .name("outside")
                        .absolute(Rect::new(300.0, 250.0, 60.0, 30.0)),
                )
        })
    });
    Fx {
        h,
        pop: view.unwrap(),
    }
}

impl Fx {
    fn open(&self) -> bool {
        self.pop.read(&self.h.app).is_open()
    }
}

#[test]
fn click_opens_below_fades_in_without_scaling_and_escape_returns_focus() {
    let mut f = fixture(Scheme::Dark, (150.0, 60.0), Placement::Bottom);
    let trigger = find(&f.h, "icon-button:Settings");
    f.h.key("tab");
    assert_eq!(f.h.app.focused(), Some(trigger));
    f.h.key("enter");
    f.h.step();
    assert!(f.open());
    // First frames: hidden, then fading in; the scale is always 1.
    f.h.step();
    let panel = find(&f.h, "popover");
    let first = f.h.app.opacity_of(panel).unwrap();
    assert!(first < 1.0, "starts transparent: {first}");
    f.h.step();
    assert_eq!(f.h.app.scale_of(panel), Some(1.0));
    f.h.run_until_idle(200);
    assert_eq!(f.h.app.opacity_of(panel), Some(1.0));
    assert_eq!(f.h.app.scale_of(panel), Some(1.0));
    assert!(!f.h.app.needs_frame(), "idle once shown");
    let a = f.h.app.bounds(trigger).unwrap();
    let b = f.h.app.bounds(panel).unwrap();
    assert!(b.y >= a.bottom() + POPOVER_GAP - 0.5, "below the trigger");
    assert!((b.center().x - a.center().x).abs() < 1.0, "centered");
    // Focus is in the panel; the trigger says it is expanded.
    assert_eq!(f.h.app.focused(), Some(panel));
    let node = f.h.app.semantic_node(trigger).unwrap();
    assert_eq!(node.semantics.as_ref().unwrap().get_expanded(), Some(true));
    let node = f.h.app.semantic_node(panel).unwrap();
    assert_eq!(node.label(), Some("Grid settings"));
    // Tab stays inside the panel.
    f.h.key("tab");
    assert_eq!(f.h.app.focused(), Some(find(&f.h, "button:First")));
    f.h.key("tab");
    f.h.key("tab");
    assert_eq!(
        f.h.app.focused(),
        Some(find(&f.h, "button:First")),
        "wraps inside"
    );
    // Escape closes, fades out, and focus returns to the trigger.
    f.h.key("escape");
    assert!(!f.open());
    assert_eq!(f.h.app.focused(), Some(trigger));
    f.h.step();
    f.h.step();
    let mid = f.h.app.opacity_of(find(&f.h, "popover")).unwrap();
    assert!(mid < 1.0, "fading out: {mid}");
    f.h.run_until_idle(200);
    assert!(f.h.app.find("popover").is_none(), "removed after the exit");
    assert!(!f.h.app.needs_frame());
}

#[test]
fn clicking_outside_dismisses_and_a_leaving_popover_takes_no_input() {
    let mut f = fixture(Scheme::Dark, (150.0, 60.0), Placement::Bottom);
    let p = center(&f.h, "icon-button:Settings");
    f.h.click(p.x, p.y);
    f.h.run_until_idle(200);
    assert!(f.open());
    // A click inside the panel keeps it open.
    let inside = center(&f.h, "popover");
    f.h.click(inside.x, inside.y);
    f.h.run_until_idle(100);
    assert!(f.open());
    // Outside (over another element): dismissed, the click is consumed.
    let o = center(&f.h, "outside");
    f.h.click(o.x, o.y);
    assert!(!f.open());
    assert_eq!(f.h.app.focused(), Some(find(&f.h, "icon-button:Settings")));
    // While it fades out, the old scrim does not capture input.
    f.h.step();
    assert_eq!(
        f.h.app.hit_test(Point::new(o.x, o.y)),
        Some(find(&f.h, "outside"))
    );
    f.h.run_until_idle(200);
    // Clicking the trigger toggles it.
    f.h.click(p.x, p.y);
    f.h.run_until_idle(200);
    assert!(f.open());
    f.h.click(p.x, p.y);
    assert!(
        !f.open(),
        "the scrim takes the click: closes without reopening"
    );
}

#[test]
fn flips_at_the_window_edge_and_reduce_motion_drops_the_slide() {
    let mut f = fixture(Scheme::Dark, (150.0, 240.0), Placement::Bottom);
    let p = center(&f.h, "icon-button:Settings");
    f.h.click(p.x, p.y);
    f.h.run_until_idle(200);
    let a = f.h.app.bounds(find(&f.h, "icon-button:Settings")).unwrap();
    let b = f.h.app.bounds(find(&f.h, "popover")).unwrap();
    assert!(
        b.bottom() <= a.y,
        "no room below: flipped above ({b:?} vs {a:?})"
    );
    assert_eq!(f.pop.read(&f.h.app).placed(&f.h.app).1, Placement::Top);
    Popover::close(f.pop, &mut f.h.app);
    f.h.run_until_idle(200);
    // Reduce Motion: the slide is an instant move, the fade stays.
    f.h.app.set_motion(MotionConfig {
        reduce_motion: true,
        ..MotionConfig::default()
    });
    Popover::open_at(f.pop, &mut f.h.app, a, None);
    f.h.step();
    f.h.step();
    let panel = find(&f.h, "popover");
    assert_eq!(f.h.app.translation(panel), Some(Vec2::ZERO));
    assert!(f.h.app.opacity_of(panel).unwrap() <= 1.0);
}

#[test]
fn programmatic_open_and_semantics() {
    let mut f = fixture(Scheme::Dark, (150.0, 60.0), Placement::Right);
    let anchor = Rect::new(150.0, 60.0, 60.0, 40.0);
    Popover::open_at(f.pop, &mut f.h.app, anchor, None);
    f.h.run_until_idle(200);
    let b = f.h.app.bounds(find(&f.h, "popover")).unwrap();
    assert_eq!(b.x, anchor.right() + POPOVER_GAP);
    let n = f.h.app.semantic_node(find(&f.h, "popover")).unwrap();
    assert_eq!(n.role(), Some(Role::Group));
    Popover::close(f.pop, &mut f.h.app);
    assert!(!f.open());
    // Closing twice is harmless.
    Popover::close(f.pop, &mut f.h.app);
}

#[test]
fn popover_scenes_in_both_themes() {
    for (scheme, name) in [
        (Scheme::Dark, "popover-dark"),
        (Scheme::Light, "popover-light"),
    ] {
        let mut f = fixture(scheme, (150.0, 60.0), Placement::Bottom);
        let p = center(&f.h, "icon-button:Settings");
        f.h.click(p.x, p.y);
        f.h.run_until_idle(200);
        assert_golden(&golden_dir(), name, &scene(&f.h));
    }
}
