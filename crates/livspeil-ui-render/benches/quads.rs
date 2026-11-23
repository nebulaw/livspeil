// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! 10,000 quads at 1920x1080: CPU encode time, and GPU frame time (submit
//! and wait) when an adapter exists. Run with `cargo xtask bench` or
//! `cargo bench -p livspeil-ui-render`.

use criterion::{Criterion, criterion_group, criterion_main};
use livspeil_ui_render::{
    Border, Color, DisplayList, FrameOptions, GpuContext, Quad, Rect, Renderer, Shadow,
    TARGET_FORMAT, encode,
};

const W: u32 = 1920;
const H: u32 = 1080;

fn ten_thousand_quads() -> DisplayList {
    let mut list = DisplayList::new();
    for i in 0..10_000u32 {
        let x = (i % 100) as f32 * 19.2;
        let y = (i / 100) as f32 * 10.8;
        let mut q = Quad::new(
            Rect::new(x, y, 16.0, 8.0),
            Color::oklch(0.5, 0.1, (i % 360) as f32),
        )
        .radius(3.0)
        .border(Border::hairline(Color::WHITE.with_alpha(0.1)));
        if i % 10 == 0 {
            q.shadows = vec![Shadow {
                offset: [0.0, 1.0],
                blur: 2.0,
                spread: 0.0,
                color: Color::BLACK.with_alpha(0.3),
            }];
        }
        list.add(q);
    }
    list
}

fn bench(c: &mut Criterion) {
    let list = ten_thousand_quads();
    let opts = FrameOptions {
        width: W,
        height: H,
        scale: 1.0,
        clear: Color::BLACK,
    };
    c.bench_function("encode 10k quads", |b| {
        b.iter(|| encode(std::hint::black_box(&list), opts))
    });
    c.bench_function("encode 10k quads + instance bytes", |b| {
        b.iter(|| encode(&list, opts).map(|f| f.instance_bytes().len()))
    });

    let ctx = match GpuContext::headless() {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("skipping GPU benchmark: {e}");
            return;
        }
    };
    eprintln!(
        "GPU adapter: {} ({:?})",
        ctx.adapter_info.name, ctx.adapter_info.backend
    );
    let mut renderer = Renderer::new(&ctx.device, TARGET_FORMAT);
    let target = ctx.device.create_texture(&wgpu_target(W, H));
    let Ok(frame) = encode(&list, opts) else {
        return;
    };
    let mut group = c.benchmark_group("gpu");
    group.sample_size(10);
    group.bench_function("render 10k quads 1080p (submit + wait)", |b| {
        b.iter(|| {
            let _ = renderer.render(&ctx.device, &ctx.queue, &frame, &target);
            let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
        })
    });
    group.finish();
}

fn wgpu_target(w: u32, h: u32) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("bench target"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TARGET_FORMAT,
        usage: Renderer::TARGET_USAGE,
        view_formats: &[],
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
