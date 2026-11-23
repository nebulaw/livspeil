// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Shaped, line-broken, aligned text.
//!
//! Coordinates are logical pixels with the origin at the layout's top-left
//! and y pointing down. Physical pixels appear only in
//! [`TextLayout::prepare_glyphs`].

use std::fmt;
use std::fmt::Write as _;
use std::ops::Range;

use parley::{AlignmentOptions, PositionedLayoutItem};
use skrifa::MetadataProvider;

use crate::atlas::{AtlasRect, GlyphAtlas};
use crate::error::TextError;
use crate::fonts::FontSystem;
use crate::raster::{GlyphKey, GlyphRasterizer};
use crate::style::{ColorRole, StyleSpan, TextStyle};

/// Horizontal alignment of lines within the layout width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Alignment {
    /// Left in LTR paragraphs, right in RTL ones.
    #[default]
    Start,
    End,
    Left,
    Center,
    Right,
    Justify,
}

impl Alignment {
    fn to_parley(self) -> parley::Alignment {
        match self {
            Alignment::Start => parley::Alignment::Start,
            Alignment::End => parley::Alignment::End,
            Alignment::Left => parley::Alignment::Left,
            Alignment::Center => parley::Alignment::Center,
            Alignment::Right => parley::Alignment::Right,
            Alignment::Justify => parley::Alignment::Justify,
        }
    }
}

/// Line breaking and alignment parameters.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutOptions {
    /// Wrap width; `None` breaks only at hard line breaks.
    pub max_width: Option<f32>,
    pub alignment: Alignment,
}

impl LayoutOptions {
    #[must_use]
    pub fn wrap(max_width: f32) -> Self {
        Self {
            max_width: Some(max_width),
            alignment: Alignment::Start,
        }
    }

    #[must_use]
    pub fn aligned(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }
}

/// An axis-aligned rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    fn from_bbox(b: parley::BoundingBox) -> Self {
        Self {
            x: b.x0 as f32,
            y: b.y0 as f32,
            width: (b.x1 - b.x0) as f32,
            height: (b.y1 - b.y0) as f32,
        }
    }

    #[must_use]
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }
}

/// Which side of a line or direction boundary a cursor sticks to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Affinity {
    /// Associated with the character after the index (the usual case).
    #[default]
    Downstream,
    /// Associated with the character before the index (e.g. end of a
    /// soft-wrapped line).
    Upstream,
}

impl Affinity {
    fn to_parley(self) -> parley::Affinity {
        match self {
            Affinity::Downstream => parley::Affinity::Downstream,
            Affinity::Upstream => parley::Affinity::Upstream,
        }
    }

    fn from_parley(a: parley::Affinity) -> Self {
        match a {
            parley::Affinity::Downstream => Affinity::Downstream,
            parley::Affinity::Upstream => Affinity::Upstream,
        }
    }
}

/// A caret position: a byte offset on a grapheme cluster boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextCursor {
    pub index: usize,
    pub affinity: Affinity,
}

impl TextCursor {
    #[must_use]
    pub fn new(index: usize) -> Self {
        Self {
            index,
            affinity: Affinity::Downstream,
        }
    }
}

/// Metrics of one visual line.
#[derive(Clone, Debug, PartialEq)]
pub struct LineInfo {
    pub text_range: Range<usize>,
    /// Top of the line box.
    pub top: f32,
    pub height: f32,
    /// Baseline y.
    pub baseline: f32,
    /// x of the line's first glyph after alignment.
    pub x: f32,
    /// Advance of the line's content (without hanging trailing whitespace).
    pub width: f32,
}

/// One glyph, positioned in layout coordinates (pen position on the
/// baseline).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedGlyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub advance: f32,
}

/// A run of glyphs sharing font, size, variation and style, in visual
/// order within its line.
#[derive(Clone)]
pub struct GlyphRun {
    pub line: usize,
    pub text_range: Range<usize>,
    pub rtl: bool,
    /// Whether the face is italic/oblique (the Inter roman and italic
    /// files share one family name).
    pub italic: bool,
    pub font_size: f32,
    /// Normalized variation coordinates (F2Dot14 bits) of the run's face.
    pub coords: Vec<i16>,
    pub color: ColorRole,
    /// Underline as (offset below baseline, thickness), if any.
    pub underline: Option<(f32, f32)>,
    pub baseline: f32,
    pub glyphs: Vec<PositionedGlyph>,
    font: parley::FontData,
}

impl fmt::Debug for GlyphRun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GlyphRun")
            .field("line", &self.line)
            .field("text_range", &self.text_range)
            .field("rtl", &self.rtl)
            .field("font", &self.font_family())
            .field("font_size", &self.font_size)
            .field("glyphs", &self.glyphs.len())
            .finish_non_exhaustive()
    }
}

impl GlyphRun {
    /// The face's family name (from its name table).
    #[must_use]
    pub fn font_family(&self) -> String {
        skrifa::FontRef::from_index(self.font.data.as_ref(), self.font.index)
            .ok()
            .and_then(|f| {
                f.localized_strings(skrifa::string::StringId::FAMILY_NAME)
                    .english_or_first()
                    .map(|s| s.to_string())
            })
            .unwrap_or_default()
    }

    /// Raw font bytes and face index, for custom rasterization.
    #[must_use]
    pub fn font_data(&self) -> (&[u8], u32) {
        (self.font.data.as_ref(), self.font.index)
    }

    fn font_id(&self) -> u64 {
        self.font.data.id()
    }
}

/// A textured quad for one glyph, in physical pixels. The renderer draws
/// `atlas` (coverage) at (`x`, `y`) tinted with the resolved `color`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphQuad {
    pub x: i32,
    pub y: i32,
    pub atlas: AtlasRect,
    pub color: ColorRole,
}

/// Shaped and laid-out text.
pub struct TextLayout {
    text: String,
    inner: parley::Layout<ColorRole>,
    options: LayoutOptions,
}

impl fmt::Debug for TextLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextLayout")
            .field("text", &self.text)
            .field("options", &self.options)
            .field("width", &self.inner.width())
            .field("height", &self.inner.height())
            .finish_non_exhaustive()
    }
}

fn check_offset(text: &str, offset: usize) -> Result<(), TextError> {
    if text.is_char_boundary(offset) {
        Ok(())
    } else {
        Err(TextError::InvalidOffset {
            offset,
            len: text.len(),
        })
    }
}

fn round64(v: f32) -> f64 {
    (f64::from(v) * 64.0).round() / 64.0
}

impl TextLayout {
    /// Shape and lay out plain text in one style.
    #[must_use]
    pub fn new(fs: &mut FontSystem, text: &str, style: &TextStyle, options: LayoutOptions) -> Self {
        Self::build(fs, text, style, &[], options)
    }

    /// Shape and lay out rich text: `spans` override `style` on byte
    /// ranges (later spans win where they overlap).
    ///
    /// # Errors
    /// [`TextError::InvalidOffset`] when a span boundary is out of range or
    /// inside a UTF-8 sequence.
    pub fn rich(
        fs: &mut FontSystem,
        text: &str,
        style: &TextStyle,
        spans: &[StyleSpan],
        options: LayoutOptions,
    ) -> Result<Self, TextError> {
        for span in spans {
            check_offset(text, span.range.start)?;
            check_offset(text, span.range.end)?;
        }
        Ok(Self::build(fs, text, style, spans, options))
    }

    fn build(
        fs: &mut FontSystem,
        text: &str,
        style: &TextStyle,
        spans: &[StyleSpan],
        options: LayoutOptions,
    ) -> Self {
        let mut b = fs.layout.ranged_builder(&mut fs.fonts, text, 1.0, true);
        style.push_defaults(&mut b);
        for span in spans {
            let range = span.range.start.min(text.len())..span.range.end.min(text.len());
            if !range.is_empty() {
                span.push(&mut b, range);
            }
        }
        let inner = b.build(text);
        let mut layout = Self {
            text: text.to_owned(),
            inner,
            options,
        };
        layout.relayout();
        layout
    }

    fn relayout(&mut self) {
        // Parley applies the alignment setting while breaking lines.
        self.inner.set_alignment(
            self.options.alignment.to_parley(),
            AlignmentOptions::default(),
        );
        self.inner.break_all_lines(self.options.max_width);
    }

    /// Re-break and re-align for a new width without reshaping.
    pub fn set_max_width(&mut self, max_width: Option<f32>) {
        if self.options.max_width != max_width {
            self.options.max_width = max_width;
            self.relayout();
        }
    }

    pub fn set_alignment(&mut self, alignment: Alignment) {
        if self.options.alignment != alignment {
            self.options.alignment = alignment;
            self.relayout();
        }
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn options(&self) -> LayoutOptions {
        self.options
    }

    /// Width of the widest line (excluding trailing whitespace).
    #[must_use]
    pub fn width(&self) -> f32 {
        self.inner.width()
    }

    #[must_use]
    pub fn height(&self) -> f32 {
        self.inner.height()
    }

    /// Whether the paragraph's base direction is right-to-left.
    #[must_use]
    pub fn is_rtl(&self) -> bool {
        self.inner.is_rtl()
    }

    #[must_use]
    pub fn line_count(&self) -> usize {
        self.inner.len()
    }

    /// Baseline of the first line (for aligning labels with controls).
    #[must_use]
    pub fn first_baseline(&self) -> Option<f32> {
        self.inner.lines().next().map(|l| l.metrics().baseline)
    }

    #[must_use]
    pub fn lines(&self) -> Vec<LineInfo> {
        self.inner
            .lines()
            .map(|line| {
                let m = line.metrics();
                LineInfo {
                    text_range: line.text_range(),
                    top: m.block_min_coord,
                    height: m.block_max_coord - m.block_min_coord,
                    baseline: m.baseline,
                    x: m.offset,
                    width: m.advance - m.hanging_advance,
                }
            })
            .collect()
    }

    /// All glyph runs, line by line in visual order.
    #[must_use]
    pub fn glyph_runs(&self) -> Vec<GlyphRun> {
        let mut out = Vec::new();
        for (line_index, line) in self.inner.lines().enumerate() {
            // A shaping run can be split into several glyph runs by style
            // changes. Walk its clusters in visual order alongside to find
            // each glyph run's own text range.
            let mut current_run: Option<usize> = None;
            let mut clusters = Vec::new();
            let mut next_cluster = 0;
            let mut cluster_x = 0.0;
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(gr) = item else {
                    continue;
                };
                let run = gr.run();
                if current_run != Some(run.index()) {
                    current_run = Some(run.index());
                    clusters = run
                        .visual_clusters()
                        .map(|c| (c.text_range(), c.advance()))
                        .collect();
                    next_cluster = 0;
                    cluster_x = gr.offset();
                }
                let end_x = gr.offset() + gr.advance();
                let mut text_range: Option<Range<usize>> = None;
                while let Some((range, advance)) = clusters.get(next_cluster) {
                    if cluster_x + advance * 0.5 > end_x + 1e-3 {
                        break;
                    }
                    text_range = Some(match text_range {
                        None => range.clone(),
                        Some(r) => r.start.min(range.start)..r.end.max(range.end),
                    });
                    cluster_x += advance;
                    next_cluster += 1;
                }
                let style = gr.style();
                let metrics = run.font_metrics();
                let underline = style.underline.as_ref().map(|u| {
                    (
                        u.offset.unwrap_or(-metrics.underline_offset),
                        u.size.unwrap_or(metrics.underline_size),
                    )
                });
                out.push(GlyphRun {
                    line: line_index,
                    text_range: text_range.unwrap_or_else(|| run.text_range()),
                    rtl: run.is_rtl(),
                    italic: run.font_attrs().style != parley::FontStyle::Normal,
                    font_size: run.font_size(),
                    coords: run
                        .normalized_coords()
                        .iter()
                        .map(|c| c.to_bits())
                        .collect(),
                    color: style.brush,
                    underline,
                    baseline: gr.baseline(),
                    glyphs: gr
                        .positioned_glyphs()
                        .map(|g| PositionedGlyph {
                            id: g.id,
                            x: g.x,
                            y: g.y,
                            advance: g.advance,
                        })
                        .collect(),
                    font: run.font().clone(),
                });
            }
        }
        out
    }

    fn parley_cursor(&self, cursor: TextCursor) -> parley::Cursor {
        parley::Cursor::from_byte_index(
            &self.inner,
            cursor.index.min(self.text.len()),
            cursor.affinity.to_parley(),
        )
    }

    fn from_parley(c: parley::Cursor) -> TextCursor {
        TextCursor {
            index: c.index(),
            affinity: Affinity::from_parley(c.affinity()),
        }
    }

    /// The caret position closest to a point (clamped into the layout).
    #[must_use]
    pub fn hit_test(&self, x: f32, y: f32) -> TextCursor {
        Self::from_parley(parley::Cursor::from_point(&self.inner, x, y))
    }

    /// Caret rectangle of `width` logical pixels for a cursor.
    #[must_use]
    pub fn cursor_rect(&self, cursor: TextCursor, width: f32) -> Rect {
        Rect::from_bbox(self.parley_cursor(cursor).geometry(&self.inner, width))
    }

    /// Highlight rectangles for a byte range (one or more per line; bidi
    /// text can produce several per line).
    #[must_use]
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let anchor = self.parley_cursor(TextCursor::new(range.start));
        let focus = self.parley_cursor(TextCursor {
            index: range.end,
            affinity: Affinity::Upstream,
        });
        parley::Selection::new(anchor, focus)
            .geometry(&self.inner)
            .into_iter()
            .map(|(b, _)| Rect::from_bbox(b))
            .collect()
    }

    /// Move visually left (`forward == false`) or right, by grapheme or by
    /// word; respects bidi ordering.
    #[must_use]
    pub fn move_visual(&self, cursor: TextCursor, right: bool, word: bool) -> TextCursor {
        let c = self.parley_cursor(cursor);
        let moved = match (right, word) {
            (true, false) => c.next_visual(&self.inner),
            (false, false) => c.previous_visual(&self.inner),
            (true, true) => c.next_visual_word(&self.inner),
            (false, true) => c.previous_visual_word(&self.inner),
        };
        Self::from_parley(moved)
    }

    /// Move `delta` visual lines up (negative) or down.
    #[must_use]
    pub fn move_lines(&self, cursor: TextCursor, delta: isize) -> TextCursor {
        let sel = parley::Selection::new(self.parley_cursor(cursor), self.parley_cursor(cursor));
        Self::from_parley(sel.move_lines(&self.inner, delta, false).focus())
    }

    /// Start or end of the visual line containing the cursor.
    #[must_use]
    pub fn line_edge(&self, cursor: TextCursor, end: bool) -> TextCursor {
        let c = self.parley_cursor(cursor);
        let sel = parley::Selection::new(c, c);
        let moved = if end {
            sel.line_end(&self.inner, false)
        } else {
            sel.line_start(&self.inner, false)
        };
        Self::from_parley(moved.focus())
    }

    /// Rasterize (as needed) and place every glyph. `origin` is the
    /// layout's top-left in logical pixels; `scale` converts to physical
    /// pixels.
    ///
    /// # Errors
    /// [`TextError::AtlasFull`] / [`TextError::GlyphTooLarge`] from the
    /// atlas. On `AtlasFull` clear the atlas and prepare the frame again.
    pub fn prepare_glyphs(
        &self,
        origin: (f32, f32),
        scale: f32,
        rasterizer: &mut GlyphRasterizer,
        atlas: &mut GlyphAtlas,
    ) -> Result<Vec<GlyphQuad>, TextError> {
        let mode = rasterizer.mode();
        let mut quads = Vec::new();
        for run in self.glyph_runs() {
            let size_px = run.font_size * scale;
            let coords_hash = GlyphKey::hash_coords(&run.coords);
            let (data, index) = run.font_data();
            for g in &run.glyphs {
                let (px, bin) = mode.quantize((origin.0 + g.x) * scale);
                let py = ((origin.1 + g.y) * scale).round() as i32;
                let key = GlyphKey {
                    font_id: run.font_id(),
                    font_index: index,
                    glyph_id: g.id,
                    size_bits: size_px.to_bits(),
                    coords_hash,
                    subpixel_x: bin,
                };
                let entry = atlas.get_or_insert_with(key, || {
                    rasterizer.rasterize(data, index, g.id, size_px, &run.coords, bin)
                })?;
                if entry.rect.is_empty() {
                    continue;
                }
                quads.push(GlyphQuad {
                    x: px + entry.left,
                    y: py + entry.top,
                    atlas: entry.rect,
                    color: run.color,
                });
            }
        }
        Ok(quads)
    }

    /// Text dump for golden tests: lines, runs, fonts and glyph ids with
    /// positions rounded to 1/64 px.
    #[must_use]
    pub fn dump(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "layout width={} height={} lines={} rtl={}",
            round64(self.width()),
            round64(self.height()),
            self.line_count(),
            self.is_rtl()
        );
        let runs = self.glyph_runs();
        for (i, line) in self.lines().iter().enumerate() {
            let _ = writeln!(
                out,
                "line {i} text={:?} {:?} top={} height={} baseline={} x={} width={}",
                line.text_range,
                &self.text[line.text_range.clone()],
                round64(line.top),
                round64(line.height),
                round64(line.baseline),
                round64(line.x),
                round64(line.width)
            );
            for run in runs.iter().filter(|r| r.line == i) {
                let _ = writeln!(
                    out,
                    "  run text={:?} {} font={:?}{} size={} coords={:?} color={:?}{}",
                    run.text_range,
                    if run.rtl { "rtl" } else { "ltr" },
                    run.font_family(),
                    if run.italic { " italic" } else { "" },
                    round64(run.font_size),
                    run.coords,
                    run.color,
                    if run.underline.is_some() {
                        " underline"
                    } else {
                        ""
                    }
                );
                for g in &run.glyphs {
                    let _ = writeln!(
                        out,
                        "    g{} x={} y={} adv={}",
                        g.id,
                        round64(g.x),
                        round64(g.y),
                        round64(g.advance)
                    );
                }
            }
        }
        out
    }
}
