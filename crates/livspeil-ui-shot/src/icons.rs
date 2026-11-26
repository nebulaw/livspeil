// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The icon sheet: every icon of the set at the design system's four sizes,
//! in the dark and the light theme, at 1x and 2x.

use livspeil_ui_core::{App, Element, Render, TestHost, ViewContext, div, icon, text};
use livspeil_ui_icons::IconId;
use livspeil_ui_layout::Size;
use livspeil_ui_paint::{FontSystem, PaintBridge};
use livspeil_ui_render::Color;
use livspeil_ui_theme::{Scheme, Theme};
use livspeil_ui_widgets::IconSize;

use crate::ShotError;
use crate::scenes::{Scene, Shot};
use crate::ui::finish;

const COLUMNS: usize = 5;
const CELL: (f32, f32) = (192.0, 46.0);
const PAD: f32 = 8.0;
// Five columns and ten rows hold the 49 icons.
const WIDTH: u32 = 5 * 192 + 16;
const HEIGHT: u32 = 10 * 46 + 16;

struct Sheet;

impl Render for Sheet {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let theme = cx.theme().clone();
        let c = theme.colors();
        let mut sheet = div().w_full().h_full().flex_col().p(PAD).bg(c.surface[0]);
        let ids: Vec<IconId> = IconId::all().collect();
        for row in ids.chunks(COLUMNS) {
            let mut line = div().flex_row();
            for id in row {
                let mut sizes = div().flex_row().items_center().gap(10.0).h(24.0);
                for s in IconSize::ALL {
                    sizes = sizes.child(icon(id.name()).size(s.px(), s.px()));
                }
                // The last one in the accent: tinting.
                sizes = sizes.child(icon(id.name()).text_color(theme.accent()));
                line = line.child(
                    div()
                        .size(CELL.0, CELL.1)
                        .flex_col()
                        .items_center()
                        .gap(3.0)
                        .child(sizes)
                        .child(text(id.name()).font_size(10.0).text_color(c.text_secondary)),
                );
            }
            sheet = sheet.child(line);
        }
        sheet
    }
}

fn sheet(scheme: Scheme, scale: f32) -> Result<Shot, ShotError> {
    let bridge = PaintBridge::new(FontSystem::test());
    let mut app = App::new();
    app.set_text_measurer(bridge.text.measurer());
    app.set_theme(Theme {
        scheme,
        ..Theme::default()
    });
    let view = app.new_view(Sheet);
    app.set_root(view);
    let mut host = TestHost::new(app, Size::new(WIDTH as f32, HEIGHT as f32));
    host.run_until_idle(100);
    let mut scene = host
        .scene()
        .cloned()
        .ok_or_else(|| ShotError::Scene("the view drew no frame".into()))?;
    crate::gallery::widen_text_runs(&mut scene);
    finish(bridge, &scene, scale, |_, _| Ok(()))
}

fn dark(scale: f32) -> Result<Shot, ShotError> {
    sheet(Scheme::Dark, scale)
}

fn light(scale: f32) -> Result<Shot, ShotError> {
    sheet(Scheme::Light, scale)
}

type Named = (&'static str, f32, fn(f32) -> Result<Shot, ShotError>);

/// `icons-<scheme>-<scale>`.
pub fn scenes() -> Vec<Scene> {
    let all: [Named; 4] = [
        ("icons-dark-1x", 1.0, dark),
        ("icons-dark-2x", 2.0, dark),
        ("icons-light-1x", 1.0, light),
        ("icons-light-2x", 2.0, light),
    ];
    all.map(|(name, scale, build)| Scene {
        name,
        description: "every icon at 14, 16, 20 and 24 px, and 16 px in the accent",
        width: WIDTH,
        height: HEIGHT,
        scale,
        build,
        // The sheet paints its own background.
        clear: || Color::BLACK,
    })
    .to_vec()
}
