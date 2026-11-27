// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The GPU data formats (frame pipeline, section 4.5).
//!
//! A frame is three arrays:
//!
//! - **records**: one arena of 16-byte [`Slot`]s. Every primitive is a
//!   record of its family's size, a whole number of slots. Effect data that
//!   only some primitives need (gradients) is a record of its own that the
//!   primitive refers to by slot offset, so common primitives do not pay for
//!   it.
//! - **properties**: transform, clip and opacity, shared by every record
//!   that has the same ones and referred to by index. Moving a canvas
//!   changes its property entries, not its records.
//! - **instances**: the draw order, one `u32` [`InstanceRef`] each (family
//!   and slot offset), read by the vertex shader as a per-instance
//!   attribute.
//!
//! Records are position independent apart from the offsets and property
//! indices they hold, so a retained renderer can keep them across frames and
//! rebuild only the instance list.
//!
//! Every record here is a `#[repr(C)]` struct of 4-byte scalars, mirrored by
//! a struct of the same name in `shaders/primitives.wgsl`; a test compares
//! the member offsets.

/// The unit of the record arena: 16 bytes.
pub type Slot = [u32; 4];

/// Bytes in one [`Slot`].
pub const SLOT_BYTES: usize = 16;

/// A value stored as consecutive 32-bit words.
trait Words: Sized {
    fn put(self, words: &mut [u32], at: &mut usize);
    fn take(words: &[u32], at: &mut usize) -> Self;
}

impl Words for u32 {
    #[inline]
    fn put(self, words: &mut [u32], at: &mut usize) {
        words[*at] = self;
        *at += 1;
    }
    #[inline]
    fn take(words: &[u32], at: &mut usize) -> u32 {
        let v = words[*at];
        *at += 1;
        v
    }
}

impl Words for f32 {
    #[inline]
    fn put(self, words: &mut [u32], at: &mut usize) {
        self.to_bits().put(words, at);
    }
    #[inline]
    fn take(words: &[u32], at: &mut usize) -> f32 {
        f32::from_bits(u32::take(words, at))
    }
}

impl<T: Words + Copy + Default, const N: usize> Words for [T; N] {
    #[inline]
    fn put(self, words: &mut [u32], at: &mut usize) {
        for v in self {
            v.put(words, at);
        }
    }
    #[inline]
    fn take(words: &[u32], at: &mut usize) -> [T; N] {
        let mut out = [T::default(); N];
        for v in &mut out {
            *v = T::take(words, at);
        }
        out
    }
}

/// Declares a record of `$slots` slots. Fields are written in declaration
/// order without gaps, which for 4-byte scalars is the `#[repr(C)]` layout;
/// the size is checked at compile time.
macro_rules! record {
    (
        $(#[$meta:meta])*
        $name:ident, slots = $slots:literal {
            $($(#[$fmeta:meta])* $field:ident: $ty:ty,)+
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Default)]
        #[repr(C)]
        pub(crate) struct $name {
            $($(#[$fmeta])* pub $field: $ty,)+
        }

        impl $name {
            /// Size in slots.
            pub const SLOTS: usize = $slots;

            /// Member names and byte offsets (for the layout test).
            #[cfg(test)]
            pub const LAYOUT: &'static [(&'static str, usize)] =
                &[$((stringify!($field), std::mem::offset_of!($name, $field)),)+];

            /// The record as arena slots.
            #[inline]
            pub fn to_slots(self) -> [Slot; $slots] {
                let mut out = [[0u32; 4]; $slots];
                let words = out.as_flattened_mut();
                let mut at = 0;
                $(Words::put(self.$field, words, &mut at);)+
                out
            }

            /// Reads a record from the slots starting at `at`; `None` when
            /// the arena is too short.
            #[inline]
            pub fn read(records: &[Slot], at: usize) -> Option<$name> {
                let words = records.get(at..at.checked_add(Self::SLOTS)?)?.as_flattened();
                let mut next = 0;
                Some($name {
                    $($field: Words::take(words, &mut next),)+
                })
            }
        }

        const _: () = assert!(std::mem::size_of::<$name>() == $slots * SLOT_BYTES);
    };
}

record! {
    /// A glyph or icon image from an R8 atlas.
    SpriteRecord, slots = 3 {
        /// Destination `(x, y, w, h)`, local units.
        rect: [f32; 4],
        /// Premultiplied sRGB.
        color: [f32; 4],
        /// Atlas texel rect, `[x | y << 16, w | h << 16]`.
        texels: [u32; 2],
        /// Property index.
        prop: u32,
        /// OKLab lightness of the text color, for the coverage remapping.
        lightness: f32,
    }
}

record! {
    /// A rounded rectangle with fill and inner border.
    QuadRecord, slots = 6 {
        rect: [f32; 4],
        /// Corner extents (superellipse box size) `tl, tr, br, bl`.
        ext: [f32; 4],
        /// Corner exponents (2 = circle).
        exps: [f32; 4],
        /// Solid fill, premultiplied sRGB.
        fill: [f32; 4],
        /// Border color, premultiplied sRGB.
        border: [f32; 4],
        prop: u32,
        /// Fill kind in bits 0..8 ([`fill_kind`]), stop count in bits 8..16.
        flags: u32,
        /// Border width, local units (0 = none).
        border_width: f32,
        /// Slot of the [`GradientRecord`] (gradient fills only).
        effect: u32,
    }
}

record! {
    /// Effect data of a gradient quad.
    GradientRecord, slots = 6 {
        /// Linear: `start, end`; radial: `center, radius`; local units.
        geometry: [f32; 4],
        /// Stop offsets.
        offsets: [f32; 4],
        /// Straight OKLab + alpha.
        stops: [[f32; 4]; 4],
    }
}

record! {
    /// One drop shadow of a shape.
    ShadowRecord, slots = 6 {
        rect: [f32; 4],
        ext: [f32; 4],
        exps: [f32; 4],
        /// `(dx, dy, spread, sigma)`.
        shadow: [f32; 4],
        /// Premultiplied sRGB.
        color: [f32; 4],
        prop: u32,
        pad: [u32; 3],
    }
}

record! {
    /// A stroked cubic Bezier.
    CurveRecord, slots = 4 {
        /// Start and first control point.
        p01: [f32; 4],
        /// Second control point and end.
        p23: [f32; 4],
        /// Premultiplied sRGB.
        color: [f32; 4],
        prop: u32,
        /// Half the stroke width, local units.
        half_width: f32,
        /// Straight pieces the curve is drawn with, 1 to
        /// [`MAX_CURVE_SEGMENTS`](crate::MAX_CURVE_SEGMENTS).
        segments: u32,
        /// Only segments this far from a box's own can be nearer to one of
        /// its pixels than its own segment is ([`curve::search_window`]
        /// (crate::curve::search_window)).
        window: u32,
    }
}

record! {
    /// A blurred, tinted region.
    BackdropRecord, slots = 7 {
        rect: [f32; 4],
        ext: [f32; 4],
        exps: [f32; 4],
        /// Premultiplied sRGB.
        tint: [f32; 4],
        /// Highlight color, premultiplied sRGB.
        rim: [f32; 4],
        /// `(highlight_width, highlight_kind, saturation, grain)`.
        params: [f32; 4],
        prop: u32,
        /// 1 when the composite samples the blurred levels.
        has_blur: u32,
        pad: [u32; 2],
    }
}

record! {
    /// Transform, clip and opacity shared by records.
    Property, slots = 5 {
        /// Local-to-screen linear part `(a, b, c, d)`.
        linear: [f32; 4],
        /// Linear part of the inverse.
        inverse: [f32; 4],
        /// `(e, f)` of the transform, then of the inverse.
        offset: [f32; 4],
        /// Clip `(x0, y0, x1, y1)`, physical pixels.
        clip: [f32; 4],
        /// Physical pixels per local unit.
        ppu: f32,
        opacity: f32,
        pad: [f32; 2],
    }
}

/// Fill kinds ([`QuadRecord::flags`]).
pub(crate) mod fill_kind {
    pub const NONE: u32 = 0;
    pub const SOLID: u32 = 1;
    pub const LINEAR: u32 = 2;
    pub const RADIAL: u32 = 3;
}

/// What a record draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// A glyph or icon: 48 bytes.
    Sprite,
    /// A rounded rectangle: 96 bytes (plus 96 for a gradient).
    Quad,
    /// A drop shadow: 96 bytes.
    Shadow,
    /// A stroked cubic Bezier: 64 bytes.
    Curve,
    /// A backdrop composite: 112 bytes.
    Backdrop,
}

impl Family {
    /// Bytes of one record of this family (without effect data).
    #[must_use]
    pub const fn record_bytes(self) -> usize {
        let slots = match self {
            Family::Sprite => SpriteRecord::SLOTS,
            Family::Quad => QuadRecord::SLOTS,
            Family::Shadow => ShadowRecord::SLOTS,
            Family::Curve => CurveRecord::SLOTS,
            Family::Backdrop => BackdropRecord::SLOTS,
        };
        slots * SLOT_BYTES
    }
}

/// Bytes of one property table entry.
pub const PROPERTY_BYTES: usize = Property::SLOTS * SLOT_BYTES;

// The size goals of the frame pipeline (section 4.5).
const _: () = assert!(Family::Sprite.record_bytes() <= 48);
const _: () = assert!(Family::Quad.record_bytes() <= 96);
const _: () = assert!(Family::Curve.record_bytes() <= 64);

/// Bits of an [`InstanceRef`] that hold the slot offset; the family is in
/// the bits above. Must match `primitives.wgsl`.
const OFFSET_BITS: u32 = 29;
const OFFSET_MASK: u32 = (1 << OFFSET_BITS) - 1;
/// Largest slot offset an instance can refer to.
pub(crate) const MAX_SLOT: usize = OFFSET_MASK as usize;

/// One entry of the draw order: which record to draw and what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceRef {
    /// The record's family.
    pub family: Family,
    /// The record's first slot in the arena.
    pub slot: u32,
}

impl InstanceRef {
    /// The `u32` the GPU reads.
    #[must_use]
    pub fn pack(self) -> u32 {
        let tag: u32 = match self.family {
            Family::Sprite => 0,
            Family::Quad => 1,
            Family::Shadow => 2,
            Family::Curve => 3,
            Family::Backdrop => 4,
        };
        (tag << OFFSET_BITS) | (self.slot & OFFSET_MASK)
    }

    /// The reverse of [`InstanceRef::pack`]; `None` for an unknown family.
    #[must_use]
    pub fn unpack(word: u32) -> Option<InstanceRef> {
        let family = match word >> OFFSET_BITS {
            0 => Family::Sprite,
            1 => Family::Quad,
            2 => Family::Shadow,
            3 => Family::Curve,
            4 => Family::Backdrop,
            _ => return None,
        };
        Some(InstanceRef {
            family,
            slot: word & OFFSET_MASK,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `to_slots` puts every field where `#[repr(C)]` says it is, and `read`
    /// is its inverse.
    #[test]
    fn records_are_their_repr_c_layout() {
        let sprite = SpriteRecord {
            rect: [1.0, 2.0, 3.0, 4.0],
            color: [0.1, 0.2, 0.3, 0.4],
            texels: [7, 8],
            prop: 9,
            lightness: 0.5,
        };
        let slots = sprite.to_slots();
        let words = slots.as_flattened();
        let word = |name: &str| {
            let (_, offset) = SpriteRecord::LAYOUT
                .iter()
                .find(|(n, _)| *n == name)
                .unwrap();
            words[offset / 4]
        };
        assert_eq!(word("rect"), 1.0f32.to_bits());
        assert_eq!(word("color"), 0.1f32.to_bits());
        assert_eq!(word("texels"), 7);
        assert_eq!(word("prop"), 9);
        assert_eq!(word("lightness"), 0.5f32.to_bits());
        let arena = [[[9u32; 4]].as_slice(), &slots].concat();
        assert_eq!(SpriteRecord::read(&arena, 1), Some(sprite));
        assert_eq!(SpriteRecord::read(&arena, 2), None);
        assert_eq!(SpriteRecord::read(&arena, usize::MAX), None);

        let gradient = GradientRecord {
            geometry: [1.0; 4],
            offsets: [2.0; 4],
            stops: [[3.0; 4], [4.0; 4], [5.0; 4], [6.0; 4]],
        };
        assert_eq!(
            GradientRecord::read(&gradient.to_slots(), 0),
            Some(gradient)
        );
    }

    #[test]
    fn record_sizes() {
        assert_eq!(Family::Sprite.record_bytes(), 48);
        assert_eq!(Family::Quad.record_bytes(), 96);
        assert_eq!(Family::Shadow.record_bytes(), 96);
        assert_eq!(Family::Curve.record_bytes(), 64);
        assert_eq!(Family::Backdrop.record_bytes(), 112);
        assert_eq!(PROPERTY_BYTES, 80);
    }

    #[test]
    fn instance_refs_round_trip() {
        for family in [
            Family::Sprite,
            Family::Quad,
            Family::Shadow,
            Family::Curve,
            Family::Backdrop,
        ] {
            for slot in [0, 1, MAX_SLOT as u32] {
                let r = InstanceRef { family, slot };
                assert_eq!(InstanceRef::unpack(r.pack()), Some(r));
            }
        }
        assert_eq!(InstanceRef::unpack(u32::MAX), None);
    }

    /// Every record struct in the shader has the same members at the same
    /// offsets as here, and the same size.
    #[test]
    fn wgsl_layout_agrees() {
        let module = naga::front::wgsl::parse_str(include_str!("shaders/primitives.wgsl"))
            .expect("the shader parses");
        let check = |name: &str, layout: &[(&str, usize)], slots: usize| {
            let ty = module
                .types
                .iter()
                .find(|(_, t)| t.name.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("no struct {name} in the shader"))
                .1;
            let naga::TypeInner::Struct { members, span } = &ty.inner else {
                panic!("{name} is not a struct");
            };
            assert_eq!(*span as usize, slots * SLOT_BYTES, "{name} size");
            let mut shader: Vec<(String, usize)> = members
                .iter()
                .map(|m| (m.name.clone().unwrap_or_default(), m.offset as usize))
                // Padding may be split differently; arrays of vectors may be
                // spelled as numbered members.
                .filter(|(n, _)| !n.starts_with("pad"))
                .collect();
            shader.retain(|(n, _)| !matches!(n.as_str(), "stop1" | "stop2" | "stop3"));
            let rust: Vec<(String, usize)> = layout
                .iter()
                .filter(|(n, _)| !n.starts_with("pad"))
                .map(|(n, o)| {
                    let n = if *n == "stops" { "stop0" } else { n };
                    (n.to_owned(), *o)
                })
                .collect();
            assert_eq!(shader, rust, "{name} members");
        };
        check("SpriteRecord", SpriteRecord::LAYOUT, SpriteRecord::SLOTS);
        check("QuadRecord", QuadRecord::LAYOUT, QuadRecord::SLOTS);
        check(
            "GradientRecord",
            GradientRecord::LAYOUT,
            GradientRecord::SLOTS,
        );
        check("ShadowRecord", ShadowRecord::LAYOUT, ShadowRecord::SLOTS);
        check("CurveRecord", CurveRecord::LAYOUT, CurveRecord::SLOTS);
        check(
            "BackdropRecord",
            BackdropRecord::LAYOUT,
            BackdropRecord::SLOTS,
        );
        check("Property", Property::LAYOUT, Property::SLOTS);
    }
}
