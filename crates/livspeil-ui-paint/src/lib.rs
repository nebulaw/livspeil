// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Paint bridge: turns a `livspeil-ui-core` [`Scene`] into a
//! `livspeil-ui-render` [`DisplayList`] plus glyph atlas uploads.
//!
//! ```text
//! App::draw_frame -> Scene --PaintBridge::paint_scene--> DisplayList + Vec<AtlasUpdate>
//!                                                          |              |
//!                                   encode -> Renderer::render   Renderer::update_atlas
//! ```
//!
//! - **API**: a conversion function ([`PaintBridge::paint_scene`]) rather
//!   than the core's `Painter` trait, because the host needs the outputs
//!   (display list, atlas updates) and errors, and the scale factor is an
//!   input. [`BridgePainter`] adapts it to `Painter` for code that wants
//!   the trait.
//! - **Quads**: radii, solid and linear-gradient fills, inner borders
//!   (width <= 1 logical px is a one-physical-pixel hairline), two-layer
//!   elevation shadows. Materials come from the core already resolved for
//!   the theme's tier (`livspeil-ui-theme`): a recipe with blur (Frost,
//!   Glass at T2/T1) becomes a [`Backdrop`](livspeil_ui_render::Backdrop)
//!   with tint, saturation, highlight or top rim, shadows and grain, plus a
//!   hairline quad; without blur (Solid, T0, reduce transparency) it is a
//!   plain quad with the opaque tint; Ink draws no surface.
//! - **Stacks**: transforms compose, opacity multiplies, clips intersect
//!   (as screen-space rectangles: rounded clips clip to their bounds).
//! - **Text**: shaped with `livspeil-ui-text` and cached by text, style
//!   and wrap width (unchanged labels are neither reshaped nor
//!   re-rasterized; glyph placements are reused while the origin's
//!   subpixel phase, scale and atlas generation match); unused layouts are
//!   evicted after ~240 frames. Glyphs are rasterized into one
//!   [`GlyphAtlas`](livspeil_ui_text::GlyphAtlas) (whole-pixel positions
//!   below 1.5x, quarter-pixel above). A full atlas is cleared and the
//!   scene painted again; the next upload is then a full one.
//! - **Layout**: [`TextContext::measurer`] is a core
//!   [`TextMeasurer`](livspeil_ui_core::TextMeasurer) sharing the same
//!   cache, so labels measured for layout are painted without reshaping.

#![forbid(unsafe_code)]

mod scene;
mod text;

use std::fmt;

use livspeil_ui_core::{Painter, Scene};
use livspeil_ui_render::{Affine, AtlasUpdate, Color, DisplayList, Rect};
pub use livspeil_ui_text::{FontSystem, TextError, TextStyle};

pub use text::{BridgeMeasurer, DEFAULT_ATLAS_SIZE, TextCacheStats, TextContext, text_style};

/// Errors from painting.
#[derive(Debug)]
pub enum PaintError {
    /// Push/pop commands do not nest.
    UnbalancedStack,
    /// Shaping or glyph atlas error (a full atlas is handled internally;
    /// this is reported when a scene does not fit even an empty atlas).
    Text(TextError),
}

impl fmt::Display for PaintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PaintError::UnbalancedStack => write!(f, "push/pop paint commands do not nest"),
            PaintError::Text(e) => write!(f, "text: {e}"),
        }
    }
}

impl std::error::Error for PaintError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PaintError::Text(e) => Some(e),
            PaintError::UnbalancedStack => None,
        }
    }
}

/// Converts scenes; owns the text context (fonts, layout cache, atlas).
#[derive(Debug)]
pub struct PaintBridge {
    /// Text state; use [`TextContext::measurer`] for the core's layout.
    pub text: TextContext,
}

/// Text drawn directly (not from a core scene), e.g. overlays a host
/// paints itself.
#[derive(Debug, Clone)]
pub struct TextItem<'a> {
    /// The text.
    pub text: &'a str,
    /// Its style (the color role is ignored; see `color`).
    pub style: &'a TextStyle,
    /// Layout top-left, local logical pixels.
    pub origin: (f32, f32),
    /// Wrap width.
    pub max_width: Option<f32>,
    /// Color.
    pub color: Color,
    /// Clip in logical screen coordinates.
    pub clip: Option<Rect>,
    /// Opacity.
    pub opacity: f32,
    /// Local-to-screen transform.
    pub transform: Affine,
}

impl<'a> TextItem<'a> {
    /// Text at `origin` with no wrap, clip or transform.
    pub fn new(text: &'a str, style: &'a TextStyle, origin: (f32, f32), color: Color) -> Self {
        TextItem {
            text,
            style,
            origin,
            max_width: None,
            color,
            clip: None,
            opacity: 1.0,
            transform: Affine::IDENTITY,
        }
    }
}

impl PaintBridge {
    /// A bridge over `fonts` (use [`FontSystem::test`] for deterministic
    /// output) with a 1024x1024 atlas.
    pub fn new(fonts: FontSystem) -> PaintBridge {
        PaintBridge {
            text: TextContext::new(fonts),
        }
    }

    /// Appends `scene` to `list` (items on layer 0) for a target at
    /// `scale` physical pixels per logical pixel and returns the atlas
    /// uploads to apply before rendering the list.
    pub fn paint_scene(
        &mut self,
        scene: &Scene,
        scale: f32,
        list: &mut DisplayList,
    ) -> Result<Vec<AtlasUpdate>, PaintError> {
        self.paint_scene_on(scene, scale, 0, list)
    }

    /// As [`PaintBridge::paint_scene`], on `layer`.
    pub fn paint_scene_on(
        &mut self,
        scene: &Scene,
        scale: f32,
        layer: i32,
        list: &mut DisplayList,
    ) -> Result<Vec<AtlasUpdate>, PaintError> {
        let start = list.items.len();
        let mut result = scene::convert(scene, &mut self.text, scale, layer, list);
        if result.as_ref().is_err_and(scene::is_atlas_full) {
            // Start over with an empty atlas: every glyph of the scene is
            // rasterized again and uploaded in full.
            list.items.truncate(start);
            self.text.clear_atlas();
            result = scene::convert(scene, &mut self.text, scale, layer, list);
        }
        self.text.end_frame();
        result?;
        Ok(self.text.take_atlas_updates())
    }

    /// Appends one text item at `scale` on `layer`. Atlas uploads are
    /// collected with [`TextContext::take_atlas_updates`] (or by the next
    /// [`PaintBridge::paint_scene`]).
    pub fn draw_text(
        &mut self,
        item: &TextItem<'_>,
        scale: f32,
        layer: i32,
        list: &mut DisplayList,
    ) -> Result<(), PaintError> {
        let state = scene::State {
            transform: item.transform,
            opacity: item.opacity,
            clip: item.clip,
        };
        scene::draw_text(
            &mut self.text,
            item.text,
            item.style,
            scene::TextDraw {
                origin: item.origin,
                max_width: item.max_width,
                color: item.color,
                scale,
                layer,
            },
            &state,
            list,
        )
    }
}

/// A core [`Painter`] that converts each scene with a [`PaintBridge`] and
/// keeps the latest output.
#[derive(Debug)]
pub struct BridgePainter {
    /// The bridge.
    pub bridge: PaintBridge,
    /// Scale factor for conversion.
    pub scale: f32,
    /// The latest display list.
    pub list: DisplayList,
    /// Atlas uploads not yet taken by the host (accumulated).
    pub atlas_updates: Vec<AtlasUpdate>,
    /// The latest error, if the last scene failed.
    pub error: Option<PaintError>,
}

impl BridgePainter {
    /// A painter at `scale`.
    pub fn new(bridge: PaintBridge, scale: f32) -> BridgePainter {
        BridgePainter {
            bridge,
            scale,
            list: DisplayList::new(),
            atlas_updates: Vec::new(),
            error: None,
        }
    }
}

impl Painter for BridgePainter {
    fn paint(&mut self, scene: &Scene) {
        self.list.items.clear();
        match self.bridge.paint_scene(scene, self.scale, &mut self.list) {
            Ok(updates) => {
                self.atlas_updates.extend(updates);
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }
}
