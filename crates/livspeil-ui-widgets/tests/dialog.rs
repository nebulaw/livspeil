// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Dialog and sheet: modality, focus trap, keys, buttons, motion.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{center, find, golden_dir, host_sized, scene};
use livspeil_testkit::assert_golden;
use livspeil_ui_core::{Element, Role, TestHost, View, div, text};
use livspeil_ui_layout::Size;
use livspeil_ui_theme::{MotionConfig, Scheme, Theme};
use livspeil_ui_widgets::{Button, Dialog, DialogButton};

struct Fx {
    h: TestHost,
    dlg: View<Dialog>,
    log: Rc<RefCell<Vec<&'static str>>>,
}

fn fixture(scheme: Scheme, sheet: bool, cancel: bool) -> Fx {
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let mut out = None;
    let (h, _) = host_sized(Size::new(520.0, 380.0), |app, env| {
        app.set_theme(Theme {
            scheme,
            ..Theme::default()
        });
        let mut d = Dialog::new(env, "Delete \u{201c}Living room\u{201d}?").message(
            "This removes the room and every Charm in it. You can undo this from the History tab.",
        );
        if sheet {
            d = d.sheet();
        }
        let (l1, l2, l3) = (l.clone(), l.clone(), l.clone());
        if cancel {
            d = d.button(
                DialogButton::new("Cancel")
                    .cancel()
                    .on_press(move |_| l1.borrow_mut().push("cancel")),
            );
        }
        d = d
            .button(DialogButton::new("Keep both").on_press(move |_| l2.borrow_mut().push("keep")))
            .button(
                DialogButton::new("Delete")
                    .destructive()
                    .default_button()
                    .on_press(move |_| l3.borrow_mut().push("delete")),
            );
        let dlg = app.new_view(d);
        out = Some(dlg);
        Box::new(move |cx| {
            let theme = cx.theme().clone();
            div()
                .w_full()
                .h_full()
                .flex_col()
                .p(24.0)
                .gap(12.0)
                .child(
                    Button::new("Open")
                        .on_click(move |_, cx| Dialog::open(dlg, cx, None))
                        .render(&theme),
                )
                .child(
                    Button::new("Behind")
                        .on_click(move |_, _| {})
                        .render(&theme),
                )
                .child(text("Background content").font_size(13.0))
                .child(Element::view(&dlg))
        })
    });
    Fx {
        h,
        dlg: out.unwrap(),
        log,
    }
}

impl Fx {
    fn open(&mut self) {
        let p = center(&self.h, "button:Open");
        self.h.click(p.x, p.y);
        self.h.run_until_idle(200);
    }
    fn is_open(&self) -> bool {
        self.dlg.read(&self.h.app).is_open()
    }
    fn log(&self) -> Vec<&'static str> {
        self.log.borrow().clone()
    }
}

#[test]
fn opens_centered_focuses_the_default_and_fades_in_without_scale() {
    let mut f = fixture(Scheme::Dark, false, true);
    assert!(f.h.app.find("dialog").is_none());
    let p = center(&f.h, "button:Open");
    f.h.click(p.x, p.y);
    f.h.step();
    f.h.step();
    let panel = find(&f.h, "dialog");
    assert!(
        f.h.app.opacity_of(panel).unwrap() < 1.0,
        "starts transparent"
    );
    assert_eq!(f.h.app.scale_of(panel), Some(1.0));
    f.h.run_until_idle(200);
    assert_eq!(f.h.app.opacity_of(panel), Some(1.0));
    assert_eq!(f.h.app.scale_of(panel), Some(1.0), "never scales");
    assert!(!f.h.app.needs_frame());
    let b = f.h.app.bounds(panel).unwrap();
    assert!((b.center().x - 260.0).abs() < 1.0, "centered: {b:?}");
    assert!(b.y > 20.0 && b.bottom() < 360.0);
    assert_eq!(
        f.h.app.focused(),
        Some(find(&f.h, "button:Delete")),
        "the default button has focus"
    );
    let scrim = f.h.app.bounds(find(&f.h, "dialog-scrim")).unwrap();
    assert_eq!((scrim.width, scrim.height), (520.0, 380.0));
    // The destructive button is filled with the danger color.
    let bg = f.h.app.background(find(&f.h, "button:Delete")).unwrap();
    assert_eq!(bg, f.h.app.theme().colors().danger);
}

#[test]
fn it_is_modal_and_traps_focus() {
    let mut f = fixture(Scheme::Dark, false, true);
    f.open();
    // Clicks on the content behind go to the scrim, not the button.
    let behind = center(&f.h, "button:Behind");
    let scrim = find(&f.h, "dialog-scrim");
    if f.h.app.hit_test(behind) != Some(scrim) {
        // The dialog panel may cover it; either way it is not the button.
        assert_ne!(f.h.app.hit_test(behind), Some(find(&f.h, "button:Behind")));
    }
    f.h.click(behind.x, behind.y);
    f.h.run_until_idle(100);
    assert!(f.is_open(), "a scrim press does not dismiss a dialog");
    // Tab cycles Cancel -> Keep both -> Delete -> Cancel, never leaving.
    let names = ["button:Cancel", "button:Keep both", "button:Delete"];
    let mut seen = Vec::new();
    for _ in 0..6 {
        f.h.key("tab");
        let id = f.h.app.focused().unwrap();
        let i = names.iter().position(|n| find(&f.h, n) == id);
        assert!(i.is_some(), "focus stayed inside the dialog");
        seen.push(i.unwrap());
    }
    assert_eq!(seen, [0, 1, 2, 0, 1, 2]);
    f.h.key("shift-tab");
    assert_eq!(f.h.app.focused(), Some(find(&f.h, "button:Keep both")));
    assert!(f.log().is_empty());
}

#[test]
fn enter_and_escape() {
    // Enter on the default button deletes; focus returns to Open.
    let mut f = fixture(Scheme::Dark, false, true);
    f.open();
    f.h.key("enter");
    assert_eq!(f.log(), ["delete"]);
    assert!(!f.is_open());
    assert_eq!(f.h.app.focused(), Some(find(&f.h, "button:Open")));
    f.h.run_until_idle(200);
    assert!(f.h.app.find("dialog").is_none());
    // Enter while another button is focused presses that one.
    f.open();
    f.h.key("shift-tab");
    f.h.key("shift-tab");
    assert_eq!(f.h.app.focused(), Some(find(&f.h, "button:Cancel")));
    f.h.key("enter");
    assert_eq!(f.log(), ["delete", "cancel"]);
    f.h.run_until_idle(200);
    // Space activates the focused button too.
    f.open();
    f.h.key("shift-tab");
    f.h.key("space");
    assert_eq!(f.log(), ["delete", "cancel", "keep"]);
    f.h.run_until_idle(200);
    // Escape presses Cancel.
    f.open();
    f.h.key("escape");
    assert_eq!(f.log(), ["delete", "cancel", "keep", "cancel"]);
    assert_eq!(f.h.app.focused(), Some(find(&f.h, "button:Open")));
    f.h.run_until_idle(200);
    // Enter with focus on the panel (clicked text) presses the default.
    f.open();
    let t = center(&f.h, "dialog-message");
    f.h.click(t.x, t.y);
    assert_eq!(f.h.app.focused(), Some(find(&f.h, "dialog")));
    f.h.key("enter");
    assert_eq!(f.log().last(), Some(&"delete"));
}

#[test]
fn escape_without_a_cancel_button_just_closes() {
    let mut f = fixture(Scheme::Dark, false, false);
    f.open();
    f.h.key("escape");
    assert!(!f.is_open());
    assert!(f.log().is_empty());
}

#[test]
fn a_leaving_dialog_fades_out_and_stops_taking_input() {
    let mut f = fixture(Scheme::Dark, false, true);
    f.open();
    let behind = center(&f.h, "button:Behind");
    f.h.key("escape");
    f.h.step();
    f.h.step();
    let panel = find(&f.h, "dialog");
    let o = f.h.app.opacity_of(panel).unwrap();
    assert!(o < 1.0, "fading out: {o}");
    assert_eq!(
        f.h.app.hit_test(behind),
        Some(find(&f.h, "button:Behind")),
        "no scrim while leaving"
    );
    f.h.run_until_idle(200);
    assert!(f.h.app.find("dialog").is_none());
    assert!(!f.h.app.needs_frame());
}

#[test]
fn sheets_hang_from_the_top_and_slide_without_scale() {
    let mut f = fixture(Scheme::Dark, true, true);
    let p = center(&f.h, "button:Open");
    f.h.click(p.x, p.y);
    f.h.step();
    f.h.step();
    let sheet = find(&f.h, "sheet");
    let t = f.h.app.translation(sheet).unwrap();
    assert!(t.y < 0.0, "starts above the window: {t:?}");
    assert_eq!(f.h.app.scale_of(sheet), Some(1.0));
    f.h.run_until_idle(300);
    let b = f.h.app.bounds(sheet).unwrap();
    assert_eq!(b.y, 0.0, "attached to the top edge");
    assert!((b.center().x - 260.0).abs() < 1.0);
    assert_eq!(f.h.app.translation(sheet).unwrap().y, 0.0);
    assert_eq!(f.h.app.scale_of(sheet), Some(1.0));
    // Reduce Motion: no slide, the fade stays.
    f.h.key("escape");
    f.h.run_until_idle(300);
    f.h.app.set_motion(MotionConfig {
        reduce_motion: true,
        ..MotionConfig::default()
    });
    let p = center(&f.h, "button:Open");
    f.h.click(p.x, p.y);
    f.h.step();
    f.h.step();
    let sheet = find(&f.h, "sheet");
    assert_eq!(f.h.app.translation(sheet).unwrap().y, 0.0);
}

#[test]
fn semantics_name_and_describe_the_dialog() {
    let mut f = fixture(Scheme::Dark, false, true);
    f.open();
    let n = f.h.app.semantic_node(find(&f.h, "dialog")).unwrap();
    assert_eq!(n.role(), Some(Role::Dialog));
    assert_eq!(n.label(), Some("Delete \u{201c}Living room\u{201d}?"));
    assert!(
        n.semantics
            .unwrap()
            .get_description()
            .is_some_and(|d| d.starts_with("This removes"))
    );
    let b = f.h.app.semantic_node(find(&f.h, "button:Delete")).unwrap();
    assert_eq!(b.role(), Some(Role::Button));
    assert_eq!(
        f.dlg.read(&f.h.app).semantics().to_string(),
        "Dialog label=\"Delete \u{201c}Living room\u{201d}?\"\n"
    );
}

#[test]
fn dialog_and_sheet_scenes_in_both_themes() {
    for (scheme, sheet, name) in [
        (Scheme::Dark, false, "dialog-dark"),
        (Scheme::Light, false, "dialog-light"),
        (Scheme::Dark, true, "sheet-dark"),
        (Scheme::Light, true, "sheet-light"),
    ] {
        let mut f = fixture(scheme, sheet, true);
        f.open();
        assert_golden(&golden_dir(), name, &scene(&f.h));
    }
}
