// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Livspeil UI text stack (UI spec §4), built on [Parley].
//!
//! - [`FontSystem`]: font collection (vendored Inter + Georgian/Arabic
//!   fallbacks, optionally system fonts) and layout scratch space. Use
//!   [`FontSystem::test`] for deterministic output: only bundled fonts.
//! - [`TextStyle`] and [`StyleSpan`]: what text looks like.
//! - [`TextLayout`]: shaped, broken and aligned text with metrics, glyph
//!   runs, hit testing, caret and selection geometry.
//! - [`Editor`]: a text editing model (grapheme/word movement, selection,
//!   IME preedit, undo/redo and an edit-op stream). It is independent of
//!   layout; visual (bidi-aware) movement takes a [`TextLayout`].
//! - [`GlyphRasterizer`] and [`GlyphAtlas`]: grayscale glyph masks with
//!   optional quarter-pixel horizontal positioning, packed into a CPU-side
//!   R8 atlas that reports dirty rectangles for upload. No GPU types here:
//!   the renderer (`livspeil-ui-render`) consumes [`GlyphQuad`]s and
//!   [`DirtyRect`]s.
//!
//! The choice of Parley over cosmic-text is recorded in
//! `docs/decisions/ui-text-stack.md`.
//!
//! [Parley]: https://github.com/linebender/parley

mod atlas;
mod editor;
mod error;
mod fonts;
mod layout;
mod raster;
mod style;

pub use atlas::{AtlasEntry, AtlasRect, DirtyRect, GlyphAtlas};
pub use editor::{Direction, EditKind, EditOp, Editor, Preedit, Selection};
pub use error::TextError;
pub use fonts::{FontSystem, UI_FAMILY};
pub use layout::{
    Affinity, Alignment, GlyphQuad, GlyphRun, LayoutOptions, LineInfo, PositionedGlyph, Rect,
    TextCursor, TextLayout,
};
pub use raster::{GlyphKey, GlyphMask, GlyphRasterizer, SubpixelMode};
pub use style::{ColorRole, FamilySpec, FontFeature, LineHeight, StyleSpan, TextStyle};
