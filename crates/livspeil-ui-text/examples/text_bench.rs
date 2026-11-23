// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Timings through the crate's API (test-mode fonts). Run with
//! `cargo run --release -p livspeil-ui-text --example text_bench`.
//! The Parley vs cosmic-text comparison lives in `spike/`.

use std::time::Instant;

use livspeil_ui_text::{
    FontFeature, FontSystem, GlyphAtlas, GlyphRasterizer, LayoutOptions, SubpixelMode, TextLayout,
    TextStyle,
};

fn median_us(runs: usize, mut f: impl FnMut()) -> f64 {
    f();
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

fn main() {
    let mut fs = FontSystem::test();
    let style = TextStyle::sized(13.0);
    let tnum = TextStyle::sized(13.0).with_feature(FontFeature::TNUM);
    let labels: Vec<String> = (0..1000)
        .map(|i| format!("Parameter {i}: {}.5", i * 3))
        .collect();

    let full = median_us(15, || {
        for s in &labels {
            std::hint::black_box(TextLayout::new(&mut fs, s, &tnum, LayoutOptions::default()));
        }
    });
    let mut layouts: Vec<TextLayout> = labels
        .iter()
        .map(|s| TextLayout::new(&mut fs, s, &style, LayoutOptions::default()))
        .collect();
    let mut w = 100.0;
    let rewrap = median_us(15, || {
        w += 1.0;
        for l in &mut layouts {
            l.set_max_width(Some(w));
        }
    });

    let para: String = "Livspeil scene charm port graph დიზაინი spring 0.125 "
        .repeat(200)
        .chars()
        .take(10_000)
        .collect();
    let para_full = median_us(9, || {
        std::hint::black_box(TextLayout::new(
            &mut fs,
            &para,
            &style,
            LayoutOptions::wrap(480.0),
        ));
    });
    let mut pl = TextLayout::new(&mut fs, &para, &style, LayoutOptions::wrap(480.0));
    let para_rewrap = median_us(9, || {
        w = if w > 600.0 { 300.0 } else { w + 37.0 };
        pl.set_max_width(Some(w));
    });

    let mut raster = GlyphRasterizer::new(SubpixelMode::Quarter);
    let mut atlas = GlyphAtlas::new(2048, 2048);
    let cold = median_us(5, || {
        atlas.clear();
        for l in layouts.iter().take(100) {
            std::hint::black_box(
                l.prepare_glyphs((0.0, 0.0), 2.0, &mut raster, &mut atlas)
                    .map_or(0, |q| q.len()),
            );
        }
    });
    let warm = median_us(15, || {
        for l in layouts.iter().take(100) {
            std::hint::black_box(
                l.prepare_glyphs((0.0, 0.0), 2.0, &mut raster, &mut atlas)
                    .map_or(0, |q| q.len()),
            );
        }
    });

    println!(
        "1k labels (tnum) full layout: {full:.0} us ({:.2} us/label)",
        full / 1000.0
    );
    println!("1k labels re-wrap only:       {rewrap:.0} us");
    println!("10k-char paragraph layout:    {para_full:.0} us, re-wrap {para_rewrap:.0} us");
    println!(
        "prepare_glyphs 100 labels @2x: cold atlas {cold:.0} us, warm {warm:.0} us ({} atlas entries)",
        atlas.len()
    );
}
