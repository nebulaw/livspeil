// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! NumberField: scrub math, scrubbing with modifiers, typing, stepping.

mod common;

use common::{ALT, center, down, find, host, moved, up};
use livspeil_ui_core::{Element, Model, Modifiers, Role, TestHost, View, div};
use livspeil_ui_layout::Point;
use livspeil_ui_widgets::{NumberField, Scrub, scrub_precision, step_decimals};

const SHIFT: Modifiers = Modifiers::SHIFT;
const NONE: Modifiers = Modifiers::NONE;

#[test]
fn scrub_math_precision_clamping_and_rounding() {
    assert_eq!(scrub_precision(NONE), 1.0);
    assert_eq!(scrub_precision(SHIFT), 10.0);
    assert_eq!(scrub_precision(ALT), 0.1);
    let both = Modifiers {
        shift: true,
        alt: true,
        ..NONE
    };
    assert_eq!(scrub_precision(both), 1.0);

    let s = Scrub {
        min: -10.0,
        max: 10.0,
        step: 0.5,
        sensitivity: 0.5,
    };
    // 7 px at ×1 = 3.5.
    assert_eq!(s.advance(0.0, 7.0, NONE), 3.5);
    // Shift ×10: 1 px = 5.
    assert_eq!(s.advance(0.0, 1.0, SHIFT), 5.0);
    // Alt ×0.1: 3 px = 0.15, shown rounded to step×0.1 = 0.05.
    let raw = s.advance(0.0, 3.0, ALT);
    assert!((raw - 0.15).abs() < 1e-12);
    assert_eq!(s.quantize(raw, ALT), 0.15);
    // Without Alt the same raw value rounds to the step.
    assert_eq!(s.quantize(raw, NONE), 0.0);
    assert_eq!(s.quantize(0.3, NONE), 0.5);
    // Clamping: the raw value stops at the bounds, so reversing responds at once.
    let raw = s.advance(9.0, 100.0, NONE);
    assert_eq!(raw, 10.0);
    assert_eq!(s.advance(raw, -2.0, NONE), 9.0);
    assert_eq!(s.advance(-9.0, -100.0, SHIFT), -10.0);
    // No float noise.
    let s = Scrub {
        min: f64::NEG_INFINITY,
        max: f64::INFINITY,
        step: 0.1,
        sensitivity: 0.1,
    };
    assert_eq!(s.quantize(s.advance(0.1, 2.0, NONE), NONE), 0.3);
    assert_eq!(step_decimals(1.0), 0);
    assert_eq!(step_decimals(0.25), 2);
    assert_eq!(step_decimals(0.001), 3);
}

struct Fx {
    h: TestHost,
    value: Model<f64>,
    field: View<NumberField>,
    name: String,
}

fn fixture(min: f64, max: f64, step: f64, initial: f64) -> Fx {
    let mut out = None;
    let (h, _) = host(|app, env| {
        let value = app.new_model(initial);
        let field = app.new_view(NumberField::new(env, value, min, max, step).label("X"));
        out = Some((value, field));
        Box::new(move |_| {
            div()
                .flex_col()
                .p(16.0)
                .gap(8.0)
                .child(Element::view(&field))
                .child(
                    livspeil_ui_core::div()
                        .name("other")
                        .size(40.0, 20.0)
                        .focusable(),
                )
        })
    });
    let (value, field) = out.unwrap();
    let name = NumberField::element_name(&field, &h.app);
    Fx {
        h,
        value,
        field,
        name,
    }
}

fn v(f: &Fx) -> f64 {
    *f.value.read(&f.h.app)
}

#[test]
fn scrub_drags_with_modifiers_and_clamps() {
    let mut f = fixture(-100.0, 100.0, 1.0, 0.0);
    let c = center(&f.h, &f.name);
    down(&mut f.h, c, NONE);
    // Below the threshold: nothing.
    moved(&mut f.h, Point::new(c.x + 2.0, c.y), NONE);
    assert_eq!(v(&f), 0.0);
    assert!(!f.field.read(&f.h.app).is_scrubbing());
    // Past it: 1 px = 1 step, counted from the press.
    moved(&mut f.h, Point::new(c.x + 10.0, c.y), NONE);
    assert!(f.field.read(&f.h.app).is_scrubbing());
    assert_eq!(v(&f), 10.0);
    // Shift ×10.
    moved(&mut f.h, Point::new(c.x + 12.0, c.y), SHIFT);
    assert_eq!(v(&f), 30.0);
    // Alt ×0.1 with finer rounding.
    moved(&mut f.h, Point::new(c.x + 15.0, c.y), ALT);
    assert!((v(&f) - 30.3).abs() < 1e-9, "{}", v(&f));
    // Releasing Alt rounds back to the step.
    moved(&mut f.h, Point::new(c.x + 16.0, c.y), NONE);
    assert_eq!(v(&f), 31.0);
    // Vertical movement does nothing; leaving the field still scrubs.
    moved(&mut f.h, Point::new(c.x + 16.0, c.y + 50.0), NONE);
    assert_eq!(v(&f), 31.0);
    moved(&mut f.h, Point::new(c.x + 216.0, c.y + 50.0), SHIFT);
    assert_eq!(v(&f), 100.0, "clamped");
    moved(&mut f.h, Point::new(c.x + 214.0, c.y), NONE);
    assert_eq!(v(&f), 98.0, "reverses at once");
    up(&mut f.h, Point::new(c.x + 214.0, c.y), NONE);
    assert!(
        !f.field.read(&f.h.app).is_editing(),
        "a scrub does not edit"
    );
    f.h.run_until_idle(100);
    assert!(f.h.app.dump_tree().contains("\"98\""));
}

#[test]
fn click_to_type_enter_escape_and_blur() {
    let mut f = fixture(0.0, 10.0, 0.5, 2.5);
    let c = center(&f.h, &f.name);
    f.h.click(c.x, c.y);
    f.h.run_until_idle(100);
    assert_eq!(f.field.read(&f.h.app).edit_text(), Some("2.5"));
    let ed = f.field.read(&f.h.app);
    assert_eq!(ed.edit_text(), Some("2.5"));
    // Typing replaces the selected text; Enter commits (clamped, not snapped).
    f.h.type_text("7.25");
    f.h.key("enter");
    f.h.run_until_idle(100);
    assert_eq!(v(&f), 7.25);
    assert!(!f.field.read(&f.h.app).is_editing());
    assert!(f.h.app.dump_tree().contains("\"7.25\""));
    // Out of range clamps.
    f.h.key("enter");
    f.h.step();
    f.h.type_text("99");
    f.h.key("enter");
    assert_eq!(v(&f), 10.0);
    // Escape reverts.
    f.h.step();
    f.h.type_text("3");
    assert!(
        f.field.read(&f.h.app).is_editing(),
        "a digit starts editing"
    );
    f.h.step();
    f.h.key("escape");
    assert_eq!(v(&f), 10.0);
    assert!(!f.field.read(&f.h.app).is_editing());
    // Invalid text reverts.
    f.h.step();
    f.h.key("space");
    f.h.step();
    f.h.type_text("abc");
    f.h.key("enter");
    assert_eq!(v(&f), 10.0);
    // Tab commits and moves focus.
    f.h.step();
    f.h.type_text("4");
    f.h.step();
    f.h.key("tab");
    f.h.run_until_idle(100);
    assert_eq!(v(&f), 4.0);
    assert_eq!(f.h.app.focused(), Some(find(&f.h, "other")));
    // Losing focus by clicking elsewhere commits.
    f.h.click(c.x, c.y);
    f.h.run_until_idle(100);
    f.h.type_text("6");
    let o = center(&f.h, "other");
    f.h.click(o.x, o.y);
    f.h.run_until_idle(100);
    assert_eq!(v(&f), 6.0);
    assert!(!f.field.read(&f.h.app).is_editing());
}

#[test]
fn keyboard_steps_and_semantics() {
    let mut f = fixture(0.0, 10.0, 1.0, 5.0);
    let id = find(&f.h, &f.name);
    f.h.key("tab");
    assert_eq!(f.h.app.focused(), Some(id));
    for (k, want) in [
        ("up", 6.0),
        ("down", 5.0),
        ("shift-up", 10.0),
        ("down", 9.0),
        ("alt-down", 8.9),
        ("alt-up", 9.0),
        ("home", 0.0),
        ("alt-up", 0.1),
        ("end", 10.0),
        ("shift-down", 0.0),
    ] {
        f.h.key(k);
        f.h.step();
        assert!((v(&f) - want).abs() < 1e-9, "{k}: {} != {want}", v(&f));
    }
    let node = f.h.app.semantic_node(id).unwrap();
    assert_eq!(node.role(), Some(Role::SpinButton));
    assert_eq!(node.label(), Some("X"));
    f.h.app
        .perform_semantic_action(
            id,
            livspeil_ui_core::SemanticRequest::SetValue(livspeil_ui_core::ValueRequest::Number(
                3.5,
            )),
        )
        .unwrap();
    assert_eq!(v(&f), 3.5);
    let s = f.field.read(&f.h.app).semantics(&f.h.app);
    assert_eq!(s.value.as_deref(), Some("3.5"));
    f.h.run_until_idle(100);
    assert!(!f.h.app.needs_frame());
}
