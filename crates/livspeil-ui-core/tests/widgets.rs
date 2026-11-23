// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Button, Toggle and Slider driven headlessly.

use std::path::{Path, PathBuf};

use livspeil_testkit::assert_golden;
use livspeil_ui_core::widgets::{SLIDER_THUMB, Slider, button, toggle};
use livspeil_ui_core::{App, Element, Model, Render, TestHost, View, ViewContext, div, text};
use livspeil_ui_layout::{Point, Size, Vec2};

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
