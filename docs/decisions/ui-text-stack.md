# Decision: UI text stack (M1.2)

Status: accepted (2026-11-23). Scope: `livspeil-ui-text` (UI spec §4).

**Decision: Parley 0.13 (linebender) for shaping and layout, with skrifa
outlines rasterized by zeno for the glyph atlas. Not cosmic-text.**

## Candidates

| | Parley 0.13.0 (2026-11-12) | cosmic-text 0.19.0 (2026-04-22) |
|---|---|---|
| Shaping | HarfRust 0.15 (Rust HarfBuzz port) | HarfRust 0.5 |
| Fonts / fallback | fontique (fontconfig, CoreText, DirectWrite) | fontdb + its own fallback lists |
| Rasterization | none built in; we use skrifa 0.49 + zeno | swash 0.2 (`SwashCache`) |
| Editing | `Cursor`/`Selection` with visual and logical moves, `PlainEditor` with IME composition | `Editor` with `Action`s; no preedit model |
| MSRV | 1.88 | 1.89 |

Both are MIT OR Apache-2.0 and pass `cargo deny` with the existing allow
list.

## Method

`crates/livspeil-ui-text/spike/` is a standalone crate (its own workspace,
so the losing stack never entered the main `Cargo.lock`) that runs both
stacks on the same inputs: bundled fonts only ("test") and with system
fonts ("system"). Run it with
`cargo run --release --manifest-path crates/livspeil-ui-text/spike/Cargo.toml`.
The machine was a shared 2-core Linux VM with other builds running (load
average 3 to 4.5), so absolute numbers are noisy. Two runs are given; the
ratios were stable.

## Findings

### Shaping quality with Inter
Same glyph ids from both stacks on every case (both use HarfRust):
- `tnum`: digits switch to tabular glyphs (e.g. `0` 1341 → 1359), all
  8.4375 px at 13 px.
- `ss01`: `6`/`9` switch to alternates (1349 → 1357, 1352 → 1358), `a`
  unchanged.
- Variable weight: Parley passes normalized `wght` coordinates per run
  (`[0, 8848]` for 700); we hand them to skrifa.

Gotcha found in the spike: the vendored file's family name is
**"Inter Variable"**. Asking for "Inter" in system mode picked the
system's static Inter (different glyph ids), and in test mode Parley
found no font at all. The crate asks for "Inter Variable" (`UI_FAMILY`).

### Bidi, mixed scripts, fallback
- `Hello مرحبا بالعالم 123 world`: the visual order is the same in both
  and correct per UBA ("123" sits leftmost in the RTL segment). Parley
  reports runs with text ranges and per-run direction. cosmic-text gives
  per-glyph levels only.
- Georgian (Inter has no Georgian; nor Arabic): in test mode both fall
  back to the bundled Noto Sans Georgian. In system mode, raw Parley took
  fontconfig's choice (DejaVu Sans). The crate fixes this by putting the
  bundled faces first in the script fallback lists. It also does not add
  generic sans-serif after Inter, because a system sans that covers
  Georgian would win before script fallback (see the
  `bundled_fallbacks_win_in_system_mode` test).
- CJK: system mode finds Noto Sans CJK in both (Parley chose JP, cosmic
  SC, for untagged text; Parley can take a locale per span). In test mode
  both give `.notdef`, which is deterministic. CJK is not bundled (too
  big).

### Line breaking and alignment
Same breaks at 120 px (4 lines, UAX #14). Parley has Start/End/Left/
Center/Right/Justify and re-wraps without reshaping. Note: Parley applies
`set_alignment` while breaking lines, so it must be called before
`break_all_lines` (the spike first did it the other way round and got no
centring).

### Rich text
Parley: style spans on byte ranges (`RangedBuilder`), or a tree. A
shaping run is split into glyph runs at style changes, and the brush
(our `ColorRole`) and underline come through per glyph run. cosmic-text:
`AttrsList` spans per line. Both are adequate. Parley's model is closer to
what widgets need (one layout per paragraph, no implicit line split).

### Editing primitives and IME
- Parley: `Cursor` with affinity, `next/previous_visual(_word)`,
  `next/previous_logical_word`, `move_lines`, line/hard-line edges,
  `Selection::geometry` (bidi selections split into several rects),
  `Cursor::geometry`, hit testing via `Cursor::from_point`. `PlainEditor`
  shows a composition model (`set_compose`/`finish_compose`) and an IME
  cursor area.
- cosmic-text: `Editor` + `Action` enum (motion, insert, backspace,
  click/drag). No preedit or composition state. Undo exists only through
  the optional `cosmic_undo_2` / vi features.

Neither has the undo-able op stream we want, so the crate has its own
`Editor` model built on Parley's cursor primitives for visual moves and on
`unicode-segmentation` for logical ones.

### Hit testing
Both are fine. Parley's `Cursor::from_point` + affinity round-trips
correctly for LTR, Georgian, mixed bidi and wrapped text (tests).

### Performance (release, medians, µs; run 1 / run 2)

| Case | Parley | cosmic-text | Ratio |
|---|---|---|---|
| 1k short labels, full layout | 8 313 / 12 109 | 14 173 / 24 208 | 1.7–2.0× |
| 1k labels with `tnum` | 8 057 / 7 973 | 14 738 / 19 006 | 1.8–2.4× |
| 1k labels, cosmic reusing buffers | – | 14 443 / 19 057 | |
| 1k labels, re-wrap only | 384 / 369 | – | |
| 2k-char paragraph, layout @480 px | 560 / 535 | 1 093 / 1 051 | 2.0× |
| 10k-char paragraph, layout | 2 614 / 2 647 | 5 451 / 10 715 | 2.1–4× |
| 10k-char paragraph, re-wrap | 101 / 130 | 227 / 279 | 2.2× |
| 10k-char paragraph, 1-char edit + relayout | 2 583 / 2 503 | 5 206 / 6 578 | 2.0–2.6× |
| Rasterize 399 glyphs @26 px, uncached | 983 / 857 (2.2–2.5 µs/glyph) | 2 230 / 2 041 (5.1–5.6) | 2.3× |
| Font system init, test / system | 1.6–6.8 ms / 19–43 ms | 0.1–1.3 ms / 9–246 ms | |

Through the crate API (`examples/text_bench.rs`, same machine): 1k `tnum`
labels 11.9 ms (≈12 µs/label, including copying the text and resolving
styles), re-wrap 0.5 ms, 10k-char paragraph 3.0 ms (re-wrap 0.1 ms),
`prepare_glyphs` for 100 labels at 2x: 0.8 ms with a cold atlas, 0.25 ms
warm.

Consequence for the < 1 ms UI CPU budget (spec §12): layouts must be
cached per widget and rebuilt only when text or style changes. Re-wrap is
cheap. Long text being edited costs ~2.5 ms per keystroke at 10k chars in
one paragraph, so the code editor and large text areas should lay out per
paragraph (cosmic-text does this internally, by line; we do it in the
widget).

### Glyph rasterization for an atlas
cosmic-text comes with `SwashCache` (swash: hinting, color/bitmap glyphs).
With Parley we rasterize ourselves: skrifa `OutlineGlyphCollection` (same
read-fonts as Parley, variation coordinates from the run) → zeno coverage
mask. That gives us full control over grayscale AA only (no LCD), the
subpixel x offset (shift the outline by ¼ px before filling) and the
placement. The swash path was 2.3× slower here and pulls a second
read-fonts/skrifa (swash 0.2.10 pins skrifa ≤ 0.44 while Parley uses
0.49). Not supported yet on our path: COLR/bitmap (emoji) glyphs; skrifa
can draw COLR later, and emoji are out of scope for M1.2.

### Dependency weight and licenses
Linux, normal deps, unique crates: Parley 55 (ICU4X segmenter and
properties data, HarfRust, fontique, fontconfig via dlopen), cosmic-text
41 (but **three** read-fonts/skrifa versions: 0.37 for HarfRust 0.5 and
skrifa 0.40, 0.41 for swash, plus font-types twice). In the workspace,
`livspeil-ui-text` adds no new duplicate crates (the hashbrown, syn,
unicode-width and wasm-encoder duplicates already exist on `main`).
Licenses: MIT/Apache-2.0, Unicode-3.0 (ICU data), Zlib; `cargo deny
check` passes without changes to `deny.toml`. Fonts are assets, not
crates, so OFL-1.1 is covered by REUSE (`LICENSES/OFL-1.1.txt`), not by
`deny.toml`.

### MSRV
Parley 0.13 needs 1.88, cosmic-text 0.19 needs 1.89. Both are fine with
the workspace MSRV of 1.95.

### Maintenance
Parley: monthly releases (0.9 in April 2026 to 0.13 in November 2026),
under linebender (Xilem/Masonry, Blitz, Vello). Its API changes between
minors (0.13 split out `parley_engine`/`parlance`), so we keep it behind
our own types. cosmic-text: 0.17 → 0.19 in Jan–Apr 2026, nothing since,
driven by COSMIC desktop needs.

## Why Parley
1. About 2× faster layout and re-wrap on every case, and our raster path
   is 2.3× faster than SwashCache.
2. Better editing primitives: affinity-aware cursors, visual (bidi) moves,
   selection geometry and a composition model. These are what TextField
   and IME need. cosmic-text has no preedit state.
3. Paragraph-level rich text with per-run brush/decoration and
   locale-aware fallback through fontique.
4. One font-parsing stack (read-fonts/skrifa) shared by layout and
   rasterization, and fewer duplicate crates.
5. Active development in the same ecosystem as Vello, which the spec
   keeps as an option for a future design canvas.

Costs we accept: frequent API changes (isolated in this crate, one place
to update), we write the rasterizer glue (~150 lines) and an editor model
ourselves, and there is no color-glyph support yet.

## What the crate provides
See `crates/livspeil-ui-text/src/lib.rs`: `FontSystem` (`test()` /
`new()`), `TextStyle` + `StyleSpan`, `TextLayout` (lines, glyph runs, hit
test, caret/selection rects, visual moves, `dump()` for goldens),
`Editor` (grapheme/word ops, selection, IME preedit, undo/redo, op
stream), `GlyphRasterizer` + `GlyphAtlas` (R8, etagere, dirty rects,
`GlyphQuad`s via `TextLayout::prepare_glyphs`).

## Follow-ups
- Color glyphs (COLR v0/v1, emoji) via skrifa.
- Per-paragraph layout caching for long texts (code editor, notes).
- Atlas growth or multi-page atlases instead of clear-on-full.
- Locale per span (CJK Han variants, Georgian/Arabic `locl`).
- Gamma-correct blending and contrast boost happen in the renderer
  (`livspeil-ui-render`), as do the color values for `ColorRole`.
