// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! IconButton: sizes, styles (plain, filled, toggle), tooltip and
//! semantics.

mod common;

use common::{center, find, golden_dir, host, scene, wait};
use livspeil_testkit::assert_golden;
use livspeil_ui_core::{IconRun, Model, PaintCmd, Role, TestHost, Toggled, div};
use livspeil_ui_theme::{Scheme, Theme};
use livspeil_ui_widgets::{
    ButtonVariant, IconButton, IconButtonStyle, IconSize, Semantics, TOOLTIP_DELAY,
};

#[test]
fn sizes_are_the_design_system_sizes() {
    assert_eq!(IconSize::ALL.map(IconSize::px), [14.0, 16.0, 20.0, 24.0]);
    let (h, _) = host(|_, _| {
        Box::new(|cx| {
            let theme = cx.theme().clone();
            let mut row = div().flex_row().p(8.0).gap(8.0).items_center();
            for s in IconSize::ALL {
                row = row.child(
                    IconButton::new("bx-star", format!("Star {}", s.px()))
                        .size(s)
                        .render(&theme),
                );
            }
            row
        })
    });
    for s in IconSize::ALL {
        let b = h
            .app
            .bounds(find(&h, &format!("icon-button:Star {}", s.px())));
        let b = b.unwrap();
        assert_eq!((b.width, b.height), (s.button(), s.button()));
    }
    let icon = h.app.bounds(find(&h, "icon:bx-star")).unwrap();
    assert_eq!(icon.width, 14.0, "first match is the 14 px icon");
}

#[test]
fn toggle_reports_state_and_flips_on_click_and_keyboard() {
    let mut on = None;
    let (mut h, _) = host(|app, _| {
        let m: Model<bool> = app.new_model(false);
        on = Some(m);
        Box::new(move |cx| {
            let theme = cx.theme().clone();
            let v = *m.read(cx);
            div().p(8.0).child(
                IconButton::new("bx-lock", "Lock")
                    .toggle(v)
                    .on_click(move |_, cx| {
                        m.update(cx, |x, cx| {
                            *x = !*x;
                            cx.notify();
                        });
                    })
                    .render(&theme),
            )
        })
    });
    let on = on.unwrap();
    let b = find(&h, "icon-button:Lock");
    let off_bg = h.app.background(b).unwrap();
    let state = |h: &TestHost| {
        h.app
            .semantic_node(find(h, "icon-button:Lock"))
            .unwrap()
            .semantics
            .and_then(|s| s.get_checked())
    };
    assert_eq!(state(&h), Some(Toggled::Off));
    let p = center(&h, "icon-button:Lock");
    h.click(p.x, p.y);
    h.run_until_idle(200);
    assert!(*on.read(&h.app));
    assert_eq!(state(&h), Some(Toggled::On));
    assert!(
        h.app.background(b).unwrap().alpha > off_bg.alpha,
        "the pill appears when on"
    );
    h.key("tab");
    assert_eq!(h.app.focused(), Some(b));
    h.key("space");
    h.run_until_idle(100);
    assert!(!*on.read(&h.app));
    assert_eq!(state(&h), Some(Toggled::Off));
    let node = h.app.semantic_node(b).unwrap();
    assert_eq!(node.role(), Some(Role::Button));
    assert_eq!(node.label(), Some("Lock"));
}

#[test]
fn tooltip_is_optional_and_uses_the_label() {
    let (mut h, _) = host(|_, env| {
        let env = env.clone();
        Box::new(move |cx| {
            let theme = cx.theme().clone();
            div()
                .flex_row()
                .p(16.0)
                .gap(16.0)
                .child(
                    IconButton::new("bx-copy", "Copy")
                        .tooltip(&env)
                        .render(&theme),
                )
                .child(IconButton::new("bx-cut", "Cut").render(&theme))
        })
    });
    let p = center(&h, "icon-button:Copy");
    h.pointer_move(p.x, p.y);
    h.step();
    wait(
        &mut h,
        u64::try_from(TOOLTIP_DELAY.as_millis()).unwrap_or(500) + 100,
    );
    h.run_until_idle(100);
    let tip = h.app.find("tooltip").expect("tooltip shown");
    assert_eq!(h.app.semantic_node(tip).unwrap().label(), Some("Copy"));
    // The anchor is described by it; the other has no tooltip.
    let q = center(&h, "icon-button:Cut");
    h.pointer_move(q.x, q.y);
    h.step();
    wait(&mut h, 800);
    h.run_until_idle(100);
    assert!(h.app.find("tooltip").is_none());
}

#[test]
fn disabled_ignores_clicks_and_semantics_are_named() {
    let s: Semantics = IconButton::new("bx-trash", "Delete")
        .disabled(true)
        .semantics();
    assert!(s.disabled && !s.focusable);
    assert_eq!(s.label.as_deref(), Some("Delete"));
    let t = IconButton::new("bx-lock", "Lock").toggle(true).semantics();
    assert_eq!(
        t.to_string(),
        "Button label=\"Lock\" toggled=True focusable actions=[Click, Focus]\n"
    );
}

#[test]
fn styles_render_distinct_fills_in_both_themes() {
    for scheme in [Scheme::Dark, Scheme::Light] {
        let (mut h, _) = host(|app, env| {
            app.set_theme(Theme {
                scheme,
                ..Theme::default()
            });
            let env = env.clone();
            Box::new(move |cx| {
                let theme = cx.theme().clone();
                let row = div().flex_row().gap(8.0).items_center().p(8.0);
                IconSize::ALL
                    .into_iter()
                    .fold(row, |r, s| {
                        r.child(
                            IconButton::new("bx-star", format!("S{}", s.px()))
                                .size(s)
                                .tooltip(&env)
                                .render(&theme),
                        )
                    })
                    .child(
                        IconButton::new("bx-grid-plus", "Add")
                            .filled()
                            .render(&theme),
                    )
                    .child(
                        IconButton::new("bx-copy", "Remove")
                            .style(IconButtonStyle::Filled)
                            .variant(ButtonVariant::Destructive)
                            .render(&theme),
                    )
                    .child(
                        IconButton::new("bx-bookmark-alt", "Pinned")
                            .toggle(true)
                            .render(&theme),
                    )
                    .child(
                        IconButton::new("bx-bookmark-alt", "Unpinned")
                            .toggle(false)
                            .render(&theme),
                    )
                    .child(
                        IconButton::new("bx-unlink", "Off")
                            .disabled(true)
                            .render(&theme),
                    )
            })
        });
        h.run_until_idle(100);
        let name = match scheme {
            Scheme::Dark => "icon-button-dark",
            Scheme::Light => "icon-button-light",
        };
        assert_golden(&golden_dir(), name, &scene(&h));
    }
}

fn icons_drawn(h: &TestHost) -> Vec<IconRun> {
    h.scene()
        .expect("a frame")
        .commands
        .iter()
        .filter_map(|c| match c {
            PaintCmd::Icon(i) => Some(i.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_icon_is_painted_in_the_foreground_color() {
    let (h, _) = host(|_, _| {
        Box::new(|cx| {
            let theme = cx.theme().clone();
            div()
                .flex_row()
                .p(8.0)
                .gap(8.0)
                .child(IconButton::new("bx-copy", "Plain").render(&theme))
                .child(IconButton::new("bx-gear", "On").toggle(true).render(&theme))
                .child(IconButton::new("bx-star", "Filled").filled().render(&theme))
        })
    });
    let drawn = icons_drawn(&h);
    let names: Vec<&str> = drawn.iter().map(|i| &*i.name).collect();
    assert_eq!(names, ["bx-copy", "bx-gear", "bx-star"]);
    let theme = Theme::default();
    assert_eq!(drawn[0].color, theme.colors().text_primary);
    assert_eq!(drawn[1].color, theme.accent(), "an on toggle is accented");
    assert_ne!(drawn[2].color, drawn[0].color, "on_accent, not plain text");
    // Centered in the button.
    let button = h.app.bounds(find(&h, "icon-button:Plain")).unwrap();
    assert_eq!(drawn[0].rect.center(), button.center());
}
