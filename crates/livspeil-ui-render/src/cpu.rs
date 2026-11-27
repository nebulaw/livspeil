// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! CPU reference rasterizer. Executes an encoded [`Frame`] with the same
//! per-pixel math as the WGSL shaders (see `shade.rs`), the same pixel
//! coverage rule (pixel centers inside a record's bounds), the same
//! dual-Kawase passes and the same 8-bit quantization after every draw, so
//! its output is deterministic, needs no GPU, and stays within a small
//! tolerance of the GPU backend. Goldens are rendered with it.
//!
//! It is a reference, not a fast path: it is meant for small test scenes.

use crate::atlas::{AtlasStore, R8Texture};
use crate::curve::{self, MAX_CURVE_SEGMENTS};
use crate::encode::{BlurPlan, Frame, Op, level_size};
use crate::image::RgbaImage;
use crate::record::{
    BackdropRecord, CurveRecord, Family, GradientRecord, Property, QuadRecord, ShadowRecord,
    SpriteRecord, fill_kind,
};
use crate::shade;

/// A float texture (one blur level).
#[derive(Debug, Clone)]
pub(crate) struct Level {
    pub w: u32,
    pub h: u32,
    pub px: Vec<[f32; 4]>,
}

impl Level {
    fn new(w: u32, h: u32) -> Level {
        Level {
            w,
            h,
            px: vec![[0.0; 4]; w as usize * h as usize],
        }
    }

    fn get(&self, x: i64, y: i64) -> [f32; 4] {
        let x = x.clamp(0, i64::from(self.w) - 1) as usize;
        let y = y.clamp(0, i64::from(self.h) - 1) as usize;
        self.px[y * self.w as usize + x]
    }

    /// Bilinear sample with clamp-to-edge addressing.
    pub fn sample(&self, uv: [f32; 2]) -> [f32; 4] {
        let x = uv[0] * self.w as f32 - 0.5;
        let y = uv[1] * self.h as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (ix, iy) = (x0 as i64, y0 as i64);
        let a = self.get(ix, iy);
        let b = self.get(ix + 1, iy);
        let c = self.get(ix, iy + 1);
        let d = self.get(ix + 1, iy + 1);
        let mut out = [0.0; 4];
        for k in 0..4 {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bottom = c[k] + (d[k] - c[k]) * fx;
            out[k] = top + (bottom - top) * fy;
        }
        out
    }
}

/// Renders a frame on the CPU. Glyphs draw nothing without atlases; use
/// [`render_cpu_with`] for text.
pub fn render_cpu(frame: &Frame) -> RgbaImage {
    render_cpu_with(frame, &AtlasStore::new())
}

/// Renders a frame on the CPU, sampling glyph coverage from `atlases`
/// (glyphs of a missing atlas draw nothing).
pub fn render_cpu_with(frame: &Frame, atlases: &AtlasStore) -> RgbaImage {
    let (w, h) = (frame.options.width, frame.options.height);
    let clear = frame.clear.map(quantize);
    let mut img = RgbaImage::new(w, h);
    for px in img.data.as_chunks_mut::<4>().0 {
        *px = clear;
    }
    for op in &frame.ops {
        match op {
            Op::Draw {
                start, end, atlas, ..
            } => {
                let tex = atlas.and_then(|a| atlases.get(a));
                for i in *start..*end {
                    draw(&mut img, frame, i as usize, tex);
                }
            }
            Op::Backdrop { instance, blur } => {
                let Some((record, prop)) = frame
                    .instance(*instance as usize)
                    .and_then(|r| BackdropRecord::read(&frame.records, r.slot as usize))
                    .and_then(|b| Some((b, property(frame, b.prop)?)))
                else {
                    continue;
                };
                let level1 = blur.map(|plan| blur_levels(&img, plan));
                let uv_scale = [
                    1.0 / (2.0 * level_size(w, 1) as f32),
                    1.0 / (2.0 * level_size(h, 1) as f32),
                ];
                let scale = frame.options.scale;
                let bounds = shade::shape_bounds(record.rect, &prop);
                raster(&mut img, &prop, bounds, scale, |px| {
                    let blurred = match &level1 {
                        Some(l) => {
                            let s = l.sample([px[0] * uv_scale[0], px[1] * uv_scale[1]]);
                            [s[0], s[1], s[2]]
                        }
                        None => [0.0; 3],
                    };
                    shade::shade_backdrop(&record, &prop, px, scale, blurred)
                });
            }
        }
    }
    img
}

fn property(frame: &Frame, index: u32) -> Option<Property> {
    frame
        .properties
        .get(index as usize)
        .and_then(|slots| Property::read(slots, 0))
}

/// Draws instance `i` (every family but backdrops, which are their own op).
/// A reference outside the frame's arrays draws nothing.
fn draw(img: &mut RgbaImage, frame: &Frame, i: usize, tex: Option<&R8Texture>) {
    let Some(r) = frame.instance(i) else { return };
    let (records, at) = (&frame.records[..], r.slot as usize);
    let scale = frame.options.scale;
    match r.family {
        Family::Sprite => {
            let (Some(sprite), Some(tex)) = (SpriteRecord::read(records, at), tex) else {
                return;
            };
            let Some(prop) = property(frame, sprite.prop) else {
                return;
            };
            let bounds = shade::sprite_bounds(&sprite, &prop);
            raster(img, &prop, bounds, scale, |px| {
                shade::shade_sprite(&sprite, &prop, px, scale, |x, y| {
                    f32::from(tex.texel(x, y)) / 255.0
                })
            });
        }
        Family::Quad => {
            let Some(quad) = QuadRecord::read(records, at) else {
                return;
            };
            let Some(prop) = property(frame, quad.prop) else {
                return;
            };
            let gradient = (quad.flags & 0xff >= fill_kind::LINEAR)
                .then(|| GradientRecord::read(records, quad.effect as usize))
                .flatten();
            let bounds = shade::shape_bounds(quad.rect, &prop);
            raster(img, &prop, bounds, scale, |px| {
                shade::shade_quad(&quad, gradient.as_ref(), &prop, px, scale)
            });
        }
        Family::Shadow => {
            let Some(shadow) = ShadowRecord::read(records, at) else {
                return;
            };
            let Some(prop) = property(frame, shadow.prop) else {
                return;
            };
            let bounds = shade::shadow_bounds(&shadow, &prop);
            raster(img, &prop, bounds, scale, |px| {
                shade::shade_shadow(&shadow, &prop, px, scale)
            });
        }
        Family::Curve => {
            let Some(curve) = CurveRecord::read(records, at) else {
                return;
            };
            if let Some(prop) = property(frame, curve.prop) {
                raster_curve(img, &curve, &prop, scale);
            }
        }
        Family::Backdrop => {}
    }
}

fn quantize(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Blends a premultiplied `src` over pixel `(x, y)`.
fn blend(img: &mut RgbaImage, x: u32, y: u32, src: [f32; 4]) {
    let i = (y as usize * img.width as usize + x as usize) * 4;
    let inv = 1.0 - src[3].clamp(0.0, 1.0);
    for (d, s) in img.data[i..i + 4].iter_mut().zip(src) {
        *d = quantize(s + f32::from(*d) / 255.0 * inv);
    }
}

/// The pixels `[x0, y0, x1, y1)` that the local rectangle `b` may touch:
/// its transformed bounds within the clip and the image.
fn pixel_range(img: &RgbaImage, prop: &Property, b: [f32; 4], scale: f32) -> [u32; 4] {
    let [a, bb, c, d] = prop.linear;
    let [e, f, _, _] = prop.offset;
    let corners = [[b[0], b[1]], [b[2], b[1]], [b[0], b[3]], [b[2], b[3]]];
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for [lx, ly] in corners {
        let sx = (a * lx + c * ly + e) * scale;
        let sy = (bb * lx + d * ly + f) * scale;
        x0 = x0.min(sx);
        y0 = y0.min(sy);
        x1 = x1.max(sx);
        y1 = y1.max(sy);
    }
    let clip = prop.clip;
    [
        (x0.max(clip[0]).floor().max(0.0)) as u32,
        (y0.max(clip[1]).floor().max(0.0)) as u32,
        (x1.min(clip[2]).ceil().max(0.0) as u32).min(img.width),
        (y1.min(clip[3]).ceil().max(0.0) as u32).min(img.height),
    ]
}

/// Shades and blends every pixel whose center lies inside the (transformed)
/// local rectangle `b`, like the GPU rasterizer.
fn raster(
    img: &mut RgbaImage,
    prop: &Property,
    b: [f32; 4],
    scale: f32,
    shade_px: impl Fn([f32; 2]) -> [f32; 4],
) {
    let [ix0, iy0, ix1, iy1] = pixel_range(img, prop, b, scale);
    for py in iy0..iy1 {
        for px in ix0..ix1 {
            let c = [px as f32 + 0.5, py as f32 + 0.5];
            let p = shade::local_pos(prop, c, scale);
            if p[0] < b[0] || p[0] >= b[2] || p[1] < b[1] || p[1] >= b[3] {
                continue;
            }
            blend(img, px, py, shade_px(c));
        }
    }
}

/// A curve: the distance of each pixel to the polyline is the smallest over
/// the segments whose reach it is in, and each pixel is blended once. (The
/// GPU finds the same distance and lets the nearest segment's box draw the
/// pixel.)
fn raster_curve(img: &mut RgbaImage, curve: &CurveRecord, prop: &Property, scale: f32) {
    let [x0, y0, x1, y1] = pixel_range(img, prop, shade::curve_bounds(curve, prop), scale);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let width = (x1 - x0) as usize;
    let mut distance = vec![f32::INFINITY; width * (y1 - y0) as usize];
    let points = shade::control_points(curve);
    let reach = shade::curve_reach(curve, prop);
    let segments = curve.segments.clamp(1, MAX_CURVE_SEGMENTS);
    for i in 0..segments {
        let [a, b] = curve::segment(points, i, segments);
        let around = [
            a[0].min(b[0]) - reach,
            a[1].min(b[1]) - reach,
            a[0].max(b[0]) + reach,
            a[1].max(b[1]) + reach,
        ];
        let [sx0, sy0, sx1, sy1] = pixel_range(img, prop, around, scale);
        for py in sy0.max(y0)..sy1.min(y1) {
            for px in sx0.max(x0)..sx1.min(x1) {
                let p = shade::local_pos(prop, [px as f32 + 0.5, py as f32 + 0.5], scale);
                let d = curve::segment_distance(p, a, b);
                let best = &mut distance[(py - y0) as usize * width + (px - x0) as usize];
                *best = best.min(d);
            }
        }
    }
    for (k, d) in distance.iter().enumerate() {
        if d.is_finite() {
            let (px, py) = (x0 + (k % width) as u32, y0 + (k / width) as u32);
            let c = [px as f32 + 0.5, py as f32 + 0.5];
            blend(img, px, py, shade::shade_curve(curve, prop, c, *d));
        }
    }
}

/// Texel range of a full-resolution pixel region at a level.
fn level_region(region: [u32; 4], level: u32, w: u32, h: u32) -> [u32; 4] {
    let d = 1u32 << level;
    [
        (region[0] / d).min(w),
        (region[1] / d).min(h),
        region[2].div_ceil(d).min(w),
        region[3].div_ceil(d).min(h),
    ]
}

/// Uniforms of one blur pass, shared with the GPU backend.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PassParams {
    pub src_uv_scale: [f32; 2],
    pub half_texel: [f32; 2],
    pub dst_scale: f32,
    pub region: [u32; 4],
}

/// The passes of a blur plan: downs into levels 1..=L, then ups into
/// levels L-1..=1. Returns `(is_down, dst_level, params)`.
pub(crate) fn blur_passes(width: u32, height: u32, plan: BlurPlan) -> Vec<(bool, u32, PassParams)> {
    let mut out = Vec::new();
    let params = |src: u32, dst: u32| {
        let (sw, sh) = (level_size(width, src), level_size(height, src));
        let (dw, dh) = (level_size(width, dst), level_size(height, dst));
        let sd = (1u32 << src) as f32;
        PassParams {
            src_uv_scale: [1.0 / (sd * sw as f32), 1.0 / (sd * sh as f32)],
            half_texel: [0.5 / sw as f32 * plan.offset, 0.5 / sh as f32 * plan.offset],
            dst_scale: (1u32 << dst) as f32,
            region: level_region(plan.region, dst, dw, dh),
        }
    };
    for l in 1..=plan.levels {
        out.push((true, l, params(l - 1, l)));
    }
    for l in (1..plan.levels).rev() {
        out.push((false, l, params(l + 1, l)));
    }
    out
}

fn blur_levels(img: &RgbaImage, plan: BlurPlan) -> Level {
    let (w, h) = (img.width, img.height);
    let mut levels: Vec<Level> = Vec::with_capacity(plan.levels as usize + 1);
    let mut l0 = Level::new(w, h);
    for (dst, src) in l0.px.iter_mut().zip(img.data.as_chunks::<4>().0) {
        *dst = [0, 1, 2, 3].map(|k| f32::from(src[k]) / 255.0);
    }
    levels.push(l0);
    for l in 1..=plan.levels {
        levels.push(Level::new(level_size(w, l), level_size(h, l)));
    }
    for (down, dst_level, p) in blur_passes(w, h, plan) {
        let src_level = if down { dst_level - 1 } else { dst_level + 1 };
        let mut dst = std::mem::replace(&mut levels[dst_level as usize], Level::new(0, 0));
        if down {
            dst.px.fill([0.0; 4]);
        }
        let src = &levels[src_level as usize];
        let [rx0, ry0, rx1, ry1] = p.region;
        for ty in ry0..ry1 {
            for tx in rx0..rx1 {
                let pos = [
                    (tx as f32 + 0.5) * p.dst_scale,
                    (ty as f32 + 0.5) * p.dst_scale,
                ];
                let uv = [pos[0] * p.src_uv_scale[0], pos[1] * p.src_uv_scale[1]];
                let hx = p.half_texel[0];
                let hy = p.half_texel[1];
                let tap = |dx: f32, dy: f32| src.sample([uv[0] + dx, uv[1] + dy]);
                let mut sum = [0.0f32; 4];
                let mut acc = |c: [f32; 4], wgt: f32| {
                    for k in 0..4 {
                        sum[k] += c[k] * wgt;
                    }
                };
                let norm = if down {
                    acc(tap(0.0, 0.0), 4.0);
                    acc(tap(-hx, -hy), 1.0);
                    acc(tap(hx, hy), 1.0);
                    acc(tap(hx, -hy), 1.0);
                    acc(tap(-hx, hy), 1.0);
                    8.0
                } else {
                    acc(tap(-hx * 2.0, 0.0), 1.0);
                    acc(tap(-hx, hy), 2.0);
                    acc(tap(0.0, hy * 2.0), 1.0);
                    acc(tap(hx, hy), 2.0);
                    acc(tap(hx * 2.0, 0.0), 1.0);
                    acc(tap(hx, -hy), 2.0);
                    acc(tap(0.0, -hy * 2.0), 1.0);
                    acc(tap(-hx, -hy), 2.0);
                    12.0
                };
                dst.px[ty as usize * dst.w as usize + tx as usize] = sum.map(|v| v / norm);
            }
        }
        levels[dst_level as usize] = dst;
    }
    levels.swap_remove(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::{KAWASE_FIT, kawase_params};

    /// Blurs a vertical step edge and measures the 16%-84% width, which is
    /// 2 sigma for a Gaussian.
    fn measured_sigma(levels: u32, offset: f32) -> f32 {
        let (w, h) = (512u32, 8u32);
        let mut img = RgbaImage::new(w, h);
        for y in 0..h {
            for x in w / 2..w {
                let i = ((y * w + x) * 4) as usize;
                img.data[i..i + 4].copy_from_slice(&[255; 4]);
            }
        }
        let l1 = blur_levels(
            &img,
            BlurPlan {
                levels,
                offset,
                region: [0, 0, w, h],
            },
        );
        let at = |x: f32| l1.sample([(x + 0.5) / (2.0 * l1.w as f32), 0.5])[0];
        let find = |target: f32| {
            let mut x = 0.0;
            while x < w as f32 && at(x) < target {
                x += 0.25;
            }
            x
        };
        (find(0.8413) - find(0.1587)) / 2.0
    }

    #[test]
    fn kawase_sigma() {
        // Print the table used to fit KAWASE_FIT; check the fit is within
        // 20% for the parameter range kawase_params produces.
        for levels in 1..=5 {
            for &o in &[0.5f32, 1.0, 1.5, 2.0] {
                let s = measured_sigma(levels, o);
                let fit = (1u32 << levels) as f32 * (KAWASE_FIT.0 + KAWASE_FIT.1 * o);
                eprintln!("levels={levels} offset={o}: sigma={s:.2} fit={fit:.2}");
            }
        }
        for &r in &[4.0f32, 12.0, 24.0, 48.0] {
            let (l, o) = kawase_params(r);
            let s = measured_sigma(l, o);
            eprintln!("radius {r}: levels={l} offset={o:.2} sigma={s:.2}");
            assert!((s / r - 1.0).abs() < 0.2, "radius {r}: sigma {s}");
        }
    }
}
