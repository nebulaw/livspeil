// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Scene snapshots (text dumps of paint commands) of every widget in its
//! states, in dark/Simple and light/Pro. Refresh with `UPDATE_GOLDEN=1`.

mod common;

use common::{center, find, golden_dir, host, scene, wait};
use livspeil_testkit::assert_golden;
use livspeil_ui_core::{App, Element, ImeEvent, TestHost, div};
use livspeil_ui_theme::{Density, Scheme, Theme};
use livspeil_ui_widgets::{
    Button, CheckState, Checkbox, IconButton, Label, NumberField, RadioGroup, SegmentedControl,
    Select, Slider, TextField, Toggle, Tooltip,
};

struct Gallery {
    h: TestHost,
}

fn gallery(scheme: Scheme, density: Density) -> Gallery {
    let (mut h, _) = host(|app: &mut App, env| {
        app.set_theme(Theme {
            scheme,
            density,
            ..Theme::default()
        });
        let value = app.new_model(30.0_f32);
        let slider = app.new_view(
            Slider::new(value, 0.0, 100.0, 5.0)
                .width(120.0)
                .show_value(),
        );
        let num = app.new_model(1.25_f64);
        let number = app.new_view(
            NumberField::new(env, num, 0.0, 10.0, 0.25)
                .label("X")
                .name("number"),
        );
        let text = app.new_view(
            TextField::new(env, "")
                .placeholder("Search")
                .width(140.0)
                .name("text"),
        );
        let sel = app.new_model(Some(0));
        let select = app.new_view(
            Select::new(env, sel, ["Linear", "Ease", "Spring"])
                .width(120.0)
                .name("select"),
        );
        let env = env.clone();
        Box::new(move |cx| {
            let theme = cx.theme().clone();
            let row = || div().flex_row().items_center().gap(8.0);
            div()
                .name("gallery")
                .flex_col()
                .p(12.0)
                .gap(8.0)
                .child(
                    row()
                        .child(Button::new("Save").primary().render(&theme))
                        .child(Button::new("Cancel").render(&theme))
                        .child(Button::new("More").ghost().render(&theme))
                        .child(Button::new("Delete").destructive().render(&theme))
                        .child(Button::new("Off").disabled(true).render(&theme))
                        .child(
                            Tooltip::new("Trash")
                                .attach(&env, IconButton::new("bx-trash", "Trash").render(&theme)),
                        ),
                )
                .child(
                    row()
                        .child(Toggle::new(true).label("On").render(&theme))
                        .child(Toggle::new(false).disabled(true).render(&theme))
                        .child(Checkbox::new(CheckState::Checked).render(&theme))
                        .child(
                            Checkbox::new(CheckState::Mixed)
                                .label("Mixed")
                                .render(&theme),
                        )
                        .child(Checkbox::new(CheckState::Unchecked).render(&theme)),
                )
                .child(
                    row()
                        .child(
                            RadioGroup::new(["A", "B"], Some(1))
                                .horizontal()
                                .disable_option(0)
                                .render(&theme),
                        )
                        .child(SegmentedControl::new(["Day", "Week"], 1).render(&theme)),
                )
                .child(
                    row()
                        .child(Element::view(&slider))
                        .child(Element::view(&number)),
                )
                .child(
                    row()
                        .child(Element::view(&text))
                        .child(Element::view(&select))
                        .child(Label::new("Label").secondary().render(&theme)),
                )
        })
    });
    h.run_until_idle(100);
    Gallery { h }
}

fn snap(h: &mut TestHost, name: &str) {
    h.run_until_idle(300);
    assert!(!h.app.needs_frame(), "{name}: settled");
    assert_golden(&golden_dir(), name, &scene(h));
}

#[test]
fn gallery_dark_simple() {
    let mut g = gallery(Scheme::Dark, Density::Simple);
    snap(&mut g.h, "gallery-dark-simple");
}

#[test]
fn gallery_light_pro() {
    let mut g = gallery(Scheme::Light, Density::Pro);
    snap(&mut g.h, "gallery-light-pro");
}

#[test]
fn interaction_states() {
    let mut g = gallery(Scheme::Dark, Density::Simple);
    let h = &mut g.h;
    // Hovered primary button.
    let p = center(h, "button:Save");
    h.pointer_move(p.x, p.y);
    snap(h, "state-button-hover");
    // Keyboard focus ring on the Cancel button.
    h.pointer_move(470.0, 350.0);
    h.app.focus(find(h, "button:Save"));
    h.key("tab");
    snap(h, "state-button-focus");
    // Text field: focused with a selection, then composing.
    let t = center(h, "text");
    h.click(t.x, t.y);
    h.run_until_idle(50);
    h.type_text("hello world");
    h.key("ctrl-shift-left");
    snap(h, "state-textfield-selection");
    h.key("end");
    h.ime(ImeEvent::Preedit {
        text: "日本".to_owned(),
        cursor: Some((0, 6)),
    });
    snap(h, "state-textfield-ime");
    h.ime(ImeEvent::Disabled);
    // Number field editing.
    let n = center(h, "number");
    h.click(n.x, n.y);
    snap(h, "state-numberfield-editing");
    h.key("escape");
    // Select open.
    let s = center(h, "select");
    h.click(s.x, s.y);
    h.key("down");
    snap(h, "state-select-open");
    h.key("escape");
    h.run_until_idle(100);
    // Tooltip shown.
    let i = center(h, "icon-button:Trash");
    h.pointer_move(i.x, i.y);
    wait(h, 600);
    snap(h, "state-tooltip");
}
