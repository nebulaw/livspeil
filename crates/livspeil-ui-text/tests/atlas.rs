// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Glyph rasterization and atlas: determinism, caching, dirty rects,
//! errors, and a PNG preview (`target/text-preview.png`).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use livspeil_testkit::assert_golden;
use livspeil_ui_text::{
    Alignment, ColorRole, FontFeature, FontSystem, GlyphAtlas, GlyphKey, GlyphQuad,
    GlyphRasterizer, LayoutOptions, StyleSpan, SubpixelMode, TextCursor, TextError, TextLayout,
    TextStyle,
};

fn layout(fs: &mut FontSystem, text: &str, style: &TextStyle) -> TextLayout {
    TextLayout::new(fs, text, style, LayoutOptions::default())
}

fn inter_a(raster: &mut GlyphRasterizer, bin: u8) -> livspeil_ui_text::GlyphMask {
    let mut fs = FontSystem::test();
    let l = layout(&mut fs, "a", &TextStyle::sized(13.0));
    let run = &l.glyph_runs()[0];
    let (data, index) = run.font_data();
    raster
        .rasterize(data, index, run.glyphs[0].id, 26.0, &run.coords, bin)
        .expect("outline glyph")
}

#[test]
fn rasterization_is_deterministic_and_subpixel_bins_differ() {
    let mut r = GlyphRasterizer::new(SubpixelMode::Quarter);
    let a0 = inter_a(&mut r, 0);
    assert_eq!(a0, inter_a(&mut r, 0));
    assert!(!a0.is_empty());
    assert!(a0.top < 0, "glyph sits above the baseline");
    let a2 = inter_a(&mut r, 2);
    assert_ne!(a0.data, a2.data, "half-pixel shift changes coverage");
    let sum = |m: &livspeil_ui_text::GlyphMask| m.data.iter().map(|&v| u64::from(v)).sum::<u64>();
    // Same ink, just shifted (coverage is conserved up to rounding).
    let (s0, s2) = (sum(&a0), sum(&a2));
    assert!(s0.abs_diff(s2) * 50 < s0, "{s0} vs {s2}");
}

#[test]
fn quantize_bins() {
    assert_eq!(SubpixelMode::None.quantize(10.6), (11, 0));
    assert_eq!(SubpixelMode::Quarter.quantize(10.6), (10, 2));
    assert_eq!(SubpixelMode::Quarter.quantize(10.9), (11, 0));
    assert_eq!(SubpixelMode::Quarter.quantize(-0.3), (-1, 3));
    assert_eq!(SubpixelMode::for_scale(1.0), SubpixelMode::None);
    assert_eq!(SubpixelMode::for_scale(2.0), SubpixelMode::Quarter);
}

#[test]
fn atlas_caches_and_reports_dirty_rects() {
    let mut fs = FontSystem::test();
    let l = layout(&mut fs, "Hello, hello!", &TextStyle::sized(13.0));
    let mut raster = GlyphRasterizer::new(SubpixelMode::None);
    let mut atlas = GlyphAtlas::new(256, 256);
    let q1 = l
        .prepare_glyphs((0.0, 0.0), 1.0, &mut raster, &mut atlas)
        .expect("fits");
    // 12 visible glyphs (space is blank).
    assert_eq!(q1.len(), 12);
    let dirty = atlas.take_dirty();
    // Unique glyphs: H e l o , h ! (+ space cached as blank, no rect).
    assert_eq!(dirty.len(), 7);
    for d in &dirty {
        assert_eq!(d.data.len() as u32, d.rect.width * d.rect.height);
        let (w, _) = atlas.size();
        let row0 = &atlas.pixels()[(d.rect.y * w + d.rect.x) as usize..][..d.rect.width as usize];
        assert_eq!(row0, &d.data[..d.rect.width as usize]);
    }
    assert!(!atlas.has_dirty());
    let q2 = l
        .prepare_glyphs((0.0, 0.0), 1.0, &mut raster, &mut atlas)
        .expect("fits");
    assert_eq!(q1, q2);
    assert!(
        atlas.take_dirty().is_empty(),
        "second frame is fully cached"
    );
    assert_eq!(atlas.len(), 8);
    // Moving by whole pixels reuses glyphs and shifts quads.
    let q3 = l
        .prepare_glyphs((3.0, 1.0), 1.0, &mut raster, &mut atlas)
        .expect("fits");
    assert!(
        q3.iter()
            .zip(&q1)
            .all(|(a, b)| a.x == b.x + 3 && a.y == b.y + 1)
    );
    assert!(atlas.take_dirty().is_empty());
}

#[test]
fn atlas_full_and_too_large() {
    let mut fs = FontSystem::test();
    let l = layout(
        &mut fs,
        "abcdefghijklmnopqrstuvwxyz",
        &TextStyle::sized(40.0),
    );
    let mut raster = GlyphRasterizer::new(SubpixelMode::None);
    let mut atlas = GlyphAtlas::new(64, 64);
    let err = l.prepare_glyphs((0.0, 0.0), 1.0, &mut raster, &mut atlas);
    assert!(matches!(err, Err(TextError::AtlasFull)), "{err:?}");
    let generation = atlas.generation();
    atlas.clear();
    assert_eq!(atlas.generation(), generation + 1);
    assert!(atlas.is_empty());
    let mut tiny = GlyphAtlas::new(8, 8);
    let err = l.prepare_glyphs((0.0, 0.0), 1.0, &mut raster, &mut tiny);
    assert!(
        matches!(err, Err(TextError::GlyphTooLarge { .. })),
        "{err:?}"
    );
}

#[test]
fn glyph_masks_golden() {
    // Placement and ink of a few glyphs at 1x and 2x (quarter bins).
    let mut fs = FontSystem::test();
    let mut out = String::new();
    for (text, style) in [
        ("Ag0", TextStyle::sized(13.0)),
        ("Ag0", TextStyle::sized(13.0).with_weight(600.0)),
        ("ბ", TextStyle::sized(13.0)),
        ("ع", TextStyle::sized(13.0)),
    ] {
        let l = layout(&mut fs, text, &style);
        for run in l.glyph_runs() {
            let (data, index) = run.font_data();
            for g in &run.glyphs {
                for (scale, bin) in [(1.0f32, 0u8), (2.0, 0), (2.0, 1), (2.0, 3)] {
                    let mut r = GlyphRasterizer::new(SubpixelMode::for_scale(scale));
                    let m = r
                        .rasterize(data, index, g.id, run.font_size * scale, &run.coords, bin)
                        .expect("outline");
                    let ink: u64 = m.data.iter().map(|&v| u64::from(v)).sum();
                    let _ = writeln!(
                        out,
                        "{text} {} wght={} g{} @{scale}x bin{bin}: {}x{} left={} top={} ink={ink}",
                        run.font_family(),
                        style.weight,
                        g.id,
                        m.width,
                        m.height,
                        m.left,
                        m.top
                    );
                }
            }
        }
    }
    assert_golden(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden"),
        "glyph_masks",
        &out,
    );
}

#[test]
fn glyph_key_hash_is_stable() {
    assert_eq!(
        GlyphKey::hash_coords(&[0, 8848]),
        GlyphKey::hash_coords(&[0, 8848])
    );
    assert_ne!(
        GlyphKey::hash_coords(&[0, 8848]),
        GlyphKey::hash_coords(&[])
    );
}

// --------------------------------------------------------------- preview

fn role_rgb(role: ColorRole) -> [u8; 3] {
    match role {
        ColorRole::TextPrimary => [236, 236, 240],
        ColorRole::TextSecondary => [150, 150, 158],
        ColorRole::TextTertiary => [104, 104, 112],
        ColorRole::Accent => [96, 165, 250],
        ColorRole::Danger => [248, 113, 113],
        ColorRole::Warning => [251, 191, 36],
        ColorRole::Success => [74, 222, 128],
        ColorRole::Srgba(v) => [(v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8],
    }
}

struct Canvas {
    w: u32,
    h: u32,
    px: Vec<[u8; 3]>,
}

impl Canvas {
    fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: [u8; 3]) {
        for yy in y.max(0)..(y + h).min(self.h as i32) {
            for xx in x.max(0)..(x + w).min(self.w as i32) {
                self.px[(yy as u32 * self.w + xx as u32) as usize] = c;
            }
        }
    }

    /// Plain sRGB-space blend; the real renderer blends gamma-correctly.
    fn blend(&mut self, x: i32, y: i32, c: [u8; 3], a: u8) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let p = &mut self.px[(y as u32 * self.w + x as u32) as usize];
        for i in 0..3 {
            let v = u32::from(p[i]) * (255 - u32::from(a)) + u32::from(c[i]) * u32::from(a);
            p[i] = (v / 255) as u8;
        }
    }

    fn draw_quads(&mut self, quads: &[GlyphQuad], atlas: &GlyphAtlas) {
        let (aw, _) = atlas.size();
        for q in quads {
            let c = role_rgb(q.color);
            for row in 0..q.atlas.height {
                for col in 0..q.atlas.width {
                    let a = atlas.pixels()[((q.atlas.y + row) * aw + q.atlas.x + col) as usize];
                    if a > 0 {
                        self.blend(q.x + col as i32, q.y + row as i32, c, a);
                    }
                }
            }
        }
    }

    fn write_png(&self, path: &Path) {
        let file = std::fs::File::create(path).expect("create png");
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), self.w, self.h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().expect("png header");
        let data: Vec<u8> = self.px.iter().flatten().copied().collect();
        writer.write_image_data(&data).expect("png data");
    }
}

#[test]
fn preview_png() {
    let scale = 2.0;
    let mut fs = FontSystem::test();
    let mut raster = GlyphRasterizer::new(SubpixelMode::for_scale(scale));
    let mut atlas = GlyphAtlas::new(512, 512);
    let (lw, lh) = (440u32, 330u32); // logical text area
    let mut canvas = Canvas {
        w: (lw as f32 * scale) as u32 + 8 + atlas.size().0,
        h: ((lh as f32 * scale) as u32).max(atlas.size().1),
        px: Vec::new(),
    };
    canvas.px = vec![[28, 28, 31]; (canvas.w * canvas.h) as usize];

    let mut y = 12.0f32;
    let mut lines: Vec<(TextLayout, (f32, f32))> = Vec::new();
    for (size, weight) in [(28.0, 600.0), (17.0, 500.0), (13.0, 400.0)] {
        let style = TextStyle::sized(size).with_weight(weight);
        let l = layout(
            &mut fs,
            &format!("Livspeil UI — Inter {size}/{weight}"),
            &style,
        );
        let h = l.height();
        lines.push((l, (16.0, y)));
        y += h + 6.0;
    }
    let tnum = TextStyle::sized(13.0)
        .with_feature(FontFeature::TNUM)
        .with_color(ColorRole::TextSecondary);
    for t in ["Opacity   0.125  1111.00", "Radius   12.500  0004.75"] {
        let l = layout(&mut fs, t, &tnum);
        let h = l.height();
        lines.push((l, (16.0, y)));
        y += h + 2.0;
    }
    y += 6.0;
    let text = "გამარჯობა, თბილისი! Bold, italic, accent.";
    let span = |word: &str| {
        let start = text.find(word).expect("word in text");
        StyleSpan::new(start..start + word.len())
    };
    let rich = TextLayout::rich(
        &mut fs,
        text,
        &TextStyle::sized(15.0),
        &[
            span("Bold").weight(700.0),
            span("italic").italic(true),
            span("accent").color(ColorRole::Accent).underline(true),
        ],
        LayoutOptions::default(),
    )
    .expect("spans");
    let h = rich.height();
    lines.push((rich, (16.0, y)));
    y += h + 6.0;
    let bidi = layout(
        &mut fs,
        "Hello مرحبا بالعالم 123 world",
        &TextStyle::sized(15.0).with_color(ColorRole::Warning),
    );
    let bidi_origin = (16.0, y);
    let h = bidi.height();
    lines.push((bidi, bidi_origin));
    y += h + 6.0;
    let para = TextLayout::new(
        &mut fs,
        "A wrapped, centred paragraph: the quick brown fox jumps over the lazy dog; \
         სწრაფი ყავისფერი მელა.",
        &TextStyle::sized(13.0).with_color(ColorRole::TextSecondary),
        LayoutOptions::wrap(400.0).aligned(Alignment::Center),
    );
    lines.push((para, (16.0, y)));

    // Selection highlight and caret on the bidi line (logical range
    // "o مرح"), drawn under the text.
    let (bidi_layout, origin) = &lines[6];
    let px = |v: f32, o: f32| ((v + o) * scale).round() as i32;
    for r in bidi_layout.selection_rects(4.."Hello مرح".len()) {
        canvas.fill_rect(
            px(r.x, origin.0),
            px(r.y, origin.1),
            (r.width * scale).round() as i32,
            (r.height * scale).round() as i32,
            [44, 74, 128],
        );
    }
    let caret = bidi_layout.cursor_rect(TextCursor::new("Hello مرحبا ب".len()), 1.0);
    canvas.fill_rect(
        px(caret.x, origin.0),
        px(caret.y, origin.1),
        (caret.width * scale).round() as i32,
        (caret.height * scale).round() as i32,
        [96, 165, 250],
    );

    for (l, origin) in &lines {
        let quads = l
            .prepare_glyphs(*origin, scale, &mut raster, &mut atlas)
            .expect("atlas fits");
        canvas.draw_quads(&quads, &atlas);
        for run in l.glyph_runs() {
            if let Some((offset, size)) = run.underline {
                let (Some(first), Some(last)) = (run.glyphs.first(), run.glyphs.last()) else {
                    continue;
                };
                canvas.fill_rect(
                    px(first.x, origin.0),
                    px(run.baseline + offset, origin.1),
                    ((last.x + last.advance - first.x) * scale).round() as i32,
                    ((size * scale).round() as i32).max(1),
                    role_rgb(run.color),
                );
            }
        }
    }
    assert!(!atlas.take_dirty().is_empty());

    // The atlas itself, to the right.
    let (aw, ah) = atlas.size();
    let ax = (lw as f32 * scale) as u32 + 8;
    for yy in 0..ah {
        for xx in 0..aw {
            let v = atlas.pixels()[(yy * aw + xx) as usize];
            canvas.px[(yy * canvas.w + ax + xx) as usize] = [v, v, v.saturating_add(24)];
        }
    }

    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/text-preview.png");
    let out: PathBuf = out;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).expect("target dir");
    }
    canvas.write_png(&out);
    assert!(out.exists());
}
