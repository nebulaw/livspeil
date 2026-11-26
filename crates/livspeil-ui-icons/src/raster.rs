// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Icons to grayscale coverage masks (zeno).

use std::fmt;

use zeno::{Mask, Scratch, Transform};

use crate::registry::IconId;
use crate::svg::{SvgError, VIEW_BOX, parse_svg};

/// Largest size an icon is rasterized at, physical pixels.
pub const MAX_ICON_PX: f32 = 512.0;

/// A square coverage mask of an icon.
#[derive(Clone, PartialEq, Eq)]
pub struct IconMask {
    /// Edge length in pixels: the requested size rounded up.
    pub side: u32,
    /// `side * side` coverage bytes, row-major, 0 = transparent.
    pub data: Vec<u8>,
}

impl fmt::Debug for IconMask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IconMask")
            .field("side", &self.side)
            .finish_non_exhaustive()
    }
}

/// Reusable rasterizer state.
#[derive(Default)]
pub struct IconRasterizer {
    scratch: Scratch,
}

impl fmt::Debug for IconRasterizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IconRasterizer").finish_non_exhaustive()
    }
}

impl IconRasterizer {
    /// A rasterizer.
    #[must_use]
    pub fn new() -> IconRasterizer {
        IconRasterizer::default()
    }

    /// `icon` with its view box scaled to `size_px` physical pixels (clamped
    /// to 1..=[`MAX_ICON_PX`]), its top left at the mask's top left.
    ///
    /// # Errors
    /// [`SvgError`] if the source is outside the supported subset (the
    /// crate's tests rule that out for the embedded set).
    pub fn rasterize(&mut self, icon: IconId, size_px: f32) -> Result<IconMask, SvgError> {
        let size = if size_px.is_finite() {
            size_px.clamp(1.0, MAX_ICON_PX)
        } else {
            1.0
        };
        let side = size.ceil() as u32;
        let scale = size / VIEW_BOX;
        let mut data = vec![0u8; (side * side) as usize];
        for shape in parse_svg(icon.svg())? {
            let (layer, _) = Mask::with_scratch(&shape, &mut self.scratch)
                .transform(Some(Transform::scale(scale, scale)))
                .size(side, side)
                .render();
            // Source over: shapes of one colour accumulate coverage.
            for (under, over) in data.iter_mut().zip(layer) {
                let rest = (u16::from(*under) * u16::from(255 - over) + 127) / 255;
                *under = over.saturating_add(rest as u8);
            }
        }
        Ok(IconMask { side, data })
    }
}
