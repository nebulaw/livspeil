// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Buttons, toggles, check boxes, radio groups, segmented controls and the
//! slider driven headlessly.

mod common;

use common::{center, click, find, host};
use livspeil_ui_core::{Element, Model, Role, Toggled, View, div};
use livspeil_ui_layout::Vec2;
use livspeil_ui_widgets::{
    Button, CheckState, Checkbox, IconButton, RadioGroup, SEGMENT_INSET, SLIDER_THUMB,
    SegmentedControl, Slider, Toggle,
};

fn counter_button(count: Model<i32>, disabled: bool) -> common::Build {
    Box::new(move |cx| {
        let theme = cx.theme().clone();
        div()
            .flex_col()
            .p(16.0)
            .gap(12.0)
            .child(
                Button::new("Save")
                    .primary()
                    .disabled(disabled)
                    .on_click(move |_, cx| {
                        count.update(cx, |c, cx| {
                            *c += 1;
                            cx.notify();
                        });
                    })
                    .render(&theme),
            )
            .child(IconButton::new("bx-trash", "Delete").render(&theme))
    })
}

#[test]
fn button_click_keyboard_and_press_animation() {
    let mut count = None;
    let (mut h, _) = host(|app, _| {
        let c = app.new_model(0);
        count = Some(c);
        counter_button(c, false)
    });
    let count = count.unwrap();
    let b = find(&h, "button:Save");
    let rest = h.app.background(b).unwrap();
    let p = center(&h, "button:Save");

    h.pointer_move(p.x, p.y);
    h.step();
    assert!(h.app.background(b).unwrap().l > rest.l, "hover animates");
    h.pointer_down(p.x, p.y);
    h.step();
    let s = h.app.scale_of(b).unwrap();
    assert!(s < 1.0 && s > 0.97, "press scale springs: {s}");
    h.pointer_up(p.x, p.y);
    h.run_until_idle(200);
    assert_eq!(*count.read(&h.app), 1);
    assert_eq!(h.app.scale_of(b), Some(1.0));

    // Keyboard: Tab to it, Space and Enter activate.
    h.app.blur();
    h.key("tab");
    assert_eq!(h.app.focused(), Some(b));
    assert!(h.app.focus_visible());
    h.key("space");
    h.key("enter");
    assert_eq!(*count.read(&h.app), 3);
    // Tab moves on to the icon button.
    h.key("tab");
    assert_eq!(h.app.focused(), Some(find(&h, "icon-button:Delete")));

    // Semantics.
    let node = h.app.semantic_node(b).unwrap();
    assert_eq!(node.role(), Some(Role::Button));
    assert_eq!(node.label(), Some("Save"));
    let icon = h.app.semantic_node(find(&h, "icon-button:Delete")).unwrap();
    assert_eq!(icon.label(), Some("Delete"));
    h.run_until_idle(300);
    assert!(!h.app.needs_frame());
}

#[test]
fn disabled_button_ignores_input_and_is_skipped_by_tab() {
    let mut count = None;
    let (mut h, _) = host(|app, _| {
        let c = app.new_model(0);
        count = Some(c);
        counter_button(c, true)
    });
    let count = count.unwrap();
    click(&mut h, "button:Save");
    assert_eq!(*count.read(&h.app), 0);
    h.key("tab");
    assert_eq!(h.app.focused(), Some(find(&h, "icon-button:Delete")));
    let b = find(&h, "button:Save");
    assert!(h.app.semantic_node(b).unwrap().is_disabled());
}

#[test]
fn toggle_and_checkbox_flip_with_springs_and_mixed_state() {
    let mut models = None;
    let (mut h, _) = host(|app, _| {
        let on = app.new_model(false);
        let check = app.new_model(CheckState::Mixed);
        models = Some((on, check));
        Box::new(move |cx| {
            let theme = cx.theme().clone();
            let is_on = *on.read(cx);
            let state = *check.read(cx);
            div()
                .flex_col()
                .p(16.0)
                .gap(12.0)
                .child(
                    Toggle::new(is_on)
                        .label("Wi-Fi")
                        .on_change(move |v, cx| {
                            on.update(cx, |x, cx| {
                                *x = v;
                                cx.notify();
                            });
                        })
                        .render(&theme),
                )
                .child(
                    Checkbox::new(state)
                        .label("All")
                        .on_change(move |v, cx| {
                            check.update(cx, |x, cx| {
                                *x = v;
                                cx.notify();
                            });
                        })
                        .render(&theme),
                )
        })
    });
    let (on, check) = models.unwrap();
    let toggle = find(&h, "toggle");
    let thumb = find(&h, "thumb");
    click(&mut h, "toggle");
    assert!(*on.read(&h.app));
    let travel = h.app.translation(thumb).unwrap().x;
    assert!(travel > 10.0, "thumb at the end: {travel}");
    // Mid-flight retarget keeps momentum (no jump).
    let p = center(&h, "toggle");
    h.click(p.x, p.y);
    h.step();
    h.step();
    let x1 = h.app.translation(thumb).unwrap().x;
    h.click(p.x, p.y);
    h.step();
    let x2 = h.app.translation(thumb).unwrap().x;
    assert!((x2 - x1).abs() < 4.0, "continuous: {x1} -> {x2}");
    h.run_until_idle(300);
    assert!(*on.read(&h.app));
    assert_eq!(
        h.app.semantic_node(toggle).unwrap().semantics.unwrap().get_checked(),
        Some(Toggled::On)
    );

    // Check box: mixed -> checked -> unchecked; keyboard too.
    let cb = find(&h, "checkbox");
    assert_eq!(
        h.app.semantic_node(cb).unwrap().semantics.unwrap().get_checked(),
        Some(Toggled::Mixed)
    );
    click(&mut h, "checkbox");
    assert_eq!(*check.read(&h.app), CheckState::Checked);
    let mark = find(&h, "mark");
    assert_eq!(h.app.scale_of(mark), Some(1.0));
    h.app.focus(cb);
    h.key("space");
    assert_eq!(*check.read(&h.app), CheckState::Unchecked);
    h.step();
    let s = h.app.scale_of(mark).unwrap();
    assert!(s < 1.0 && s > 0.6, "mark springs out: {s}");
    h.run_until_idle(300);
    assert!(!h.app.needs_frame());
}

fn radio_host(disabled_option: Option<usize>) -> (livspeil_ui_core::TestHost, Model<Option<usize>>) {
    let mut model = None;
    let (h, _) = host(|app, _| {
        let sel = app.new_model(None::<usize>);
        model = Some(sel);
        Box::new(move |cx| {
            let theme = cx.theme().clone();
            let mut g = RadioGroup::new(["Small", "Medium", "Large"], *sel.read(cx))
                .label("Size")
                .on_change(move |i, cx| {
                    sel.update(cx, |x, cx| {
                        *x = Some(i);
                        cx.notify();
                    });
                });
            if let Some(d) = disabled_option {
                g = g.disable_option(d);
            }
            div().p(16.0).child(g.render(&theme))
        })
    });
    (h, model.unwrap())
}

#[test]
fn radio_group_arrow_keys_wrap_and_skip_disabled() {
    let (mut h, sel) = radio_host(Some(1));
    // One tab stop: the group.
    key(&mut h, "tab");
    let group = find(&h, "radio-group");
    assert_eq!(h.app.focused(), Some(group));
    key(&mut h, "tab");
    assert_eq!(h.app.focused(), Some(group), "only tab stop");
    // Space selects the first when none is selected.
    key(&mut h, "space");
    assert_eq!(*sel.read(&h.app), Some(0));
    key(&mut h, "down");
    assert_eq!(*sel.read(&h.app), Some(2), "skips the disabled option");
    key(&mut h, "down");
    assert_eq!(*sel.read(&h.app), Some(0), "wraps");
    key(&mut h, "up");
    assert_eq!(*sel.read(&h.app), Some(2));
    key(&mut h, "home");
    assert_eq!(*sel.read(&h.app), Some(0));
    key(&mut h, "end");
    assert_eq!(*sel.read(&h.app), Some(2));
    // Clicking a disabled option does nothing; an enabled one selects.
    click(&mut h, "radio:Medium");
    assert_eq!(*sel.read(&h.app), Some(2));
    click(&mut h, "radio:Small");
    assert_eq!(*sel.read(&h.app), Some(0));
    let small = find(&h, "radio:Small");
    let s = h.app.semantic_node(small).unwrap().semantics.unwrap().clone();
    assert_eq!(s.get_checked(), Some(Toggled::On));
    assert_eq!(s.get_selected(), Some(true));
    assert_eq!(h.app.semantic_node(group).unwrap().role(), Some(Role::Group));
}

fn segmented_host() -> (livspeil_ui_core::TestHost, Model<usize>) {
    let mut model = None;
    let (h, _) = host(|app, _| {
        let sel = app.new_model(0usize);
        model = Some(sel);
        Box::new(move |cx| {
            let theme = cx.theme().clone();
            div().p(16.0).child(
                SegmentedControl::new(["Day", "Week", "Month", "Year"], *sel.read(cx))
                    .on_change(move |i, cx| {
                        sel.update(cx, |x, cx| {
                            *x = i;
                            cx.notify();
                        });
                    })
                    .render(&theme),
            )
        })
    });
    (h, model.unwrap())
}

#[test]
fn segmented_pill_springs_and_settles() {
    let (mut h, sel) = segmented_host();
    let pill = find(&h, "pill");
    let w = h.app.bounds(find(&h, "segment:Day")).unwrap().width;
    assert_eq!(h.app.translation(pill), Some(Vec2::ZERO));
    click_no_settle(&mut h, "segment:Month");
    assert_eq!(*sel.read(&h.app), 2);
    let target = 2.0 * w;
    let mut xs = Vec::new();
    for _ in 0..120 {
        if h.step().is_none() {
            break;
        }
        xs.push(h.app.translation(pill).unwrap().x);
    }
    let frames = xs.len();
    assert!(
        frames > 5 && frames < 90,
        "standard spring settles in a fraction of a second: {frames}"
    );
    assert!(xs[0] > 0.0 && xs[0] < target, "moves from the first frame");
    let peak = xs.iter().copied().fold(0.0_f32, f32::max);
    assert!(peak - target < 0.05 * target, "little overshoot: {peak}");
    assert!((h.app.translation(pill).unwrap().x - target).abs() < 1e-3);
    assert!(!h.app.needs_frame());
    // Pill is inset like the segments.
    let seg = h.app.bounds(find(&h, "segment:Day")).unwrap();
    assert_eq!(h.app.bounds(pill).unwrap().x, seg.x);
    let _ = SEGMENT_INSET;

    // Retargeting mid-flight keeps velocity: moving right, then back to 0,
    // the pill keeps going right for a moment.
    click_no_settle(&mut h, "segment:Year");
    h.step();
    h.step();
    let a = h.app.translation(pill).unwrap().x;
    click_no_settle(&mut h, "segment:Day");
    h.step();
    let b = h.app.translation(pill).unwrap().x;
    assert!(
        b > 0.5 * a && b <= a + 0.25 * w,
        "continuous retarget, no jump back: {a} -> {b}"
    );
    h.run_until_idle(300);
    assert_eq!(h.app.translation(pill), Some(Vec2::ZERO));

    // Keyboard: one tab stop, arrows (no wrap), Home/End.
    key(&mut h, "tab");
    key(&mut h, "left");
    assert_eq!(*sel.read(&h.app), 0);
    key(&mut h, "right");
    key(&mut h, "right");
    assert_eq!(*sel.read(&h.app), 2);
    key(&mut h, "end");
    assert_eq!(*sel.read(&h.app), 3);
    key(&mut h, "right");
    assert_eq!(*sel.read(&h.app), 3);
    key(&mut h, "home");
    assert_eq!(*sel.read(&h.app), 0);
    let tab = find(&h, "segment:Day");
    assert_eq!(h.app.semantic_node(tab).unwrap().role(), Some(Role::Tab));
}

/// A key press followed by a frame (builders capture state at render).
fn key(h: &mut livspeil_ui_core::TestHost, k: &str) {
    h.key(k);
    h.step();
}

fn click_no_settle(h: &mut livspeil_ui_core::TestHost, name: &str) {
    let p = center(h, name);
    h.click(p.x, p.y);
}

#[test]
fn slider_drag_with_velocity_keyboard_and_value_label() {
    let mut models = None;
    let (mut h, _) = host(|app, _| {
        let value = app.new_model(50.0_f32);
        let slider = app.new_view(
            Slider::new(value, 0.0, 100.0, 5.0)
                .label("Volume")
                .show_value(),
        );
        models = Some((value, slider));
        Box::new(move |_| div().p(16.0).child(Element::view(&slider)))
    });
    let (value, _slider): (Model<f32>, View<Slider>) = models.unwrap();
    let s = find(&h, "slider");
    let track = h.app.bounds(s).unwrap();
    let thumb = h
        .app
        .children_of(s)
        .into_iter()
        .find(|c| h.app.label(*c).as_deref() == Some("thumb"))
        .unwrap();
    let usable = track.width - SLIDER_THUMB;
    let x_for = |v: f32| track.x + SLIDER_THUMB / 2.0 + usable * v / 100.0;
    let y = track.center().y;

    h.pointer_down(x_for(21.0), y);
    h.pointer_up(x_for(21.0), y);
    assert_eq!(*value.read(&h.app), 20.0);
    h.run_until_idle(300);
    assert!(h.app.dump_tree().contains("\"20\""), "value label");

    h.pointer_move(x_for(20.0), y);
    h.pointer_down(x_for(20.0), y);
    for i in 1..=10 {
        h.step();
        #[allow(clippy::cast_precision_loss)]
        h.pointer_move(x_for(20.0 + i as f32 * 4.1), y);
    }
    h.step();
    assert!((*value.read(&h.app) - 61.0).abs() < 0.01);
    let tx = h.app.translation(thumb).unwrap().x;
    assert!((tx - usable * 0.61).abs() < 0.05, "thumb follows 1:1: {tx}");
    h.pointer_up(x_for(61.0), y);
    assert_eq!(*value.read(&h.app), 60.0);
    h.step();
    assert!(h.app.translation(thumb).unwrap().x > tx, "release velocity");
    h.run_until_idle(300);

    assert_eq!(h.app.focused(), Some(s));
    h.key("right");
    assert_eq!(*value.read(&h.app), 65.0);
    h.key("shift-left");
    assert_eq!(*value.read(&h.app), 15.0);
    h.key("end");
    h.key("right");
    assert_eq!(*value.read(&h.app), 100.0);
    h.key("home");
    assert_eq!(*value.read(&h.app), 0.0);

    // Assistive technology sets the value (snapped).
    h.app
        .perform_semantic_action(
            s,
            livspeil_ui_core::SemanticRequest::SetValue(livspeil_ui_core::ValueRequest::Number(
                42.0,
            )),
        )
        .unwrap();
    assert_eq!(*value.read(&h.app), 40.0);
    let node = h.app.semantic_node(s).unwrap();
    assert_eq!(node.role(), Some(Role::Slider));
    assert_eq!(node.label(), Some("Volume"));
    h.run_until_idle(300);
    assert!(!h.app.needs_frame());
}
