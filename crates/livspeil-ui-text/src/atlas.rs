// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! CPU-side R8 glyph atlas with a shelf allocator (etagere) and dirty
//! rectangle tracking. The renderer owns the GPU texture and uploads what
//! [`GlyphAtlas::take_dirty`] returns; after [`GlyphAtlas::clear`] (or on
//! first use) it re-uploads everything (see [`GlyphAtlas::generation`]).

use std::collections::HashMap;
use std::fmt;

use etagere::{AtlasAllocator, size2};

use crate::error::TextError;
use crate::raster::{GlyphKey, GlyphMask};

/// Transparent border kept around each glyph so bilinear sampling never
/// bleeds into a neighbour.
const PADDING: u32 = 1;

/// A rectangle in atlas pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AtlasRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl AtlasRect {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Where a glyph lives in the atlas and how to place it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AtlasEntry {
    /// The glyph's pixels (without padding). Empty for blank glyphs
    /// (spaces) and glyphs without outlines.
    pub rect: AtlasRect,
    /// Offset of the image's left edge from the pen x (physical px).
    pub left: i32,
    /// Offset of the image's top edge from the baseline (physical px).
    pub top: i32,
}

/// Pixels to upload: `rect` in the atlas and its tightly packed rows
/// (`rect.width` bytes per row, R8 coverage).
#[derive(Clone, PartialEq, Eq)]
pub struct DirtyRect {
    pub rect: AtlasRect,
    pub data: Vec<u8>,
}

impl fmt::Debug for DirtyRect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirtyRect")
            .field("rect", &self.rect)
            .finish_non_exhaustive()
    }
}

/// An 8-bit coverage atlas.
pub struct GlyphAtlas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    allocator: AtlasAllocator,
    entries: HashMap<GlyphKey, AtlasEntry>,
    dirty: Vec<AtlasRect>,
    generation: u64,
}

impl fmt::Debug for GlyphAtlas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GlyphAtlas")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("glyphs", &self.entries.len())
            .field("dirty", &self.dirty.len())
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl GlyphAtlas {
    /// A new, empty atlas. Sizes are clamped to 1..=8192.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        let width = width.clamp(1, 8192);
        let height = height.clamp(1, 8192);
        Self {
            width,
            height,
            pixels: vec![0; (width * height) as usize],
            allocator: AtlasAllocator::new(size2(width as i32, height as i32)),
            entries: HashMap::new(),
            dirty: Vec::new(),
            generation: 0,
        }
    }

    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// All atlas pixels, row-major, `width` bytes per row.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Bumped by [`GlyphAtlas::clear`]: when it changes, previously returned
    /// entries are invalid and the whole texture should be re-uploaded.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Number of cached glyphs (including blank ones).
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn get(&self, key: &GlyphKey) -> Option<AtlasEntry> {
        self.entries.get(key).copied()
    }

    /// Look up `key`, rasterizing with `rasterize` on a miss. A `None`
    /// from `rasterize` is cached as a blank entry.
    ///
    /// # Errors
    /// [`TextError::AtlasFull`] when there is no room (clear and retry),
    /// [`TextError::GlyphTooLarge`] when the glyph can never fit.
    pub fn get_or_insert_with(
        &mut self,
        key: GlyphKey,
        rasterize: impl FnOnce() -> Option<GlyphMask>,
    ) -> Result<AtlasEntry, TextError> {
        if let Some(entry) = self.entries.get(&key) {
            return Ok(*entry);
        }
        let entry = match rasterize() {
            Some(mask) => self.place(&mask)?,
            None => AtlasEntry::default(),
        };
        self.entries.insert(key, entry);
        Ok(entry)
    }

    /// Insert (or replace the cached entry for) a rasterized mask.
    ///
    /// # Errors
    /// As for [`GlyphAtlas::get_or_insert_with`].
    pub fn insert(&mut self, key: GlyphKey, mask: &GlyphMask) -> Result<AtlasEntry, TextError> {
        let entry = self.place(mask)?;
        self.entries.insert(key, entry);
        Ok(entry)
    }

    fn place(&mut self, mask: &GlyphMask) -> Result<AtlasEntry, TextError> {
        if mask.is_empty() {
            return Ok(AtlasEntry {
                rect: AtlasRect::default(),
                left: mask.left,
                top: mask.top,
            });
        }
        let (w, h) = (mask.width + 2 * PADDING, mask.height + 2 * PADDING);
        if w > self.width || h > self.height {
            return Err(TextError::GlyphTooLarge {
                width: mask.width,
                height: mask.height,
            });
        }
        let alloc = self
            .allocator
            .allocate(size2(w as i32, h as i32))
            .ok_or(TextError::AtlasFull)?;
        let rect = AtlasRect {
            x: alloc.rectangle.min.x as u32 + PADDING,
            y: alloc.rectangle.min.y as u32 + PADDING,
            width: mask.width,
            height: mask.height,
        };
        let stride = self.width as usize;
        for row in 0..mask.height as usize {
            let src = &mask.data[row * mask.width as usize..(row + 1) * mask.width as usize];
            let start = (rect.y as usize + row) * stride + rect.x as usize;
            self.pixels[start..start + src.len()].copy_from_slice(src);
        }
        self.dirty.push(rect);
        Ok(AtlasEntry {
            rect,
            left: mask.left,
            top: mask.top,
        })
    }

    /// Rectangles written since the last call, with their pixels. Padding
    /// is not included: it is zero from creation/clear, which the renderer
    /// covers with its full upload.
    pub fn take_dirty(&mut self) -> Vec<DirtyRect> {
        let stride = self.width as usize;
        std::mem::take(&mut self.dirty)
            .into_iter()
            .map(|rect| {
                let mut data = Vec::with_capacity((rect.width * rect.height) as usize);
                for row in 0..rect.height as usize {
                    let start = (rect.y as usize + row) * stride + rect.x as usize;
                    data.extend_from_slice(&self.pixels[start..start + rect.width as usize]);
                }
                DirtyRect { rect, data }
            })
            .collect()
    }

    /// Whether there are pending dirty rectangles.
    #[must_use]
    pub fn has_dirty(&self) -> bool {
        !self.dirty.is_empty()
    }

    /// Drop every glyph (e.g. on [`TextError::AtlasFull`] or a scale
    /// change). Bumps [`GlyphAtlas::generation`].
    pub fn clear(&mut self) {
        self.allocator.clear();
        self.entries.clear();
        self.dirty.clear();
        self.pixels.fill(0);
        self.generation += 1;
    }
}
