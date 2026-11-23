// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Shaping and layout goldens (test mode: bundled fonts only) and hit-test
//! round trips.

use std::path::{Path, PathBuf};

use livspeil_testkit::assert_golden;
use livspeil_ui_text::{
    Affinity, Alignment, ColorRole, FontFeature, FontSystem, LayoutOptions, StyleSpan, TextCursor,
    TextLayout, TextStyle,
};
use unicode_segmentation::UnicodeSegmentation;

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn layout(text: &str, style: &TextStyle, options: LayoutOptions) -> TextLayout {
    let mut fs = FontSystem::test();
    TextLayout::new(&mut fs, text, style, options)
}

fn golden(name: &str, l: &TextLayout) {
    assert_golden(&golden_dir(), name, &l.dump());
}

#[test]
fn bundled_families_are_registered() {
    let mut fs = FontSystem::test();
    let names = fs.family_names();
    for family in ["Inter Variable", "Noto Sans Georgian", "Noto Sans Arabic"] {
        assert!(names.iter().any(|n| n == family), "{family} in {names:?}");
    }
    assert!(!fs.has_system_fonts());
}

#[test]
fn latin_default_and_tnum() {
    let style = TextStyle::sized(13.0);
    golden(
        "latin_default",
        &layout("Livspeil 0.125 → 1111", &style, LayoutOptions::default()),
    );
    let tnum = style.clone().with_feature(FontFeature::TNUM);
    let l = layout("0.125 1111", &tnum, LayoutOptions::default());
    golden("latin_tnum", &l);
    // Tabular figures: every digit has the same advance.
    let advances: Vec<f32> = l.glyph_runs()[0]
        .glyphs
        .iter()
        .zip("0.125 1111".chars())
        .filter(|(_, c)| c.is_ascii_digit())
        .map(|(g, _)| g.advance)
        .collect();
    assert!(advances.windows(2).all(|w| (w[0] - w[1]).abs() < 1e-4));
}

#[test]
fn ss01_changes_glyphs() {
    let base = layout("a69", &TextStyle::sized(13.0), LayoutOptions::default());
    let ss01 = layout(
        "a69",
        &TextStyle::sized(13.0).with_feature(FontFeature::SS01),
        LayoutOptions::default(),
    );
    let ids =
        |l: &TextLayout| -> Vec<u32> { l.glyph_runs()[0].glyphs.iter().map(|g| g.id).collect() };
    assert_eq!(ids(&base)[0], ids(&ss01)[0], "'a' unchanged");
    assert_ne!(ids(&base)[1..], ids(&ss01)[1..], "6 and 9 use alternates");
}

#[test]
fn rich_spans_weight_and_color() {
    let mut fs = FontSystem::test();
    let text = "Regular Bold Italic Accent";
    let spans = [
        StyleSpan::new(8..12).weight(700.0),
        StyleSpan::new(13..19).italic(true),
        StyleSpan::new(20..26)
            .color(ColorRole::Accent)
            .underline(true),
    ];
    let l = TextLayout::rich(
        &mut fs,
        text,
        &TextStyle::sized(15.0),
        &spans,
        LayoutOptions::default(),
    )
    .expect("valid spans");
    golden("rich_spans", &l);
    let runs = l.glyph_runs();
    let bold = runs
        .iter()
        .find(|r| r.text_range.start == 8)
        .expect("bold run");
    assert_ne!(bold.coords, runs[0].coords, "weight axis differs");
    assert!(
        runs.iter()
            .any(|r| r.font_family() == "Inter Variable" && r.text_range.start == 13)
    );
    assert!(
        runs.iter()
            .any(|r| r.color == ColorRole::Accent && r.underline.is_some())
    );
}

#[test]
fn rich_rejects_bad_offsets() {
    let mut fs = FontSystem::test();
    let err = TextLayout::rich(
        &mut fs,
        "ა",
        &TextStyle::default(),
        &[StyleSpan::new(0..1).weight(700.0)],
        LayoutOptions::default(),
    );
    assert!(err.is_err());
}

#[test]
fn georgian_falls_back_to_noto() {
    let l = layout(
        "გამარჯობა, თბილისი! Hello",
        &TextStyle::sized(13.0),
        LayoutOptions::default(),
    );
    golden("georgian", &l);
    let fonts: Vec<String> = l.glyph_runs().iter().map(|r| r.font_family()).collect();
    assert!(fonts.contains(&"Noto Sans Georgian".to_owned()));
    assert!(fonts.contains(&"Inter Variable".to_owned()));
    assert!(
        l.glyph_runs()
            .iter()
            .all(|r| r.glyphs.iter().all(|g| g.id != 0)),
        "no tofu"
    );
}

#[test]
fn arabic_bidi_runs() {
    let l = layout(
        "Hello مرحبا بالعالم 123 world",
        &TextStyle::sized(13.0),
        LayoutOptions::default(),
    );
    golden("bidi_ltr_paragraph", &l);
    let runs = l.glyph_runs();
    assert!(
        runs.iter()
            .any(|r| r.rtl && r.font_family() == "Noto Sans Arabic")
    );
    assert!(!l.is_rtl());
    // RTL paragraph (first strong character is Arabic).
    let rtl = layout(
        "مرحبا Livspeil!",
        &TextStyle::sized(13.0),
        LayoutOptions::wrap(300.0),
    );
    golden("bidi_rtl_paragraph", &rtl);
    assert!(rtl.is_rtl());
    let line = &rtl.lines()[0];
    assert!(line.x > 100.0, "start-aligned RTL text sits on the right");
}

#[test]
fn wrap_and_center() {
    let text = "The quick brown fox jumps over the lazy dog. ის ძალიან სწრაფია.";
    let mut l = layout(
        text,
        &TextStyle::sized(13.0),
        LayoutOptions::wrap(120.0).aligned(Alignment::Center),
    );
    golden("wrap_center_120", &l);
    assert!(l.line_count() >= 4);
    for line in l.lines() {
        assert!(line.width <= 120.0 + 0.01);
        let centre = line.x + line.width / 2.0;
        assert!((centre - 60.0).abs() < 0.6, "line centred: {line:?}");
    }
    // Re-wrap without reshaping.
    l.set_max_width(Some(1000.0));
    assert_eq!(l.line_count(), 1);
}

#[test]
fn cjk_in_test_mode_is_notdef() {
    // No bundled CJK face: deterministic .notdef glyphs (system mode would
    // fall back to e.g. Noto Sans CJK).
    let l = layout(
        "Inter 日本語",
        &TextStyle::sized(13.0),
        LayoutOptions::default(),
    );
    golden("cjk_test_mode", &l);
    let last = l.glyph_runs().pop().expect("runs");
    assert!(last.glyphs.iter().any(|g| g.id == 0));
}

#[test]
fn line_height_and_letter_spacing() {
    let style = TextStyle {
        line_height: livspeil_ui_text::LineHeight::Absolute(24.0),
        letter_spacing: 1.0,
        ..TextStyle::sized(13.0)
    };
    let l = layout("ab\ncd", &style, LayoutOptions::default());
    assert_eq!(l.line_count(), 2);
    assert!((l.height() - 48.0).abs() < 0.01, "height {}", l.height());
    let plain = layout("ab", &TextStyle::sized(13.0), LayoutOptions::default());
    let spaced = layout(
        "ab",
        &TextStyle {
            letter_spacing: 1.0,
            ..TextStyle::sized(13.0)
        },
        LayoutOptions::default(),
    );
    assert!((spaced.width() - plain.width() - 2.0).abs() < 0.01);
}

fn boundaries(text: &str) -> Vec<usize> {
    let mut v: Vec<usize> = text.grapheme_indices(true).map(|(i, _)| i).collect();
    v.push(text.len());
    v
}

/// cursor -> rect -> hit test -> same caret position (same visual x and
/// line; for LTR text the same index).
fn round_trip(text: &str, options: LayoutOptions, exact_index: bool) {
    let l = layout(text, &TextStyle::sized(13.0), options);
    for index in boundaries(text) {
        let c = TextCursor::new(index);
        let r = l.cursor_rect(c, 1.0);
        let hit = l.hit_test(r.x, r.y + r.height / 2.0);
        let r2 = l.cursor_rect(hit, 1.0);
        assert!(
            (r.x - r2.x).abs() < 0.01 && (r.y - r2.y).abs() < 0.01,
            "{text:?} index {index}: {r:?} -> {hit:?} {r2:?}"
        );
        if exact_index {
            assert_eq!(hit.index, index, "{text:?}");
        }
    }
}

#[test]
fn hit_test_round_trips() {
    round_trip("Livspeil e\u{301} ok", LayoutOptions::default(), true);
    round_trip("თბილისი ქალაქი", LayoutOptions::default(), true);
    round_trip("Hello مرحبا world", LayoutOptions::default(), false);
    round_trip("مرحبا بالعالم", LayoutOptions::default(), false);
    round_trip(
        "wrapped text across several lines of Inter",
        LayoutOptions::wrap(80.0),
        false,
    );
}

#[test]
fn hit_test_outside_clamps() {
    let l = layout("abc", &TextStyle::sized(13.0), LayoutOptions::default());
    assert_eq!(l.hit_test(-50.0, 5.0).index, 0);
    assert_eq!(l.hit_test(500.0, 5.0).index, 3);
    assert_eq!(l.hit_test(500.0, 500.0).index, 3);
}

#[test]
fn selection_rects_cover_range() {
    let l = layout(
        "Hello مرحبا world",
        &TextStyle::sized(13.0),
        LayoutOptions::default(),
    );
    // Mixed-direction selection: "o مر" spans an LTR and an RTL run.
    let start = 4;
    let end = "Hello مر".len();
    let rects = l.selection_rects(start..end);
    assert!(rects.len() >= 2, "bidi selection is split: {rects:?}");
    assert!(rects.iter().all(|r| r.width > 0.0 && r.height > 0.0));
    let wrapped = layout(
        "one two three four five",
        &TextStyle::sized(13.0),
        LayoutOptions::wrap(60.0),
    );
    let rects = wrapped.selection_rects(0..wrapped.text().len());
    assert_eq!(rects.len(), wrapped.line_count());
}

#[test]
fn visual_moves_follow_bidi() {
    let text = "ab مرح cd";
    let l = layout(text, &TextStyle::sized(13.0), LayoutOptions::default());
    // Walk right from the start until the end; every step increases x.
    let mut c = TextCursor::new(0);
    let mut last_x = l.cursor_rect(c, 1.0).x;
    let mut steps = 0;
    loop {
        let next = l.move_visual(c, true, false);
        if next == c || steps > 20 {
            break;
        }
        let x = l.cursor_rect(next, 1.0).x;
        assert!(
            x >= last_x - 0.01,
            "moving right never goes left: {c:?} -> {next:?}"
        );
        last_x = x;
        c = next;
        steps += 1;
    }
    assert!(steps >= 8, "visited every caret stop ({steps})");
    let end = l.line_edge(TextCursor::new(0), true);
    assert_eq!(end.index, text.len());
    let _ = Affinity::Upstream;
}
