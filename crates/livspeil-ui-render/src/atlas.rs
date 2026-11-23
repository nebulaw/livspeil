// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! R8 coverage atlases (glyph masks): the upload interface between whoever
//! rasterizes glyphs (the paint bridge, fed by `livspeil-ui-text`) and the
//! backends. The renderer does not depend on the text stack: it only sees
//! [`AtlasUpdate`]s and texel rectangles in [`Glyphs`](crate::Glyphs).
//!
//! Protocol: an atlas is identified by `atlas` and versioned by
//! `(generation, size)`. An update whose version differs from the stored
//! one (or the first update for an atlas) replaces the atlas with a zeroed
//! texture of `size` before writing its region, so after a clear the
//! producer sends one full-size update and then dirty rectangles.

use std::collections::HashMap;

use crate::RenderError;

/// A write into an R8 atlas.
#[derive(Clone, PartialEq, Eq)]
pub struct AtlasUpdate {
    /// Atlas id (matches [`Glyphs::atlas`](crate::Glyphs::atlas)).
    pub atlas: u32,
    /// Producer's generation; a change reallocates the atlas.
    pub generation: u64,
    /// Atlas size `[width, height]` in texels; a change reallocates.
    pub size: [u32; 2],
    /// Region written, `[x, y, width, height]`.
    pub region: [u32; 4],
    /// `width * height` coverage bytes, row-major, tightly packed.
    pub data: Vec<u8>,
}

impl std::fmt::Debug for AtlasUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AtlasUpdate")
            .field("atlas", &self.atlas)
            .field("generation", &self.generation)
            .field("size", &self.size)
            .field("region", &self.region)
            .finish_non_exhaustive()
    }
}

/// Largest atlas side accepted.
pub const MAX_ATLAS_SIZE: u32 = 8192;

impl AtlasUpdate {
    /// Checks size, region and data length.
    pub fn validate(&self) -> Result<(), RenderError> {
        let [w, h] = self.size;
        if w == 0 || h == 0 || w > MAX_ATLAS_SIZE || h > MAX_ATLAS_SIZE {
            return Err(RenderError::Atlas("atlas size out of range"));
        }
        let [x, y, rw, rh] = self.region;
        if x.checked_add(rw).is_none_or(|r| r > w) || y.checked_add(rh).is_none_or(|b| b > h) {
            return Err(RenderError::Atlas("region outside the atlas"));
        }
        if self.data.len() != rw as usize * rh as usize {
            return Err(RenderError::Atlas("data length does not match the region"));
        }
        Ok(())
    }

    /// Whether this update reallocates an atlas currently at `version`.
    pub(crate) fn reallocates(&self, version: Option<(u64, [u32; 2])>) -> bool {
        version != Some((self.generation, self.size))
    }
}

/// One CPU-side R8 texture.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct R8Texture {
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl R8Texture {
    /// Texel at integer coordinates, 0 outside.
    pub fn texel(&self, x: i64, y: i64) -> u8 {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return 0;
        }
        self.data[y as usize * self.width as usize + x as usize]
    }
}

/// Atlases for the CPU reference rasterizer (see [`render_cpu_with`]).
///
/// [`render_cpu_with`]: crate::render_cpu_with
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AtlasStore {
    atlases: HashMap<u32, R8Texture>,
}

impl AtlasStore {
    /// No atlases.
    pub fn new() -> AtlasStore {
        AtlasStore::default()
    }

    /// Applies one update.
    pub fn apply(&mut self, update: &AtlasUpdate) -> Result<(), RenderError> {
        update.validate()?;
        let version = self
            .atlases
            .get(&update.atlas)
            .map(|t| (t.generation, [t.width, t.height]));
        if update.reallocates(version) {
            let [w, h] = update.size;
            self.atlases.insert(
                update.atlas,
                R8Texture {
                    generation: update.generation,
                    width: w,
                    height: h,
                    data: vec![0; w as usize * h as usize],
                },
            );
        }
        let Some(tex) = self.atlases.get_mut(&update.atlas) else {
            return Err(RenderError::Atlas("atlas missing after allocation"));
        };
        let [x, y, rw, _] = update.region;
        let stride = tex.width as usize;
        if rw > 0 {
            for (row, src) in update.data.chunks_exact(rw as usize).enumerate() {
                let start = (y as usize + row) * stride + x as usize;
                tex.data[start..start + src.len()].copy_from_slice(src);
            }
        }
        Ok(())
    }

    /// Applies several updates in order.
    pub fn apply_all<'a>(
        &mut self,
        updates: impl IntoIterator<Item = &'a AtlasUpdate>,
    ) -> Result<(), RenderError> {
        for u in updates {
            self.apply(u)?;
        }
        Ok(())
    }

    pub(crate) fn get(&self, atlas: u32) -> Option<&R8Texture> {
        self.atlases.get(&atlas)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(generation: u64, region: [u32; 4], fill: u8) -> AtlasUpdate {
        AtlasUpdate {
            atlas: 0,
            generation,
            size: [8, 4],
            region,
            data: vec![fill; (region[2] * region[3]) as usize],
        }
    }

    #[test]
    fn dirty_rects_patch_and_generations_reset() {
        let mut store = AtlasStore::new();
        store.apply(&update(0, [0, 0, 8, 4], 0)).unwrap();
        store.apply(&update(0, [2, 1, 2, 2], 200)).unwrap();
        let t = store.get(0).unwrap();
        assert_eq!(t.texel(2, 1), 200);
        assert_eq!(t.texel(3, 2), 200);
        assert_eq!(t.texel(4, 2), 0);
        assert_eq!(t.texel(-1, 0), 0);
        // New generation: everything else is cleared.
        store.apply(&update(1, [0, 0, 1, 1], 9)).unwrap();
        let t = store.get(0).unwrap();
        assert_eq!((t.texel(0, 0), t.texel(2, 1)), (9, 0));
    }

    #[test]
    fn bad_updates_are_typed_errors() {
        let mut store = AtlasStore::new();
        let mut u = update(0, [6, 0, 4, 1], 1);
        assert!(matches!(store.apply(&u), Err(RenderError::Atlas(_))));
        u.region = [0, 0, 4, 1];
        u.data.pop();
        assert!(matches!(store.apply(&u), Err(RenderError::Atlas(_))));
        u.size = [0, 4];
        assert!(matches!(store.apply(&u), Err(RenderError::Atlas(_))));
    }
}
