// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Icons: element to paint command to atlas sprite to pixels.

use livspeil_ui_core::{
    App, Element, ElementId, IconRun, PaintCmd, Render, Scene, TestHost, ViewContext, div, icon,
};
use livspeil_ui_icons::{IconId, IconRasterizer};
use livspeil_ui_layout::{Corners, Rect, Size, Transform2D, Vec2};
use livspeil_ui_paint::{FontSystem, PaintBridge};
use livspeil_ui_render::{
    Affine, AtlasStore, AtlasUpdate, Color, DisplayList, FrameOptions, GlyphSprite, Glyphs,
    Primitive, RgbaImage, encode, render_cpu_with,
};
use livspeil_ui_theme::{Color as ThemeColor, Theme};

fn run(name: &str, rect: Rect) -> PaintCmd {
    PaintCmd::Icon(IconRun {
        element: ElementId::ROOT,
        rect,
        name: name.into(),
        color: ThemeColor::WHITE,
    })
}

fn paint(
    bridge: &mut PaintBridge,
    commands: Vec<PaintCmd>,
    scale: f32,
) -> (DisplayList, Vec<AtlasUpdate>) {
    let mut list = DisplayList::new();
    let updates = bridge
        .paint_scene(&Scene { commands }, scale, &mut list)
        .unwrap();
    (list, updates)
}

fn sprites(list: &DisplayList) -> Vec<(&Glyphs, GlyphSprite)> {
    list.items
        .iter()
        .filter_map(|i| match &i.primitive {
            Primitive::Glyphs(g) => Some((g, g.glyphs[0])),
            _ => None,
        })
        .collect()
}

/// The sprite's rectangle in physical pixels.
fn physical(g: &Glyphs, s: GlyphSprite, scale: f32) -> [f32; 4] {
    let r = g.transform.bounds(s.rect);
    [r.x * scale, r.y * scale, r.w * scale, r.h * scale]
}

fn assert_close(got: [f32; 4], want: [f32; 4]) {
    assert!(
        got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-3),
        "{got:?} != {want:?}"
    );
}

#[test]
fn icons_are_snapped_and_map_texels_to_pixels() {
    let mut bridge = PaintBridge::new(FontSystem::test());
    for scale in [1.0, 1.25, 2.0] {
        let (list, _) = paint(
            &mut bridge,
            vec![run("bx-copy", Rect::new(10.3, 20.6, 16.0, 16.0))],
            scale,
        );
        let (g, s) = sprites(&list)[0];
        let [x, y, w, h] = physical(g, s, scale);
        assert!((x - x.round()).abs() < 1e-3, "{scale}: x {x}");
        assert!((y - y.round()).abs() < 1e-3, "{scale}: y {y}");
        // One texel per physical pixel.
        assert_eq!(s.texels[2], (16.0 * scale) as u32);
        assert!((w - s.texels[2] as f32).abs() < 1e-3);
        assert!((h - s.texels[3] as f32).abs() < 1e-3);
        // Within half a pixel of where the element is.
        assert!((x - 10.3 * scale).abs() <= 0.5 + 1e-3);
        assert!((y - 20.6 * scale).abs() <= 0.5 + 1e-3);
    }
}

#[test]
fn icons_move_with_their_parent_transform_like_quads() {
    let mut bridge = PaintBridge::new(FontSystem::test());
    let at = Rect::new(4.0, 4.0, 16.0, 16.0);
    let inside = |t: Transform2D| {
        vec![
            PaintCmd::PushTransform(t),
            run("bx-copy", at),
            PaintCmd::PopTransform,
        ]
    };
    // Translation.
    let (list, _) = paint(
        &mut bridge,
        inside(Transform2D::translate(Vec2::new(100.0, 50.0))),
        2.0,
    );
    let (g, s) = sprites(&list)[0];
    assert_close(physical(g, s, 2.0), [208.0, 108.0, 32.0, 32.0]);
    // Uniform scale (zoomed canvas): rasterized at the size on screen.
    let (list, _) = paint(&mut bridge, inside(Transform2D::scale(2.0, 2.0)), 1.0);
    let (g, s) = sprites(&list)[0];
    assert_eq!(s.texels[2], 32);
    assert_close(physical(g, s, 1.0), [8.0, 8.0, 32.0, 32.0]);
    // Rotation: the sprite carries the transform, like a quad.
    let turn = Transform2D::translate(Vec2::new(40.0, 0.0));
    let turn = Transform2D {
        a: 0.0,
        b: 1.0,
        c: -1.0,
        d: 0.0,
        ..turn
    };
    let (list, _) = paint(&mut bridge, inside(turn), 1.0);
    let (g, s) = sprites(&list)[0];
    assert_eq!(
        g.transform,
        Affine {
            m: [turn.a, turn.b, turn.c, turn.d, turn.e, turn.f]
        }
    );
    assert_eq!((s.rect.x, s.rect.y, s.rect.w), (4.0, 4.0, 16.0));
}

#[test]
fn icons_are_cached_per_physical_size_and_inherit_opacity_and_clip() {
    let mut bridge = PaintBridge::new(FontSystem::test());
    let two = vec![
        run("bx-copy", Rect::new(0.0, 0.0, 16.0, 16.0)),
        run("bx-copy", Rect::new(20.0, 0.0, 16.0, 16.0)),
    ];
    let (list, updates) = paint(&mut bridge, two.clone(), 1.0);
    assert_eq!(bridge.text.stats().atlas_glyphs, 1);
    assert_eq!(updates.len(), 1);
    assert_eq!(sprites(&list)[0].1.texels, sprites(&list)[1].1.texels);
    // Nothing to upload the second time; a new scale adds one entry.
    assert!(paint(&mut bridge, two.clone(), 1.0).1.is_empty());
    assert_eq!(paint(&mut bridge, two, 2.0).1.len(), 1);
    assert_eq!(bridge.text.stats().atlas_glyphs, 2);

    let (list, _) = paint(
        &mut bridge,
        vec![
            PaintCmd::PushOpacity(0.5),
            PaintCmd::PushClip {
                rect: Rect::new(0.0, 0.0, 8.0, 8.0),
                radii: Corners::default(),
            },
            run("bx-gear", Rect::new(0.0, 0.0, 16.0, 16.0)),
            PaintCmd::PopClip,
            PaintCmd::PopOpacity,
        ],
        1.0,
    );
    let (g, _) = sprites(&list)[0];
    assert_eq!(g.opacity, 0.5);
    assert_eq!(g.clip.map(|c| (c.w, c.h)), Some((8.0, 8.0)));
}

#[test]
fn a_full_atlas_is_cleared_and_the_icons_drawn_again() {
    let mut bridge = PaintBridge::new(FontSystem::test());
    bridge.text = livspeil_ui_paint::TextContext::with_atlas(FontSystem::test(), 0, 64);
    let one = |name: &str| vec![run(name, Rect::new(0.0, 0.0, 40.0, 40.0))];
    paint(&mut bridge, one("bx-copy"), 1.0);
    let (list, updates) = paint(&mut bridge, one("bx-gear"), 1.0);
    assert_eq!(sprites(&list).len(), 1);
    assert_eq!(updates[0].region[2..], updates[0].size);
}

#[test]
fn unknown_names_draw_a_visible_box() {
    let mut bridge = PaintBridge::new(FontSystem::test());
    let (list, updates) = paint(
        &mut bridge,
        vec![run("bx-nonsense", Rect::new(2.0, 2.0, 16.0, 16.0))],
        1.0,
    );
    let Primitive::Quad(q) = &list.items[0].primitive else {
        panic!("{list:?}")
    };
    assert!(q.fill.is_none() && q.border.is_some_and(|b| b.color.alpha > 0.5));
    assert_eq!((q.rect.x, q.rect.w), (2.0, 16.0));
    // Only the initial empty atlas.
    assert!(updates.iter().all(|u| u.data.iter().all(|&b| b == 0)));
}

struct Row;

impl Render for Row {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let accent = cx.theme().accent();
        div()
            .flex_row()
            .items_center()
            .w(40.0)
            .child(icon("bx-copy"))
            .child(icon("bx-gear").size(24.0, 24.0).text_color(accent))
    }
}

#[test]
fn icon_elements_have_a_size_do_not_shrink_and_take_a_color() {
    let mut app = App::new();
    let view = app.new_view(Row);
    app.set_root(view);
    // 16 + 24 fills the 40 px row; a narrower window must not squash them.
    let mut host = TestHost::new(app, Size::new(30.0, 40.0));
    host.run_until_idle(20);
    let runs: Vec<IconRun> = host
        .scene()
        .unwrap()
        .commands
        .iter()
        .filter_map(|c| match c {
            PaintCmd::Icon(i) => Some(i.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(runs.len(), 2);
    assert_eq!((runs[0].rect.width, runs[0].rect.height), (16.0, 16.0));
    assert_eq!((runs[1].rect.width, runs[1].rect.height), (24.0, 24.0));
    assert_eq!(runs[0].color, Theme::default().colors().text_primary);
    assert_eq!(runs[1].color, Theme::default().accent());
}

/// The CPU renderer puts the mask's coverage on the pixels it was snapped
/// to: pixel (x, y) is lit exactly where mask (x - 10, y - 6) is.
#[test]
fn cpu_pixels_match_the_mask() {
    let mut bridge = PaintBridge::new(FontSystem::test());
    let (list, updates) = paint(
        &mut bridge,
        vec![run("bx-copy", Rect::new(5.0, 3.0, 16.0, 16.0))],
        2.0,
    );
    let frame = encode(
        &list,
        FrameOptions {
            width: 64,
            height: 48,
            scale: 2.0,
            clear: Color::BLACK,
        },
    )
    .unwrap();
    let mut atlases = AtlasStore::new();
    atlases.apply_all(&updates).unwrap();
    let img: RgbaImage = render_cpu_with(&frame, &atlases);
    let mask = IconRasterizer::new()
        .rasterize(IconId::from_name("bx-copy").unwrap(), 32.0)
        .unwrap();
    for y in 0..48u32 {
        for x in 0..64u32 {
            let m = match (x.checked_sub(10), y.checked_sub(6)) {
                (Some(mx), Some(my)) if mx < 32 && my < 32 => mask.data[(my * 32 + mx) as usize],
                _ => 0,
            };
            let got = img.data[((y * 64 + x) * 4) as usize];
            match m {
                0 => assert!(got <= 2, "({x},{y}) lit outside the mask: {got}"),
                255 => assert!(got >= 250, "({x},{y}) not solid: {got}"),
                _ => assert!(got > 0, "({x},{y}) partial coverage lost"),
            }
        }
    }
}
