// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The wgpu backend against the CPU reference on display lists that
//! exercise every op path. Skipped (with a message) when no adapter exists.

use livspeil_ui_render::{
    Affine, AtlasStore, AtlasUpdate, Backdrop, Border, Color, CornerStyle, Curve, DisplayList,
    Fill, FrameOptions, GlyphSprite, Glyphs, GpuContext, GradientStop, Highlight, HighlightKind,
    Op, Quad, Radii, Rect, RenderError, Renderer, RgbaImage, Shadow, TARGET_FORMAT, encode,
    render_cpu, render_cpu_with,
};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Software Vulkan (lavapipe in CI) can crash when several devices are
/// created and dropped on parallel test threads, so GPU tests hold this
/// lock for their whole run.
static GPU_LOCK: Mutex<()> = Mutex::new(());

fn gpu() -> Option<(MutexGuard<'static, ()>, GpuContext, Renderer)> {
    let guard = GPU_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    match GpuContext::headless() {
        Ok(ctx) => {
            let r = Renderer::new(&ctx.device, TARGET_FORMAT);
            Some((guard, ctx, r))
        }
        Err(RenderError::NoAdapter(e)) => {
            eprintln!("SKIPPING GPU test: no wgpu adapter ({e})");
            None
        }
        Err(e) => panic!("{e}"),
    }
}

/// `(max channel delta, pixels with delta > 4)`.
fn diff(a: &RgbaImage, b: &RgbaImage) -> (u8, usize) {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let mut max = 0;
    let mut over = 0;
    for (p, q) in a
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.data.as_chunks::<4>().0)
    {
        let d = p
            .iter()
            .zip(q)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        max = max.max(d);
        if d > 4 {
            over += 1;
        }
    }
    (max, over)
}

fn opts(w: u32, h: u32, scale: f32) -> FrameOptions {
    FrameOptions {
        width: w,
        height: h,
        scale,
        clear: Color::oklch(0.2, 0.02, 250.0),
    }
}

fn mixed_list() -> DisplayList {
    let mut list = DisplayList::new();
    list.add(
        Quad::new(Rect::new(0.0, 0.0, 160.0, 120.0), Color::BLACK).fill(Fill::Linear {
            start: [0.0, 0.0],
            end: [1.0, 0.3],
            stops: vec![
                GradientStop {
                    offset: 0.0,
                    color: Color::oklch(0.5, 0.2, 20.0),
                },
                GradientStop {
                    offset: 0.5,
                    color: Color::oklch(0.8, 0.1, 120.0),
                },
                GradientStop {
                    offset: 1.0,
                    color: Color::oklch(0.4, 0.15, 260.0),
                },
            ],
        }),
    );
    for i in 0..8 {
        let t = i as f32;
        let mut q = Quad::new(
            Rect::new(8.0 + t * 18.0, 10.0 + (t * 7.0) % 30.0, 14.0 + t, 20.0),
            Color::oklch(0.6 + 0.04 * t, 0.12, t * 45.0).with_alpha(0.5 + 0.06 * t),
        )
        .radius(t)
        .border(Border::hairline(Color::WHITE.with_alpha(0.3)))
        .shadows(&[Shadow {
            offset: [1.0, 2.0],
            blur: t * 2.0,
            spread: t - 3.0,
            color: Color::BLACK.with_alpha(0.4),
        }]);
        if i % 2 == 1 {
            q.corners = CornerStyle::Circular;
        }
        if i % 3 == 0 {
            q.transform = Affine::rotate_about(0.2 * t, 60.0, 60.0);
        }
        if i == 5 {
            q.clip = Some(Rect::new(90.0, 0.0, 15.5, 40.0));
            q.opacity = 0.7;
        }
        list.add(q);
    }
    // Backdrop touching the edge of the target, rotated.
    list.add(Backdrop {
        radii: Radii {
            tl: 12.0,
            tr: 2.0,
            br: 8.0,
            bl: 0.0,
        },
        highlight: Some(Highlight {
            width: 1.0,
            color: Color::WHITE.with_alpha(0.2),
            kind: HighlightKind::TopRim,
        }),
        saturation: 1.5,
        grain: 0.02,
        transform: Affine::rotate_about(-0.1, 120.0, 80.0),
        shadows: vec![Shadow {
            offset: [0.0, 4.0],
            blur: 12.0,
            spread: 0.0,
            color: Color::BLACK.with_alpha(0.5),
        }],
        ..Backdrop::new(
            Rect::new(90.0, 50.0, 80.0, 60.0),
            10.0,
            Color::WHITE.with_alpha(0.2),
        )
    });
    // Tint-only backdrop, then quads on top in a higher layer.
    list.add(Backdrop::new(
        Rect::new(10.0, 70.0, 50.0, 40.0),
        0.0,
        Color::BLACK.with_alpha(0.4),
    ));
    list.push(
        1,
        Quad::new(Rect::new(20.5, 80.25, 30.0, 10.0), Color::WHITE).radius(5.0),
    );
    list
}

#[test]
fn gpu_matches_cpu() {
    let Some((_gpu, ctx, mut renderer)) = gpu() else {
        return;
    };
    eprintln!("adapter: {:?}", ctx.adapter_info);
    for (scale, w, h) in [(1.0, 160, 120), (2.0, 320, 240), (1.5, 240, 180)] {
        let frame = encode(&mixed_list(), opts(w, h, scale)).unwrap();
        let cpu = render_cpu(&frame);
        let gpu = ctx.render_to_image(&mut renderer, &frame).unwrap();
        let (max, over) = diff(&cpu, &gpu);
        eprintln!("scale {scale}: max delta {max}, {over} pixels over 4");
        let limit = if ctx.is_software() { 0.002 } else { 0.01 };
        assert!(
            (over as f64) <= limit * f64::from(w * h),
            "scale {scale}: {over} pixels differ"
        );
    }
}

#[test]
fn backdrop_first_and_empty_frames() {
    let Some((_gpu, ctx, mut renderer)) = gpu() else {
        return;
    };
    let mut list = DisplayList::new();
    list.add(Backdrop::new(
        Rect::new(4.0, 4.0, 20.0, 20.0),
        6.0,
        Color::WHITE.with_alpha(0.1),
    ));
    for l in [list, DisplayList::new()] {
        let frame = encode(&l, opts(32, 32, 1.0)).unwrap();
        let cpu = render_cpu(&frame);
        let gpu = ctx.render_to_image(&mut renderer, &frame).unwrap();
        assert!(diff(&cpu, &gpu).0 <= 4);
    }
}

#[test]
fn invalid_input_is_a_typed_error() {
    let mut list = DisplayList::new();
    list.add(Quad::new(Rect::new(0.0, 0.0, f32::NAN, 1.0), Color::WHITE));
    assert!(matches!(
        encode(&list, opts(8, 8, 1.0)),
        Err(RenderError::InvalidPrimitive { index: 0, .. })
    ));
    let mut list = DisplayList::new();
    let stop = GradientStop {
        offset: 0.0,
        color: Color::WHITE,
    };
    list.add(
        Quad::new(Rect::new(0.0, 0.0, 4.0, 4.0), Color::WHITE).fill(Fill::Linear {
            start: [0.0, 0.0],
            end: [1.0, 0.0],
            stops: vec![stop; 5],
        }),
    );
    assert!(matches!(
        encode(&list, opts(8, 8, 1.0)),
        Err(RenderError::GradientStops { count: 5, .. })
    ));
    assert!(matches!(
        encode(&DisplayList::new(), opts(0, 8, 1.0)),
        Err(RenderError::EmptyTarget)
    ));
}

#[test]
fn cpu_reference_basics() {
    // An opaque white square at 1x covers exactly its pixels.
    let mut list = DisplayList::new();
    list.add(Quad::new(Rect::new(2.0, 2.0, 4.0, 4.0), Color::WHITE));
    let img = render_cpu(&encode(&list, opts(8, 8, 1.0)).unwrap());
    assert_eq!(img.pixel(2, 2), Some([255, 255, 255, 255]));
    assert_eq!(img.pixel(5, 5), Some([255, 255, 255, 255]));
    assert_ne!(img.pixel(6, 6), Some([255, 255, 255, 255]));
    // A hairline is one physical pixel wide at 2x.
    let mut list = DisplayList::new();
    list.add(Quad {
        fill: None,
        ..Quad::new(Rect::new(1.0, 1.0, 6.0, 6.0), Color::WHITE)
            .border(Border::hairline(Color::WHITE))
    });
    let img = render_cpu(&encode(&list, opts(16, 16, 2.0)).unwrap());
    assert_eq!(img.pixel(2, 8), Some([255, 255, 255, 255]));
    let inner = img.pixel(3, 8).unwrap();
    let clear = img.pixel(8, 8).unwrap();
    assert_eq!(inner, clear);
}

/// A 32x16 atlas with two "glyphs": a ring (0..12) and a soft disc (16..28).
fn glyph_atlas(generation: u64) -> Vec<AtlasUpdate> {
    let mut data = vec![0u8; 32 * 16];
    for y in 0..12 {
        for x in 0..12 {
            let (dx, dy) = (x as f32 - 5.5, y as f32 - 5.5);
            let r = (dx * dx + dy * dy).sqrt();
            data[y * 32 + x] = ((1.0 - (r - 4.0).abs() / 1.5).clamp(0.0, 1.0) * 255.0) as u8;
            data[y * 32 + 16 + x] = ((5.5 - r).clamp(0.0, 1.0) * 255.0) as u8;
        }
    }
    // Full upload, then a dirty rect that fills a 2x2 block in a corner.
    vec![
        AtlasUpdate {
            atlas: 0,
            generation,
            size: [32, 16],
            region: [0, 0, 32, 16],
            data,
        },
        AtlasUpdate {
            atlas: 0,
            generation,
            size: [32, 16],
            region: [26, 10, 2, 2],
            data: vec![255; 4],
        },
    ]
}

fn glyph_list(scale: f32) -> DisplayList {
    let s = |v: f32| v / scale;
    let ring = |x: f32, y: f32| GlyphSprite {
        rect: Rect::new(s(x), s(y), s(12.0), s(12.0)),
        texels: [0, 0, 12, 12],
    };
    let disc = |x: f32, y: f32| GlyphSprite {
        rect: Rect::new(s(x), s(y), s(12.0), s(12.0)),
        texels: [16, 0, 12, 12],
    };
    let mut list = DisplayList::new();
    list.add(Quad::new(Rect::new(0.0, 0.0, 30.0, 60.0), Color::WHITE));
    list.add(Glyphs::new(
        0,
        vec![ring(2.0, 2.0), disc(16.0, 2.0)],
        Color::BLACK,
    ));
    list.add(Quad::new(
        Rect::new(30.0, 0.0, 34.0, 60.0),
        Color::oklch(0.3, 0.05, 250.0),
    ));
    list.add(Glyphs {
        opacity: 0.6,
        clip: Some(Rect::new(0.0, 0.0, 44.0, 64.0)),
        ..Glyphs::new(
            0,
            vec![ring(62.0, 4.0), disc(76.0, 4.0)],
            Color::oklch(0.9, 0.1, 80.0),
        )
    });
    list.add(Glyphs {
        transform: Affine::rotate_about(0.4, 40.0, 40.0),
        ..Glyphs::new(0, vec![ring(70.0, 70.0), disc(90.0, 70.0)], Color::WHITE)
    });
    // A second (missing) atlas: draws nothing, but splits the batch.
    list.add(Glyphs::new(7, vec![ring(2.0, 40.0)], Color::WHITE));
    list
}

#[test]
fn glyphs_gpu_matches_cpu() {
    for scale in [1.0, 2.0] {
        let (w, h) = ((64.0 * scale) as u32, (64.0 * scale) as u32);
        let frame = encode(&glyph_list(scale), opts(w, h, scale)).unwrap();
        // Quads and glyphs of one atlas batch together; the other atlas
        // starts a new draw.
        let draws: Vec<_> = frame
            .ops
            .iter()
            .map(|op| match op {
                Op::Draw { atlas, .. } => *atlas,
                Op::Backdrop { .. } => None,
            })
            .collect();
        assert_eq!(draws, vec![Some(0), Some(7)]);
        let mut store = AtlasStore::new();
        store.apply_all(&glyph_atlas(0)).unwrap();
        let cpu = render_cpu_with(&frame, &store);
        // Texels map 1:1 (sprite rects are given in physical pixels / scale):
        // the disc's center pixel is fully black on white.
        assert_eq!(cpu.pixel(21, 7), Some([0, 0, 0, 255]));
        // Without the atlas no text is drawn.
        assert_eq!(render_cpu(&frame).pixel(21, 7), Some([255, 255, 255, 255]));
        let Some((_gpu, ctx, mut renderer)) = gpu() else {
            return;
        };
        renderer
            .update_atlases(&ctx.device, &ctx.queue, &glyph_atlas(0))
            .unwrap();
        let gpu = ctx.render_to_image(&mut renderer, &frame).unwrap();
        let (max, over) = diff(&cpu, &gpu);
        eprintln!("glyphs scale {scale}: max delta {max}, {over} pixels over 4");
        assert!(
            over as f64 <= 0.002 * f64::from(w * h),
            "{over} pixels differ"
        );
        // A new generation clears the atlas: only the patched block exists.
        let mut next = glyph_atlas(1);
        next.remove(0);
        renderer
            .update_atlases(&ctx.device, &ctx.queue, &next)
            .unwrap();
        store.apply_all(&next).unwrap();
        let cpu = render_cpu_with(&frame, &store);
        let gpu = ctx.render_to_image(&mut renderer, &frame).unwrap();
        assert!(diff(&cpu, &gpu).0 <= 4);
    }
}

/// Curves that fold onto themselves: a loop, a cusp, a collinear stroke that
/// turns back over itself (a node editor's backward level link), a bend
/// tighter than the stroke is wide, and an ordinary link.
const FOLDING: [[[f32; 2]; 4]; 5] = [
    [[10.0, 50.0], [90.0, -20.0], [-20.0, -20.0], [60.0, 50.0]],
    [[80.0, 10.0], [140.0, 60.0], [80.0, 60.0], [140.0, 10.0]],
    [[40.0, 70.0], [100.0, 70.0], [-10.0, 70.0], [50.0, 70.0]],
    [[100.0, 80.0], [180.0, 82.0], [180.0, 86.0], [100.0, 88.0]],
    [[8.0, 100.0], [80.0, 100.0], [80.0, 114.0], [152.0, 114.0]],
];

const STROKE: f32 = 8.0;

/// One translucent white stroke on black, on its own: one blend gives level
/// 102, two would give 163.
fn folding_list(points: [[f32; 2]; 4]) -> DisplayList {
    let mut list = DisplayList::new();
    list.add(Curve::new(points, STROKE, Color::WHITE.with_alpha(0.4)));
    list
}

fn bezier(p: [[f32; 2]; 4], t: f32) -> [f32; 2] {
    let u = 1.0 - t;
    let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    [0, 1].map(|k| (0..4).map(|i| w[i] * p[i][k]).sum())
}

/// A translucent stroke is blended exactly once everywhere it covers: no
/// pixel is brighter than one blend, and every pixel well inside the stroke
/// has the full single blend (no gaps), at both renderers' hands.
fn assert_single_blend(img: &RgbaImage, points: [[f32; 2]; 4], scale: f32, who: &str) {
    let radius = STROKE * 0.5 * scale;
    // Distance of every pixel center near the curve to the curve itself.
    let mut distance = vec![f32::MAX; (img.width * img.height) as usize];
    let reach = radius + 2.0;
    for i in 0..=1500 {
        let [x, y] = bezier(points, i as f32 / 1500.0).map(|v| v * scale);
        for py in (y - reach).max(0.0) as u32..((y + reach) as u32 + 1).min(img.height) {
            for px in (x - reach).max(0.0) as u32..((x + reach) as u32 + 1).min(img.width) {
                let d = (px as f32 + 0.5 - x).hypot(py as f32 + 0.5 - y);
                let best = &mut distance[(py * img.width + px) as usize];
                *best = best.min(d);
            }
        }
    }
    let full = 102;
    let mut covered = 0;
    for (k, d) in distance.into_iter().enumerate() {
        let (x, y) = (k as u32 % img.width, k as u32 / img.width);
        let v = img.pixel(x, y).unwrap()[0];
        assert!(v <= full, "{who}: double blend {v} at {x},{y}");
        if d <= radius - 1.0 {
            assert_eq!(v, full, "{who}: gap at {x},{y} ({d} px from the curve)");
            covered += 1;
        }
        if d >= radius + 1.0 {
            assert_eq!(v, 0, "{who}: stray pixel at {x},{y}");
        }
    }
    assert!(covered > 100, "{who}: only {covered} interior pixels");
}

#[test]
fn folded_translucent_curves_are_blended_once() {
    let mut gpu = gpu();
    for (i, points) in FOLDING.into_iter().enumerate() {
        for scale in [1.0, 2.0] {
            let options = FrameOptions {
                width: (160.0 * scale) as u32,
                height: (120.0 * scale) as u32,
                scale,
                clear: Color::BLACK,
            };
            let frame = encode(&folding_list(points), options).unwrap();
            let cpu = render_cpu(&frame);
            assert_single_blend(&cpu, points, scale, &format!("cpu curve {i} x{scale}"));
            if let Some((_, ctx, renderer)) = gpu.as_mut() {
                let img = ctx.render_to_image(renderer, &frame).unwrap();
                assert_single_blend(&img, points, scale, &format!("gpu curve {i} x{scale}"));
                let (max, over) = diff(&cpu, &img);
                assert_eq!(over, 0, "curve {i} x{scale}: max delta {max}");
            }
        }
    }
}

/// Curves among the other families: translucent, thinner than a pixel,
/// transformed, clipped, faded, with different segment counts in one run,
/// and quads and sprites above and below them.
fn curve_list() -> DisplayList {
    let mut list = DisplayList::new();
    list.add(Quad::new(
        Rect::new(10.0, 10.0, 90.0, 60.0),
        Color::oklch(0.5, 0.1, 140.0),
    ));
    for (i, points) in FOLDING.into_iter().enumerate() {
        list.add(Curve::new(
            points,
            1.0 + i as f32,
            Color::oklch(0.8, 0.12, 60.0 * i as f32).with_alpha(0.7),
        ));
    }
    // A straight one (one segment) in the same run, and hairlines.
    list.add(Curve::new(
        [[5.0, 5.0], [55.0, 15.0], [105.0, 25.0], [155.0, 35.0]],
        3.0,
        Color::WHITE,
    ));
    for (i, width) in [0.25, 0.5, 1.0].into_iter().enumerate() {
        let y = 90.5 + 4.0 * i as f32;
        list.add(Curve::new(
            [[90.0, y], [110.0, y], [130.0, y + 9.0], [155.0, y + 9.0]],
            width,
            Color::WHITE,
        ));
    }
    list.add(Quad::new(Rect::new(60.0, 40.0, 30.0, 30.0), Color::BLACK).radius(8.0));
    list.add(Glyphs::new(
        0,
        vec![GlyphSprite {
            rect: Rect::new(66.0, 46.0, 12.0, 12.0),
            texels: [16, 0, 12, 12],
        }],
        Color::WHITE,
    ));
    let moved = Curve::new(FOLDING[4], 4.0, Color::oklch(0.7, 0.2, 20.0));
    list.add(Curve {
        transform: Affine::rotate_about(-0.5, 80.0, 60.0).then_after(Affine::scale(1.3, 0.7)),
        opacity: 0.6,
        ..moved
    });
    list.add(Curve {
        transform: Affine::translate(0.25, -60.0),
        clip: Some(Rect::new(30.5, 30.0, 60.0, 25.25)),
        ..moved
    });
    list
}

#[test]
fn curves_gpu_matches_cpu() {
    let mut store = AtlasStore::new();
    store.apply_all(&glyph_atlas(0)).unwrap();
    let Some((_gpu, ctx, mut renderer)) = gpu() else {
        return;
    };
    renderer
        .update_atlases(&ctx.device, &ctx.queue, &glyph_atlas(0))
        .unwrap();
    for (scale, w, h) in [(1.0, 160, 120), (2.0, 320, 240), (1.5, 240, 180)] {
        let frame = encode(&curve_list(), opts(w, h, scale)).unwrap();
        // Quads, sprites and curves in painter order, curves in their own
        // draws.
        assert_eq!(frame.draw_calls(), 4, "{:?}", frame.ops);
        let cpu = render_cpu_with(&frame, &store);
        let gpu = ctx.render_to_image(&mut renderer, &frame).unwrap();
        let (max, over) = diff(&cpu, &gpu);
        eprintln!("curves scale {scale}: max delta {max}, {over} pixels over 4");
        let limit = if ctx.is_software() { 0.002 } else { 0.001 };
        assert!(
            (over as f64) <= limit * f64::from(w * h),
            "scale {scale}: {over} pixels differ"
        );
    }
}
