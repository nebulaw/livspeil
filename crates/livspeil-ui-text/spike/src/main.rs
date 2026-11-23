// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! M1.2 text stack spike: Parley 0.13 vs cosmic-text 0.19 on the same
//! inputs. Prints shaping observations and timings (median of N runs).
//! Run in release mode; see docs/decisions/ui-text-stack.md.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

const FONTS: &[&str] = &[
    "inter/InterVariable.ttf",
    "inter/InterVariable-Italic.ttf",
    "noto-sans-georgian/NotoSansGeorgian-Variable.ttf",
    "noto-sans-arabic/NotoSansArabic-Variable.ttf",
];

fn font_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets/fonts")
}

fn median_us(runs: usize, mut f: impl FnMut()) -> f64 {
    f(); // warm-up
    let mut t: Vec<f64> = (0..runs)
        .map(|_| {
            let s = Instant::now();
            f();
            s.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    t.sort_by(f64::total_cmp);
    t[t.len() / 2]
}

fn paragraph(chars: usize) -> String {
    let words = [
        "Livspeil",
        "scene",
        "charm",
        "port",
        "graph",
        "დიზაინი",
        "node",
        "spring",
        "dream",
        "live",
        "shader",
        "params",
        "tnum",
        "0.125",
        "layout",
        "glyph",
    ];
    let mut s = String::new();
    let mut i = 0usize;
    while s.chars().count() < chars {
        s.push_str(words[(i * 7 + i / 3) % words.len()]);
        s.push(if i % 17 == 16 { '.' } else { ' ' });
        i += 1;
    }
    s
}

// ---------------------------------------------------------------- Parley

mod p {
    use super::*;
    use parley::fontique::{Blob, Collection, CollectionOptions, Script, SourceCache};
    use parley::{
        Alignment, AlignmentOptions, FontContext, FontFamily, FontFeature, FontFeatures, Layout,
        LayoutContext, PositionedLayoutItem, StyleProperty,
    };

    pub fn font_cx(system: bool) -> FontContext {
        let mut collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: system,
        });
        let mut ids = Vec::new();
        for f in FONTS {
            let data = std::fs::read(font_dir().join(f)).unwrap();
            for (fam, _) in collection.register_fonts(Blob::new(Arc::new(data)), None) {
                ids.push(fam);
            }
        }
        if !system {
            collection.set_fallbacks(Script::from_bytes(*b"Geor"), ids[2..3].iter().copied());
            collection.set_fallbacks(Script::from_bytes(*b"Arab"), ids[3..4].iter().copied());
        }
        FontContext {
            collection,
            source_cache: SourceCache::default(),
        }
    }

    pub fn layout(
        fcx: &mut FontContext,
        lcx: &mut LayoutContext<()>,
        text: &str,
        size: f32,
        features: &[FontFeature],
        width: Option<f32>,
    ) -> Layout<()> {
        let mut b = lcx.ranged_builder(fcx, text, 1.0, true);
        b.push_default(StyleProperty::FontFamily(FontFamily::from(
            "Inter Variable",
        )));
        b.push_default(StyleProperty::FontSize(size));
        b.push_default(StyleProperty::FontFeatures(FontFeatures::List(
            features.into(),
        )));
        let mut l = b.build(text);
        l.break_all_lines(width);
        l.set_alignment(Alignment::Start, AlignmentOptions::default());
        l
    }

    pub fn describe(l: &Layout<()>) -> String {
        let mut out = String::new();
        for line in l.lines() {
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(gr) = item {
                    let run = gr.run();
                    let font = run.font();
                    let name = skrifa::FontRef::from_index(font.data.as_ref(), font.index)
                        .ok()
                        .and_then(|f| {
                            use skrifa::MetadataProvider;
                            f.localized_strings(skrifa::string::StringId::FAMILY_NAME)
                                .english_or_first()
                                .map(|s| s.to_string())
                        })
                        .unwrap_or_default();
                    let ids: Vec<u32> = gr.glyphs().map(|g| g.id).collect();
                    out += &format!(
                        "  run {:?} rtl={} font={name} x={:.2} adv={:.2} glyphs={:?}\n",
                        run.text_range(),
                        run.is_rtl(),
                        gr.offset(),
                        gr.advance(),
                        &ids[..ids.len().min(12)]
                    );
                }
            }
        }
        out
    }

    pub fn tag(s: &[u8; 4], v: u16) -> FontFeature {
        FontFeature::new(parley::setting::Tag::new(s), v)
    }

    /// Rasterize every glyph of a layout via skrifa outlines + zeno.
    pub fn raster(l: &Layout<()>) -> usize {
        use skrifa::MetadataProvider;
        use skrifa::outline::{DrawSettings, OutlinePen};
        struct Pen(Vec<zeno::Command>);
        impl OutlinePen for Pen {
            fn move_to(&mut self, x: f32, y: f32) {
                self.0.push(zeno::Command::MoveTo([x, -y].into()));
            }
            fn line_to(&mut self, x: f32, y: f32) {
                self.0.push(zeno::Command::LineTo([x, -y].into()));
            }
            fn quad_to(&mut self, a: f32, b: f32, x: f32, y: f32) {
                self.0
                    .push(zeno::Command::QuadTo([a, -b].into(), [x, -y].into()));
            }
            fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
                self.0.push(zeno::Command::CurveTo(
                    [a, -b].into(),
                    [c, -d].into(),
                    [x, -y].into(),
                ));
            }
            fn close(&mut self) {
                self.0.push(zeno::Command::Close);
            }
        }
        let mut px = 0;
        for line in l.lines() {
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(gr) = item {
                    let run = gr.run();
                    let font = run.font();
                    let fref = skrifa::FontRef::from_index(font.data.as_ref(), font.index).unwrap();
                    let coords: Vec<skrifa::instance::NormalizedCoord> = run
                        .normalized_coords()
                        .iter()
                        .map(|c| skrifa::instance::NormalizedCoord::from_bits(c.to_bits()))
                        .collect();
                    let outlines = fref.outline_glyphs();
                    for g in gr.positioned_glyphs() {
                        let Some(og) = outlines.get(skrifa::GlyphId::new(g.id)) else {
                            continue;
                        };
                        let mut pen = Pen(Vec::new());
                        let settings = DrawSettings::unhinted(
                            skrifa::instance::Size::new(run.font_size()),
                            coords.as_slice(),
                        );
                        if og.draw(settings, &mut pen).is_err() {
                            continue;
                        }
                        let (mask, placement) =
                            zeno::Mask::new(&pen.0).offset([g.x.fract(), 0.0]).render();
                        px += mask.len() + placement.width as usize;
                    }
                }
            }
        }
        px
    }
}

// ----------------------------------------------------------- cosmic-text

mod c {
    use super::*;
    use cosmic_text::{
        Attrs, Buffer, Family, FeatureTag, FontFeatures, FontSystem, Metrics, Shaping, SwashCache,
    };

    pub fn font_system(system: bool) -> FontSystem {
        if system {
            let mut fs = FontSystem::new();
            for f in FONTS {
                fs.db_mut().load_font_file(font_dir().join(f)).unwrap();
            }
            fs
        } else {
            let mut db = cosmic_text::fontdb::Database::new();
            for f in FONTS {
                db.load_font_file(font_dir().join(f)).unwrap();
            }
            FontSystem::new_with_locale_and_db("en-US".into(), db)
        }
    }

    pub fn buffer(
        fs: &mut FontSystem,
        text: &str,
        size: f32,
        features: Option<&[&[u8; 4]]>,
        width: Option<f32>,
    ) -> Buffer {
        let mut b = Buffer::new(fs, Metrics::new(size, size * 1.25));
        let mut ff = FontFeatures::new();
        for t in features.unwrap_or(&[]) {
            ff.enable(FeatureTag::new(t));
        }
        let attrs = Attrs::new()
            .family(Family::Name("Inter Variable"))
            .font_features(ff);
        b.set_size(width, None);
        b.set_text(text, &attrs, Shaping::Advanced, None);
        b.shape_until_scroll(fs, false);
        b
    }

    pub fn describe(fs: &FontSystem, b: &Buffer) -> String {
        let mut out = String::new();
        for run in b.layout_runs() {
            let mut cur: Option<(cosmic_text::fontdb::ID, bool)> = None;
            let mut ids = Vec::new();
            let mut x0 = 0.0;
            let flush = |cur: &Option<(cosmic_text::fontdb::ID, bool)>,
                             ids: &mut Vec<u16>,
                             x0: f32,
                             out: &mut String| {
                if let Some((id, rtl)) = cur {
                    let name = fs
                        .db()
                        .face(*id)
                        .and_then(|f| f.families.first().map(|n| n.0.clone()))
                        .unwrap_or_default();
                    *out += &format!(
                        "  run rtl={rtl} font={name} x={x0:.2} glyphs={:?}\n",
                        &ids[..ids.len().min(12)]
                    );
                }
                ids.clear();
            };
            for g in run.glyphs {
                let key = (g.font_id, g.level.is_rtl());
                if cur != Some(key) {
                    flush(&cur, &mut ids, x0, &mut out);
                    cur = Some(key);
                    x0 = g.x;
                }
                ids.push(g.glyph_id);
            }
            flush(&cur, &mut ids, x0, &mut out);
        }
        out
    }

    pub fn raster(fs: &mut FontSystem, cache: &mut SwashCache, b: &Buffer) -> usize {
        let mut px = 0;
        for run in b.layout_runs() {
            for g in run.glyphs {
                let phys = g.physical((0.0, 0.0), 1.0);
                if let Some(img) = cache.get_image_uncached(fs, phys.cache_key) {
                    px += img.data.len();
                }
            }
        }
        px
    }
}

fn main() {
    let t0 = Instant::now();
    let mut pf = p::font_cx(false);
    let p_init = t0.elapsed();
    let t0 = Instant::now();
    let mut pfs = p::font_cx(true);
    let p_init_sys = t0.elapsed();
    let mut lcx = parley::LayoutContext::<()>::new();
    let t0 = Instant::now();
    let mut cf = c::font_system(false);
    let c_init = t0.elapsed();
    let t0 = Instant::now();
    let mut cfs = c::font_system(true);
    let c_init_sys = t0.elapsed();
    println!(
        "init: parley test {p_init:?} system {p_init_sys:?} | cosmic test {c_init:?} system {c_init_sys:?}"
    );

    // ---------------------------------------------------- shaping checks
    let cases: &[(&str, &str)] = &[
        ("digits", "0123456789 1111"),
        ("ss01", "a 6 9 Ga"),
        ("georgian", "გამარჯობა, თბილისი! Hello"),
        ("bidi", "Hello مرحبا بالعالم 123 world"),
        ("cjk", "Inter 日本語 中文"),
    ];
    for (name, text) in cases {
        for (fname, pfeat, cfeat) in [
            ("default", vec![], vec![]),
            ("tnum", vec![p::tag(b"tnum", 1)], vec![b"tnum"]),
            ("ss01", vec![p::tag(b"ss01", 1)], vec![b"ss01"]),
        ] {
            if (*name != "digits" && fname == "tnum") || (*name != "ss01" && fname == "ss01") {
                continue;
            }
            println!("== {name} [{fname}] {text:?}");
            let l = p::layout(&mut pf, &mut lcx, text, 13.0, &pfeat, None);
            print!(" parley test (w={:.2}):\n{}", l.width(), p::describe(&l));
            let b = c::buffer(&mut cf, text, 13.0, Some(&cfeat), None);
            print!(" cosmic test:\n{}", c::describe(&cf, &b));
            if *name == "cjk" || *name == "georgian" {
                let l = p::layout(&mut pfs, &mut lcx, text, 13.0, &pfeat, None);
                print!(" parley system:\n{}", p::describe(&l));
                let b = c::buffer(&mut cfs, text, 13.0, Some(&cfeat), None);
                print!(" cosmic system:\n{}", c::describe(&cfs, &b));
            }
        }
    }

    // wrapping + alignment observation
    let text = "The quick brown fox jumps over the lazy dog. ის ძალიან სწრაფია.";
    let mut l = p::layout(&mut pf, &mut lcx, text, 13.0, &[], Some(120.0));
    l.set_alignment(
        parley::Alignment::Center,
        parley::AlignmentOptions::default(),
    );
    println!("== wrap@120 center: parley lines={}", l.len());
    for line in l.lines() {
        println!("  {:?} {:?}", line.text_range(), &text[line.text_range()]);
    }
    let b = c::buffer(&mut cf, text, 13.0, None, Some(120.0));
    println!("   cosmic lines={}", b.layout_runs().count());

    // ----------------------------------------------------------- timings
    let labels: Vec<String> = (0..1000)
        .map(|i| format!("Parameter {i}: {}.5", i * 3))
        .collect();
    let p_labels = median_us(15, || {
        for s in &labels {
            let l = p::layout(&mut pf, &mut lcx, s, 13.0, &[], None);
            std::hint::black_box(l.width());
        }
    });
    let c_labels = median_us(15, || {
        for s in &labels {
            let b = c::buffer(&mut cf, s, 13.0, None, None);
            std::hint::black_box(b.layout_runs().count());
        }
    });
    let p_labels_tnum = median_us(15, || {
        for s in &labels {
            let l = p::layout(&mut pf, &mut lcx, s, 13.0, &[p::tag(b"tnum", 1)], None);
            std::hint::black_box(l.width());
        }
    });
    let c_labels_tnum = median_us(15, || {
        for s in &labels {
            let b = c::buffer(&mut cf, s, 13.0, Some(&[b"tnum"]), None);
            std::hint::black_box(b.layout_runs().count());
        }
    });
    // reuse buffers across frames (cosmic's intended use for static labels)
    let mut bufs: Vec<_> = labels
        .iter()
        .map(|s| c::buffer(&mut cf, s, 13.0, None, None))
        .collect();
    let c_labels_reuse = median_us(15, || {
        for (b, s) in bufs.iter_mut().zip(&labels) {
            let attrs =
                cosmic_text::Attrs::new().family(cosmic_text::Family::Name("Inter Variable"));
            b.set_text(s, &attrs, cosmic_text::Shaping::Advanced, None);
            b.shape_until_scroll(&mut cf, false);
        }
    });
    let mut layouts: Vec<_> = labels
        .iter()
        .map(|s| p::layout(&mut pf, &mut lcx, s, 13.0, &[], None))
        .collect();
    let p_labels_rebreak = median_us(15, || {
        for l in &mut layouts {
            l.break_all_lines(Some(200.0));
            l.set_alignment(
                parley::Alignment::Start,
                parley::AlignmentOptions::default(),
            );
        }
    });
    println!(
        "1k labels full layout: parley {:.0} us, cosmic {:.0} us (tnum: {:.0} / {:.0}); \
         cosmic set_text into reused buffers {:.0} us; parley re-break only {:.0} us",
        p_labels, c_labels, p_labels_tnum, c_labels_tnum, c_labels_reuse, p_labels_rebreak
    );

    for n in [2_000usize, 10_000] {
        let para = paragraph(n);
        let p_full = median_us(9, || {
            std::hint::black_box(p::layout(&mut pf, &mut lcx, &para, 13.0, &[], Some(480.0)));
        });
        let c_full = median_us(9, || {
            std::hint::black_box(c::buffer(&mut cf, &para, 13.0, None, Some(480.0)));
        });
        let mut l = p::layout(&mut pf, &mut lcx, &para, 13.0, &[], Some(480.0));
        let mut w = 300.0;
        let p_rebreak = median_us(9, || {
            w = if w > 600.0 { 300.0 } else { w + 37.0 };
            l.break_all_lines(Some(w));
            l.set_alignment(
                parley::Alignment::Start,
                parley::AlignmentOptions::default(),
            );
        });
        let mut b = c::buffer(&mut cf, &para, 13.0, None, Some(480.0));
        let c_rebreak = median_us(9, || {
            w = if w > 600.0 { 300.0 } else { w + 37.0 };
            b.set_size(Some(w), None);
            b.shape_until_scroll(&mut cf, false);
        });
        // one-character edit in the middle -> relayout
        let mid = para.char_indices().nth(n / 2).map(|(i, _)| i).unwrap_or(0);
        let mut edited = para.clone();
        let p_edit = median_us(9, || {
            edited.insert(mid, 'x');
            std::hint::black_box(p::layout(
                &mut pf,
                &mut lcx,
                &edited,
                13.0,
                &[],
                Some(480.0),
            ));
        });
        let mut ed = cosmic_text::Editor::new(c::buffer(&mut cf, &para, 13.0, None, Some(480.0)));
        use cosmic_text::Edit;
        ed.set_cursor(cosmic_text::Cursor::new(0, mid));
        let c_edit = median_us(9, || {
            ed.insert_string("x", None);
            ed.shape_as_needed(&mut cf, false);
        });
        println!(
            "paragraph {n} chars @480px: full layout parley {p_full:.0} us / cosmic {c_full:.0} us; \
             re-wrap parley {p_rebreak:.0} us / cosmic {c_rebreak:.0} us; \
             1-char edit parley {p_edit:.0} us / cosmic {c_edit:.0} us"
        );
    }

    // rasterization: all glyphs of a 2x-scale paragraph
    let para = paragraph(400);
    let l = p::layout(&mut pf, &mut lcx, &para, 26.0, &[], Some(900.0));
    let b = c::buffer(&mut cf, &para, 26.0, None, Some(900.0));
    let glyphs: usize = b.layout_runs().map(|r| r.glyphs.len()).sum();
    let mut cache = cosmic_text::SwashCache::new();
    let p_r = median_us(7, || {
        std::hint::black_box(p::raster(&l));
    });
    let c_r = median_us(7, || {
        std::hint::black_box(c::raster(&mut cf, &mut cache, &b));
    });
    println!(
        "raster {glyphs} glyphs @26px uncached: parley+skrifa+zeno {p_r:.0} us ({:.2} us/glyph), \
         cosmic SwashCache {c_r:.0} us ({:.2} us/glyph)",
        p_r / glyphs as f64,
        c_r / glyphs as f64
    );
}
