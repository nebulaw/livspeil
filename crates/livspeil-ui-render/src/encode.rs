// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Encoding: turns a [`DisplayList`] into GPU-ready records (see
//! [`record`](crate::record)) and a short list of operations. Both backends
//! (wgpu and the CPU reference) consume the same [`Frame`], so everything
//! that is not per-pixel math (corner shapes, color conversion of solid
//! colors, culling, curve subdivision, batching, blur plans) is computed
//! once, here.
//!
//! Batching: items are sorted by layer (stable, so insertion order holds
//! within a layer) and drawn in that order whatever their family. One
//! instanced pipeline reads every family from the record arena, so changing
//! family costs nothing. A new draw call starts only for a [`Backdrop`]
//! (it has to blur what was drawn before it), for another glyph atlas, and
//! where a run of curves begins or ends (curves have more vertices per
//! instance than the other families).

use std::collections::HashMap;

use crate::RenderError;
use crate::color::Color;
use crate::curve;
use crate::geom::{Affine, Rect};
use crate::list::{
    Backdrop, BorderWidth, CornerStyle, Curve, DisplayList, Fill, Glyphs, GradientStop,
    HighlightKind, MAX_GRADIENT_STOPS, Primitive, Quad, Radii, Shadow,
};
use crate::record::{
    BackdropRecord, CurveRecord, Family, GradientRecord, InstanceRef, MAX_SLOT, PROPERTY_BYTES,
    Property, QuadRecord, SLOT_BYTES, ShadowRecord, Slot, SpriteRecord, fill_kind,
};
use crate::shade;

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
    /// Draw instances `start..end` with the primitive pipeline.
    Draw {
        /// First instance.
        start: u32,
        /// One past the last instance.
        end: u32,
        /// The coverage atlas sprites in the range sample (`None` when the
        /// range has no sprites). A different atlas starts a new draw.
        atlas: Option<u32>,
        /// Vertices per instance: 6, or for a run of curves 6 times the
        /// most segments any of them has.
        vertices: u32,
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
    /// The draw order: one packed [`InstanceRef`] each.
    pub instances: Vec<u32>,
    /// The record arena the instances refer to.
    pub records: Vec<Slot>,
    /// Transform, clip and opacity entries the records refer to.
    pub properties: Vec<[Slot; Property::SLOTS]>,
    /// Operations in order.
    pub ops: Vec<Op>,
}

impl Frame {
    /// An empty frame for [`encode_into`].
    #[must_use]
    pub fn new(options: FrameOptions) -> Frame {
        Frame {
            options,
            clear: [0.0; 4],
            instances: Vec::new(),
            records: Vec::new(),
            properties: Vec::new(),
            ops: Vec::new(),
        }
    }

    fn clear_data(&mut self) {
        self.instances.clear();
        self.records.clear();
        self.properties.clear();
        self.ops.clear();
    }

    /// Number of draw calls the GPU backend issues (instanced draws plus
    /// backdrop composites; blur passes not counted).
    pub fn draw_calls(&self) -> usize {
        self.ops.len()
    }

    /// Instance `i` of the draw order.
    pub fn instance(&self, i: usize) -> Option<InstanceRef> {
        self.instances.get(i).copied().and_then(InstanceRef::unpack)
    }

    /// Bytes of instance data a backend uploads for this frame: the draw
    /// order, the record arena and the property table.
    pub fn instance_byte_len(&self) -> usize {
        self.instances.len() * 4
            + self.records.len() * SLOT_BYTES
            + self.properties.len() * PROPERTY_BYTES
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

/// Curves whose control polygon is no longer than this draw nothing.
const MIN_CURVE_LENGTH: f32 = 1e-4;

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

/// Where a property entry is and what it says.
#[derive(Debug, Clone, Copy)]
struct Prop {
    index: u32,
    data: Property,
}

/// Everything a property entry is made of: transform, clip, opacity.
type PropKey = [u32; 11];

struct Encoder<'a> {
    frame: &'a mut Frame,
    /// First instance of the draw being built.
    pending_start: u32,
    pending_atlas: Option<u32>,
    /// Segments of the longest curve of the pending run; 0 outside curves.
    pending_segments: u32,
    viewport: [f32; 2],
    colors: ColorMemo,
    /// Entries of the property table by content, and the last one used (runs
    /// of items share their transform, clip and opacity).
    props: HashMap<PropKey, Prop>,
    last_prop: Option<(PropKey, Prop)>,
}

/// A small direct-mapped memo of color conversions: a frame uses a few
/// dozen distinct colors over thousands of items, and OKLab to sRGB costs a
/// cube root and a power per channel.
struct ColorMemo {
    keys: Box<[[u32; 4]; Self::SLOTS]>,
    values: Box<[[f32; 4]; Self::SLOTS]>,
    used: Box<[bool; Self::SLOTS]>,
}

impl ColorMemo {
    const SLOTS: usize = 64;

    fn new() -> Self {
        ColorMemo {
            keys: Box::new([[0; 4]; Self::SLOTS]),
            values: Box::new([[0.0; 4]; Self::SLOTS]),
            used: Box::new([false; Self::SLOTS]),
        }
    }

    fn srgb_premul(&mut self, c: Color) -> [f32; 4] {
        let key = c.to_array().map(f32::to_bits);
        let h = key.iter().fold(0x9e37_79b9_u32, |h, k| {
            (h.rotate_left(5) ^ k).wrapping_mul(0x85eb_ca6b)
        });
        let slot = (h >> 8) as usize % Self::SLOTS;
        if self.used[slot] && self.keys[slot] == key {
            return self.values[slot];
        }
        let v = c.to_srgb_premul();
        self.keys[slot] = key;
        self.values[slot] = v;
        self.used[slot] = true;
        v
    }
}

/// Encodes a display list for a target.
pub fn encode(list: &DisplayList, options: FrameOptions) -> Result<Frame, RenderError> {
    let mut frame = Frame::new(options);
    encode_into(list, options, &mut frame)?;
    Ok(frame)
}

/// As [`encode`], into `frame`, whose buffers are reused (encoding into the
/// previous frame avoids allocating them anew every frame). On error `frame`
/// is left cleared.
pub fn encode_into(
    list: &DisplayList,
    options: FrameOptions,
    frame: &mut Frame,
) -> Result<(), RenderError> {
    frame.clear_data();
    if options.width == 0 || options.height == 0 {
        return Err(RenderError::EmptyTarget);
    }
    if !(options.scale.is_finite() && options.scale > 0.0) {
        return Err(RenderError::InvalidScale(options.scale));
    }
    frame.options = options;
    frame.clear = options.clear.to_srgb_premul();
    let mut enc = Encoder {
        frame: &mut *frame,
        pending_start: 0,
        pending_atlas: None,
        pending_segments: 0,
        viewport: [options.width as f32, options.height as f32],
        colors: ColorMemo::new(),
        props: HashMap::new(),
        last_prop: None,
    };
    let result = enc.run(list);
    enc.flush();
    if result.is_err() {
        frame.clear_data();
    }
    result
}

fn invalid(index: usize, reason: &'static str) -> RenderError {
    RenderError::InvalidPrimitive { index, reason }
}

fn valid_rect(index: usize, rect: Rect) -> Result<(), RenderError> {
    if rect.is_valid() {
        Ok(())
    } else {
        Err(invalid(index, "rect is not finite or has negative size"))
    }
}

impl Encoder<'_> {
    fn run(&mut self, list: &DisplayList) -> Result<(), RenderError> {
        let sorted = list.items.windows(2).all(|w| w[0].layer <= w[1].layer);
        // Layers in order (the usual case): no index vector needed.
        let order: Option<Vec<usize>> = (!sorted).then(|| {
            let mut order: Vec<usize> = (0..list.items.len()).collect();
            order.sort_by_key(|&i| list.items[i].layer);
            order
        });
        self.frame.instances.reserve(list.items.len());
        for k in 0..list.items.len() {
            let index = match &order {
                Some(order) => order[k],
                None => k,
            };
            match &list.items[index].primitive {
                Primitive::Quad(q) => self.quad(index, q)?,
                Primitive::Backdrop(b) => self.backdrop(index, b)?,
                Primitive::Glyphs(g) => self.glyphs(index, g)?,
                Primitive::Curve(c) => self.curve(index, c)?,
                // Typed hook: images draw nothing yet.
                Primitive::Image(_) => {}
            }
            if self.frame.records.len() > MAX_SLOT {
                return Err(invalid(index, "the frame has too many records"));
            }
        }
        Ok(())
    }

    /// Ends the pending draw.
    fn flush(&mut self) {
        let end = self.frame.instances.len() as u32;
        if end > self.pending_start {
            self.frame.ops.push(Op::Draw {
                start: self.pending_start,
                end,
                atlas: self.pending_atlas,
                vertices: 6 * self.pending_segments.max(1),
            });
        }
        self.pending_start = end;
        self.pending_atlas = None;
        self.pending_segments = 0;
    }

    /// Appends effect data (not drawn by itself) and returns its slot.
    #[inline]
    fn effect<const N: usize>(&mut self, slots: [Slot; N]) -> u32 {
        let at = self.frame.records.len() as u32;
        self.frame.records.extend_from_slice(&slots);
        at
    }

    /// Appends a record and its instance to the draw order.
    #[inline]
    fn push<const N: usize>(&mut self, family: Family, slots: [Slot; N]) {
        let slot = self.effect(slots);
        self.frame
            .instances
            .push(InstanceRef { family, slot }.pack());
    }

    /// Appends a record that is drawn with six vertices.
    fn push_box<const N: usize>(&mut self, family: Family, slots: [Slot; N]) {
        if self.pending_segments > 0 {
            self.flush();
        }
        self.push(family, slots);
    }

    /// The property entry for a transform, clip and opacity, shared with
    /// earlier items that had the same. `None` when nothing would be seen
    /// (singular transform, zero opacity).
    fn property(
        &mut self,
        index: usize,
        transform: Affine,
        clip: Option<Rect>,
        opacity: f32,
    ) -> Result<Option<Prop>, RenderError> {
        if !opacity.is_finite() {
            return Err(invalid(index, "opacity is not finite"));
        }
        let opacity = opacity.clamp(0.0, 1.0);
        if opacity == 0.0 {
            return Ok(None);
        }
        let s = self.frame.options.scale;
        let clip_px = match clip {
            Some(c) if !c.is_valid() => return Err(invalid(index, "clip is not finite")),
            Some(c) => [c.x * s, c.y * s, c.right() * s, c.bottom() * s],
            None => NO_CLIP,
        };
        let [a, b, c, d, e, f] = transform.m;
        let key: PropKey = [
            a, b, c, d, e, f, clip_px[0], clip_px[1], clip_px[2], clip_px[3], opacity,
        ]
        .map(f32::to_bits);
        if let Some((k, prop)) = &self.last_prop
            && *k == key
        {
            return Ok(Some(*prop));
        }
        let prop = match self.props.get(&key) {
            Some(prop) => *prop,
            None => {
                let Some(inv) = transform.inverse() else {
                    return Ok(None);
                };
                let [ia, ib, ic, id, ie, iff] = inv.m;
                let data = Property {
                    linear: [a, b, c, d],
                    inverse: [ia, ib, ic, id],
                    offset: [e, f, ie, iff],
                    clip: clip_px,
                    ppu: s * transform.det().abs().sqrt(),
                    opacity,
                    pad: [0.0; 2],
                };
                let prop = Prop {
                    index: self.frame.properties.len() as u32,
                    data,
                };
                self.frame.properties.push(data.to_slots());
                self.props.insert(key, prop);
                prop
            }
        };
        self.last_prop = Some((key, prop));
        Ok(Some(prop))
    }

    /// Whether anything of the local rectangle `[x0, y0, x1, y1]` is inside
    /// the clip and the target.
    fn visible(&self, bounds: [f32; 4], transform: Affine, prop: &Prop) -> bool {
        let local = Rect::new(
            bounds[0],
            bounds[1],
            bounds[2] - bounds[0],
            bounds[3] - bounds[1],
        );
        let s = self.frame.options.scale;
        let b = transform.bounds(local);
        let c = prop.data.clip;
        let (x0, y0) = ((b.x * s).max(c[0]).max(0.0), (b.y * s).max(c[1]).max(0.0));
        let (x1, y1) = (
            (b.right() * s).min(c[2]).min(self.viewport[0]),
            (b.bottom() * s).min(c[3]).min(self.viewport[1]),
        );
        x1 > x0 && y1 > y0
    }

    fn glyphs(&mut self, index: usize, g: &Glyphs) -> Result<(), RenderError> {
        if g.glyphs.is_empty() || g.color.alpha <= 0.0 {
            return Ok(());
        }
        if !g.color.to_array().iter().all(|v| v.is_finite()) {
            return Err(invalid(index, "glyph color is not finite"));
        }
        let Some(prop) = self.property(index, g.transform, g.clip, g.opacity)? else {
            return Ok(());
        };
        let color = self.colors.srgb_premul(g.color);
        let lightness = g.color.l.clamp(0.0, 1.0);
        for sprite in &g.glyphs {
            valid_rect(index, sprite.rect)?;
            let [tx, ty, tw, th] = sprite.texels;
            if tw == 0 || th == 0 || sprite.rect.w <= 0.0 || sprite.rect.h <= 0.0 {
                continue;
            }
            if sprite.texels.iter().any(|&v| v > u32::from(u16::MAX)) {
                return Err(invalid(index, "atlas texels are out of range"));
            }
            let record = SpriteRecord {
                rect: [sprite.rect.x, sprite.rect.y, sprite.rect.w, sprite.rect.h],
                color,
                texels: [tx | (ty << 16), tw | (th << 16)],
                prop: prop.index,
                lightness,
            };
            let bounds = shade::sprite_bounds(&record, &prop.data);
            if !self.visible(bounds, g.transform, &prop) {
                continue;
            }
            if self.pending_atlas.is_some_and(|p| p != g.atlas) {
                self.flush();
            }
            self.push_box(Family::Sprite, record.to_slots());
            self.pending_atlas = Some(g.atlas);
        }
        Ok(())
    }

    fn shadows(
        &mut self,
        index: usize,
        shape: &Shape,
        prop: &Prop,
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
            // Below about half a physical pixel a Gaussian is a hard edge;
            // keep sigma there so the analytic integral stays smooth.
            let sigma = (sh.blur * 0.5).max(0.5 / prop.data.ppu);
            let record = ShadowRecord {
                rect: shape.rect,
                ext: shape.ext,
                exps: shape.exps,
                shadow: [sh.offset[0], sh.offset[1], sh.spread, sigma],
                color: self.colors.srgb_premul(sh.color),
                prop: prop.index,
                pad: [0; 3],
            };
            if self.visible(shade::shadow_bounds(&record, &prop.data), transform, prop) {
                self.push_box(Family::Shadow, record.to_slots());
            }
        }
        Ok(())
    }

    fn quad(&mut self, index: usize, q: &Quad) -> Result<(), RenderError> {
        valid_rect(index, q.rect)?;
        let Some(prop) = self.property(index, q.transform, q.clip, q.opacity)? else {
            return Ok(());
        };
        let shape = Shape::new(q.rect, q.radii, q.corners);
        self.shadows(index, &shape, &prop, q.transform, &q.shadows)?;

        let mut record = QuadRecord {
            rect: shape.rect,
            ext: shape.ext,
            exps: shape.exps,
            prop: prop.index,
            ..QuadRecord::default()
        };
        let r = q.rect;
        let mut gradient = None;
        match &q.fill {
            None => record.flags = fill_kind::NONE,
            Some(Fill::Solid(c)) => {
                record.flags = fill_kind::SOLID;
                record.fill = self.colors.srgb_premul(*c);
            }
            Some(Fill::Linear { start, end, stops }) => {
                let geometry = [
                    r.x + start[0] * r.w,
                    r.y + start[1] * r.h,
                    r.x + end[0] * r.w,
                    r.y + end[1] * r.h,
                ];
                gradient = Some(gradient_record(index, geometry, stops)?);
                record.flags = fill_kind::LINEAR | ((stops.len() as u32) << 8);
            }
            Some(Fill::Radial {
                center,
                radius,
                stops,
            }) => {
                let geometry = [
                    r.x + center[0] * r.w,
                    r.y + center[1] * r.h,
                    (radius[0] * r.w).max(1e-6),
                    (radius[1] * r.h).max(1e-6),
                ];
                gradient = Some(gradient_record(index, geometry, stops)?);
                record.flags = fill_kind::RADIAL | ((stops.len() as u32) << 8);
            }
        }
        if let Some(border) = q.border {
            record.border_width = match border.width {
                BorderWidth::Hairline => 1.0 / prop.data.ppu,
                BorderWidth::Logical(w) if w.is_finite() => w.max(0.0),
                BorderWidth::Logical(_) => {
                    return Err(invalid(index, "border width is not finite"));
                }
            };
            record.border = self.colors.srgb_premul(border.color);
        }
        if record.flags == fill_kind::NONE && record.border_width == 0.0 {
            return Ok(());
        }
        if self.visible(
            shade::shape_bounds(record.rect, &prop.data),
            q.transform,
            &prop,
        ) {
            if let Some(gradient) = gradient {
                record.effect = self.effect(gradient.to_slots());
            }
            self.push_box(Family::Quad, record.to_slots());
        }
        Ok(())
    }

    fn curve(&mut self, index: usize, c: &Curve) -> Result<(), RenderError> {
        let finite = c.points.iter().flatten().all(|v| v.is_finite()) && c.width.is_finite();
        if !finite || !c.color.to_array().iter().all(|v| v.is_finite()) {
            return Err(invalid(index, "curve is not finite"));
        }
        let p = c.points;
        let length: f32 = (0..3)
            .map(|i| curve::length(p[i + 1][0] - p[i][0], p[i + 1][1] - p[i][1]))
            .sum();
        if c.width <= 0.0 || c.color.alpha <= 0.0 || length <= MIN_CURVE_LENGTH {
            return Ok(());
        }
        let Some(prop) = self.property(index, c.transform, c.clip, c.opacity)? else {
            return Ok(());
        };
        // A stroke is at least one physical pixel wide; thinner ones fade.
        let min_width = 1.0 / prop.data.ppu;
        let (width, color) = if c.width < min_width {
            (
                min_width,
                c.color.with_alpha(c.color.alpha * (c.width / min_width)),
            )
        } else {
            (c.width, c.color)
        };
        let px_per_unit = self.frame.options.scale * curve::max_stretch(c.transform);
        let half_width = width * 0.5;
        let segments = curve::segment_count(p, px_per_unit);
        let record = CurveRecord {
            p01: [p[0][0], p[0][1], p[1][0], p[1][1]],
            p23: [p[2][0], p[2][1], p[3][0], p[3][1]],
            color: self.colors.srgb_premul(color),
            prop: prop.index,
            half_width,
            segments,
            window: curve::search_window(
                p,
                segments,
                shade::curve_claim(half_width, prop.data.ppu),
            ),
        };
        if self.visible(shade::curve_bounds(&record, &prop.data), c.transform, &prop) {
            if self.pending_segments == 0 {
                self.flush();
            }
            self.push(Family::Curve, record.to_slots());
            self.pending_segments = self.pending_segments.max(record.segments);
        }
        Ok(())
    }

    fn backdrop(&mut self, index: usize, b: &Backdrop) -> Result<(), RenderError> {
        valid_rect(index, b.rect)?;
        let Some(prop) = self.property(index, b.transform, b.clip, b.opacity)? else {
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
        let shape = Shape::new(b.rect, b.radii, b.corners);
        let mut record = BackdropRecord {
            rect: shape.rect,
            ext: shape.ext,
            exps: shape.exps,
            tint: self.colors.srgb_premul(b.tint),
            params: [0.0, 0.0, b.saturation.max(0.0), b.grain.max(0.0)],
            prop: prop.index,
            ..BackdropRecord::default()
        };
        if let Some(h) = b.highlight
            && h.width.is_finite()
            && h.width > 0.0
            && h.color.alpha > 0.0
        {
            record.rim = self.colors.srgb_premul(h.color);
            record.params[0] = h.width;
            record.params[1] = match h.kind {
                HighlightKind::Full => 0.0,
                HighlightKind::TopRim => 1.0,
            };
        }
        if self.visible(
            shade::shape_bounds(record.rect, &prop.data),
            b.transform,
            &prop,
        ) {
            let blur = self.blur_plan(b);
            record.has_blur = u32::from(blur.is_some());
            self.flush();
            let instance = self.frame.instances.len() as u32;
            self.push(Family::Backdrop, record.to_slots());
            self.frame.ops.push(Op::Backdrop { instance, blur });
            self.pending_start = self.frame.instances.len() as u32;
        }
        // Shadows go after the composite (they are clipped out of the shape,
        // so the order does not change the picture, and the blur does not
        // pick up the panel's own shadow).
        self.shadows(index, &shape, &prop, b.transform, &b.shadows)
    }

    fn blur_plan(&self, b: &Backdrop) -> Option<BlurPlan> {
        let s = self.frame.options.scale;
        let radius_px = b.blur * s * b.transform.det().abs().sqrt();
        if radius_px < 0.5 {
            return None;
        }
        let (levels, offset) = kawase_params(radius_px);
        let reach = kawase_reach(levels, offset);
        let bb = b.transform.bounds(b.rect);
        let x0 = (bb.x * s - reach).floor().max(0.0);
        let y0 = (bb.y * s - reach).floor().max(0.0);
        let x1 = (bb.right() * s + reach).ceil().min(self.viewport[0]);
        let y1 = (bb.bottom() * s + reach).ceil().min(self.viewport[1]);
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

/// A rounded rectangle as records hold it.
struct Shape {
    rect: [f32; 4],
    ext: [f32; 4],
    exps: [f32; 4],
}

impl Shape {
    fn new(rect: Rect, radii: Radii, corners: CornerStyle) -> Shape {
        let (ext, exps) = corner_params(rect, radii, corners);
        Shape {
            rect: [rect.x, rect.y, rect.w, rect.h],
            ext,
            exps,
        }
    }
}

fn gradient_record(
    index: usize,
    geometry: [f32; 4],
    stops: &[GradientStop],
) -> Result<GradientRecord, RenderError> {
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
    let mut record = GradientRecord {
        geometry,
        ..GradientRecord::default()
    };
    let mut prev = f32::MIN;
    for (i, s) in stops.iter().enumerate() {
        // Offsets must not decrease (CSS clamps them the same way).
        prev = prev.max(s.offset);
        record.offsets[i] = prev;
        record.stops[i] = s.color.to_array();
    }
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> FrameOptions {
        FrameOptions {
            width: 200,
            height: 100,
            scale: 2.0,
            clear: Color::BLACK,
        }
    }

    fn sample_list(n: usize) -> DisplayList {
        let mut list = DisplayList::new();
        for i in 0..n {
            let c = Color::oklab(0.3 + 0.01 * (i % 7) as f32, 0.02, -0.01);
            let mut q = Quad::new(Rect::new(i as f32 * 3.0, 4.0, 20.0, 10.0), c);
            q.radii = Radii::all(3.0);
            // Out of order layers exercise the sorted path too.
            list.push(if i % 5 == 0 { 1 } else { 0 }, q);
        }
        list
    }

    #[test]
    fn encode_into_reuses_buffers_and_matches_encode() {
        let list = sample_list(40);
        let fresh = encode(&list, opts()).unwrap();
        let mut frame = encode(&sample_list(300), opts()).unwrap();
        let cap = (frame.instances.capacity(), frame.records.capacity());
        encode_into(&list, opts(), &mut frame).unwrap();
        assert_eq!(frame, fresh);
        assert!(
            frame.instances.capacity() >= cap.0 && frame.records.capacity() >= cap.1,
            "capacity is kept"
        );
        // Layer-ordered input takes the no-sort path and gives the same.
        let mut flat = DisplayList::new();
        for it in &list.items {
            flat.push(0, it.primitive.clone());
        }
        let a = encode(&flat, opts()).unwrap();
        encode_into(&flat, opts(), &mut frame).unwrap();
        assert_eq!(frame, a);
    }

    #[test]
    fn encode_into_clears_the_frame_on_error() {
        let mut frame = encode(&sample_list(3), opts()).unwrap();
        let bad = FrameOptions { width: 0, ..opts() };
        assert!(encode_into(&sample_list(3), bad, &mut frame).is_err());
        assert!(frame.instances.is_empty() && frame.ops.is_empty());
        assert!(frame.records.is_empty() && frame.properties.is_empty());
    }

    #[test]
    fn color_memo_matches_direct_conversion() {
        let mut memo = ColorMemo::new();
        for i in 0..500 {
            let c = Color::oklab(0.001 * i as f32, 0.0005 * (i % 13) as f32, -0.01)
                .with_alpha(0.5 + 0.001 * (i % 7) as f32);
            assert_eq!(memo.srgb_premul(c), c.to_srgb_premul());
            // Again: a hit.
            assert_eq!(memo.srgb_premul(c), c.to_srgb_premul());
        }
    }

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
                atlas: None,
                vertices: 6
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

    fn family_order(frame: &Frame) -> Vec<Family> {
        (0..frame.instances.len())
            .map(|i| frame.instance(i).unwrap().family)
            .collect()
    }

    fn sprites(n: usize) -> Glyphs {
        let sprite = crate::list::GlyphSprite {
            rect: Rect::new(1.0, 1.0, 4.0, 4.0),
            texels: [0, 0, 4, 4],
        };
        Glyphs::new(0, vec![sprite; n], Color::WHITE)
    }

    fn wire(y: f32) -> Curve {
        Curve::new(
            [[0.0, y], [40.0, y], [40.0, y + 30.0], [80.0, y + 30.0]],
            2.0,
            Color::WHITE,
        )
    }

    #[test]
    fn families_keep_painter_order_and_batch() {
        let mut list = DisplayList::new();
        let q = Quad::new(Rect::new(0.0, 0.0, 9.0, 9.0), Color::WHITE);
        list.add(q.clone().shadows(&[Shadow {
            offset: [0.0, 1.0],
            blur: 2.0,
            spread: 0.0,
            color: Color::BLACK,
        }]));
        list.add(sprites(2));
        list.add(q.clone());
        list.add(sprites(1));
        list.add(wire(1.0));
        list.add(wire(5.0));
        list.add(q);
        let frame = encode(&list, opts()).unwrap();
        use Family::{Curve as C, Quad as Q, Shadow as S, Sprite as P};
        assert_eq!(family_order(&frame), [S, Q, P, P, Q, P, C, C, Q]);
        // Quads, shadows and sprites share a draw; the curves are one.
        let draws: Vec<_> = frame
            .ops
            .iter()
            .map(|op| match *op {
                Op::Draw {
                    start,
                    end,
                    atlas,
                    vertices,
                } => (start, end, atlas, vertices),
                Op::Backdrop { .. } => panic!("no backdrops here"),
            })
            .collect();
        let segments = (0..2)
            .map(|i| {
                let slot = frame.instance(6 + i).unwrap().slot as usize;
                CurveRecord::read(&frame.records, slot).unwrap().segments
            })
            .max()
            .unwrap();
        assert!(segments > 1);
        assert_eq!(
            draws,
            [
                (0, 6, Some(0), 6),
                (6, 8, None, 6 * segments),
                (8, 9, None, 6)
            ]
        );
        // One property entry for all of them.
        assert_eq!(frame.properties.len(), 1);
    }

    #[test]
    fn records_have_their_family_sizes_and_effects_are_extra() {
        let upload = |list: &DisplayList| {
            let f = encode(list, opts()).unwrap();
            f.records.len() * SLOT_BYTES
        };
        let mut sprite = DisplayList::new();
        sprite.add(sprites(1));
        assert_eq!(upload(&sprite), 48);
        let q = Quad::new(Rect::new(0.0, 0.0, 9.0, 9.0), Color::WHITE);
        let mut solid = DisplayList::new();
        solid.add(q.clone());
        assert_eq!(upload(&solid), 96);
        let mut gradient = DisplayList::new();
        gradient.add(q.fill(Fill::vertical(Color::WHITE, Color::BLACK)));
        assert_eq!(upload(&gradient), 96 + 96);
        let mut curve = DisplayList::new();
        curve.add(wire(1.0));
        assert_eq!(upload(&curve), 64);
        // The instance entry and one shared property.
        let f = encode(&curve, opts()).unwrap();
        assert_eq!(f.instance_byte_len(), 4 + 64 + 80);
    }

    #[test]
    fn properties_are_shared_by_content() {
        let mut list = DisplayList::new();
        let q = Quad::new(Rect::new(0.0, 0.0, 9.0, 9.0), Color::WHITE);
        let moved = Quad {
            transform: Affine::translate(3.0, 0.0),
            ..q.clone()
        };
        let faded = Quad {
            opacity: 0.5,
            ..q.clone()
        };
        let clipped = Quad {
            clip: Some(Rect::new(0.0, 0.0, 5.0, 5.0)),
            ..q.clone()
        };
        for _ in 0..3 {
            for q in [&q, &moved, &faded, &clipped] {
                list.add(q.clone());
            }
        }
        let frame = encode(&list, opts()).unwrap();
        assert_eq!(frame.instances.len(), 12);
        assert_eq!(frame.properties.len(), 4);
    }

    #[test]
    fn a_thin_curve_is_widened_to_a_pixel_and_faded() {
        let mut list = DisplayList::new();
        list.add(Curve {
            width: 0.125,
            ..wire(1.0)
        });
        let frame = encode(&list, opts()).unwrap();
        let slot = frame.instance(0).unwrap().slot as usize;
        let record = CurveRecord::read(&frame.records, slot).unwrap();
        // Scale 2: a physical pixel is 0.5 units, the stroke a quarter of it.
        assert_eq!(record.half_width, 0.25);
        assert_eq!(record.color, Color::WHITE.with_alpha(0.25).to_srgb_premul());
    }

    #[test]
    fn invisible_and_degenerate_curves_are_dropped_and_bad_ones_refused() {
        let w = wire(1.0);
        for c in [
            Curve { width: 0.0, ..w },
            Curve { opacity: 0.0, ..w },
            Curve {
                color: Color::TRANSPARENT,
                ..w
            },
            Curve {
                points: [[3.0, 3.0]; 4],
                ..w
            },
            Curve {
                transform: Affine::translate(1000.0, 0.0),
                ..w
            },
        ] {
            let mut list = DisplayList::new();
            list.add(c);
            let frame = encode(&list, opts()).unwrap();
            assert!(frame.instances.is_empty() && frame.ops.is_empty(), "{c:?}");
        }
        let mut list = DisplayList::new();
        list.add(Curve {
            points: [[f32::NAN, 0.0]; 4],
            ..w
        });
        assert!(matches!(
            encode(&list, opts()),
            Err(RenderError::InvalidPrimitive { index: 0, .. })
        ));
    }
}
