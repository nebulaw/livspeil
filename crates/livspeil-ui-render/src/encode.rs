// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Encoding: turns a [`DisplayList`] into GPU-ready instance data and a short
//! list of operations. Both backends (wgpu and the CPU reference) consume
//! the same [`Frame`], so everything that is not per-pixel math (corner
//! shapes, color conversion of solid colors, bounds, culling, batching,
//! blur plans) is computed once, here.
//!
//! Batching: items are sorted by layer (stable, so insertion order holds
//! within a layer). Quads, their shadows and backdrop shadows all go through
//! one instanced pipeline, so consecutive items become one draw call; only a
//! [`Backdrop`] breaks the batch, because it has to blur what was drawn
//! before it.

use crate::RenderError;
use crate::color::Color;
use crate::geom::{Affine, Rect};
use crate::list::{
    Backdrop, BorderWidth, CornerStyle, DisplayList, Fill, Glyphs, GradientStop, HighlightKind,
    MAX_GRADIENT_STOPS, Primitive, Quad, Radii, Shadow,
};

/// Number of `f32`s in one instance (20 `vec4<f32>`).
pub const INSTANCE_FLOATS: usize = 80;

/// One instance: 20 `vec4<f32>`, laid out as `Instance` in `quad.wgsl`.
pub type Instance = [f32; INSTANCE_FLOATS];

/// Instance kinds (`kind` field).
pub(crate) mod kind {
    pub const QUAD: f32 = 0.0;
    pub const SHADOW: f32 = 1.0;
    pub const BACKDROP: f32 = 2.0;
    pub const GLYPH: f32 = 3.0;
}

/// Field offsets (in floats) inside an [`Instance`]. Must match `quad.wgsl`.
pub(crate) mod field {
    /// Shape rect `(x, y, w, h)`, local units.
    pub const RECT: usize = 0;
    /// Forward transform `(a, b, c, d)`.
    pub const XFORM: usize = 4;
    /// `(e, f, px_per_unit, kind)`.
    pub const XFORM2: usize = 8;
    /// Inverse transform `(a, b, c, d)`.
    pub const INV: usize = 12;
    /// `(e, f, opacity, dither)`.
    pub const INV2: usize = 16;
    /// Corner extents (superellipse box size) `tl, tr, br, bl`.
    pub const RADII: usize = 20;
    /// Corner exponents (2 = circle).
    pub const EXPS: usize = 24;
    /// Clip `(x0, y0, x1, y1)`, physical pixels.
    pub const CLIP: usize = 28;
    /// Draw bounds `(x0, y0, x1, y1)`, local units.
    pub const BOUNDS: usize = 32;
    /// Gradient geometry; for backdrops `(uv_scale_x, uv_scale_y, has_blur, 0)`;
    /// for glyphs the atlas texel rect `(x, y, w, h)`.
    pub const GRAD: usize = 36;
    /// `(fill_kind, n_stops, 0, 0)`; fill kind 0 none, 1 solid, 2 linear, 3 radial.
    pub const FILL: usize = 40;
    /// Stop offsets.
    pub const STOP_OFFSETS: usize = 44;
    /// Stop colors (4 x vec4); solid fill and backdrop tint use the first as
    /// premultiplied sRGB, gradients store straight OKLab + alpha.
    pub const STOPS: usize = 48;
    /// Border (or backdrop highlight) color, premultiplied sRGB.
    pub const BORDER_COLOR: usize = 64;
    /// `(border_width, 0, 0, 0)`; backdrop:
    /// `(highlight_width, highlight_kind, saturation, grain)`; glyphs:
    /// `(text lightness, 1 / TEXT_GAMMA, 0, 0)`.
    pub const BORDER: usize = 68;
    /// Shadow `(dx, dy, spread, sigma)`.
    pub const SHADOW: usize = 72;
    /// Shadow color, premultiplied sRGB.
    pub const SHADOW_COLOR: usize = 76;
}

/// Fill kinds.
pub(crate) mod fill_kind {
    pub const NONE: f32 = 0.0;
    pub const SOLID: f32 = 1.0;
    pub const LINEAR: f32 = 2.0;
    pub const RADIAL: f32 = 3.0;
}

/// Exponent of the glyph coverage remapping (see the crate docs, "Text").
pub const TEXT_GAMMA: f32 = 1.8;

/// Frame size and settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameOptions {
    /// Target width in physical pixels.
    pub width: u32,
    /// Target height in physical pixels.
    pub height: u32,
    /// Physical pixels per logical pixel.
    pub scale: f32,
    /// Clear color.
    pub clear: Color,
}

/// How a backdrop's blur is computed: dual-Kawase with `levels` downsample
/// passes (each halving the resolution) at sample `offset`, then
/// `levels - 1` upsample passes back to half resolution. The composite pass
/// samples the half-resolution result with bilinear filtering.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlurPlan {
    /// Number of downsample passes (1..=[`MAX_BLUR_LEVELS`]).
    pub levels: u32,
    /// Sample offset in source texels (dual-Kawase "offset").
    pub offset: f32,
    /// Region processed at full resolution, `[x0, y0, x1, y1)` in physical
    /// pixels: the backdrop's bounds grown by the blur's reach, clamped to
    /// the target.
    pub region: [u32; 4],
}

/// Deepest blur pyramid (1/64 resolution).
pub const MAX_BLUR_LEVELS: u32 = 6;

/// One step of a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Draw instances `start..end` with the quad pipeline.
    Draw {
        /// First instance.
        start: u32,
        /// One past the last instance.
        end: u32,
        /// The coverage atlas glyph instances in the range sample (`None`
        /// when the range has no glyphs). A different atlas starts a new
        /// draw.
        atlas: Option<u32>,
    },
    /// Blur the target under a backdrop (if `blur` is set), then composite
    /// instance `instance` with the backdrop pipeline.
    Backdrop {
        /// The backdrop's instance.
        instance: u32,
        /// The blur, or `None` for tint only.
        blur: Option<BlurPlan>,
    },
}

/// An encoded frame, ready for a backend.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// Size and settings.
    pub options: FrameOptions,
    /// Clear color, premultiplied gamma sRGB.
    pub clear: [f32; 4],
    /// Instance data.
    pub instances: Vec<Instance>,
    /// Operations in order.
    pub ops: Vec<Op>,
}

impl Frame {
    /// Number of draw calls the GPU backend issues (instanced quad draws plus
    /// backdrop composites; blur passes not counted).
    pub fn draw_calls(&self) -> usize {
        self.ops.len()
    }

    /// Instance data as little-endian bytes (for upload).
    pub fn instance_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.instances.len() * INSTANCE_FLOATS * 4);
        for inst in &self.instances {
            for v in inst {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }
}

/// Size of level `level` of the blur pyramid for a target dimension.
pub fn level_size(full: u32, level: u32) -> u32 {
    let d = 1u32 << level;
    full.div_ceil(d).max(1)
}

/// Dual-Kawase parameters for a blur radius in physical pixels.
///
/// Measured with the CPU reference (`kawase_sigma` test): with offset `o`
/// and `L` levels the result is close to a Gaussian whose sigma grows
/// linearly with `o` and doubles per level. We pick the smallest `L` that
/// reaches the radius with `o <= 1.5`, then solve for `o`.
pub fn kawase_params(radius_px: f32) -> (u32, f32) {
    // sigma(L, o) ~= 2^L * (A + B * o), fitted in the tests.
    const A: f32 = KAWASE_FIT.0;
    const B: f32 = KAWASE_FIT.1;
    let mut levels = 1;
    while levels < MAX_BLUR_LEVELS && radius_px > (1u32 << levels) as f32 * (A + MAX_OFFSET * B) {
        levels += 1;
    }
    let base = (1u32 << levels) as f32;
    let offset = ((radius_px / base - A) / B).clamp(0.25, MAX_OFFSET);
    (levels, offset)
}

/// Larger offsets leave gaps between taps (visible as a lattice), so a
/// deeper pyramid is preferred beyond this.
const MAX_OFFSET: f32 = 1.5;

/// `(A, B)` in `sigma ~= 2^levels * (A + B * offset)`; see [`kawase_params`].
pub(crate) const KAWASE_FIT: (f32, f32) = (0.12, 0.72);

/// Physical-pixel reach of a blur plan: how far outside a pixel its result
/// can read.
fn kawase_reach(levels: u32, offset: f32) -> f32 {
    let mut reach = 0.0;
    for l in 1..=levels {
        // Down pass into level l reads level l-1 at +-offset/2 texels plus
        // the bilinear footprint.
        reach += (0.5 * offset + 1.0) * (1u32 << (l - 1)) as f32;
    }
    for l in 1..levels {
        // Up pass into level l reads level l+1 at up to +-offset texels.
        reach += (offset + 1.0) * (1u32 << (l + 1)) as f32;
    }
    // Final bilinear sample of level 1.
    reach + 2.0
}

const NO_CLIP: [f32; 4] = [-1.0e9, -1.0e9, 1.0e9, 1.0e9];

/// Continuous corners start this much further along the edge than circular
/// ones of the same radius.
const CONTINUOUS_EXTENT: f32 = 1.7;
/// `1 - 1/sqrt(2)`: inset of a circular corner's 45-degree point.
const CIRCLE_DIAGONAL_INSET: f32 = 0.292_893_2;

/// Corner extents (the superellipse's box) and exponents for a shape.
/// Radii that do not fit are scaled down together, as CSS does.
pub fn corner_params(rect: Rect, radii: Radii, style: CornerStyle) -> ([f32; 4], [f32; 4]) {
    let r = radii
        .to_array()
        .map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 });
    let k = match style {
        CornerStyle::Circular => 1.0,
        CornerStyle::Continuous => CONTINUOUS_EXTENT,
    };
    let mut ext = r.map(|v| v * k);
    // [tl, tr, br, bl]: top = tl+tr, right = tr+br, bottom = br+bl, left = bl+tl.
    let sums = [
        (rect.w, ext[0] + ext[1]),
        (rect.h, ext[1] + ext[2]),
        (rect.w, ext[2] + ext[3]),
        (rect.h, ext[3] + ext[0]),
    ];
    let mut f: f32 = 1.0;
    for (len, sum) in sums {
        if sum > 0.0 && sum > len {
            f = f.min(len.max(0.0) / sum);
        }
    }
    ext = ext.map(|v| v * f);
    let mut exps = [2.0; 4];
    for i in 0..4 {
        if ext[i] > 0.0 && style == CornerStyle::Continuous {
            let rc = r[i].min(ext[i]);
            let t = 1.0 - CIRCLE_DIAGONAL_INSET * rc / ext[i];
            exps[i] = (-1.0 / t.log2()).clamp(2.0, 16.0);
        }
    }
    (ext, exps)
}

struct Encoder {
    options: FrameOptions,
    instances: Vec<Instance>,
    ops: Vec<Op>,
    pending_start: u32,
    pending_atlas: Option<u32>,
    viewport: Rect,
}

/// Encodes a display list for a target.
pub fn encode(list: &DisplayList, options: FrameOptions) -> Result<Frame, RenderError> {
    if options.width == 0 || options.height == 0 {
        return Err(RenderError::EmptyTarget);
    }
    if !(options.scale.is_finite() && options.scale > 0.0) {
        return Err(RenderError::InvalidScale(options.scale));
    }
    let mut order: Vec<usize> = (0..list.items.len()).collect();
    order.sort_by_key(|&i| list.items[i].layer);
    let mut enc = Encoder {
        options,
        instances: Vec::with_capacity(list.items.len() * 2),
        ops: Vec::new(),
        pending_start: 0,
        pending_atlas: None,
        viewport: Rect::new(0.0, 0.0, options.width as f32, options.height as f32),
    };
    for index in order {
        match &list.items[index].primitive {
            Primitive::Quad(q) => enc.quad(index, q)?,
            Primitive::Backdrop(b) => enc.backdrop(index, b)?,
            Primitive::Glyphs(g) => enc.glyphs(index, g)?,
            // Typed hook: images draw nothing yet.
            Primitive::Image(_) => {}
        }
    }
    enc.flush();
    Ok(Frame {
        options,
        clear: options.clear.to_srgb_premul(),
        instances: enc.instances,
        ops: enc.ops,
    })
}

fn invalid(index: usize, reason: &'static str) -> RenderError {
    RenderError::InvalidPrimitive { index, reason }
}

fn put(inst: &mut Instance, at: usize, v: [f32; 4]) {
    inst[at..at + 4].copy_from_slice(&v);
}

impl Encoder {
    fn flush(&mut self) {
        let end = self.instances.len() as u32;
        if end > self.pending_start {
            self.ops.push(Op::Draw {
                start: self.pending_start,
                end,
                atlas: self.pending_atlas,
            });
        }
        self.pending_start = end;
        self.pending_atlas = None;
    }

    fn glyphs(&mut self, index: usize, g: &Glyphs) -> Result<(), RenderError> {
        if g.glyphs.is_empty() || g.color.alpha <= 0.0 {
            return Ok(());
        }
        if !g.color.to_array().iter().all(|v| v.is_finite()) {
            return Err(invalid(index, "glyph color is not finite"));
        }
        let color = g.color.to_srgb_premul();
        let lightness = g.color.l.clamp(0.0, 1.0);
        // Texels map 1:1 to pixels only without rotation or scale; then the
        // instance covers exactly its rect. Otherwise grow it by a pixel so
        // the bilinear edge is drawn.
        let [a, b, c, d, _, _] = g.transform.m;
        let exact = a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0;
        for sprite in &g.glyphs {
            let Some((mut inst, ppu)) = self.base(
                index,
                sprite.rect,
                g.transform,
                g.clip,
                g.opacity,
                kind::GLYPH,
            )?
            else {
                return Ok(());
            };
            let [tx, ty, tw, th] = sprite.texels;
            if tw == 0 || th == 0 || sprite.rect.w <= 0.0 || sprite.rect.h <= 0.0 {
                continue;
            }
            put(
                &mut inst,
                field::GRAD,
                [tx as f32, ty as f32, tw as f32, th as f32],
            );
            put(&mut inst, field::STOPS, color);
            put(
                &mut inst,
                field::BORDER,
                [lightness, 1.0 / TEXT_GAMMA, 0.0, 0.0],
            );
            let local = if exact {
                sprite.rect
            } else {
                sprite.rect.inflate(1.0 / ppu)
            };
            if !self.bounds(&mut inst, local, g.transform) {
                continue;
            }
            if self.pending_atlas.is_some_and(|p| p != g.atlas) {
                self.flush();
            }
            self.pending_atlas = Some(g.atlas);
            self.instances.push(inst);
        }
        Ok(())
    }

    /// Common setup: transform, clip, opacity. `None` when the instance is
    /// invisible (singular transform, zero opacity, fully clipped).
    fn base(
        &self,
        index: usize,
        rect: Rect,
        transform: Affine,
        clip: Option<Rect>,
        opacity: f32,
        kind_value: f32,
    ) -> Result<Option<(Instance, f32)>, RenderError> {
        if !rect.is_valid() {
            return Err(invalid(index, "rect is not finite or has negative size"));
        }
        if !opacity.is_finite() {
            return Err(invalid(index, "opacity is not finite"));
        }
        let opacity = opacity.clamp(0.0, 1.0);
        if opacity == 0.0 {
            return Ok(None);
        }
        let Some(inv) = transform.inverse() else {
            return Ok(None);
        };
        let s = self.options.scale;
        let ppu = s * transform.det().abs().sqrt();
        let clip_px = match clip {
            Some(c) if !c.is_valid() => return Err(invalid(index, "clip is not finite")),
            Some(c) => [c.x * s, c.y * s, c.right() * s, c.bottom() * s],
            None => NO_CLIP,
        };
        let mut inst = [0.0; INSTANCE_FLOATS];
        put(&mut inst, field::RECT, [rect.x, rect.y, rect.w, rect.h]);
        let [a, b, c, d, e, f] = transform.m;
        put(&mut inst, field::XFORM, [a, b, c, d]);
        put(&mut inst, field::XFORM2, [e, f, ppu, kind_value]);
        let [ia, ib, ic, id, ie, iff] = inv.m;
        put(&mut inst, field::INV, [ia, ib, ic, id]);
        put(&mut inst, field::INV2, [ie, iff, opacity, 0.0]);
        put(&mut inst, field::CLIP, clip_px);
        Ok(Some((inst, ppu)))
    }

    /// Sets the draw bounds; returns false if they are off screen.
    fn bounds(&self, inst: &mut Instance, local: Rect, transform: Affine) -> bool {
        put(
            inst,
            field::BOUNDS,
            [local.x, local.y, local.right(), local.bottom()],
        );
        let s = self.options.scale;
        let b = transform.bounds(local);
        let c = &inst[field::CLIP..field::CLIP + 4];
        let (x0, y0) = ((b.x * s).max(c[0]).max(0.0), (b.y * s).max(c[1]).max(0.0));
        let (x1, y1) = (
            (b.right() * s).min(c[2]).min(self.viewport.w),
            (b.bottom() * s).min(c[3]).min(self.viewport.h),
        );
        x1 > x0 && y1 > y0
    }

    fn shadows(
        &mut self,
        index: usize,
        base: &Instance,
        ppu: f32,
        rect: Rect,
        transform: Affine,
        shadows: &[Shadow],
    ) -> Result<(), RenderError> {
        for sh in shadows {
            let vals = [sh.offset[0], sh.offset[1], sh.blur, sh.spread];
            if vals.iter().any(|v| !v.is_finite()) || sh.blur < 0.0 {
                return Err(invalid(index, "shadow has a non-finite or negative value"));
            }
            if sh.color.alpha <= 0.0 {
                continue;
            }
            let mut inst = *base;
            inst[field::XFORM2 + 3] = kind::SHADOW;
            inst[field::INV2 + 3] = 1.0;
            // Below about half a physical pixel a Gaussian is a hard edge;
            // keep sigma there so the analytic integral stays smooth.
            let sigma = (sh.blur * 0.5).max(0.5 / ppu);
            put(
                &mut inst,
                field::SHADOW,
                [sh.offset[0], sh.offset[1], sh.spread, sigma],
            );
            put(&mut inst, field::SHADOW_COLOR, sh.color.to_srgb_premul());
            let shape = rect
                .translate(sh.offset[0], sh.offset[1])
                .inflate(sh.spread.max(-0.5 * rect.w.min(rect.h)));
            let local = shape.inflate(3.0 * sigma + 1.0 / ppu);
            if self.bounds(&mut inst, local, transform) {
                self.instances.push(inst);
            }
        }
        Ok(())
    }

    fn quad(&mut self, index: usize, q: &Quad) -> Result<(), RenderError> {
        let Some((mut inst, ppu)) =
            self.base(index, q.rect, q.transform, q.clip, q.opacity, kind::QUAD)?
        else {
            return Ok(());
        };
        let (ext, exps) = corner_params(q.rect, q.radii, q.corners);
        put(&mut inst, field::RADII, ext);
        put(&mut inst, field::EXPS, exps);
        self.shadows(index, &inst, ppu, q.rect, q.transform, &q.shadows)?;

        let r = q.rect;
        match &q.fill {
            None => inst[field::FILL] = fill_kind::NONE,
            Some(Fill::Solid(c)) => {
                inst[field::FILL] = fill_kind::SOLID;
                put(&mut inst, field::STOPS, c.to_srgb_premul());
            }
            Some(Fill::Linear { start, end, stops }) => {
                inst[field::FILL] = fill_kind::LINEAR;
                put(
                    &mut inst,
                    field::GRAD,
                    [
                        r.x + start[0] * r.w,
                        r.y + start[1] * r.h,
                        r.x + end[0] * r.w,
                        r.y + end[1] * r.h,
                    ],
                );
                put_stops(index, &mut inst, stops)?;
            }
            Some(Fill::Radial {
                center,
                radius,
                stops,
            }) => {
                inst[field::FILL] = fill_kind::RADIAL;
                put(
                    &mut inst,
                    field::GRAD,
                    [
                        r.x + center[0] * r.w,
                        r.y + center[1] * r.h,
                        (radius[0] * r.w).max(1e-6),
                        (radius[1] * r.h).max(1e-6),
                    ],
                );
                put_stops(index, &mut inst, stops)?;
            }
        }
        if inst[field::FILL] >= fill_kind::LINEAR {
            inst[field::INV2 + 3] = 1.0;
        }
        if let Some(border) = q.border {
            let w = match border.width {
                BorderWidth::Hairline => 1.0 / ppu,
                BorderWidth::Logical(w) if w.is_finite() => w.max(0.0),
                BorderWidth::Logical(_) => {
                    return Err(invalid(index, "border width is not finite"));
                }
            };
            put(
                &mut inst,
                field::BORDER_COLOR,
                border.color.to_srgb_premul(),
            );
            inst[field::BORDER] = w;
        }
        if inst[field::FILL] == fill_kind::NONE && inst[field::BORDER] == 0.0 {
            return Ok(());
        }
        if self.bounds(&mut inst, q.rect.inflate(1.0 / ppu), q.transform) {
            self.instances.push(inst);
        }
        Ok(())
    }

    fn backdrop(&mut self, index: usize, b: &Backdrop) -> Result<(), RenderError> {
        let Some((mut inst, ppu)) = self.base(
            index,
            b.rect,
            b.transform,
            b.clip,
            b.opacity,
            kind::BACKDROP,
        )?
        else {
            return Ok(());
        };
        let finite = [b.blur, b.saturation, b.grain]
            .iter()
            .all(|v| v.is_finite());
        if !finite || b.blur < 0.0 {
            return Err(invalid(
                index,
                "backdrop blur, saturation or grain is not valid",
            ));
        }
        let (ext, exps) = corner_params(b.rect, b.radii, b.corners);
        put(&mut inst, field::RADII, ext);
        put(&mut inst, field::EXPS, exps);
        put(&mut inst, field::STOPS, b.tint.to_srgb_premul());
        inst[field::INV2 + 3] = 1.0;
        let (hw, hk) = match b.highlight {
            Some(h) if h.width.is_finite() && h.width > 0.0 && h.color.alpha > 0.0 => {
                put(&mut inst, field::BORDER_COLOR, h.color.to_srgb_premul());
                let k = match h.kind {
                    HighlightKind::Full => 0.0,
                    HighlightKind::TopRim => 1.0,
                };
                (h.width, k)
            }
            _ => (0.0, 0.0),
        };
        put(
            &mut inst,
            field::BORDER,
            [hw, hk, b.saturation.max(0.0), b.grain.max(0.0)],
        );
        let visible = self.bounds(&mut inst, b.rect.inflate(1.0 / ppu), b.transform);
        if visible {
            let blur = self.blur_plan(b);
            let (w1, h1) = (
                level_size(self.options.width, 1),
                level_size(self.options.height, 1),
            );
            put(
                &mut inst,
                field::GRAD,
                [
                    1.0 / (2.0 * w1 as f32),
                    1.0 / (2.0 * h1 as f32),
                    if blur.is_some() { 1.0 } else { 0.0 },
                    0.0,
                ],
            );
            self.flush();
            let id = self.instances.len() as u32;
            self.instances.push(inst);
            self.ops.push(Op::Backdrop { instance: id, blur });
            self.pending_start = self.instances.len() as u32;
        }
        // Shadows go after the composite (they are clipped out of the shape,
        // so the order does not change the picture, and the blur does not
        // pick up the panel's own shadow).
        let base = inst;
        self.shadows(index, &base, ppu, b.rect, b.transform, &b.shadows)
    }

    fn blur_plan(&self, b: &Backdrop) -> Option<BlurPlan> {
        let s = self.options.scale;
        let radius_px = b.blur * s * b.transform.det().abs().sqrt();
        if radius_px < 0.5 {
            return None;
        }
        let (levels, offset) = kawase_params(radius_px);
        let reach = kawase_reach(levels, offset);
        let bb = b.transform.bounds(b.rect);
        let x0 = (bb.x * s - reach).floor().max(0.0);
        let y0 = (bb.y * s - reach).floor().max(0.0);
        let x1 = (bb.right() * s + reach).ceil().min(self.viewport.w);
        let y1 = (bb.bottom() * s + reach).ceil().min(self.viewport.h);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Some(BlurPlan {
            levels,
            offset,
            region: [x0 as u32, y0 as u32, x1 as u32, y1 as u32],
        })
    }
}

fn put_stops(index: usize, inst: &mut Instance, stops: &[GradientStop]) -> Result<(), RenderError> {
    if stops.is_empty() || stops.len() > MAX_GRADIENT_STOPS {
        return Err(RenderError::GradientStops {
            index,
            count: stops.len(),
        });
    }
    let ok = stops
        .iter()
        .all(|s| s.offset.is_finite() && s.color.to_array().iter().all(|v| v.is_finite()));
    if !ok {
        return Err(invalid(index, "gradient stop is not finite"));
    }
    inst[field::FILL + 1] = stops.len() as f32;
    let mut offsets = [0.0; 4];
    let mut prev = f32::MIN;
    for (i, s) in stops.iter().enumerate() {
        // Offsets must not decrease (CSS clamps them the same way).
        prev = prev.max(s.offset);
        offsets[i] = prev;
        put(inst, field::STOPS + 4 * i, s.color.to_array());
    }
    put(inst, field::STOP_OFFSETS, offsets);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circular_corners_are_circles() {
        let (ext, exps) = corner_params(
            Rect::new(0.0, 0.0, 100.0, 50.0),
            Radii::all(10.0),
            CornerStyle::Circular,
        );
        assert_eq!(ext, [10.0; 4]);
        assert_eq!(exps, [2.0; 4]);
    }

    #[test]
    fn continuous_pill_degrades_to_circle() {
        let (ext, exps) = corner_params(
            Rect::new(0.0, 0.0, 100.0, 20.0),
            Radii::all(10.0),
            CornerStyle::Continuous,
        );
        for i in 0..4 {
            assert!((ext[i] - 10.0).abs() < 1e-4);
            assert!((exps[i] - 2.0).abs() < 1e-3, "{exps:?}");
        }
        let (ext, exps) = corner_params(
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Radii::all(10.0),
            CornerStyle::Continuous,
        );
        assert!((ext[0] - 17.0).abs() < 1e-4);
        assert!(exps[0] > 3.0 && exps[0] < 4.0, "{exps:?}");
    }

    #[test]
    fn backdrops_break_batches() {
        let mut list = DisplayList::new();
        list.add(Quad::new(Rect::new(0.0, 0.0, 10.0, 10.0), Color::WHITE));
        list.add(Quad::new(Rect::new(5.0, 5.0, 10.0, 10.0), Color::BLACK));
        list.add(Backdrop::new(
            Rect::new(2.0, 2.0, 10.0, 10.0),
            8.0,
            Color::WHITE.with_alpha(0.5),
        ));
        list.push(-1, Quad::new(Rect::new(0.0, 0.0, 20.0, 20.0), Color::BLACK));
        let frame = encode(
            &list,
            FrameOptions {
                width: 20,
                height: 20,
                scale: 1.0,
                clear: Color::BLACK,
            },
        )
        .unwrap();
        assert_eq!(frame.ops.len(), 2);
        assert_eq!(
            frame.ops[0],
            Op::Draw {
                start: 0,
                end: 3,
                atlas: None
            }
        );
        assert!(matches!(
            frame.ops[1],
            Op::Backdrop {
                instance: 3,
                blur: Some(_)
            }
        ));
    }
}
