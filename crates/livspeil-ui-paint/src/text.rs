// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Text: a shaped-layout cache shared with the core's [`TextMeasurer`],
//! glyph and icon rasterization into one [`GlyphAtlas`], and atlas uploads.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use livspeil_ui_core::{FontSpec, TextMeasurer};
use livspeil_ui_icons::{IconId, IconRasterizer};
use livspeil_ui_layout::Size;
use livspeil_ui_render::{AtlasUpdate, GlyphSprite, Rect as RRect};
use livspeil_ui_text::{
    AtlasKey, AtlasRect, FamilySpec, FontSystem, GlyphAtlas, GlyphMask, GlyphQuad, GlyphRasterizer,
    LayoutOptions, LineHeight, SubpixelMode, TextError, TextLayout, TextStyle,
};

/// Layouts unused for this many frames are dropped.
const EVICT_AFTER_FRAMES: u64 = 240;

/// The parts of a [`TextStyle`] that change shaping (color is applied at
/// draw time and is not part of the key).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct StyleKey {
    family: (u8, String),
    size: u32,
    weight: u32,
    italic: bool,
    features: Vec<([u8; 4], u16)>,
    line_height: (u8, u32),
    letter_spacing: u32,
    underline: bool,
}

impl StyleKey {
    fn new(s: &TextStyle) -> StyleKey {
        let family = match &s.family {
            FamilySpec::Ui => (0, String::new()),
            FamilySpec::Named(n) => (1, n.clone()),
            FamilySpec::SansSerif => (2, String::new()),
            FamilySpec::Serif => (3, String::new()),
            FamilySpec::Monospace => (4, String::new()),
        };
        let line_height = match s.line_height {
            LineHeight::Metrics(f) => (0, f.to_bits()),
            LineHeight::FontSize(f) => (1, f.to_bits()),
            LineHeight::Absolute(f) => (2, f.to_bits()),
        };
        StyleKey {
            family,
            size: s.size.to_bits(),
            weight: s.weight.to_bits(),
            italic: s.italic,
            features: s.features.iter().map(|f| (f.tag, f.value)).collect(),
            line_height,
            letter_spacing: s.letter_spacing.to_bits(),
            underline: s.underline,
        }
    }
}

/// What prepared glyphs depend on: the scale and the atlas generation they
/// live in (runs snap to whole pixels, so the origin does not matter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlaceKey {
    scale: u32,
    generation: u64,
}

#[derive(Debug)]
struct Entry {
    text: String,
    style: StyleKey,
    max_width: Option<u32>,
    layout: TextLayout,
    last_used: u64,
    prepared: Option<(PlaceKey, Vec<GlyphQuad>)>,
}

/// Cache counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextCacheStats {
    /// Layouts in the cache.
    pub layouts: usize,
    /// Layout lookups that hit since the bridge was created.
    pub hits: u64,
    /// Layouts shaped since the bridge was created.
    pub shaped: u64,
    /// Glyph placements reused (no atlas lookups) since creation.
    pub placements_reused: u64,
    /// Glyphs and icons in the atlas.
    pub atlas_glyphs: usize,
}

/// Shared shaping state: fonts and the layout cache.
#[derive(Debug)]
pub(crate) struct Shaper {
    fonts: FontSystem,
    entries: HashMap<u64, Vec<Entry>>,
    frame: u64,
    stats: TextCacheStats,
}

fn hash_key(text: &str, style: &StyleKey, max_width: Option<u32>) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    text.hash(&mut h);
    style.hash(&mut h);
    max_width.hash(&mut h);
    h.finish()
}

impl Shaper {
    pub(crate) fn new(fonts: FontSystem) -> Shaper {
        Shaper {
            fonts,
            entries: HashMap::new(),
            frame: 0,
            stats: TextCacheStats::default(),
        }
    }

    /// The cached layout for `text` in `style` (wrapped at `max_width`),
    /// shaping it on a miss. Returns `(hash, index)` of the entry.
    fn lookup(&mut self, text: &str, style: &TextStyle, max_width: Option<f32>) -> (u64, usize) {
        let key = StyleKey::new(style);
        let mw = max_width.map(f32::to_bits);
        let h = hash_key(text, &key, mw);
        let frame = self.frame;
        let bucket = self.entries.entry(h).or_default();
        if let Some(i) = bucket
            .iter()
            .position(|e| e.max_width == mw && e.text == text && e.style == key)
        {
            bucket[i].last_used = frame;
            self.stats.hits += 1;
            return (h, i);
        }
        let options = LayoutOptions {
            max_width,
            ..LayoutOptions::default()
        };
        let layout = TextLayout::new(&mut self.fonts, text, style, options);
        bucket.push(Entry {
            text: text.to_owned(),
            style: key,
            max_width: mw,
            layout,
            last_used: frame,
            prepared: None,
        });
        self.stats.shaped += 1;
        (h, bucket.len() - 1)
    }

    fn entry(&mut self, at: (u64, usize)) -> Option<&mut Entry> {
        self.entries.get_mut(&at.0).and_then(|b| b.get_mut(at.1))
    }

    fn measure_style(&mut self, text: &str, style: &TextStyle) -> Size {
        let at = self.lookup(text, style, None);
        self.entry(at).map_or(Size::new(0.0, 0.0), |e| {
            Size::new(e.layout.width(), e.layout.height())
        })
    }

    fn end_frame(&mut self) {
        self.frame += 1;
        if self.frame.is_multiple_of(60) {
            let frame = self.frame;
            self.entries.retain(|_, bucket| {
                bucket.retain(|e| e.last_used + EVICT_AFTER_FRAMES >= frame);
                !bucket.is_empty()
            });
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
    }
}

/// The core's [`TextMeasurer`], backed by the bridge's layout cache (a
/// label measured for layout is not shaped again when it is painted).
#[derive(Debug, Clone)]
pub struct BridgeMeasurer {
    shaper: Rc<RefCell<Shaper>>,
}

impl TextMeasurer for BridgeMeasurer {
    fn measure(&self, text: &str, font: &FontSpec) -> Size {
        // A measurer re-entered while the bridge is painting (it is not)
        // would report an empty size rather than panic.
        match self.shaper.try_borrow_mut() {
            Ok(mut s) => s.measure_style(text, &text_style(font)),
            Err(_) => Size::new(0.0, 0.0),
        }
    }

    // The painter's own line breaker, so a box is as tall as what is drawn.
    fn wrapped_height(&self, text: &str, font: &FontSpec, width: f32) -> f32 {
        match self.shaper.try_borrow_mut() {
            Ok(mut s) => {
                let at = s.lookup(text, &text_style(font), Some(width));
                s.entry(at).map_or(0.0, |e| e.layout.height())
            }
            Err(_) => 0.0,
        }
    }
}

/// The `livspeil-ui-text` style of a core font (UI family).
#[must_use]
pub fn text_style(font: &FontSpec) -> TextStyle {
    let mut style = TextStyle::sized(font.size).with_weight(font.weight);
    for f in &font.features {
        style = style.with_feature(livspeil_ui_text::FontFeature {
            tag: f.tag,
            value: f.value,
        });
    }
    style
}

/// Text placement inputs for [`TextContext::glyphs`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct Placement {
    /// Layout top-left, local logical pixels.
    pub origin: (f32, f32),
    /// Wrap width.
    pub max_width: Option<f32>,
    /// Physical pixels per logical pixel (times the transform's scale).
    pub scale: f32,
}

/// Fonts, layout cache, rasterizers and the atlas of glyphs and icons.
#[derive(Debug)]
pub struct TextContext {
    /// Logical pixels added to the width of every text run that has one
    /// before deciding whether it wraps (0 by default). Layout rounds box
    /// sizes to whole pixels, so a label measured at 65.8 px can get a
    /// 65 px box and wrap; hosts set 1.0 to absorb the rounding without
    /// copying the scene.
    pub width_slack: f32,
    pub(crate) shaper: Rc<RefCell<Shaper>>,
    atlas: GlyphAtlas,
    atlas_id: u32,
    uploaded: Option<u64>,
    whole: GlyphRasterizer,
    quarter: GlyphRasterizer,
    icons: IconRasterizer,
}

/// Default atlas side in texels.
pub const DEFAULT_ATLAS_SIZE: u32 = 1024;

impl TextContext {
    /// A context over `fonts` with a 1024x1024 atlas (id 0).
    pub fn new(fonts: FontSystem) -> TextContext {
        TextContext::with_atlas(fonts, 0, DEFAULT_ATLAS_SIZE)
    }

    /// A context with atlas id `atlas_id` of `size` x `size` texels.
    pub fn with_atlas(fonts: FontSystem, atlas_id: u32, size: u32) -> TextContext {
        TextContext {
            width_slack: 0.0,
            shaper: Rc::new(RefCell::new(Shaper::new(fonts))),
            atlas: GlyphAtlas::new(size, size),
            atlas_id,
            uploaded: None,
            whole: GlyphRasterizer::new(SubpixelMode::None),
            quarter: GlyphRasterizer::new(SubpixelMode::Quarter),
            icons: IconRasterizer::new(),
        }
    }

    /// A measurer for [`App::set_text_measurer`](livspeil_ui_core::App::set_text_measurer)
    /// that shares this context's fonts and layout cache.
    pub fn measurer(&self) -> Rc<dyn TextMeasurer> {
        Rc::new(BridgeMeasurer {
            shaper: Rc::clone(&self.shaper),
        })
    }

    /// Size of `text` in `style` (unwrapped), logical pixels.
    pub fn measure(&self, text: &str, style: &TextStyle) -> Size {
        match self.shaper.try_borrow_mut() {
            Ok(mut s) => s.measure_style(text, style),
            Err(_) => Size::new(0.0, 0.0),
        }
    }

    /// The atlas id glyphs reference.
    pub fn atlas_id(&self) -> u32 {
        self.atlas_id
    }

    /// Cache counters.
    pub fn stats(&self) -> TextCacheStats {
        let mut s = self
            .shaper
            .try_borrow()
            .map(|s| {
                let mut st = s.stats;
                st.layouts = s.entries.values().map(Vec::len).sum();
                st
            })
            .unwrap_or_default();
        s.atlas_glyphs = self.atlas.len();
        s
    }

    /// Drops every cached layout (the atlas keeps its glyphs).
    pub fn clear_layouts(&mut self) {
        if let Ok(mut s) = self.shaper.try_borrow_mut() {
            s.clear();
        }
    }

    /// Drops every glyph and icon; the next upload is a full one.
    pub fn clear_atlas(&mut self) {
        self.atlas.clear();
    }

    pub(crate) fn end_frame(&mut self) {
        if let Ok(mut s) = self.shaper.try_borrow_mut() {
            s.end_frame();
        }
    }

    /// Layout width of `text` unwrapped, to decide on wrapping.
    pub(crate) fn natural_width(&mut self, text: &str, style: &TextStyle) -> f32 {
        self.measure(text, style).width
    }

    /// Shapes (cached), rasterizes (cached in the atlas) and places
    /// `text`, returning sprites in local logical coordinates.
    pub(crate) fn glyphs(
        &mut self,
        text: &str,
        style: &TextStyle,
        at: Placement,
    ) -> Result<Vec<GlyphSprite>, TextError> {
        // The shaper is only shared with the measurer, which never holds
        // its borrow across calls; draw nothing rather than panic.
        let Ok(mut shaper) = self.shaper.try_borrow_mut() else {
            return Ok(Vec::new());
        };
        let key = shaper.lookup(text, style, at.max_width);
        let scale = at.scale;
        let (px, py) = (at.origin.0 * scale, at.origin.1 * scale);
        // The whole run snaps to the pixel grid once. Snapping glyph by
        // glyph from a fractional origin would change letter spacing and
        // line positions while the text moves (zoomed or dragged cards).
        let (ix, iy) = ((px + 0.5).floor(), (py + 0.5).floor());
        let place = PlaceKey {
            scale: scale.to_bits(),
            generation: self.atlas.generation(),
        };
        let mode = SubpixelMode::for_scale(scale);
        let rasterizer = match mode {
            SubpixelMode::None => &mut self.whole,
            SubpixelMode::Quarter => &mut self.quarter,
        };
        let atlas = &mut self.atlas;
        let mut reused = false;
        let quads = {
            let Some(entry) = shaper.entry(key) else {
                return Ok(Vec::new());
            };
            match &entry.prepared {
                Some((k, q)) if *k == place => {
                    reused = true;
                    q.clone()
                }
                _ => {
                    let q = entry
                        .layout
                        .prepare_glyphs((0.0, 0.0), scale, rasterizer, atlas)?;
                    entry.prepared = Some((place, q.clone()));
                    q
                }
            }
        };
        if reused {
            shaper.stats.placements_reused += 1;
        }
        let inv = 1.0 / scale;
        Ok(quads
            .iter()
            .map(|q| GlyphSprite {
                rect: RRect::new(
                    (ix + q.x as f32) * inv,
                    (iy + q.y as f32) * inv,
                    q.atlas.width as f32 * inv,
                    q.atlas.height as f32 * inv,
                ),
                texels: [q.atlas.x, q.atlas.y, q.atlas.width, q.atlas.height],
            })
            .collect())
    }

    /// Where `icon` rasterized at `size_px` physical pixels is in the
    /// atlas, rasterizing it on the first use.
    pub(crate) fn icon(&mut self, icon: IconId, size_px: f32) -> Result<AtlasRect, TextError> {
        let key = AtlasKey::Icon {
            icon: u32::from(icon.index()),
            size_bits: size_px.to_bits(),
        };
        let raster = &mut self.icons;
        let entry = self.atlas.get_or_insert_with(key, || {
            // Cannot fail for the embedded set (tested in its crate).
            let mask = raster.rasterize(icon, size_px).ok()?;
            Some(GlyphMask {
                width: mask.side,
                height: mask.side,
                left: 0,
                top: 0,
                data: mask.data,
            })
        })?;
        Ok(entry.rect)
    }

    /// Atlas uploads since the last call: everything after a clear (or the
    /// first time), dirty rectangles otherwise.
    pub fn take_atlas_updates(&mut self) -> Vec<AtlasUpdate> {
        let generation = self.atlas.generation();
        let (w, h) = self.atlas.size();
        let size = [w, h];
        if self.uploaded != Some(generation) {
            self.uploaded = Some(generation);
            let _ = self.atlas.take_dirty();
            return vec![AtlasUpdate {
                atlas: self.atlas_id,
                generation,
                size,
                region: [0, 0, w, h],
                data: self.atlas.pixels().to_vec(),
            }];
        }
        self.atlas
            .take_dirty()
            .into_iter()
            .map(|d| AtlasUpdate {
                atlas: self.atlas_id,
                generation,
                size,
                region: [d.rect.x, d.rect.y, d.rect.width, d.rect.height],
                data: d.data,
            })
            .collect()
    }
}
