// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! M1.10 scenes: scroll view, menus, context menu, titlebar, dock panels
//! and the node canvas, driven headlessly inside a widget root (overlay
//! layer for menus).

use std::rc::Rc;

use livspeil_ui_core::{
    App, Element, Fill, Modifiers, MouseButton, PaintCmd, PointerKind, Quad, Render,
    Scene as CoreScene, ScrollDelta, TestHost, View, ViewContext, div, text,
};
use livspeil_ui_layout::{Corners, DockEdge, Point, Rect, Size, Vec2};
use livspeil_ui_paint::{FontSystem, PaintBridge};
use livspeil_ui_theme::{Color as ThemeColor, Theme};
use livspeil_ui_widgets::{
    Button, ContextMenu, DemoGraph, DockView, Env, Menu, MenuBar, MenuItem, NodeCanvas,
    NumberField, Panel, ScrollPhysics, ScrollView, Titlebar, Toggle,
};

use crate::ShotError;
use crate::scenes::{Scene, Shot};
use crate::ui::{finish, pair};

/// A root view built by a closure.
struct Content {
    build: Build,
}

impl Render for Content {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        (self.build)(cx)
    }
}

type Build = Box<dyn Fn(&mut ViewContext<'_, Content>) -> Element>;

/// Mounts the content `setup` returns in a widget root (fonts and text
/// measurement from `bridge`'s test fonts), applies `drive`, settles and
/// returns the scene.
fn settle_env(
    bridge: &PaintBridge,
    size: Size,
    setup: impl FnOnce(&mut App, &Env) -> Build,
    drive: impl FnOnce(&mut TestHost, &Env),
) -> Result<CoreScene, ShotError> {
    let mut app = App::new();
    app.set_text_measurer(bridge.text.measurer());
    let env = Env::new(&mut app, FontSystem::test());
    let build = setup(&mut app, &env);
    let content = app.new_view(Content { build });
    let root = env.root(&mut app, content);
    app.set_root(root);
    let mut host = TestHost::new(app, size);
    host.run_until_idle(200);
    drive(&mut host, &env);
    host.run_until_idle(200);
    host.scene()
        .cloned()
        .ok_or_else(|| ShotError::Scene("the view drew no frame".into()))
}

fn center(host: &TestHost, name: &str) -> Option<Point> {
    Some(host.app.bounds(host.app.find(name)?)?.center())
}

fn backdrop(scene: &mut CoreScene, size: Size) {
    scene.commands.insert(
        0,
        PaintCmd::Quad(Quad {
            element: livspeil_ui_core::ElementId::ROOT,
            rect: Rect::new(0.0, 0.0, size.width, size.height),
            radii: Corners::default(),
            fill: Some(Fill::LinearGradient {
                start: Vec2::new(0.0, 0.0),
                end: Vec2::new(size.width, size.height),
                from: ThemeColor::oklch(0.42, 0.13, 280.0, 1.0),
                to: ThemeColor::oklch(0.33, 0.10, 210.0, 1.0),
            }),
            border: None,
            shadow: None,
            material: None,
        }),
    );
}

// ---- scroll ----

fn scroll(scale: f32) -> Result<Shot, ShotError> {
    let bridge = PaintBridge::new(FontSystem::test());
    let size = Size::new(360.0, 260.0);
    let scene = settle_env(
        &bridge,
        size,
        |app, env| {
            let sv = app.new_view(
                ScrollView::new(env, |app, _| {
                    let theme = app.theme().clone();
                    let c = theme.colors();
                    let mut col = div().flex_col().p(8.0).gap(2.0);
                    for i in 0..30 {
                        let hue = (i * 37 % 360) as f32;
                        col = col.child(
                            div()
                                .h(30.0)
                                .px(10.0)
                                .flex_row()
                                .items_center()
                                .gap(10.0)
                                .rounded(theme.tokens.radii.sm)
                                .bg(c.surface[if i % 2 == 0 { 1 } else { 2 }])
                                .child(
                                    div()
                                        .size(10.0, 10.0)
                                        .rounded(5.0)
                                        .bg(ThemeColor::oklch(0.72, 0.13, hue, 1.0)),
                                )
                                .child(text(format!("Layer {}", i + 1)))
                                .child(div().grow())
                                .child(
                                    text(format!("{}%", 100 - i * 3))
                                        .font_size(11.0)
                                        .text_color(c.text_tertiary),
                                ),
                        );
                    }
                    col
                })
                .name("sv")
                .physics(ScrollPhysics {
                    bar_linger: 1e6,
                    ..ScrollPhysics::default()
                }),
            );
            Box::new(move |cx| {
                let theme = cx.theme().clone();
                div().size(360.0, 260.0).p(12.0).child(
                    div()
                        .w_full()
                        .h_full()
                        .rounded(theme.tokens.radii.md)
                        .bg(theme.colors().surface[0])
                        .clip()
                        .child(Element::view(&sv)),
                )
            })
        },
        |host, _| {
            if let Some(p) = center(host, "sv") {
                host.pointer_move(p.x, p.y);
                host.app.dispatch(livspeil_ui_core::InputEvent::Pointer(
                    livspeil_ui_core::PointerEvent {
                        kind: PointerKind::Scroll(ScrollDelta::Lines(Vec2::new(0.0, -4.0))),
                        position: p,
                        modifiers: Modifiers::NONE,
                        time: host.now(),
                    },
                ));
            }
        },
    )?;
    finish(bridge, &scene, scale, |_, _| Ok(()))
}

// ---- menus ----

fn edit_menu() -> Menu {
    Menu::new()
        .item(MenuItem::new("Undo").shortcut("Ctrl+Z"))
        .item(
            MenuItem::new("Redo")
                .shortcut("Ctrl+Shift+Z")
                .disabled(true),
        )
        .separator()
        .item(MenuItem::new("Cut").shortcut("Ctrl+X"))
        .item(MenuItem::new("Copy").icon("bx-copy").shortcut("Ctrl+C"))
        .item(MenuItem::new("Paste").shortcut("Ctrl+V"))
        .separator()
        .item(MenuItem::new("Snap to Grid").checked(true))
        .submenu(
            "Arrange",
            Menu::new()
                .item(MenuItem::new("Bring to Front"))
                .item(MenuItem::new("Send to Back"))
                .separator()
                .item(MenuItem::new("Align Left"))
                .item(MenuItem::new("Align Center")),
        )
}

fn menu_open(scale: f32) -> Result<Shot, ShotError> {
    let bridge = PaintBridge::new(FontSystem::test());
    let size = Size::new(520.0, 360.0);
    let mut scene = settle_env(
        &bridge,
        size,
        |app, env| {
            let bar = app.new_view(
                MenuBar::new(env)
                    .menu("Livspeil", Menu::new().item(MenuItem::new("About")))
                    .menu("File", Menu::new().item(MenuItem::new("New Scene")))
                    .menu("Edit", edit_menu())
                    .menu("View", Menu::new().item(MenuItem::new("Zoom In"))),
            );
            env.set_menubar(bar);
            Box::new(move |_| div().w_full().h_full().child(Element::view(&bar)))
        },
        |host, env| {
            env.focus_root(&mut host.app);
            host.key("alt-e");
            host.run_until_idle(100);
            for k in ["up", "right", "down"] {
                host.key(k);
                host.run_until_idle(100);
            }
        },
    )?;
    backdrop(&mut scene, size);
    finish(bridge, &scene, scale, |_, _| Ok(()))
}

fn context_menu(scale: f32) -> Result<Shot, ShotError> {
    let bridge = PaintBridge::new(FontSystem::test());
    let size = Size::new(420.0, 300.0);
    let mut scene = settle_env(
        &bridge,
        size,
        |_, env| {
            let env = env.clone();
            Box::new(move |cx| {
                let theme = cx.theme().clone();
                let area = div().name("area").w_full().h_full().p(16.0).child(
                    text("Right-click the canvas")
                        .font_size(15.0)
                        .text_color(theme.colors().text_secondary),
                );
                ContextMenu::new(
                    Menu::new()
                        .item(
                            MenuItem::new("Add Node…")
                                .icon("bx-grid-plus")
                                .shortcut("Shift+A"),
                        )
                        .item(MenuItem::new("Paste").shortcut("Ctrl+V"))
                        .separator()
                        .item(MenuItem::new("Show Minimap").checked(true))
                        .item(MenuItem::new("Show Grid").checked(false))
                        .separator()
                        .item(MenuItem::new("Frame All").shortcut("Home")),
                )
                .attach(&env, area)
            })
        },
        |host, _| {
            let p = Point::new(330.0, 210.0);
            host.pointer_move(p.x, p.y);
            for kind in [
                PointerKind::Down(MouseButton::Right),
                PointerKind::Up(MouseButton::Right),
            ] {
                host.app.dispatch(livspeil_ui_core::InputEvent::Pointer(
                    livspeil_ui_core::PointerEvent {
                        kind,
                        position: p,
                        modifiers: Modifiers::NONE,
                        time: host.now(),
                    },
                ));
            }
            host.run_until_idle(100);
            if let Some(i) = center(host, "menuitem:Show Minimap") {
                host.pointer_move(i.x, i.y);
            }
        },
    )?;
    backdrop(&mut scene, size);
    finish(bridge, &scene, scale, |_, _| Ok(()))
}

// ---- titlebar ----

fn traffic_lights(theme: &Theme) -> Element {
    let _ = theme;
    let light = |hue: f32| {
        div()
            .size(12.0, 12.0)
            .rounded(6.0)
            .bg(ThemeColor::oklch(0.72, 0.16, hue, 1.0))
    };
    div()
        .absolute(Rect::new(20.0, 0.0, 52.0, 0.0))
        .h_full()
        .flex_row()
        .items_center()
        .gap(8.0)
        .hit_test(false)
        .child(light(25.0))
        .child(light(85.0))
        .child(light(145.0))
}

fn titlebar(scale: f32) -> Result<Shot, ShotError> {
    let bridge = PaintBridge::new(FontSystem::test());
    let size = Size::new(640.0, 140.0);
    let scene = settle_env(
        &bridge,
        size,
        |_, _| {
            Box::new(|cx| {
                let theme = cx.theme().clone();
                let c = theme.colors();
                let window = |bar: Element, h: f32| {
                    div()
                        .w_full()
                        .h(h)
                        .flex_col()
                        .bg(c.surface[1])
                        .rounded(theme.tokens.radii.window)
                        .clip()
                        .child(div().w_full().child(bar).child(traffic_lights(&theme)))
                };
                div()
                    .size(640.0, 140.0)
                    .p(10.0)
                    .gap(10.0)
                    .flex_col()
                    .child(window(Titlebar::new("Untitled Scene").render(&theme), 40.0))
                    .child(window(
                        Titlebar::new("Lighthouse.room")
                            .frost()
                            .leading(Button::new("‹").ghost().render(&theme).key("back"))
                            .trailing(Button::new("Share").render(&theme).key("share"))
                            .trailing(Button::new("Play").primary().render(&theme).key("play"))
                            .render(&theme),
                        60.0,
                    ))
            })
        },
        |_, _| {},
    )?;
    finish(bridge, &scene, scale, |_, _| Ok(()))
}

// ---- dock-panels ----

fn dock_panels(scale: f32) -> Result<Shot, ShotError> {
    let bridge = PaintBridge::new(FontSystem::test());
    let size = Size::new(720.0, 420.0);
    let mut scene = settle_env(
        &bridge,
        size,
        |app, env| {
            let opacity = app.new_model(0.8);
            let field = app.new_view(NumberField::new(env, opacity, 0.0, 1.0, 0.01).width(96.0));
            let field = Rc::new(field);
            let dock = app.new_view(
                DockView::new(|app| {
                    let c = app.theme().colors();
                    div()
                        .w_full()
                        .h_full()
                        .items_center()
                        .justify_center()
                        .child(text("Viewport").font_size(17.0).text_color(c.text_tertiary))
                })
                .panel(DockEdge::Left, 180.0, 120.0, |app| {
                    let theme = app.theme().clone();
                    let c = theme.colors();
                    let mut p = Panel::new("Layers").frost();
                    for (i, l) in ["Sky", "Island", "Lighthouse", "Water", "Fog"]
                        .into_iter()
                        .enumerate()
                    {
                        let sel = i == 2;
                        let row = div().h(26.0).mx_row().px(10.0).items_center().child(
                            text(l).text_color(if sel {
                                c.text_primary
                            } else {
                                c.text_secondary
                            }),
                        );
                        p = p.child(if sel {
                            row.bg(theme.accent().with_alpha(0.28))
                        } else {
                            row
                        });
                    }
                    p.render(&theme)
                })
                .panel(DockEdge::Right, 220.0, 160.0, move |app| {
                    let theme = app.theme().clone();
                    let c = theme.colors();
                    let row = |label: &str, control: Element| {
                        div()
                            .h(34.0)
                            .px(12.0)
                            .flex_row()
                            .items_center()
                            .gap(8.0)
                            .child(text(label).text_color(c.text_secondary).grow())
                            .child(control)
                    };
                    Panel::new("Inspector")
                        .child(row("Opacity", Element::view(&field)))
                        .child(row("Visible", Toggle::new(true).render(&theme)))
                        .child(row("Cast shadow", Toggle::new(false).render(&theme)))
                        .render(&theme)
                })
                .panel(DockEdge::Bottom, 110.0, 60.0, |app| {
                    let theme = app.theme().clone();
                    Panel::new("Timeline")
                        .child(
                            div().p(12.0).child(
                                div()
                                    .h(6.0)
                                    .w_full()
                                    .rounded(3.0)
                                    .bg(theme.colors().surface[3]),
                            ),
                        )
                        .render(&theme)
                }),
            );
            Box::new(move |_| div().w_full().h_full().child(Element::view(&dock)))
        },
        |host, _| {
            if let Some(p) = center(host, "splitter:right") {
                host.pointer_move(p.x, p.y);
            }
        },
    )?;
    backdrop(&mut scene, size);
    finish(bridge, &scene, scale, |_, _| Ok(()))
}

trait RowExt {
    fn mx_row(self) -> Self;
}

impl RowExt for Element {
    /// Inset rows from the panel edges.
    fn mx_row(self) -> Self {
        self.style(|s| {
            let l = livspeil_ui_layout::style::LengthPercentageAuto::length(6.0);
            s.margin.left = l;
            s.margin.right = l;
        })
        .rounded(5.0)
    }
}

// ---- node-canvas ----

fn node_canvas(scale: f32) -> Result<Shot, ShotError> {
    let bridge = PaintBridge::new(FontSystem::test());
    let size = Size::new(960.0, 520.0);
    let out: Rc<std::cell::Cell<Option<View<NodeCanvas<DemoGraph>>>>> = Rc::default();
    let out2 = out.clone();
    let scene = settle_env(
        &bridge,
        size,
        |app, env| {
            let graph = app.new_model(DemoGraph::sample());
            let view = app.new_view(NodeCanvas::new(env, graph).with_canvas(
                livspeil_ui_layout::Canvas {
                    pan: Vec2::new(30.0, 40.0),
                    zoom: 0.9,
                    ..livspeil_ui_layout::Canvas::default()
                },
            ));
            out2.set(Some(view));
            Box::new(move |_| div().w_full().h_full().child(Element::view(&view)))
        },
        |host, _| {
            let Some(view) = out.get() else { return };
            host.pointer_move(1.0, 1.0);
            host.run_until_idle(50);
            let canvas = view.read(&host.app).canvas();
            let to_view = |host: &TestHost, name: &str, dx: f32, dy: f32| {
                let b = host.app.bounds(host.app.find(name)?)?;
                Some(canvas.to_view(Point::new(b.x + dx, b.y + dy)))
            };
            if let Some(p) = to_view(host, "node:Mix", 60.0, 14.0) {
                host.click(p.x, p.y);
                host.run_until_idle(50);
            }
            let from = to_view(host, "port:5:out:0", 5.0, 5.0);
            let to = to_view(host, "node:Output", 40.0, 250.0);
            if let (Some(a), Some(b)) = (from, to) {
                host.pointer_move(a.x, a.y);
                host.pointer_down(a.x, a.y);
                for i in 1..=6 {
                    host.step();
                    let p = a + (b - a) * (i as f32 / 6.0);
                    host.pointer_move(p.x, p.y);
                }
            }
        },
    )?;
    finish(bridge, &scene, scale, |_, _| Ok(()))
}

/// The scenes, each at 1x and 2x.
pub fn scenes() -> Vec<Scene> {
    [
        pair(
            ["scroll-1x", "scroll-2x"],
            "ScrollView: a list scrolled with the wheel, overlay scrollbar shown",
            360,
            260,
            scroll,
        ),
        pair(
            ["menu-open-1x", "menu-open-2x"],
            "MenuBar with Edit open (shortcuts, separators, check mark, disabled item) and the Arrange submenu",
            520,
            360,
            menu_open,
        ),
        pair(
            ["context-menu-1x", "context-menu-2x"],
            "ContextMenu opened near the bottom right, flipped to stay on screen, an item hovered",
            420,
            300,
            context_menu,
        ),
        pair(
            ["titlebar-1x", "titlebar-2x"],
            "Titlebars: plain, and Frost with toolbar items, traffic-light inset",
            640,
            140,
            titlebar,
        ),
        pair(
            ["dock-panels-1x", "dock-panels-2x"],
            "DockView: Frost layers panel, Solid inspector, timeline, splitters",
            720,
            420,
            dock_panels,
        ),
        pair(
            ["node-canvas-1x", "node-canvas-2x"],
            "NodeCanvas: sample graph with wires, a selected node, a pending wire and the minimap",
            960,
            520,
            node_canvas,
        ),
    ]
    .into_iter()
    .flatten()
    .collect()
}
