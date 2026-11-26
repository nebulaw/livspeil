// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The blur paths (D63): a render-only target (what a Metal drawable is)
//! draws through the offscreen copy and equals the direct path; a forced
//! fallback draws the backdrop's plain colour. Skipped without an adapter.

use livspeil_ui_render::{
    Backdrop, BlurPath, BlurSupport, Color, DisplayList, FrameOptions, GpuContext, Quad,
    Rect, RenderError, Renderer, RgbaImage, TARGET_FORMAT, encode,
};
use std::sync::{Mutex, MutexGuard, PoisonError};

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

fn opts() -> FrameOptions {
    FrameOptions {
        width: 160,
        height: 120,
        scale: 1.0,
        clear: Color::oklch(0.2, 0.02, 250.0),
    }
}

fn list() -> DisplayList {
    let mut l = DisplayList::new();
    for (i, hue) in [20.0, 140.0, 260.0].into_iter().enumerate() {
        l.add(Quad::new(
            Rect::new(i as f32 * 50.0, 0.0, 40.0, 120.0),
            Color::oklch(0.7, 0.2, hue),
        ));
    }
    let mut b = Backdrop::new(
        Rect::new(30.0, 30.0, 100.0, 60.0),
        6.0,
        Color::oklch(0.3, 0.02, 250.0).with_alpha(0.6),
    );
    b.radii = livspeil_ui_render::Radii::all(10.0);
    l.add(b);
    l
}

fn target(ctx: &GpuContext, usage: wgpu::TextureUsages) -> wgpu::Texture {
    ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test target"),
        size: wgpu::Extent3d {
            width: 160,
            height: 120,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TARGET_FORMAT,
        usage,
        view_formats: &[],
    })
}

fn render_into(
    ctx: &GpuContext,
    r: &mut Renderer,
    list: &DisplayList,
    usage: wgpu::TextureUsages,
) -> Result<RgbaImage, RenderError> {
    let frame = encode(list, opts()).expect("encode");
    let t = target(ctx, usage);
    r.render(&ctx.device, &ctx.queue, &frame, &t)?;
    ctx.read_texture(&t, 160, 120)
}

#[test]
fn render_only_target_matches_the_direct_path() {
    let Some((_g, ctx, mut r)) = gpu() else { return };
    assert_eq!(*r.blur_support(), BlurSupport::Supported);
    let direct_usage = Renderer::TARGET_USAGE | wgpu::TextureUsages::COPY_SRC;
    let render_only = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
    assert_eq!(r.blur_path(direct_usage), BlurPath::Direct);
    assert_eq!(r.blur_path(render_only), BlurPath::Offscreen);
    let a = render_into(&ctx, &mut r, &list(), direct_usage).expect("direct");
    let b = render_into(&ctx, &mut r, &list(), render_only).expect("offscreen");
    assert_eq!(a, b, "offscreen path differs from the direct path");
    // The backdrop really blurred: its interior differs from the plain
    // fallback rendering.
    let mut plain = list();
    plain.without_backdrops();
    let c = render_into(&ctx, &mut r, &plain, direct_usage).expect("plain");
    assert_ne!(a, c);
    // A second frame reuses the offscreen texture.
    let b2 = render_into(&ctx, &mut r, &list(), render_only).expect("again");
    assert_eq!(b, b2);
}

#[test]
fn forced_fallback_draws_the_plain_background_colour() {
    let Some((_g, ctx, mut r)) = gpu() else { return };
    let usage = Renderer::TARGET_USAGE | wgpu::TextureUsages::COPY_SRC;
    let reference = {
        let mut plain = list();
        plain.without_backdrops();
        render_into(&ctx, &mut r, &plain, usage).expect("reference")
    };
    r.force_blur_fallback("forced by the test");
    assert_eq!(r.blur_support().fallback_reason(), Some("forced by the test"));
    // A frame with backdrops is refused with the reason...
    match render_into(&ctx, &mut r, &list(), usage) {
        Err(RenderError::BlurUnavailable(why)) => assert_eq!(why, "forced by the test"),
        other => panic!("expected BlurUnavailable, got {other:?}"),
    }
    // ...and the caller's fallback (plain colour, no blur) renders.
    let mut plain = list();
    plain.without_backdrops();
    let out = render_into(&ctx, &mut r, &plain, usage).expect("fallback render");
    assert_eq!(out, reference);
    // The centre is the backdrop's own colour at full opacity: no tint of
    // the scene behind shows through.
    let px = |img: &RgbaImage, x: usize, y: usize| {
        let i = (y * 160 + x) * 4;
        [img.data[i], img.data[i + 1], img.data[i + 2], img.data[i + 3]]
    };
    let c = px(&out, 80, 60);
    assert_eq!(c[3], 255);
    assert!(c[0] < 90 && c[1] < 90 && c[2] < 110, "{c:?}");
}
