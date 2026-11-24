// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! A window with the core's proof widgets (Button, Toggle, Slider) on a
//! dark surface.
//!
//! ```sh
//! cargo run -p livspeil-ui-host-winit --example hello
//! ```
//!
//! Text is not drawn yet: [`QuadsOnlyRenderer`] below is a TEMPORARY
//! stand-in that draws only quads. `livspeil-ui-paint` (Scene to display
//! list with text) will replace it.

use livspeil_ui_core::{
    App, CursorShape, Element, KEYMAP_FILE, Model, PaintCmd, Render, Scene, View, ViewContext, div,
    text, widgets,
};
use livspeil_ui_host_winit::{HostError, HostOptions, RenderTarget, SceneRenderer, Titlebar, run};
use livspeil_ui_render as render;
use livspeil_ui_theme::{ConfigKind, ConfigWatcher, config_dir};

/// TEMPORARY: maps `Quad` paint commands to `livspeil-ui-render` quads
/// (solid fills, borders, shadows, transforms, opacity, rectangular clips)
/// and ignores text, gradients' second color and materials' blur. Replaced
/// by the `livspeil-ui-paint` renderer.
struct QuadsOnlyRenderer {
    clear: render::Color,
    renderer: Option<(wgpu::TextureFormat, render::Renderer)>,
}

fn color(c: livspeil_ui_theme::Color) -> render::Color {
    render::Color::oklab(c.l, c.a, c.b).with_alpha(c.alpha)
}

fn intersect(a: render::Rect, b: render::Rect) -> render::Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let r = a.right().min(b.right());
    let btm = a.bottom().min(b.bottom());
    render::Rect::new(x, y, (r - x).max(0.0), (btm - y).max(0.0))
}

/// Flatten a scene into a display list.
fn display_list(scene: &Scene) -> render::DisplayList {
    let mut list = render::DisplayList::new();
    let mut transforms = vec![render::Affine::IDENTITY];
    let mut opacities = vec![1.0_f32];
    let mut clips: Vec<Option<render::Rect>> = vec![None];
    for cmd in &scene.commands {
        let t = *transforms.last().unwrap_or(&render::Affine::IDENTITY);
        let o = *opacities.last().unwrap_or(&1.0);
        let clip = clips.last().copied().flatten();
        match cmd {
            PaintCmd::PushTransform(m) => transforms.push(t.then_after(render::Affine {
                m: [m.a, m.b, m.c, m.d, m.e, m.f],
            })),
            PaintCmd::PopTransform => {
                transforms.pop();
            }
            PaintCmd::PushOpacity(v) => opacities.push(o * v),
            PaintCmd::PopOpacity => {
                opacities.pop();
            }
            PaintCmd::PushClip { rect, .. } => {
                let r = t.bounds(render::Rect::new(rect.x, rect.y, rect.width, rect.height));
                clips.push(Some(match clip {
                    Some(c) => intersect(c, r),
                    None => r,
                }));
            }
            PaintCmd::PopClip => {
                clips.pop();
            }
            PaintCmd::Quad(q) => {
                let fill = q.fill.map(|f| match f {
                    livspeil_ui_core::Fill::Solid(c) => render::Fill::Solid(color(c)),
                    livspeil_ui_core::Fill::LinearGradient { from, .. } => {
                        render::Fill::Solid(color(from))
                    }
                });
                let shadows = q
                    .shadow
                    .map(|(_, s)| {
                        std::iter::once(s.key)
                            .chain(s.contact)
                            .map(|l| render::Shadow {
                                offset: [l.x, l.y],
                                blur: l.blur,
                                spread: l.spread,
                                color: render::Color::BLACK.with_alpha(l.alpha),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                list.add(render::Quad {
                    rect: render::Rect::new(q.rect.x, q.rect.y, q.rect.width, q.rect.height),
                    radii: render::Radii {
                        tl: q.radii.top_left,
                        tr: q.radii.top_right,
                        br: q.radii.bottom_right,
                        bl: q.radii.bottom_left,
                    },
                    corners: render::CornerStyle::Continuous,
                    fill,
                    border: q.border.map(|b| render::Border {
                        width: render::BorderWidth::Logical(b.width),
                        color: color(b.color),
                    }),
                    shadows,
                    clip,
                    opacity: o,
                    transform: t,
                });
            }
            PaintCmd::Text(_) => {}
        }
    }
    list
}

impl SceneRenderer for QuadsOnlyRenderer {
    fn render(
        &mut self,
        gpu: &render::GpuContext,
        target: &RenderTarget<'_>,
        scene: &Scene,
    ) -> Result<(), HostError> {
        if self.renderer.as_ref().map(|(f, _)| *f) != Some(target.format) {
            self.renderer = Some((
                target.format,
                render::Renderer::new(&gpu.device, target.format),
            ));
        }
        let Some((_, renderer)) = &mut self.renderer else {
            return Ok(());
        };
        let frame = render::encode(
            &display_list(scene),
            render::FrameOptions {
                width: target.size_px.0,
                height: target.size_px.1,
                scale: target.scale,
                clear: self.clear,
            },
        )?;
        renderer.render(&gpu.device, &gpu.queue, &frame, target.texture)?;
        Ok(())
    }
}

struct Hello {
    clicks: Model<u32>,
    on: Model<bool>,
    slider: View<widgets::Slider>,
}

impl Render for Hello {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let theme = cx.theme().clone();
        let c = theme.colors();
        let clicks = self.clicks;
        let on = self.on;
        let n = *clicks.read(cx);
        let is_on = *on.read(cx);
        let row = |label: String, control: Element| {
            div()
                .flex_row()
                .items_center()
                .gap(16.0)
                .child(text(label).w(120.0))
                .child(control)
        };
        div()
            .w_full()
            .h_full()
            .bg(c.surface[0])
            .flex_col()
            .px(32.0)
            .py(24.0)
            .gap(20.0)
            .child(div().h(Titlebar::MACOS_INSET))
            .child(
                div()
                    .flex_col()
                    .gap(20.0)
                    .p(24.0)
                    .rounded(theme.tokens.radii.sm * 2.0)
                    .bg(c.surface[1])
                    .child(row(
                        format!("Clicked {n}x"),
                        widgets::button(&theme, "Click me")
                            .key("button")
                            .cursor(CursorShape::Pointer)
                            .on_click(move |_, cx| {
                                clicks.update(cx, |v, cx| {
                                    *v += 1;
                                    cx.notify();
                                });
                            }),
                    ))
                    .child(row(
                        "Toggle".to_owned(),
                        widgets::toggle(&theme, is_on)
                            .key("toggle")
                            .cursor(CursorShape::Pointer)
                            .on_click(move |_, cx| {
                                on.update(cx, |v, cx| {
                                    *v = !*v;
                                    cx.notify();
                                });
                            }),
                    ))
                    .child(row(
                        "Slider".to_owned(),
                        div()
                            .key("slider")
                            .cursor(CursorShape::Grab)
                            .child(Element::view(&self.slider)),
                    )),
            )
    }
}

fn main() {
    let mut app = App::new();
    let clicks = app.new_model(0_u32);
    let on = app.new_model(false);
    let value = app.new_model(40.0_f32);
    let slider = app.new_view(widgets::Slider::new(value, 0.0, 100.0, 1.0));
    let root = app.new_view(Hello { clicks, on, slider });
    app.set_root(root);

    let clear = color(app.theme().colors().surface[0]);
    let mut options = HostOptions::default()
        .title("Livspeil: hello")
        .size(720.0, 420.0);
    // Hot reload `~/.config/livspeil/ui/{theme,motion,keymap}.toml` if the
    // directory exists; otherwise no timer runs at all.
    if let Some(dir) = config_dir().filter(|d| d.is_dir()) {
        let mut watcher = ConfigWatcher::new();
        watcher.watch_dir(&dir);
        watcher.watch(dir.join(KEYMAP_FILE), ConfigKind::Raw("keymap"));
        options = options.config(watcher);
    }
    let renderer = QuadsOnlyRenderer {
        clear,
        renderer: None,
    };
    if let Err(e) = run(app, renderer, options) {
        eprintln!("hello: {e}");
        std::process::exit(1);
    }
}
