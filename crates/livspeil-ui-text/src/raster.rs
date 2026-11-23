// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Glyph rasterization: skrifa outlines (unhinted, variable-font aware)
//! filled by zeno into 8-bit grayscale coverage masks. No LCD subpixel AA
//! (UI spec §4); horizontal subpixel *positioning* is optional.

use std::fmt;
use std::hash::{Hash, Hasher};

use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, GlyphId, MetadataProvider};
use zeno::{Command, Mask, Scratch};

/// Horizontal glyph positioning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SubpixelMode {
    /// Snap every glyph origin to whole pixels (crisp at 1x).
    #[default]
    None,
    /// Four x-offsets per glyph (0, ¼, ½, ¾ px): smoother spacing on HiDPI
    /// at up to 4x the atlas entries per glyph.
    Quarter,
}

impl SubpixelMode {
    /// The spec's default: quarter-pixel positioning on HiDPI (scale ≥ 1.5),
    /// whole pixels otherwise.
    #[must_use]
    pub fn for_scale(scale: f32) -> Self {
        if scale >= 1.5 {
            SubpixelMode::Quarter
        } else {
            SubpixelMode::None
        }
    }

    /// Split a physical x into (whole pixels, subpixel bin 0..4).
    #[must_use]
    pub fn quantize(self, x: f32) -> (i32, u8) {
        match self {
            SubpixelMode::None => (x.round() as i32, 0),
            SubpixelMode::Quarter => {
                let q = (x * 4.0).round() as i32;
                (q.div_euclid(4), q.rem_euclid(4) as u8)
            }
        }
    }
}

/// Identity of a rasterized glyph image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    /// Identity of the font data blob (process-local).
    pub font_id: u64,
    pub font_index: u32,
    pub glyph_id: u32,
    /// Physical pixel size (`f32::to_bits`).
    pub size_bits: u32,
    /// Hash of the variation coordinates (weight etc.).
    pub coords_hash: u64,
    /// Subpixel bin, 0..4 (always 0 for [`SubpixelMode::None`]).
    pub subpixel_x: u8,
}

impl GlyphKey {
    /// Deterministic hash of normalized variation coordinates.
    #[must_use]
    pub fn hash_coords(coords: &[i16]) -> u64 {
        // `DefaultHasher::new()` uses fixed keys: stable within a build.
        let mut h = std::hash::DefaultHasher::new();
        coords.hash(&mut h);
        h.finish()
    }
}

/// A grayscale coverage mask with its placement relative to the glyph
/// origin (pen position on the baseline, y pointing down).
#[derive(Clone, PartialEq, Eq)]
pub struct GlyphMask {
    pub width: u32,
    pub height: u32,
    /// Offset of the mask's left edge from the pen x.
    pub left: i32,
    /// Offset of the mask's top edge from the baseline (negative = above).
    pub top: i32,
    /// `width * height` coverage bytes, row-major, 0 = transparent.
    pub data: Vec<u8>,
}

impl fmt::Debug for GlyphMask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GlyphMask")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("left", &self.left)
            .field("top", &self.top)
            .finish_non_exhaustive()
    }
}

impl GlyphMask {
    /// Whether the glyph has no visible pixels (e.g. a space).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

struct ZenoPen<'a> {
    path: &'a mut Vec<Command>,
    dx: f32,
}

impl OutlinePen for ZenoPen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.push(Command::MoveTo([x + self.dx, -y].into()));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path.push(Command::LineTo([x + self.dx, -y].into()));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.path.push(Command::QuadTo(
            [cx + self.dx, -cy].into(),
            [x + self.dx, -y].into(),
        ));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.path.push(Command::CurveTo(
            [cx0 + self.dx, -cy0].into(),
            [cx1 + self.dx, -cy1].into(),
            [x + self.dx, -y].into(),
        ));
    }
    fn close(&mut self) {
        self.path.push(Command::Close);
    }
}

/// Reusable rasterizer state (path and scanline scratch buffers).
pub struct GlyphRasterizer {
    mode: SubpixelMode,
    path: Vec<Command>,
    coords: Vec<NormalizedCoord>,
    scratch: Scratch,
}

impl fmt::Debug for GlyphRasterizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GlyphRasterizer")
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

impl GlyphRasterizer {
    #[must_use]
    pub fn new(mode: SubpixelMode) -> Self {
        Self {
            mode,
            path: Vec::new(),
            coords: Vec::new(),
            scratch: Scratch::new(),
        }
    }

    #[must_use]
    pub fn mode(&self) -> SubpixelMode {
        self.mode
    }

    /// Rasterize one glyph of the face at `index` in `font_data` at
    /// `size_px` physical pixels per em, shifted right by
    /// `subpixel_x / 4` px. Returns `None` when the font or glyph has no
    /// outline (bitmap/COLR-only glyphs are not supported yet).
    pub fn rasterize(
        &mut self,
        font_data: &[u8],
        index: u32,
        glyph_id: u32,
        size_px: f32,
        coords: &[i16],
        subpixel_x: u8,
    ) -> Option<GlyphMask> {
        let font = FontRef::from_index(font_data, index).ok()?;
        let outlines = font.outline_glyphs();
        let glyph = outlines.get(GlyphId::new(glyph_id))?;
        self.coords.clear();
        self.coords
            .extend(coords.iter().map(|c| NormalizedCoord::from_bits(*c)));
        self.path.clear();
        let mut pen = ZenoPen {
            path: &mut self.path,
            dx: f32::from(subpixel_x) * 0.25,
        };
        let settings =
            DrawSettings::unhinted(Size::new(size_px), LocationRef::new(self.coords.as_slice()));
        glyph.draw(settings, &mut pen).ok()?;
        if self.path.is_empty() {
            return Some(GlyphMask {
                width: 0,
                height: 0,
                left: 0,
                top: 0,
                data: Vec::new(),
            });
        }
        let (data, placement) = Mask::with_scratch(&self.path, &mut self.scratch).render();
        Some(GlyphMask {
            width: placement.width,
            height: placement.height,
            left: placement.left,
            top: placement.top,
            data,
        })
    }
}
