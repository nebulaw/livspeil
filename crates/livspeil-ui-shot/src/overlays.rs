// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Scenes of the building blocks Dream Mode needs: icon buttons, tabs, a
//! popover, a dialog, a sheet and toasts, each in the dark and the light
//! theme (real widgets driven headlessly inside a widget root).

use livspeil_ui_core::{
    App, Element, Render, Scene as CoreScene, TestHost, ViewContext, div, text,
};
use livspeil_ui_layout::Size;
use livspeil_ui_paint::{FontSystem, PaintBridge};
use livspeil_ui_render::Color;
use livspeil_ui_theme::{Scheme, Theme};
use livspeil_ui_widgets::{
    Button, Dialog, DialogButton, Env, IconButton, IconSize, Placement, Popover, TabItem, Tabs,
    Toast, ToastTone, Toasts,
};

use crate::ShotError;
use crate::scenes::{Scene, Shot, surface};
use crate::ui::finish;

type Build = Box<dyn Fn(&mut ViewContext<'_, Content>) -> Element>;

struct Content {
    build: Build,
}

impl Render for Content {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        (self.build)(cx)
    }
}

const SIZE: Size = Size::new(560.0, 340.0);

fn theme(scheme: Scheme) -> Theme {
    Theme {
        scheme,
        ..Theme::default()
    }
}

/// Mount, drive, settle: like the container scenes, with a theme.
fn settle(
    bridge: &PaintBridge,
    scheme: Scheme,
    setup: impl FnOnce(&mut App, &Env) -> Build,
    drive: impl FnOnce(&mut TestHost),
) -> Result<CoreScene, ShotError> {
    let mut app = App::new();
    app.set_text_measurer(bridge.text.measurer());
    app.set_theme(theme(scheme));
    let env = Env::new(&mut app, FontSystem::test());
    let build = setup(&mut app, &env);
    let content = app.new_view(Content { build });
    let root = env.root(&mut app, content);
    app.set_root(root);
    let mut host = TestHost::new(app, SIZE);
    host.run_until_idle(200);
    drive(&mut host);
    host.advance(std::time::Duration::from_millis(700));
    host.run_until_idle(200);
    host.scene()
        .cloned()
        .ok_or_else(|| ShotError::Scene("the view drew no frame".into()))
}

fn click(host: &mut TestHost, name: &str) {
    if let Some(id) = host.app.find(name)
        && let Some(b) = host.app.bounds(id)
    {
        host.click(b.center().x, b.center().y);
        host.run_until_idle(200);
    }
}

fn finish_scene(bridge: PaintBridge, scene: &CoreScene, scale: f32) -> Result<Shot, ShotError> {
    // As the host's renderer does: one more pixel per text run.
    let mut scene = scene.clone();
    crate::gallery::widen_text_runs(&mut scene);
    finish(bridge, &scene, scale, |_, _| Ok(()))
}

/// A page-like backdrop: a heading, a few cards and text, so Glass and the
/// scrim have something behind them.
fn page(theme: &Theme, env: &Env, children: Vec<Element>) -> Element {
    let c = theme.colors();
    let _ = env;
    let mut cards = div().flex_row().gap(12.0);
    for (i, title) in ["Living room", "Kitchen", "Studio"].into_iter().enumerate() {
        cards = cards.child(
            div()
                .w(160.0)
                .h(96.0)
                .p(12.0)
                .flex_col()
                .gap(6.0)
                .rounded(theme.tokens.radii.md)
                .bg(c.surface[1 + i % 2])
                .child(
                    text(title)
                        .font_size(13.0)
                        .font_weight(600.0)
                        .text_color(c.text_primary),
                )
                .child(
                    text(format!("{} Charms", 4 + i * 3))
                        .font_size(12.0)
                        .text_color(c.text_secondary),
                ),
        );
    }
    let mut col = div()
        .w_full()
        .h_full()
        .flex_col()
        .p(20.0)
        .gap(14.0)
        .bg(c.surface[0])
        .child(
            text("Scenes")
                .font_size(22.0)
                .font_weight(600.0)
                .text_color(c.text_primary),
        )
        .child(cards);
    for ch in children {
        col = col.child(ch);
    }
    col
}

// ---- icon buttons ----

fn icon_buttons(scheme: Scheme, scale: f32) -> Result<Shot, ShotError> {
    let bridge = PaintBridge::new(FontSystem::test());
    let scene = settle(
        &bridge,
        scheme,
        |_, env| {
            let env = env.clone();
            Box::new(move |cx| {
                let theme = cx.theme().clone();
                let row = || div().flex_row().items_center().gap(10.0);
                let label = |s: &str| {
                    div().w(70.0).child(
                        text(s.to_owned())
                            .font_size(12.0)
                            .text_color(theme.colors().text_secondary),
                    )
                };
                page(
                    &theme,
                    &env,
                    vec![
                        row().child(label("Sizes")).children(IconSize::ALL.map(|s| {
                            IconButton::new("bx-star", format!("Star {} px", s.px()))
                                .size(s)
                                .tooltip(&env)
                                .render(&theme)
                        })),
                        row()
                            .child(label("Styles"))
                            .child(IconButton::new("bx-plus", "Add").render(&theme))
                            .child(
                                IconButton::new("bx-plus", "Add (filled)")
                                    .filled()
                                    .render(&theme),
                            )
                            .child(
                                IconButton::new("bx-pin", "Pinned")
                                    .toggle(true)
                                    .render(&theme),
                            )
                            .child(
                                IconButton::new("bx-pin", "Not pinned")
                                    .toggle(false)
                                    .render(&theme),
                            )
                            .child(
                                IconButton::new("bx-lock", "Locked")
                                    .disabled(true)
                                    .render(&theme),
                            ),
                    ],
                )
            })
        },
        |_| {},
    )?;
    finish_scene(bridge, &scene, scale)
}

// ---- tabs ----

fn tabs(scheme: Scheme, scale: f32) -> Result<Shot, ShotError> {
    let bridge = PaintBridge::new(FontSystem::test());
    let scene = settle(
        &bridge,
        scheme,
        |app, env| {
            let a = app.new_model(1usize);
            let b = app.new_model(0usize);
            let c = app.new_model(5usize);
            let plain = app.new_view(
                Tabs::new(env, a, ["Scene", "Charms", "Timeline"])
                    .name("plain")
                    .label("View"),
            );
            let closable = app.new_view(
                Tabs::new(
                    env,
                    b,
                    [
                        TabItem::new("Living room"),
                        TabItem::new("Kitchen"),
                        TabItem::new("Studio"),
                    ],
                )
                .closable(true)
                .name("closable")
                .label("Rooms"),
            );
            let overflow = app.new_view(
                Tabs::new(
                    env,
                    c,
                    [
                        "Scene", "Charms", "Timeline", "Assets", "History", "Settings",
                    ],
                )
                .name("overflow")
                .label("Panels"),
            );
            let env = env.clone();
            Box::new(move |cx| {
                let theme = cx.theme().clone();
                page(
                    &theme,
                    &env,
                    vec![
                        Element::view(&plain),
                        Element::view(&closable),
                        div().w(260.0).child(Element::view(&overflow)),
                    ],
                )
            })
        },
        |_| {},
    )?;
    finish_scene(bridge, &scene, scale)
}

// ---- popover ----

fn popover(scheme: Scheme, scale: f32) -> Result<Shot, ShotError> {
    let bridge = PaintBridge::new(FontSystem::test());
    let scene = settle(
        &bridge,
        scheme,
        |app, env| {
            let pop = app.new_view(
                Popover::new(env, |app| {
                    let theme = app.theme().clone();
                    let c = theme.colors();
                    div()
                        .flex_col()
                        .gap(8.0)
                        .child(
                            text("Canvas")
                                .font_size(13.0)
                                .font_weight(600.0)
                                .text_color(c.text_primary),
                        )
                        .child(
                            text("Snap new Charms to the grid.")
                                .font_size(12.0)
                                .text_color(c.text_secondary),
                        )
                        .child(Button::new("Grid settings").render(&theme))
                })
                .trigger(|theme| {
                    IconButton::new("bx-cog", "Settings")
                        .toggle(true)
                        .size(IconSize::S20)
                        .render(theme)
                })
                .placement(Placement::Bottom)
                .size(200.0, 112.0)
                .label("Canvas settings"),
            );
            let env = env.clone();
            Box::new(move |cx| {
                let theme = cx.theme().clone();
                page(
                    &theme,
                    &env,
                    vec![div().flex_row().p(4.0).child(Element::view(&pop))],
                )
            })
        },
        |host| click(host, "icon-button:Settings"),
    )?;
    finish_scene(bridge, &scene, scale)
}

// ---- dialog and sheet ----

fn dialog_scene(scheme: Scheme, sheet: bool, scale: f32) -> Result<Shot, ShotError> {
    let bridge = PaintBridge::new(FontSystem::test());
    let scene = settle(
        &bridge,
        scheme,
        |app, env| {
            let mut d = if sheet {
                Dialog::new(env, "Save changes to \u{201c}Living room\u{201d}?")
                    .sheet()
                    .message("Your changes are lost if you don\u{2019}t save them.")
                    .button(DialogButton::new("Don\u{2019}t save").destructive())
                    .button(DialogButton::new("Cancel").cancel())
                    .button(DialogButton::new("Save").default_button())
            } else {
                Dialog::new(env, "Delete \u{201c}Lighthouse\u{201d}?")
                    .message("The layer and its three Charms are removed from this scene. This can be undone from History.")
                    .button(DialogButton::new("Cancel").cancel())
                    .button(DialogButton::new("Delete").destructive().default_button())
            };
            d = d.width(if sheet { 400.0 } else { 320.0 });
            let dlg = app.new_view(d);
            let env = env.clone();
            Box::new(move |cx| {
                let theme = cx.theme().clone();
                page(
                    &theme,
                    &env,
                    vec![
                        Button::new("Open")
                            .on_click(move |_, cx| Dialog::open(dlg, cx, None))
                            .render(&theme),
                        Element::view(&dlg),
                    ],
                )
            })
        },
        |host| click(host, "button:Open"),
    )?;
    finish_scene(bridge, &scene, scale)
}

// ---- toasts ----

fn toasts(scheme: Scheme, scale: f32) -> Result<Shot, ShotError> {
    let bridge = PaintBridge::new(FontSystem::test());
    let stack = std::rc::Rc::new(std::cell::Cell::new(None));
    let st = stack.clone();
    let scene = settle(
        &bridge,
        scheme,
        |app, env| {
            st.set(Some(Toasts::new(env, app)));
            let env = env.clone();
            Box::new(move |cx| {
                let theme = cx.theme().clone();
                page(&theme, &env, Vec::new())
            })
        },
        |host| {
            if let Some(t) = stack.get() {
                Toasts::show(
                    t,
                    &mut host.app,
                    Toast::new("Moved 3 Charms to Kitchen")
                        .title("Moved")
                        .action("Undo", |_| {})
                        .sticky(),
                );
                Toasts::show(
                    t,
                    &mut host.app,
                    Toast::new("Living room saved")
                        .tone(ToastTone::Success)
                        .sticky(),
                );
                Toasts::show(
                    t,
                    &mut host.app,
                    Toast::new("The disk is full. Free some space and try again.")
                        .title("Save failed")
                        .tone(ToastTone::Danger)
                        .sticky(),
                );
            }
        },
    )?;
    finish_scene(bridge, &scene, scale)
}

fn dark() -> Color {
    surface(0)
}

macro_rules! themed {
    ($name:ident, $f:expr) => {
        fn $name(scheme: Scheme) -> fn(f32) -> Result<Shot, ShotError> {
            match scheme {
                Scheme::Dark => |s| $f(Scheme::Dark, s),
                Scheme::Light => |s| $f(Scheme::Light, s),
            }
        }
    };
}

themed!(icon_buttons_for, icon_buttons);
themed!(tabs_for, tabs);
themed!(popover_for, popover);
themed!(toasts_for, toasts);

fn dialog_for(scheme: Scheme) -> fn(f32) -> Result<Shot, ShotError> {
    match scheme {
        Scheme::Dark => |s| dialog_scene(Scheme::Dark, false, s),
        Scheme::Light => |s| dialog_scene(Scheme::Light, false, s),
    }
}

fn sheet_for(scheme: Scheme) -> fn(f32) -> Result<Shot, ShotError> {
    match scheme {
        Scheme::Dark => |s| dialog_scene(Scheme::Dark, true, s),
        Scheme::Light => |s| dialog_scene(Scheme::Light, true, s),
    }
}

/// The overlay and tab scenes at 1x, dark and light.
pub fn scenes() -> Vec<Scene> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (width, height) = (SIZE.width as u32, SIZE.height as u32);
    let mut v = Vec::new();
    for scheme in [Scheme::Dark, Scheme::Light] {
        let names: [(
            &'static str,
            &'static str,
            fn(f32) -> Result<Shot, ShotError>,
        ); 6] = match scheme {
            Scheme::Dark => [
                (
                    "icon-buttons-dark",
                    "IconButton: sizes, plain, filled, toggle, disabled, dark",
                    icon_buttons_for(scheme),
                ),
                (
                    "tabs-dark",
                    "Tabs: plain, closable and overflowing bars, dark",
                    tabs_for(scheme),
                ),
                (
                    "popover-dark",
                    "Popover opened from an icon button, dark",
                    popover_for(scheme),
                ),
                (
                    "dialog-dark",
                    "Dialog over a scrim with a destructive default button, dark",
                    dialog_for(scheme),
                ),
                (
                    "sheet-dark",
                    "Sheet attached to the window top, dark",
                    sheet_for(scheme),
                ),
                (
                    "toast-dark",
                    "Toast stack: action, success, danger, dark",
                    toasts_for(scheme),
                ),
            ],
            Scheme::Light => [
                (
                    "icon-buttons-light",
                    "IconButton: sizes, plain, filled, toggle, disabled, light",
                    icon_buttons_for(scheme),
                ),
                (
                    "tabs-light",
                    "Tabs: plain, closable and overflowing bars, light",
                    tabs_for(scheme),
                ),
                (
                    "popover-light",
                    "Popover opened from an icon button, light",
                    popover_for(scheme),
                ),
                (
                    "dialog-light",
                    "Dialog over a scrim with a destructive default button, light",
                    dialog_for(scheme),
                ),
                (
                    "sheet-light",
                    "Sheet attached to the window top, light",
                    sheet_for(scheme),
                ),
                (
                    "toast-light",
                    "Toast stack: action, success, danger, light",
                    toasts_for(scheme),
                ),
            ],
        };
        for (name, description, build) in names {
            v.push(Scene {
                name,
                description,
                width,
                height,
                scale: 1.0,
                build,
                clear: dark,
            });
        }
    }
    v
}
