// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Bridge conversion of a scene with 500 labels and 200 quads at 2x:
//! warm (layouts and glyphs cached), cold layouts (reshaping, atlas warm)
//! and fully cold (reshaping and rasterizing every glyph).

use criterion::{Criterion, criterion_group, criterion_main};
use livspeil_ui_core::{Border, ElementId, Fill, PaintCmd, Quad, Scene, TextRun};
use livspeil_ui_layout::{Corners, Rect};
use livspeil_ui_paint::{FontSystem, PaintBridge};
use livspeil_ui_render::DisplayList;
use livspeil_ui_theme::Color;

fn scene() -> Scene {
    let mut commands = Vec::new();
    for i in 0..200u32 {
        let (x, y) = ((i % 10) as f32 * 190.0, (i / 10) as f32 * 50.0);
        commands.push(PaintCmd::Quad(Quad {
            element: ElementId::ROOT,
            rect: Rect::new(x, y, 180.0, 44.0),
            radii: Corners::all(6.0),
            fill: Some(Fill::Solid(Color::oklch(0.2, 0.01, 265.0, 1.0))),
            border: Some(Border {
                width: 1.0,
                color: Color::WHITE.with_alpha(0.06),
            }),
            shadow: None,
            material: None,
        }));
    }
    let words = ["Opacity", "Blend mode", "Position", "Rotation", "Scale"];
    for i in 0..500u32 {
        let (x, y) = ((i % 10) as f32 * 190.0 + 8.0, (i / 10) as f32 * 20.0 + 4.0);
        commands.push(PaintCmd::Text(TextRun {
            element: ElementId::ROOT,
            rect: Rect::new(x, y, 170.0, 16.0),
            text: format!("{} {}", words[i as usize % words.len()], i).into(),
            size: if i % 3 == 0 { 12.0 } else { 13.0 },
            color: Color::WHITE,
        }));
    }
    Scene { commands }
}

fn bench(c: &mut Criterion) {
    let scene = scene();
    let mut bridge = PaintBridge::new(FontSystem::test());
    let mut list = DisplayList::new();
    let convert = |bridge: &mut PaintBridge, list: &mut DisplayList| {
        list.items.clear();
        let updates = bridge.paint_scene(&scene, 2.0, list);
        assert!(updates.is_ok());
    };
    convert(&mut bridge, &mut list);
    c.bench_function("bridge 500 labels + 200 quads, warm", |b| {
        b.iter(|| convert(&mut bridge, &mut list));
    });
    c.bench_function("bridge 500 labels + 200 quads, cold layouts", |b| {
        b.iter(|| {
            bridge.text.clear_layouts();
            convert(&mut bridge, &mut list);
        });
    });
    c.bench_function(
        "bridge 500 labels + 200 quads, cold layouts and atlas",
        |b| {
            b.iter(|| {
                bridge.text.clear_layouts();
                bridge.text.clear_atlas();
                convert(&mut bridge, &mut list);
            });
        },
    );
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(20);
    targets = bench
}
criterion_main!(benches);
