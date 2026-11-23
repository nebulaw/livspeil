// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! CPU reference rasterizer. Executes an encoded [`Frame`] with the same
//! per-pixel math as the WGSL shaders (see `shade.rs`), the same pixel
//! coverage rule (pixel centers inside an instance's bounds), the same
//! dual-Kawase passes and the same 8-bit quantization after every draw, so
//! its output is deterministic, needs no GPU, and stays within a small
//! tolerance of the GPU backend. Goldens are rendered with it.
//!
//! It is a reference, not a fast path: it is meant for small test scenes.

use crate::encode::{BlurPlan, Frame, Instance, Op, field, level_size};
use crate::image::RgbaImage;
use crate::shade::{self, v4};

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

/// Renders a frame on the CPU.
pub fn render_cpu(frame: &Frame) -> RgbaImage {
    let (w, h) = (frame.options.width, frame.options.height);
    let clear = frame.clear.map(quantize);
    let mut img = RgbaImage::new(w, h);
    for px in img.data.chunks_exact_mut(4) {
        px.copy_from_slice(&clear);
    }
    let scale = frame.options.scale;
    for op in &frame.ops {
        match op {
            Op::Draw { start, end } => {
                for inst in frame
                    .instances
                    .get(*start as usize..*end as usize)
                    .unwrap_or(&[])
                {
                    raster(&mut img, inst, scale, |px| shade::shade(inst, px, scale));
                }
            }
            Op::Backdrop { instance, blur } => {
                let Some(inst) = frame.instances.get(*instance as usize) else {
                    continue;
                };
                let level1 = blur.map(|plan| blur_levels(&img, plan));
                let uv_scale = [inst[field::GRAD], inst[field::GRAD + 1]];
                raster(&mut img, inst, scale, |px| {
                    let blurred = match &level1 {
                        Some(l) => {
                            let s = l.sample([px[0] * uv_scale[0], px[1] * uv_scale[1]]);
                            [s[0], s[1], s[2]]
                        }
                        None => [0.0; 3],
                    };
                    shade::shade_backdrop(inst, px, scale, blurred)
                });
            }
        }
    }
    img
}

fn quantize(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Shades and blends every pixel whose center lies inside the instance's
/// (transformed) bounds, like the GPU rasterizer.
fn raster(
    img: &mut RgbaImage,
    inst: &Instance,
    scale: f32,
    shade_px: impl Fn([f32; 2]) -> [f32; 4],
) {
    let b = v4(inst, field::BOUNDS);
    let x = v4(inst, field::XFORM);
    let (e, f) = (inst[field::XFORM2], inst[field::XFORM2 + 1]);
    let corners = [[b[0], b[1]], [b[2], b[1]], [b[0], b[3]], [b[2], b[3]]];
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for [lx, ly] in corners {
        let sx = (x[0] * lx + x[2] * ly + e) * scale;
        let sy = (x[1] * lx + x[3] * ly + f) * scale;
        x0 = x0.min(sx);
        y0 = y0.min(sy);
        x1 = x1.max(sx);
        y1 = y1.max(sy);
    }
    let clip = v4(inst, field::CLIP);
    let ix0 = (x0.max(clip[0]).floor().max(0.0)) as u32;
    let iy0 = (y0.max(clip[1]).floor().max(0.0)) as u32;
    let ix1 = (x1.min(clip[2]).ceil().max(0.0) as u32).min(img.width);
    let iy1 = (y1.min(clip[3]).ceil().max(0.0) as u32).min(img.height);
    for py in iy0..iy1 {
        for px in ix0..ix1 {
            let c = [px as f32 + 0.5, py as f32 + 0.5];
            let p = shade::local_pos(inst, c, scale);
            if p[0] < b[0] || p[0] >= b[2] || p[1] < b[1] || p[1] >= b[3] {
                continue;
            }
            let src = shade_px(c);
            let i = (py as usize * img.width as usize + px as usize) * 4;
            let inv = 1.0 - src[3].clamp(0.0, 1.0);
            for (d, s) in img.data[i..i + 4].iter_mut().zip(src) {
                *d = quantize(s + f32::from(*d) / 255.0 * inv);
            }
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
    for (dst, src) in l0.px.iter_mut().zip(img.data.chunks_exact(4)) {
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
