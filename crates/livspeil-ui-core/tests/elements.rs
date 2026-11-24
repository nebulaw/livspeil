// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Hover, press, focus, transitions, drags and keyboard actions driven
//! headlessly, on button-, switch- and slider-like fixtures built from
//! plain elements (the real widgets live in `livspeil-ui-widgets`, which
//! depends on this crate).

use std::path::{Path, PathBuf};

use livspeil_testkit::assert_golden;
use livspeil_ui_core::{
    App, DragPhase, Element, EventCx, Model, Render, Role, SemanticValue, SharedString, TestHost,
    Transition, ValueRequest, View, ViewContext, actions, div, text,
};
use livspeil_ui_layout::{Point, Rect, Size, Vec2};
use livspeil_ui_theme::{MotionClass, Theme};

/// A push button: hover, press and keyboard focus animate with the
/// micro spring; Space/Enter activate it when focused. Add `.on_click`.
#[must_use]
fn button(theme: &Theme, label: impl Into<SharedString>) -> Element {
    let c = theme.colors();
    let label: SharedString = label.into();
    div()
        .name(format!("button:{label}"))
        .h(theme.metrics().row_height)
        .px(theme.tokens.spacing.step(3))
        .items_center()
        .justify_center()
        .rounded(theme.tokens.radii.sm)
        .bg(c.surface[2])
        .border(1.0, livspeil_ui_theme::Color::WHITE.with_alpha(0.06))
        .hover(|r| r.bg(c.surface[3]))
        .active(|r| r.bg(c.surface[3]).scale(0.97))
        .focus_visual(|r| r.border_color(c.focus_ring))
        .transition(MotionClass::Micro)
        .focusable()
        .role(Role::Button)
        .semantics(|s| s.label(label.clone()))
        .child(text(label).name("label").hit_test(false))
}

/// Toggle geometry.
const TRACK_W: f32 = 36.0;
const TRACK_H: f32 = 20.0;
const THUMB: f32 = 16.0;

/// A switch. The thumb (child key `thumb`) springs between positions with
/// the micro spring. Add `.on_click` to flip the value.
#[must_use]
fn toggle(theme: &Theme, on: bool) -> Element {
    let c = theme.colors();
    let travel = TRACK_W - THUMB - 4.0;
    div()
        .name("toggle")
        .size(TRACK_W, TRACK_H)
        .rounded(TRACK_H / 2.0)
        .bg(if on { theme.accent() } else { c.surface[3] })
        .transition(MotionClass::Micro)
        .focusable()
        .role(Role::Switch)
        .semantics(|s| s.checked(on))
        .child(
            div()
                .key("thumb")
                .absolute(Rect::new(2.0, 2.0, THUMB, THUMB))
                .rounded(THUMB / 2.0)
                .bg(livspeil_ui_theme::Color::WHITE)
                .translate(Vec2::new(if on { travel } else { 0.0 }, 0.0))
                .transition(MotionClass::Micro)
                .hit_test(false),
        )
}

/// A horizontal slider bound to a `Model<f32>`.
///
/// Pointer down jumps to the position; dragging follows the pointer 1:1
/// (no spring while dragging); on release the value snaps to `step` and the
/// thumb springs there carrying the release velocity. Arrow keys step,
/// Shift+arrows step ×10, Home/End go to the ends (key context `Slider`).
#[derive(Debug)]
struct Slider {
    /// The value.
    value: Model<f32>,
    /// Minimum.
    min: f32,
    /// Maximum.
    max: f32,
    /// Step (> 0).
    step: f32,
    /// Track width.
    width: f32,
    /// Accessible name.
    label: Option<SharedString>,
    dragging: bool,
}

/// Slider thumb size.
const SLIDER_THUMB: f32 = 16.0;

impl Slider {
    /// A slider over `min..=max` in steps of `step`, 200 px wide.
    #[must_use]
    fn new(value: Model<f32>, min: f32, max: f32, step: f32) -> Self {
        Slider {
            value,
            min,
            max: max.max(min),
            step: if step > 0.0 { step } else { 1.0 },
            width: 200.0,
            label: None,
            dragging: false,
        }
    }

    fn snap(&self, v: f32) -> f32 {
        let steps = ((v - self.min) / self.step).round();
        (self.min + steps * self.step).clamp(self.min, self.max)
    }

    fn fraction(&self, v: f32) -> f32 {
        let span = self.max - self.min;
        if span <= 0.0 {
            0.0
        } else {
            ((v - self.min) / span).clamp(0.0, 1.0)
        }
    }

    fn value_at(&self, x: f32, track: Rect) -> f32 {
        let usable = (track.width - SLIDER_THUMB).max(1.0);
        let f = ((x - track.x - SLIDER_THUMB / 2.0) / usable).clamp(0.0, 1.0);
        self.min + f * (self.max - self.min)
    }

    /// Thumb x offset for value `v`.
    #[must_use]
    fn thumb_offset(&self, v: f32) -> f32 {
        self.fraction(v) * (self.width - SLIDER_THUMB)
    }
}

fn set_value(slider: &View<Slider>, cx: &mut App, f: impl FnOnce(&Slider, f32) -> f32) {
    let (model, new) = {
        let s = slider.read(cx);
        let current = *s.value.read(cx);
        (s.value, f(s, current))
    };
    model.update(cx, |v, mcx| {
        if (*v - new).abs() > f32::EPSILON {
            *v = new;
            mcx.notify();
        }
    });
}

impl Render for Slider {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let value = *self.value.read(cx);
        let view = cx.view();
        let theme = cx.theme();
        let c = theme.colors();
        let accent = theme.accent();
        let offset = self.thumb_offset(value);
        let step_action = |delta: f32| {
            move |cx: &mut EventCx<'_>| {
                set_value(&view, cx, |s, v| s.snap(v + delta * s.step));
            }
        };
        div()
            .name("slider")
            .size(self.width, SLIDER_THUMB + 4.0)
            .key_context("Slider")
            .focusable()
            .on_pointer_down(move |ev, cx| {
                let track = cx.bounds();
                set_value(&view, cx, |s, _| s.snap(s.value_at(ev.position.x, track)));
            })
            .on_drag(move |ev, cx| {
                let track = cx.bounds();
                let thumb = cx.element_id().child("thumb");
                match ev.phase {
                    DragPhase::Start | DragPhase::Move => {
                        view.update(cx, |s, vcx| {
                            if !s.dragging {
                                s.dragging = true;
                                vcx.notify();
                            }
                        });
                        set_value(&view, cx, |s, _| s.value_at(ev.position.x, track));
                    }
                    DragPhase::End => {
                        view.update(cx, |s, vcx| {
                            s.dragging = false;
                            vcx.notify();
                        });
                        set_value(&view, cx, |s, v| s.snap(v));
                        cx.set_translate_velocity(thumb, Vec2::new(ev.velocity.x, 0.0));
                    }
                }
            })
            .on_action(actions::INCREMENT, step_action(1.0))
            .on_action(actions::DECREMENT, step_action(-1.0))
            .on_action(actions::INCREMENT_LARGE, step_action(10.0))
            .on_action(actions::DECREMENT_LARGE, step_action(-10.0))
            .on_action(actions::MIN, move |cx| set_value(&view, cx, |s, _| s.min))
            .on_action(actions::MAX, move |cx| set_value(&view, cx, |s, _| s.max))
            .role(Role::Slider)
            .semantics(|s| {
                let s = s.value(SemanticValue::number(
                    f64::from(value),
                    f64::from(self.min),
                    f64::from(self.max),
                    f64::from(self.step),
                ));
                match &self.label {
                    Some(l) => s.label(l.clone()),
                    None => s,
                }
            })
            .on_set_value(move |req, cx| {
                if let ValueRequest::Number(v) = req {
                    #[allow(clippy::cast_possible_truncation)]
                    let v = *v as f32;
                    set_value(&view, cx, |s, _| s.snap(v));
                }
            })
            .child(
                div()
                    .key("track")
                    .absolute(Rect::new(
                        0.0,
                        (SLIDER_THUMB + 4.0) / 2.0 - 2.0,
                        self.width,
                        4.0,
                    ))
                    .rounded(2.0)
                    .bg(c.surface[3])
                    .hit_test(false),
            )
            .child(
                div()
                    .key("thumb")
                    .absolute(Rect::new(0.0, 2.0, SLIDER_THUMB, SLIDER_THUMB))
                    .rounded(SLIDER_THUMB / 2.0)
                    .bg(accent)
                    .translate(Vec2::new(offset, 0.0))
                    .transition_with(
                        (!self.dragging).then_some(Transition::Motion(MotionClass::Micro)),
                    )
                    .hit_test(false),
            )
    }
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

struct Demo {
    count: Model<i32>,
    on: Model<bool>,
    slider: View<Slider>,
}

impl Render for Demo {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let (count, on) = (self.count, self.on);
        let n = *count.read(cx);
        let is_on = *on.read(cx);
        let theme = cx.theme().clone();
        div()
            .name("demo")
            .flex_col()
            .p(16.0)
            .gap(12.0)
            .child(text(format!("Count: {n}")).name("count"))
            .child(
                button(&theme, "Increment")
                    .key("inc")
                    .on_click(move |_, cx| {
                        count.update(cx, |c, cx| {
                            *c += 1;
                            cx.notify();
                        });
                    }),
            )
            .child(toggle(&theme, is_on).key("toggle").on_click(move |_, cx| {
                on.update(cx, |v, cx| {
                    *v = !*v;
                    cx.notify();
                });
            }))
            .child(Element::view(&self.slider))
    }
}

struct Fixture {
    host: TestHost,
    count: Model<i32>,
    on: Model<bool>,
    value: Model<f32>,
}

fn fixture() -> Fixture {
    let mut app = App::new();
    let count = app.new_model(0);
    let on = app.new_model(false);
    let value = app.new_model(50.0_f32);
    let slider = app.new_view(Slider::new(value, 0.0, 100.0, 5.0));
    let demo = app.new_view(Demo { count, on, slider });
    app.set_root(demo);
    let mut host = TestHost::new(app, Size::new(400.0, 300.0));
    host.run_until_idle(10);
    Fixture {
        host,
        count,
        on,
        value,
    }
}

fn center(h: &TestHost, name: &str) -> Point {
    let id = h.app.find(name).unwrap_or_else(|| panic!("no {name}"));
    h.app.bounds(id).unwrap().center()
}

#[test]
fn demo_snapshot() {
    let f = fixture();
    assert_golden(&golden_dir(), "demo-layout", &f.host.app.dump_tree());
    let scene = f.host.scene().unwrap().clone();
    assert_golden(&golden_dir(), "demo-scene", &f.host.app.dump_scene(&scene));
}

#[test]
fn button_hover_press_animate_and_click() {
    let mut f = fixture();
    let b = f.host.app.find("inc").unwrap();
    let rest = f.host.app.background(b).unwrap();
    let p = center(&f.host, "inc");

    f.host.pointer_move(p.x, p.y);
    assert!(f.host.app.is_hovered(b));
    f.host.step();
    let mid = f.host.app.background(b).unwrap();
    assert!(mid.l > rest.l, "hover starts animating in the same frame");
    let frames = f.host.run_until_idle(200);
    assert!(
        frames > 2 && frames < 60,
        "micro spring settles quickly: {frames}"
    );
    let hovered = f.host.app.background(b).unwrap();
    assert!((hovered.l - f.host.app.theme().colors().surface[3].l).abs() < 1e-3);

    f.host.pointer_down(p.x, p.y);
    assert!(f.host.app.is_pressed(b));
    f.host.step();
    let s = f.host.app.scale_of(b).unwrap();
    assert!(s < 1.0 && s > 0.97, "press scale springs: {s}");
    f.host.pointer_up(p.x, p.y);
    assert!(!f.host.app.is_pressed(b));
    f.host.run_until_idle(200);
    assert_eq!(*f.count.read(&f.host.app), 1);
    assert_eq!(f.host.app.scale_of(b), Some(1.0));
    assert!(f.host.app.dump_tree().contains("\"Count: 1\""));

    // Keyboard: Tab focuses it (ring), Space and Enter activate.
    f.host.app.blur();
    f.host.key("tab");
    assert_eq!(f.host.app.focused(), Some(b));
    f.host.key("space");
    f.host.key("enter");
    f.host.run_until_idle(200);
    assert_eq!(*f.count.read(&f.host.app), 3);

    // Leaving: back to rest, then idle.
    f.host.pointer_move(390.0, 290.0);
    f.host.run_until_idle(200);
    assert_eq!(f.host.app.background(b), Some(rest));
    assert!(!f.host.app.needs_frame());
}

#[test]
fn toggle_thumb_springs() {
    let mut f = fixture();
    let thumb = f.host.app.find("thumb").unwrap();
    let p = center(&f.host, "toggle");
    f.host.click(p.x, p.y);
    assert!(*f.on.read(&f.host.app));
    f.host.step();
    let x1 = f.host.app.translation(thumb).unwrap().x;
    assert!(x1 > 0.0 && x1 < 16.0, "moving: {x1}");
    f.host.step();
    let x2 = f.host.app.translation(thumb).unwrap().x;
    assert!(x2 > x1);
    // Interrupt mid-flight: retarget back keeps momentum (no jump).
    f.host.click(p.x, p.y);
    f.host.step();
    let x3 = f.host.app.translation(thumb).unwrap().x;
    assert!(
        (x3 - x2).abs() < 3.0,
        "continuous, no jump back: {x2} -> {x3}"
    );
    f.host.run_until_idle(300);
    assert_eq!(f.host.app.translation(thumb), Some(Vec2::ZERO));
}

#[test]
fn slider_drag_with_velocity_and_keyboard_steps() {
    let mut f = fixture();
    let slider = f.host.app.find("slider").unwrap();
    let track = f.host.app.bounds(slider).unwrap();
    let thumb = f
        .host
        .app
        .children_of(slider)
        .into_iter()
        .find(|c| f.host.app.label(*c).as_deref() == Some("thumb"))
        .unwrap();
    let usable = track.width - SLIDER_THUMB;
    let x_for = |v: f32| track.x + SLIDER_THUMB / 2.0 + usable * v / 100.0;
    let y = track.center().y;

    // Press jumps (snapped to the step).
    f.host.pointer_down(x_for(21.0), y);
    f.host.pointer_up(x_for(21.0), y);
    assert_eq!(*f.value.read(&f.host.app), 20.0);
    f.host.run_until_idle(300);

    // Drag: the thumb follows 1:1 while dragging (no spring lag).
    f.host.pointer_move(x_for(20.0), y);
    f.host.pointer_down(x_for(20.0), y);
    for i in 1..=10 {
        f.host.step();
        f.host.pointer_move(x_for(20.0 + i as f32 * 4.1), y);
    }
    f.host.step();
    let v = *f.value.read(&f.host.app);
    assert!((v - 61.0).abs() < 0.01, "unsnapped while dragging: {v}");
    let tx = f.host.app.translation(thumb).unwrap().x;
    assert!((tx - usable * 0.61).abs() < 0.05, "thumb at pointer: {tx}");
    // Release: value snaps to 60, thumb springs there with the release velocity
    // (keeps moving right first even though the target is to the left).
    f.host.pointer_up(x_for(61.0), y);
    assert_eq!(*f.value.read(&f.host.app), 60.0);
    f.host.step();
    let after = f.host.app.translation(thumb).unwrap().x;
    assert!(after > tx, "release velocity carried: {tx} -> {after}");
    f.host.run_until_idle(300);
    assert!((f.host.app.translation(thumb).unwrap().x - usable * 0.6).abs() < 0.05);

    // Keyboard (focused by the press): arrows step, shift ×10, home/end.
    assert_eq!(f.host.app.focused(), Some(slider));
    f.host.key("right");
    assert_eq!(*f.value.read(&f.host.app), 65.0);
    f.host.key("shift-left");
    assert_eq!(*f.value.read(&f.host.app), 15.0);
    f.host.key("end");
    assert_eq!(*f.value.read(&f.host.app), 100.0);
    f.host.key("right");
    assert_eq!(*f.value.read(&f.host.app), 100.0, "clamped");
    f.host.key("home");
    assert_eq!(*f.value.read(&f.host.app), 0.0);
    f.host.run_until_idle(300);
    assert!(!f.host.app.needs_frame());
}
