// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! UI scenes: real `livspeil-ui-core` views (driven headlessly with a
//! `TestHost`, laid out with the bridge's text measurer) painted through
//! `livspeil-ui-paint`, with the bundled fonts only (deterministic).

use livspeil_ui_core::{
    App, Element, ElementId, Fill, FontFeature, Model, PaintCmd, Quad, Render, Scene as CoreScene,
    TestHost, View, ViewContext, div, text,
};
use livspeil_ui_layout::{Corners, Rect, Size, Transform2D, Vec2};
use livspeil_ui_paint::{FontSystem, PaintBridge};
use livspeil_ui_render::{Color, DisplayList};
use livspeil_ui_theme::{Color as ThemeColor, Material, Theme};
use livspeil_ui_widgets::{Button, Slider, Toggle};

use crate::ShotError;
use crate::scenes::{Scene, Shot, surface};

fn paint_err(e: impl std::fmt::Display) -> ShotError {
    ShotError::Scene(e.to_string())
}

/// Mounts `view` in a fresh app measured by `bridge`, applies `drive`,
/// runs to idle and returns the settled scene.
fn settle<V: Render>(
    bridge: &PaintBridge,
    view: impl FnOnce(&mut App) -> V,
    size: Size,
    drive: impl FnOnce(&mut TestHost),
) -> Result<CoreScene, ShotError> {
    let mut app = App::new();
    app.set_text_measurer(bridge.text.measurer());
    let v = view(&mut app);
    let v = app.new_view(v);
    app.set_root(v);
    let mut host = TestHost::new(app, size);
    host.run_until_idle(200);
    drive(&mut host);
    host.run_until_idle(200);
    host.scene()
        .cloned()
        .ok_or_else(|| ShotError::Scene("the view drew no frame".into()))
}

fn center(host: &TestHost, name: &str) -> Option<(f32, f32)> {
    let b = host.app.bounds(host.app.find(name)?)?;
    let c = b.center();
    Some((c.x, c.y))
}

fn translated(scene: CoreScene, dx: f32, dy: f32, out: &mut CoreScene) {
    out.commands
        .push(PaintCmd::PushTransform(Transform2D::translate(Vec2::new(
            dx, dy,
        ))));
    out.commands.extend(scene.commands);
    out.commands.push(PaintCmd::PopTransform);
}

fn finish(
    mut bridge: PaintBridge,
    scene: &CoreScene,
    scale: f32,
    extra: impl FnOnce(&mut PaintBridge, &mut DisplayList) -> Result<(), ShotError>,
) -> Result<Shot, ShotError> {
    let mut list = DisplayList::new();
    let mut atlas = bridge
        .paint_scene(scene, scale, &mut list)
        .map_err(paint_err)?;
    extra(&mut bridge, &mut list)?;
    atlas.extend(bridge.text.take_atlas_updates());
    Ok(Shot { list, atlas })
}

// ---- button-states ----

struct ButtonCell {
    caption: &'static str,
    disabled: bool,
}

impl Render for ButtonCell {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let theme = cx.theme().clone();
        let b = Button::new("Apply")
            .disabled(self.disabled)
            .render(&theme)
            .key("b");
        div()
            .size(112.0, 84.0)
            .flex_col()
            .items_center()
            .justify_center()
            .gap(10.0)
            .child(b)
            .child(
                text(self.caption)
                    .font_size(11.0)
                    .text_color(theme.colors().text_tertiary),
            )
    }
}

fn button_states(scale: f32) -> Result<Shot, ShotError> {
    let bridge = PaintBridge::new(FontSystem::test());
    let mut all = CoreScene::default();
    let states = ["Rest", "Hover", "Pressed", "Focused", "Disabled"];
    for (i, caption) in states.into_iter().enumerate() {
        let scene = settle(
            &bridge,
            |_| ButtonCell {
                caption,
                disabled: caption == "Disabled",
            },
            Size::new(112.0, 84.0),
            |host| {
                let at = center(host, "button:Apply");
                match (caption, at) {
                    ("Hover", Some((x, y))) => host.pointer_move(x, y),
                    ("Pressed", Some((x, y))) => {
                        host.pointer_move(x, y);
                        host.pointer_down(x, y);
                    }
                    ("Focused", _) => host.key("tab"),
                    _ => {}
                }
            },
        )?;
        translated(scene, 8.0 + i as f32 * 112.0, 8.0, &mut all);
    }
    finish(bridge, &all, scale, |_, _| Ok(()))
}

// ---- toggle-slider ----

struct Controls {
    off: bool,
    a: View<Slider>,
    b: View<Slider>,
}

impl Render for Controls {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let theme = cx.theme().clone();
        let caption = |s: &'static str| text(s).text_color(theme.colors().text_secondary);
        let row = || div().flex_row().items_center().gap(12.0).h(28.0);
        div()
            .size(320.0, 168.0)
            .flex_col()
            .p(16.0)
            .gap(8.0)
            .child(
                row()
                    .child(Toggle::new(!self.off).render(&theme).key("t1"))
                    .child(caption("Snap to grid")),
            )
            .child(
                row()
                    .child(Toggle::new(self.off).render(&theme).key("t2"))
                    .child(caption("Reduce motion")),
            )
            .child(row().child(Element::view(&self.a)).child(caption("30%")))
            .child(row().child(Element::view(&self.b)).child(caption("75%")))
    }
}

fn slider(app: &mut App, v: f32) -> View<Slider> {
    let m: Model<f32> = app.new_model(v);
    app.new_view(Slider::new(m, 0.0, 1.0, 0.01).width(200.0))
}

fn toggle_slider(scale: f32) -> Result<Shot, ShotError> {
    let bridge = PaintBridge::new(FontSystem::test());
    let scene = settle(
        &bridge,
        |app| Controls {
            off: false,
            a: slider(app, 0.3),
            b: slider(app, 0.75),
        },
        Size::new(320.0, 168.0),
        |_| {},
    )?;
    finish(bridge, &scene, scale, |_, _| Ok(()))
}

// ---- text-samples ----

const PARAGRAPH: &str = "Dream Mode edits the scene; Live Mode uses it. Every \
    surface is a Charm, and every Charm is a small graph of typed nodes that \
    you can open, rewire and share.";

struct TextSamples;

/// Inter weights and proportional vs tabular digits, as core text elements
/// with a font weight and the `tnum` feature.
fn weights_and_digits(theme: &Theme) -> Element {
    let c = theme.colors();
    let mut block = div().flex_col().h(118.0);
    for (w, label) in [
        (400.0, "Regular 400"),
        (500.0, "Medium 500"),
        (600.0, "Semibold 600"),
    ] {
        block = block.child(
            div()
                .h(22.0)
                .child(text(label).font_size(15.0).font_weight(w)),
        );
    }
    for (tnum, label) in [(false, "proportional"), (true, "tnum")] {
        let digits = text("1111.11  8888.88").font_size(13.0);
        let digits = if tnum {
            digits.font_feature(FontFeature::TNUM)
        } else {
            digits
        };
        block = block.child(
            div()
                .h(20.0)
                .flex_row()
                .child(div().w(150.0).child(digits))
                .child(
                    div()
                        .style(|s| s.padding.top = livspeil_ui_layout::style::length(2.0))
                        .child(text(label).font_size(11.0).text_color(c.text_secondary)),
                ),
        );
    }
    block
}

impl Render for TextSamples {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let theme = cx.theme().clone();
        let c = theme.colors();
        let mut col = div().flex_col().gap(4.0).w(330.0);
        for size in theme.tokens.type_scale.sizes {
            col = col.child(text(format!("{size} The quick brown fox")).font_size(size));
        }
        div()
            .size(640.0, 380.0)
            .flex_row()
            .p(16.0)
            .gap(16.0)
            .child(col)
            .child(
                div()
                    .flex_col()
                    .gap(8.0)
                    .w(250.0)
                    .child(weights_and_digits(&theme))
                    .child(text("ქართული ენა · გამარჯობა").font_size(15.0))
                    .child(text("Livspeil: مرحبا بالعالم 2026").font_size(15.0))
                    .child(
                        text(PARAGRAPH)
                            .font_size(13.0)
                            .w(250.0)
                            .h(96.0)
                            .text_color(c.text_secondary),
                    ),
            )
    }
}

fn text_samples(scale: f32) -> Result<Shot, ShotError> {
    let bridge = PaintBridge::new(FontSystem::test());
    let scene = settle(&bridge, |_| TextSamples, Size::new(640.0, 380.0), |_| {})?;
    finish(bridge, &scene, scale, |_, _| Ok(()))
}

// ---- frost-sidebar ----

struct Sidebar;

impl Render for Sidebar {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let theme = cx.theme().clone();
        let c = theme.colors();
        let blob = |r: Rect, color: ThemeColor| div().absolute(r).rounded(r.width / 2.0).bg(color);
        let item = |label: &'static str, selected: bool| {
            let row = div()
                .h(28.0)
                .px(8.0)
                .items_center()
                .rounded(theme.tokens.radii.sm);
            let row = if selected {
                row.bg(theme.accent().with_alpha(0.28))
            } else {
                row
            };
            row.child(text(label).text_color(if selected {
                c.text_primary
            } else {
                c.text_secondary
            }))
        };
        div()
            .size(480.0, 300.0)
            .child(blob(
                Rect::new(120.0, 40.0, 140.0, 140.0),
                ThemeColor::oklch(0.72, 0.17, 30.0, 1.0),
            ))
            .child(blob(
                Rect::new(40.0, 170.0, 110.0, 110.0),
                ThemeColor::oklch(0.8, 0.15, 150.0, 1.0),
            ))
            .child(
                div()
                    .absolute(Rect::new(12.0, 12.0, 184.0, 276.0))
                    .material(Material::Frost)
                    .rounded(theme.tokens.radii.lg)
                    .flex_col()
                    .p(12.0)
                    .gap(2.0)
                    .child(text("LIBRARY").font_size(11.0).text_color(c.text_tertiary))
                    .child(item("Scenes", false))
                    .child(item("Charms", true))
                    .child(item("Rooms", false))
                    .child(item("Assets", false)),
            )
            .child(
                div()
                    .absolute(Rect::new(220.0, 210.0, 236.0, 0.0))
                    .child(text("Frost over a gradient").font_size(17.0)),
            )
    }
}

fn frost_sidebar(scale: f32) -> Result<Shot, ShotError> {
    let bridge = PaintBridge::new(FontSystem::test());
    let mut scene = settle(&bridge, |_| Sidebar, Size::new(480.0, 300.0), |_| {})?;
    let full = Rect::new(0.0, 0.0, 480.0, 300.0);
    scene.commands.insert(
        0,
        PaintCmd::Quad(Quad {
            element: ElementId::ROOT,
            rect: full,
            radii: Corners::default(),
            fill: Some(Fill::LinearGradient {
                start: Vec2::new(0.0, 0.0),
                end: Vec2::new(480.0, 300.0),
                from: ThemeColor::oklch(0.45, 0.14, 280.0, 1.0),
                to: ThemeColor::oklch(0.35, 0.12, 200.0, 1.0),
            }),
            border: None,
            shadow: None,
            material: None,
        }),
    );
    finish(bridge, &scene, scale, |_, _| Ok(()))
}

fn dark() -> Color {
    surface(0)
}

/// A scene at 1x and 2x (`names` are the two scene names).
fn pair(
    names: [&'static str; 2],
    description: &'static str,
    width: u32,
    height: u32,
    build: fn(f32) -> Result<Shot, ShotError>,
) -> [Scene; 2] {
    [1.0, 2.0].map(|scale| Scene {
        name: if scale == 1.0 { names[0] } else { names[1] },
        description,
        width,
        height,
        scale,
        build,
        clear: dark,
    })
}

/// The UI scenes, each at 1x and 2x.
pub fn scenes() -> Vec<Scene> {
    [
        pair(
            ["button-states-1x", "button-states-2x"],
            "Button (livspeil-ui-widgets): rest, hover, pressed, focused, disabled",
            576,
            100,
            button_states,
        ),
        pair(
            ["toggle-slider-1x", "toggle-slider-2x"],
            "Toggles on/off and sliders at 30% / 75% with labels",
            320,
            168,
            toggle_slider,
        ),
        pair(
            ["text-samples-1x", "text-samples-2x"],
            "Inter sizes and weights, tnum digits, Georgian, bidi, a wrapped paragraph",
            640,
            380,
            text_samples,
        ),
        pair(
            ["frost-sidebar-1x", "frost-sidebar-2x"],
            "a Frost sidebar with labels over a gradient",
            480,
            300,
            frost_sidebar,
        ),
    ]
    .into_iter()
    .flatten()
    .collect()
}
