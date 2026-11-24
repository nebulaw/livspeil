// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! A window with `livspeil-ui-widgets` (Button, Toggle, Slider) on a Frost
//! card over a gradient, drawn through `livspeil-ui-paint` (text and
//! materials) and `livspeil-ui-render` with the host's `PaintRenderer`.
//! Cursor shapes, caret blink, IME, the clipboard and accessibility come
//! from the host.
//!
//! ```sh
//! cargo run -p livspeil-ui-host-winit --example hello
//! ```

use livspeil_ui_core::{App, Element, KEYMAP_FILE, Model, Render, View, ViewContext, div};
use livspeil_ui_host_winit::{HostOptions, PaintRenderer, Titlebar, WidgetClipboard, run};
use livspeil_ui_layout::Rect;
use livspeil_ui_paint::{FontSystem, PaintBridge};
use livspeil_ui_theme::{Color, ConfigKind, ConfigWatcher, Material, config_dir};
use livspeil_ui_widgets::{Button, Env, Label, Slider, Toggle, Tone};

struct Hello {
    clicks: Model<u32>,
    on: Model<bool>,
    slider: View<Slider>,
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
                .child(Label::new(label).render(&theme).w(120.0))
                .child(control)
        };
        let blob = |r: Rect, color: Color| div().absolute(r).rounded(r.width / 2.0).bg(color);
        div()
            .w_full()
            .h_full()
            .bg(c.surface[0])
            .flex_col()
            .px(32.0)
            .py(24.0)
            .gap(20.0)
            .child(blob(
                Rect::new(360.0, 40.0, 220.0, 220.0),
                Color::oklch(0.62, 0.17, 280.0, 1.0),
            ))
            .child(blob(
                Rect::new(140.0, 230.0, 160.0, 160.0),
                Color::oklch(0.7, 0.14, 200.0, 1.0),
            ))
            .child(div().h(Titlebar::MACOS_INSET))
            .child(
                Label::new("Hello, Livspeil")
                    .tone(Tone::Primary)
                    .render(&theme)
                    .font_size(theme.tokens.type_scale.sizes[5]),
            )
            .child(
                div()
                    .flex_col()
                    .gap(20.0)
                    .p(24.0)
                    .rounded(theme.tokens.radii.lg)
                    .material(Material::Frost)
                    .child(row(
                        format!("Clicked {n}x"),
                        Button::new("Click me")
                            .primary()
                            .on_click(move |_, cx| {
                                clicks.update(cx, |v, cx| {
                                    *v += 1;
                                    cx.notify();
                                });
                            })
                            .render(&theme),
                    ))
                    .child(row(
                        "Toggle".to_owned(),
                        Toggle::new(is_on)
                            .label(if is_on { "On" } else { "Off" })
                            .on_change(move |v, cx| {
                                on.update(cx, |o, cx| {
                                    *o = v;
                                    cx.notify();
                                });
                            })
                            .render(&theme),
                    ))
                    .child(row("Slider".to_owned(), Element::view(&self.slider))),
            )
    }
}

fn main() {
    let mut app = App::new();
    let renderer = PaintRenderer::for_theme(PaintBridge::new(FontSystem::new()), app.theme());
    // Layout measures text with the bridge's cache, so labels are shaped
    // once for layout and painting.
    renderer.install_measurer(&mut app);
    let env = Env::new(&mut app, FontSystem::new());
    WidgetClipboard::install_system(&env);
    let clicks = app.new_model(0_u32);
    let on = app.new_model(false);
    let value = app.new_model(40.0_f32);
    let slider = app.new_view(
        Slider::new(value, 0.0, 100.0, 1.0)
            .label("Volume")
            .show_value(),
    );
    let content = app.new_view(Hello { clicks, on, slider });
    let root = env.root(&mut app, content);
    app.set_root(root);

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
    if let Err(e) = run(app, renderer, options) {
        eprintln!("hello: {e}");
        std::process::exit(1);
    }
}
