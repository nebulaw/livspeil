// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Scene conversion, text caching and atlas uploads.

use livspeil_ui_core::{
    App, Element, ElementId, FontFeature, FontSpec, PaintCmd, Quad, Render, Scene, TestHost,
    TextRun, ViewContext, div, text,
};
use livspeil_ui_layout::{Corners, Rect, Size, Transform2D, Vec2};
use livspeil_ui_paint::{FontSystem, PaintBridge, PaintError, TextContext};
use livspeil_ui_render::{BorderWidth, DisplayList, Primitive};
use livspeil_ui_theme::{Color, Material, Theme, Tier};
use livspeil_ui_widgets::Button;

struct Demo;

impl Render for Demo {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let theme = cx.theme().clone();
        div()
            .flex_col()
            .p(16.0)
            .gap(8.0)
            .child(text("Hello, Livspeil").name("hello"))
            .child(Button::new("Apply").render(&theme).key("apply"))
    }
}

fn demo_scene(bridge: &PaintBridge) -> (TestHost, Scene) {
    let mut app = App::new();
    app.set_text_measurer(bridge.text.measurer());
    let view = app.new_view(Demo);
    app.set_root(view);
    let mut host = TestHost::new(app, Size::new(240.0, 120.0));
    host.run_until_idle(50);
    let scene = host.scene().cloned().unwrap();
    (host, scene)
}

fn glyph_runs(list: &DisplayList) -> usize {
    list.items
        .iter()
        .filter(|i| matches!(i.primitive, Primitive::Glyphs(_)))
        .count()
}

#[test]
fn core_view_paints_quads_and_text_with_cached_layouts() {
    let mut bridge = PaintBridge::new(FontSystem::test());
    let (host, scene) = demo_scene(&bridge);
    // Measured, not the character-count estimate: Inter at 13 px is ~6.5
    // px per char.
    let hello = host.app.bounds(host.app.find("hello").unwrap()).unwrap();
    assert!(hello.width > 60.0 && hello.width < 110.0, "{hello:?}");
    assert!(hello.height > 13.0 && hello.height < 20.0, "{hello:?}");

    let shaped_by_layout = bridge.text.stats().shaped;
    let mut list = DisplayList::new();
    let updates = bridge.paint_scene(&scene, 2.0, &mut list).unwrap();
    assert_eq!(glyph_runs(&list), 2);
    // The button: hairline border, continuous corners.
    let button = list
        .items
        .iter()
        .find_map(|i| match &i.primitive {
            Primitive::Quad(q) if q.border.is_some() => Some(q),
            _ => None,
        })
        .unwrap();
    assert_eq!(button.border.unwrap().width, BorderWidth::Hairline);
    // First frame: one full upload.
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].region[2..], updates[0].size);
    // Painting reused the layouts measured for layout.
    assert_eq!(bridge.text.stats().shaped, shaped_by_layout);

    // Second frame: nothing reshaped, placements reused, nothing uploaded.
    let before = bridge.text.stats();
    let mut again = DisplayList::new();
    let updates = bridge.paint_scene(&scene, 2.0, &mut again).unwrap();
    assert!(updates.is_empty());
    assert_eq!(again, list);
    let after = bridge.text.stats();
    assert_eq!(after.shaped, before.shaped);
    assert_eq!(after.placements_reused, before.placements_reused + 2);

    // A new scale rasterizes new glyphs: dirty rectangles, not a full upload.
    let mut one_x = DisplayList::new();
    let updates = bridge.paint_scene(&scene, 1.0, &mut one_x).unwrap();
    assert!(!updates.is_empty());
    assert!(updates.iter().all(|u| u.region[2..] != u.size));
}

fn run(text: &str, rect: Rect) -> PaintCmd {
    PaintCmd::Text(TextRun {
        element: ElementId::ROOT,
        rect,
        text: text.into(),
        font: FontSpec::sized(13.0),
        color: Color::WHITE,
    })
}

fn material_quad(theme: &Theme, m: Material) -> PaintCmd {
    let recipe = theme.material(m);
    PaintCmd::Quad(Quad {
        element: ElementId::ROOT,
        rect: Rect::new(10.0, 10.0, 100.0, 50.0),
        radii: Corners::all(10.0),
        fill: None,
        border: None,
        shadow: recipe.elevation.map(|e| (e, theme.shadow(e))),
        material: Some((m, recipe)),
    })
}

#[test]
fn materials_follow_the_tier() {
    let mut bridge = PaintBridge::new(FontSystem::test());
    let mut theme = Theme::default();
    for (tier, backdrop) in [(Tier::T2, true), (Tier::T1, true), (Tier::T0, false)] {
        theme.tier = tier;
        let scene = Scene {
            commands: vec![material_quad(&theme, Material::Frost)],
        };
        let mut list = DisplayList::new();
        bridge.paint_scene(&scene, 1.0, &mut list).unwrap();
        match &list.items[0].primitive {
            Primitive::Backdrop(b) => {
                assert!(backdrop, "{tier:?}");
                let want = if tier == Tier::T2 { 24.0 } else { 12.0 };
                assert_eq!(b.blur, want);
                assert_eq!(b.shadows.len(), 2, "e2 has two layers");
                assert!(b.highlight.is_some());
            }
            Primitive::Quad(q) => {
                assert!(!backdrop, "{tier:?}");
                assert!(q.fill.is_none() || q.shadows.len() == 2);
            }
            other => panic!("{other:?}"),
        }
    }
    // Glass uses a top rim; Ink draws no surface.
    theme.tier = Tier::T2;
    let scene = Scene {
        commands: vec![
            material_quad(&theme, Material::Glass),
            material_quad(&theme, Material::Ink),
        ],
    };
    let mut list = DisplayList::new();
    bridge.paint_scene(&scene, 1.0, &mut list).unwrap();
    let Primitive::Backdrop(b) = &list.items[0].primitive else {
        panic!()
    };
    assert_eq!(
        b.highlight.map(|h| h.kind),
        Some(livspeil_ui_render::HighlightKind::TopRim)
    );
    assert!(
        list.items[1..]
            .iter()
            .all(|i| !matches!(i.primitive, Primitive::Backdrop(_)))
    );
}

#[test]
fn stacks_apply_to_quads_and_text() {
    let mut bridge = PaintBridge::new(FontSystem::test());
    let scene = Scene {
        commands: vec![
            PaintCmd::PushTransform(Transform2D::translate(Vec2::new(10.0, 20.0))),
            PaintCmd::PushOpacity(0.5),
            PaintCmd::PushClip {
                rect: Rect::new(0.0, 0.0, 50.0, 30.0),
                radii: Corners::default(),
            },
            PaintCmd::PushOpacity(0.5),
            run("Clipped", Rect::new(0.0, 0.0, 200.0, 20.0)),
            PaintCmd::PopOpacity,
            PaintCmd::PopClip,
            PaintCmd::PopOpacity,
            PaintCmd::PopTransform,
            run("Free", Rect::new(0.0, 0.0, 200.0, 20.0)),
        ],
    };
    let mut list = DisplayList::new();
    bridge.paint_scene(&scene, 1.0, &mut list).unwrap();
    let Primitive::Glyphs(g) = &list.items[0].primitive else {
        panic!()
    };
    assert_eq!(g.opacity, 0.25);
    // Translation is folded into the (pixel-snapped) glyph positions.
    assert_eq!(g.transform, livspeil_ui_render::Affine::IDENTITY);
    assert!(
        g.glyphs
            .iter()
            .all(|s| s.rect.x >= 10.0 && s.rect.y >= 20.0)
    );
    let clip = g.clip.unwrap();
    assert_eq!((clip.x, clip.y, clip.w, clip.h), (10.0, 20.0, 50.0, 30.0));
    let Primitive::Glyphs(g) = &list.items[1].primitive else {
        panic!()
    };
    assert_eq!((g.opacity, g.clip), (1.0, None));
    // Texels map 1:1 at 1x: sprite rects are whole pixels of texel size.
    for s in &g.glyphs {
        assert_eq!(s.rect.x.fract(), 0.0);
        assert_eq!(s.rect.w, s.texels[2] as f32);
    }

    let bad = Scene {
        commands: vec![PaintCmd::PopClip],
    };
    assert!(matches!(
        bridge.paint_scene(&bad, 1.0, &mut list),
        Err(PaintError::UnbalancedStack)
    ));
    let bad = Scene {
        commands: vec![PaintCmd::PushOpacity(0.5), PaintCmd::PopTransform],
    };
    assert!(matches!(
        bridge.paint_scene(&bad, 1.0, &mut list),
        Err(PaintError::UnbalancedStack)
    ));
}

#[test]
fn narrow_boxes_wrap() {
    let mut bridge = PaintBridge::new(FontSystem::test());
    let para = "The quick brown fox jumps over the lazy dog and keeps running far away";
    let lines = |width: f32, bridge: &mut PaintBridge| {
        let scene = Scene {
            commands: vec![run(para, Rect::new(0.0, 0.0, width, 100.0))],
        };
        let mut list = DisplayList::new();
        bridge.paint_scene(&scene, 1.0, &mut list).unwrap();
        let Primitive::Glyphs(g) = &list.items[0].primitive else {
            panic!()
        };
        // Glyph bottoms sit near the baseline: count 10 px rows.
        let mut tops: Vec<i32> = g
            .glyphs
            .iter()
            .map(|s| (s.rect.bottom() / 10.0) as i32)
            .collect();
        tops.sort_unstable();
        tops.dedup();
        tops.len()
    };
    assert!(lines(1000.0, &mut bridge) <= 2);
    assert!(lines(120.0, &mut bridge) >= 4);
}

#[test]
fn full_atlas_is_cleared_and_repainted() {
    let mut bridge = PaintBridge {
        text: TextContext::with_atlas(FontSystem::test(), 3, 128),
    };
    let mut commands = Vec::new();
    for size in [13.0f32, 15.0, 17.0, 22.0, 28.0] {
        commands.push(PaintCmd::Text(TextRun {
            element: ElementId::ROOT,
            rect: Rect::new(0.0, 0.0, 800.0, 40.0),
            text: "ABCDEFGHIJKLMNOPQRSTUVWXYZ".into(),
            font: FontSpec::sized(size),
            color: Color::WHITE,
        }));
    }
    let small = Scene {
        commands: commands[..1].to_vec(),
    };
    let big = Scene { commands };
    let mut list = DisplayList::new();
    bridge.paint_scene(&small, 1.0, &mut list).unwrap();
    // The big scene does not fit next to the small one, nor alone.
    list.items.clear();
    let r = bridge.paint_scene(&big, 1.0, &mut list);
    assert!(matches!(r, Err(PaintError::Text(_))), "{r:?}");
    // After the failure the atlas was cleared; the small scene paints again
    // and its upload is a full one (new generation).
    list.items.clear();
    let updates = bridge.paint_scene(&small, 1.0, &mut list).unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].atlas, 3);
    assert_eq!(updates[0].region[2..], updates[0].size);
}

#[test]
fn core_text_weight_and_features_reach_the_shaper() {
    let mut bridge = PaintBridge::new(FontSystem::test());
    let measurer = bridge.text.measurer();
    let regular = measurer.measure("Semibold 1111", &FontSpec::sized(15.0));
    let bold = measurer.measure(
        "Semibold 1111",
        &FontSpec {
            weight: 600.0,
            ..FontSpec::sized(15.0)
        },
    );
    assert!(bold.width > regular.width, "{bold:?} vs {regular:?}");
    let digits = |f: Vec<FontFeature>| {
        measurer
            .measure(
                "1111",
                &FontSpec {
                    features: f,
                    ..FontSpec::sized(13.0)
                },
            )
            .width
    };
    // Inter's proportional 1 is narrower than its tabular 1.
    assert!(digits(vec![FontFeature::TNUM]) > digits(Vec::new()));

    // Runs with different fonts shape separately and both paint.
    let rect = Rect::new(0.0, 0.0, 200.0, 20.0);
    let scene = Scene {
        commands: vec![
            run("1111", rect),
            PaintCmd::Text(TextRun {
                element: ElementId::ROOT,
                rect,
                text: "1111".into(),
                font: FontSpec {
                    weight: 600.0,
                    features: vec![FontFeature::TNUM],
                    ..FontSpec::sized(13.0)
                },
                color: Color::WHITE,
            }),
        ],
    };
    let mut list = DisplayList::new();
    bridge.paint_scene(&scene, 1.0, &mut list).unwrap();
    let runs: Vec<_> = list
        .items
        .iter()
        .filter_map(|i| match &i.primitive {
            Primitive::Glyphs(g) => Some(g),
            _ => None,
        })
        .collect();
    assert_eq!(runs.len(), 2);
    assert_ne!(runs[0].glyphs, runs[1].glyphs);
}

#[test]
fn a_wire_becomes_one_curve_under_the_current_state() {
    use livspeil_ui_core::Wire;
    use livspeil_ui_layout::{Point, Transform2D};
    use livspeil_ui_render::{Affine, Primitive};
    let w = Wire::link(
        Point::new(10.0, 10.0),
        Point::new(150.0, 80.0),
        2.0,
        Color::WHITE,
    );
    let scene = Scene {
        commands: vec![
            PaintCmd::PushTransform(Transform2D {
                a: 0.5,
                b: 0.0,
                c: 0.0,
                d: 0.5,
                e: 3.0,
                f: 4.0,
            }),
            PaintCmd::PushOpacity(0.5),
            PaintCmd::Wire(w),
            PaintCmd::PopOpacity,
            PaintCmd::PopTransform,
        ],
    };
    let mut bridge = PaintBridge::new(FontSystem::test());
    let mut list = DisplayList::new();
    bridge.paint_scene(&scene, 2.0, &mut list).unwrap();
    assert_eq!(list.len(), 1);
    let Primitive::Curve(c) = &list.items[0].primitive else {
        panic!("not a curve");
    };
    assert_eq!(
        c.points,
        [[10.0, 10.0], [80.0, 10.0], [80.0, 80.0], [150.0, 80.0]]
    );
    // The width stays in local units; the renderer applies the transform.
    assert_eq!(c.width, 2.0);
    assert_eq!(c.opacity, 0.5);
    assert_eq!(
        c.transform,
        Affine {
            m: [0.5, 0.0, 0.0, 0.5, 3.0, 4.0]
        }
    );
    assert_eq!(c.clip, None);
}

/// A text run moved by fractions of a pixel keeps the same glyph positions
/// relative to each other: it only ever shifts as a whole, by whole pixels.
#[test]
fn moving_text_keeps_its_glyph_spacing() {
    for scale in [1.0, 1.25, 2.0] {
        let mut bridge = PaintBridge::new(FontSystem::test());
        let (_host, scene) = demo_scene(&bridge);
        let run = scene
            .commands
            .iter()
            .find_map(|c| match c {
                PaintCmd::Text(t) => Some(t.clone()),
                _ => None,
            })
            .unwrap();
        let mut first: Option<Vec<(f32, f32)>> = None;
        for step in 0..20 {
            let mut moved = run.clone();
            moved.rect.x += step as f32 * 0.15;
            moved.rect.y += step as f32 * 0.15;
            let one = Scene {
                commands: vec![PaintCmd::Text(moved)],
            };
            let mut list = DisplayList::new();
            bridge.paint_scene(&one, scale, &mut list).unwrap();
            let glyphs = list
                .items
                .iter()
                .find_map(|i| match &i.primitive {
                    Primitive::Glyphs(g) => Some(g.glyphs.clone()),
                    _ => None,
                })
                .unwrap();
            let origin = (glyphs[0].rect.x, glyphs[0].rect.y);
            let relative: Vec<(f32, f32)> = glyphs
                .iter()
                .map(|g| {
                    (
                        ((g.rect.x - origin.0) * scale).round(),
                        ((g.rect.y - origin.1) * scale).round(),
                    )
                })
                .collect();
            assert_eq!(*first.get_or_insert_with(|| relative.clone()), relative);
        }
    }
}

/// Layout asks for the height of wrapped text; the bridge answers with its
/// own line breaking, so boxes fit what is painted.
#[test]
fn wrapped_height_follows_the_painted_lines() {
    let bridge = PaintBridge::new(FontSystem::test());
    let measurer = bridge.text.measurer();
    let font = FontSpec::sized(13.0);
    let text = "one two three four five six seven eight nine ten";
    let one_line = measurer.measure(text, &font);
    let wide = measurer.wrapped_height(text, &font, one_line.width + 1.0);
    assert!((wide - one_line.height).abs() < 0.5, "{wide} {one_line:?}");
    let narrow = measurer.wrapped_height(text, &font, one_line.width / 3.0);
    assert!(
        narrow >= one_line.height * 3.0 - 0.5,
        "{narrow} {one_line:?}"
    );
}
