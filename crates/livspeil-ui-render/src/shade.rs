// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Per-record math, mirrored from `shaders/primitives.wgsl`: the geometry
//! each record covers (also what the encoder culls by) and the color of one
//! pixel. The CPU reference rasterizer runs these functions; keep both files
//! in step (the GPU-vs-CPU tests catch drift).

use crate::color::oklab_to_srgb;
use crate::encode::TEXT_GAMMA;
use crate::record::{
    BackdropRecord, CurveRecord, GradientRecord, Property, QuadRecord, ShadowRecord, SpriteRecord,
    fill_kind,
};

type V2 = [f32; 2];
type V4 = [f32; 4];

fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

const BAYER4: [f32; 16] = [
    0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0,
];

/// Ordered dither in -0.5..0.5 for an integer pixel.
pub(crate) fn bayer(px: V2) -> f32 {
    let x = (px[0].max(0.0) as u32) & 3;
    let y = (px[1].max(0.0) as u32) & 3;
    (BAYER4[(y * 4 + x) as usize] + 0.5) / 16.0 - 0.5
}

/// White noise in -0.5..0.5 for an integer pixel (integer hash, identical
/// on every backend).
pub(crate) fn noise(px: V2) -> f32 {
    let x = px[0].max(0.0) as u32;
    let y = px[1].max(0.0) as u32;
    let mut h = x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    (h >> 8) as f32 / 16_777_216.0 - 0.5
}

/// Local position of a physical pixel center.
pub(crate) fn local_pos(prop: &Property, px: V2, scale: f32) -> V2 {
    let [a, b, c, d] = prop.inverse;
    let [_, _, e, f] = prop.offset;
    let (sx, sy) = (px[0] / scale, px[1] / scale);
    [a * sx + c * sy + e, b * sx + d * sy + f]
}

/// `[x0, y0, x1, y1]` of `(x, y, w, h)` grown by `d` on every side.
fn grown(rect: V4, d: f32) -> V4 {
    let [x, y, w, h] = rect;
    let (x, y) = (x - d, y - d);
    [x, y, x + (w + 2.0 * d), y + (h + 2.0 * d)]
}

/// What a quad or backdrop covers: its rect and a pixel for the AA ramp.
pub(crate) fn shape_bounds(rect: V4, prop: &Property) -> V4 {
    grown(rect, 1.0 / prop.ppu)
}

/// What a sprite covers. Texels map 1:1 to pixels only without rotation or
/// scale, then it is exactly its rect; otherwise it grows by a pixel so the
/// bilinear edge is drawn.
pub(crate) fn sprite_bounds(sprite: &SpriteRecord, prop: &Property) -> V4 {
    let exact = prop.linear == [1.0, 0.0, 0.0, 1.0];
    let d = if exact { 0.0 } else { 1.0 / prop.ppu };
    grown(sprite.rect, d)
}

/// What a shadow covers: the spread shape and three sigmas around it.
pub(crate) fn shadow_bounds(sh: &ShadowRecord, prop: &Property) -> V4 {
    let [x, y, w, h] = sh.rect;
    let [dx, dy, spread, sigma] = sh.shadow;
    let spread = spread.max(-0.5 * w.min(h));
    let shape = [
        x + dx - spread,
        y + dy - spread,
        w + 2.0 * spread,
        h + 2.0 * spread,
    ];
    grown(shape, 3.0 * sigma + 1.0 / prop.ppu)
}

/// How far from its center line a curve's segment boxes reach: half the
/// width and a pixel for the AA ramp.
pub(crate) fn curve_reach(curve: &CurveRecord, prop: &Property) -> f32 {
    curve.half_width + 1.0 / prop.ppu
}

/// How far from its own segment a pixel can be and still be drawn by that
/// segment's box: the stroke and its AA ramp, with room for rounding.
/// Mirrored in the shader's `shade_curve`.
pub(crate) fn curve_claim(half_width: f32, ppu: f32) -> f32 {
    half_width + 0.75 / ppu
}

/// What a curve covers: its control hull grown by [`curve_reach`].
pub(crate) fn curve_bounds(curve: &CurveRecord, prop: &Property) -> V4 {
    let p = control_points(curve);
    let (mut lo, mut hi) = (p[0], p[0]);
    for q in &p[1..] {
        lo = [lo[0].min(q[0]), lo[1].min(q[1])];
        hi = [hi[0].max(q[0]), hi[1].max(q[1])];
    }
    let r = curve_reach(curve, prop);
    [lo[0] - r, lo[1] - r, hi[0] + r, hi[1] + r]
}

pub(crate) fn control_points(curve: &CurveRecord) -> crate::curve::Points {
    let [ax, ay, bx, by] = curve.p01;
    let [cx, cy, dx, dy] = curve.p23;
    [[ax, ay], [bx, by], [cx, cy], [dx, dy]]
}

fn scaled(c: V4, k: f32) -> V4 {
    [c[0] * k, c[1] * k, c[2] * k, c[3] * k]
}

/// Opacity times the clip's coverage at `px`.
fn visibility(prop: &Property, px: V2) -> f32 {
    prop.opacity * clip_coverage(px, prop.clip)
}

fn corner_index(q: V2) -> usize {
    if q[0] < 0.0 {
        if q[1] < 0.0 { 0 } else { 3 }
    } else if q[1] < 0.0 {
        1
    } else {
        2
    }
}

/// Signed distance (local units, negative inside) to the rounded rect with
/// superellipse corners: extent `ext` and exponent `n` per corner.
pub(crate) fn sd_shape(p: V2, rect: V4, ext: V4, exps: V4) -> f32 {
    let half = [rect[2] * 0.5, rect[3] * 0.5];
    let rel = [p[0] - (rect[0] + half[0]), p[1] - (rect[1] + half[1])];
    let i = corner_index(rel);
    let r = ext[i];
    let n = exps[i];
    let q = [rel[0].abs() - half[0] + r, rel[1].abs() - half[1] + r];
    if r > 0.0 && q[0] > 0.0 && q[1] > 0.0 {
        // Superellipse norm of the corner-local point, normalized by r; the
        // distance is corrected by the gradient length so the AA ramp is
        // one pixel wide along the whole curve.
        let u = [(q[0] / r).max(1e-6), (q[1] / r).max(1e-6)];
        let un = [u[0].powf(n), u[1].powf(n)];
        let norm = (un[0] + un[1]).powf(1.0 / n);
        let g = [u[0].powf(n - 1.0), u[1].powf(n - 1.0)];
        let grad = (g[0] * g[0] + g[1] * g[1]).sqrt() / norm.powf(n - 1.0);
        return r * (norm - 1.0) / grad.max(1e-6);
    }
    let m = [q[0].max(0.0), q[1].max(0.0)];
    q[0].max(q[1]).min(0.0) + (m[0] * m[0] + m[1] * m[1]).sqrt() - r
}

/// Coverage of a pixel by a shape at signed distance `d` (local units).
pub(crate) fn coverage(d: f32, ppu: f32) -> f32 {
    clamp01(0.5 - d * ppu)
}

pub(crate) fn clip_coverage(px: V2, clip: V4) -> f32 {
    clamp01(px[0] - clip[0] + 0.5)
        * clamp01(clip[2] - px[0] + 0.5)
        * clamp01(px[1] - clip[1] + 0.5)
        * clamp01(clip[3] - px[1] + 0.5)
}

fn erf(x: f32) -> f32 {
    let s = if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    };
    let a = x.abs();
    let t = 1.0 + (0.278_393 + (0.230_389 + 0.078_108 * (a * a)) * a) * a;
    let t2 = t * t;
    s - s / (t2 * t2)
}

fn gaussian(x: f32, sigma: f32) -> f32 {
    (-(x * x) / (2.0 * sigma * sigma)).exp() / (2.506_628_3 * sigma)
}

fn shadow_x(x: f32, y: f32, sigma: f32, r: f32, half: V2) -> f32 {
    let delta = (half[1] - r - y.abs()).min(0.0);
    let curved = half[0] - r + (r * r - delta * delta).max(0.0).sqrt();
    let k = std::f32::consts::FRAC_1_SQRT_2 / sigma;
    let lo = 0.5 + 0.5 * erf((x - curved) * k);
    let hi = 0.5 + 0.5 * erf((x + curved) * k);
    hi - lo
}

/// Gaussian-blurred rounded box (closed form along x, 4 samples along y).
fn box_shadow(p: V2, half: V2, sigma: f32, r: f32) -> f32 {
    let low = p[1] - half[1];
    let high = p[1] + half[1];
    let start = (-3.0 * sigma).clamp(low, high);
    let end = (3.0 * sigma).clamp(low, high);
    let step = (end - start) * 0.25;
    let mut y = start + step * 0.5;
    let mut v = 0.0;
    for _ in 0..4 {
        v += shadow_x(p[0], p[1] - y, sigma, r, half) * gaussian(y, sigma) * step;
        y += step;
    }
    v
}

/// Radius of the circular corner with the same 45-degree inset.
fn circle_equivalent(ext: f32, n: f32) -> f32 {
    ext * (1.0 - (-1.0 / n).exp2()) / 0.292_893_2
}

/// A drop shadow at one pixel, premultiplied gamma sRGB.
pub(crate) fn shade_shadow(sh: &ShadowRecord, prop: &Property, px: V2, scale: f32) -> V4 {
    let p = local_pos(prop, px, scale);
    let rect = sh.rect;
    let [dx, dy, spread, sigma] = sh.shadow;
    let half = [
        (rect[2] * 0.5 + spread).max(0.0),
        (rect[3] * 0.5 + spread).max(0.0),
    ];
    let center = [rect[0] + rect[2] * 0.5 + dx, rect[1] + rect[3] * 0.5 + dy];
    let rel = [p[0] - center[0], p[1] - center[1]];
    let i = corner_index(rel);
    let r = (circle_equivalent(sh.ext[i], sh.exps[i]) + spread)
        .max(0.0)
        .min(half[0].min(half[1]));
    let mut a = box_shadow(rel, half, sigma, r);
    // Not under the shape itself.
    a *= 1.0 - coverage(sd_shape(p, rect, sh.ext, sh.exps), prop.ppu);
    let c = scaled(sh.color, a);
    let n = bayer(px) / 255.0 * c[3].min(1.0);
    scaled([c[0] + n, c[1] + n, c[2] + n, c[3]], visibility(prop, px))
}

fn gradient(g: &GradientRecord, flags: u32, p: V2) -> V4 {
    let kind = flags & 0xff;
    let n = ((flags >> 8) as usize).min(g.stops.len());
    let geo = g.geometry;
    let t = if kind == fill_kind::LINEAR {
        let d = [geo[2] - geo[0], geo[3] - geo[1]];
        let len2 = (d[0] * d[0] + d[1] * d[1]).max(1e-12);
        ((p[0] - geo[0]) * d[0] + (p[1] - geo[1]) * d[1]) / len2
    } else {
        let u = [(p[0] - geo[0]) / geo[2], (p[1] - geo[1]) / geo[3]];
        (u[0] * u[0] + u[1] * u[1]).sqrt()
    };
    let offs = g.offsets;
    let stop = |i: usize| -> V4 {
        let c = g.stops[i];
        [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]]
    };
    let mut lab = stop(0);
    if n > 1 && t > offs[0] {
        lab = stop(n - 1);
        for i in 0..n - 1 {
            if t < offs[i + 1] {
                let f = clamp01((t - offs[i]) / (offs[i + 1] - offs[i]).max(1e-6));
                let (a, b) = (stop(i), stop(i + 1));
                lab = [
                    a[0] + (b[0] - a[0]) * f,
                    a[1] + (b[1] - a[1]) * f,
                    a[2] + (b[2] - a[2]) * f,
                    a[3] + (b[3] - a[3]) * f,
                ];
                break;
            }
        }
    }
    let alpha = lab[3];
    if alpha <= 0.0 {
        return [0.0; 4];
    }
    let rgb = oklab_to_srgb([lab[0] / alpha, lab[1] / alpha, lab[2] / alpha]);
    [rgb[0] * alpha, rgb[1] * alpha, rgb[2] * alpha, alpha]
}

/// A quad's fill and border at one pixel. `grad` is its effect record
/// (gradient fills only).
pub(crate) fn shade_quad(
    q: &QuadRecord,
    grad: Option<&GradientRecord>,
    prop: &Property,
    px: V2,
    scale: f32,
) -> V4 {
    let p = local_pos(prop, px, scale);
    let ppu = prop.ppu;
    let d = sd_shape(p, q.rect, q.ext, q.exps);
    let shape = coverage(d, ppu);
    let kind = q.flags & 0xff;
    let fill = match (kind, grad) {
        (fill_kind::SOLID, _) => q.fill,
        (fill_kind::LINEAR | fill_kind::RADIAL, Some(g)) => gradient(g, q.flags, p),
        _ => [0.0; 4],
    };
    let bw = q.border_width;
    let mut c = if bw > 0.0 {
        let bcov = shape - coverage(d + bw, ppu);
        let b = q.border;
        let k = shape - bcov * b[3];
        [
            b[0] * bcov + fill[0] * k,
            b[1] * bcov + fill[1] * k,
            b[2] * bcov + fill[2] * k,
            b[3] * bcov + fill[3] * k,
        ]
    } else {
        scaled(fill, shape)
    };
    if kind >= fill_kind::LINEAR {
        let n = bayer(px) / 255.0 * c[3];
        c = [c[0] + n, c[1] + n, c[2] + n, c[3]];
    }
    scaled(c, visibility(prop, px))
}

/// A curve at a pixel `distance` local units from its polyline.
pub(crate) fn shade_curve(curve: &CurveRecord, prop: &Property, px: V2, distance: f32) -> V4 {
    let cov = coverage(distance - curve.half_width, prop.ppu);
    scaled(curve.color, cov * visibility(prop, px))
}

/// Remaps glyph coverage `a` for a text color of lightness `l` (see the
/// crate docs, "Text"); `inv_g` is `1 / TEXT_GAMMA`.
pub(crate) fn glyph_alpha(a: f32, l: f32, inv_g: f32) -> f32 {
    let light = a.powf(inv_g);
    let dark = 1.0 - (1.0 - a).powf(inv_g);
    dark + (light - dark) * l
}

/// A sprite at one pixel. `texel(x, y)` reads the atlas (0..1, 0 outside
/// it); texels outside the sprite's own rect count as empty, so a manual
/// bilinear filter never bleeds.
pub(crate) fn shade_sprite(
    sprite: &SpriteRecord,
    prop: &Property,
    px: V2,
    scale: f32,
    texel: impl Fn(i64, i64) -> f32,
) -> V4 {
    let p = local_pos(prop, px, scale);
    let rect = sprite.rect;
    let [origin, size] = sprite.texels;
    let t = [
        (origin & 0xffff) as f32,
        (origin >> 16) as f32,
        (size & 0xffff) as f32,
        (size >> 16) as f32,
    ];
    let tx = t[0] + (p[0] - rect[0]) / rect[2] * t[2] - 0.5;
    let ty = t[1] + (p[1] - rect[1]) / rect[3] * t[3] - 0.5;
    let (x0, y0) = (tx.floor(), ty.floor());
    let (fx, fy) = (tx - x0, ty - y0);
    let tap = |x: f32, y: f32| -> f32 {
        if x < t[0] || y < t[1] || x >= t[0] + t[2] || y >= t[1] + t[3] {
            0.0
        } else {
            texel(x as i64, y as i64)
        }
    };
    let top = tap(x0, y0) + (tap(x0 + 1.0, y0) - tap(x0, y0)) * fx;
    let bottom = tap(x0, y0 + 1.0) + (tap(x0 + 1.0, y0 + 1.0) - tap(x0, y0 + 1.0)) * fx;
    let cov = clamp01(top + (bottom - top) * fy);
    let a = glyph_alpha(cov, sprite.lightness, 1.0 / TEXT_GAMMA) * visibility(prop, px);
    scaled(sprite.color, a)
}

/// CSS `saturate()` on gamma sRGB.
fn saturate(c: [f32; 3], s: f32) -> [f32; 3] {
    let r = (0.213 + 0.787 * s) * c[0] + (0.715 - 0.715 * s) * c[1] + (0.072 - 0.072 * s) * c[2];
    let g = (0.213 - 0.213 * s) * c[0] + (0.715 + 0.285 * s) * c[1] + (0.072 - 0.072 * s) * c[2];
    let b = (0.213 - 0.213 * s) * c[0] + (0.715 - 0.715 * s) * c[1] + (0.072 + 0.928 * s) * c[2];
    [clamp01(r), clamp01(g), clamp01(b)]
}

/// A backdrop composite at one pixel; `blurred` is the bilinear sample of
/// the blurred level (ignored without blur).
pub(crate) fn shade_backdrop(
    b: &BackdropRecord,
    prop: &Property,
    px: V2,
    scale: f32,
    blurred: [f32; 3],
) -> V4 {
    let p = local_pos(prop, px, scale);
    let ppu = prop.ppu;
    let d = sd_shape(p, b.rect, b.ext, b.exps);
    let shape = coverage(d, ppu);
    let tint = b.tint;
    let [hw, hk, sat, grain] = b.params;
    let base = if b.has_blur != 0 {
        let c = saturate(blurred, sat);
        let k = 1.0 - tint[3];
        [
            c[0] * k + tint[0],
            c[1] * k + tint[1],
            c[2] * k + tint[2],
            1.0,
        ]
    } else {
        tint
    };
    let mut rim = 0.0;
    if hw > 0.0 {
        rim = shape - coverage(d + hw, ppu);
        if hk > 0.0 {
            let fade = b.ext[0].max(b.ext[1]).max(4.0);
            rim *= clamp01(1.0 - (p[1] - b.rect[1]) / fade);
        }
    }
    let h = b.rim;
    let k = shape - rim * h[3];
    let mut c = [
        h[0] * rim + base[0] * k,
        h[1] * rim + base[1] * k,
        h[2] * rim + base[2] * k,
        h[3] * rim + base[3] * k,
    ];
    let n = (bayer(px) / 255.0 + noise(px) * grain) * c[3];
    c = [c[0] + n, c[1] + n, c[2] + n, c[3]];
    scaled(c, visibility(prop, px))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sd_of_box_is_exact_on_edges() {
        let rect = [0.0, 0.0, 10.0, 10.0];
        let d = sd_shape([5.0, -1.0], rect, [2.0; 4], [4.0; 4]);
        assert!((d - 1.0).abs() < 1e-5);
        let d = sd_shape([5.0, 5.0], rect, [2.0; 4], [4.0; 4]);
        assert!((d + 5.0).abs() < 1e-5);
    }

    #[test]
    fn circular_corner_distance() {
        let rect = [0.0, 0.0, 10.0, 10.0];
        // Corner center at (3, 3), radius 3; point on the diagonal.
        let p = [
            3.0 - 4.0 * std::f32::consts::FRAC_1_SQRT_2,
            3.0 - 4.0 * std::f32::consts::FRAC_1_SQRT_2,
        ];
        let d = sd_shape(p, rect, [3.0; 4], [2.0; 4]);
        assert!((d - 1.0).abs() < 1e-4, "{d}");
    }

    #[test]
    fn shadow_integrates_to_box() {
        // Far inside a large box the blurred box is ~1.
        let v = box_shadow([0.0, 0.0], [50.0, 50.0], 4.0, 5.0);
        assert!((v - 1.0).abs() < 0.01, "{v}");
        let v = box_shadow([200.0, 0.0], [50.0, 50.0], 4.0, 5.0);
        assert!(v.abs() < 1e-4, "{v}");
    }
}
