// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The wgpu backend against the CPU reference on display lists that
//! exercise every op path. Skipped (with a message) when no adapter exists.

use livspeil_ui_render::{
    Affine, Backdrop, Border, Color, CornerStyle, DisplayList, Fill, FrameOptions, GpuContext,
    GradientStop, Highlight, HighlightKind, Quad, Radii, Rect, RenderError, Renderer, RgbaImage,
    Shadow, TARGET_FORMAT, encode, render_cpu,
};

fn gpu() -> Option<(GpuContext, Renderer)> {
    match GpuContext::headless() {
        Ok(ctx) => {
            let r = Renderer::new(&ctx.device, TARGET_FORMAT);
            Some((ctx, r))
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
    let Some((ctx, mut renderer)) = gpu() else {
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
    let Some((ctx, mut renderer)) = gpu() else {
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
