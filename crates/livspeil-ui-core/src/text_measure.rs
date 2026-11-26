// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Text measurement hook and font attributes. The core does not shape
//! text; a host installs a [`TextMeasurer`] (for example
//! `livspeil-ui-paint`'s, backed by `livspeil-ui-text`) and text elements
//! get their real size for layout. Without one (headless tests), text is
//! sized by a character-count estimate.

use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use livspeil_ui_layout::{Measure, Size, TextBlock};

use crate::app::App;
use crate::element::TextSpec;

/// An OpenType feature setting, e.g. `tnum=1` (tabular figures).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontFeature {
    /// The four-byte feature tag.
    pub tag: [u8; 4],
    /// Its value (0 off, 1 on, or an alternate index).
    pub value: u16,
}

impl FontFeature {
    /// Tabular figures (numbers in parameter fields line up).
    pub const TNUM: FontFeature = FontFeature::on(*b"tnum");
    /// Inter stylistic set 1 (open digits).
    pub const SS01: FontFeature = FontFeature::on(*b"ss01");
    /// Slashed zero.
    pub const ZERO: FontFeature = FontFeature::on(*b"zero");

    /// `tag` turned on.
    #[must_use]
    pub const fn on(tag: [u8; 4]) -> Self {
        FontFeature { tag, value: 1 }
    }

    /// `tag` turned off.
    #[must_use]
    pub const fn off(tag: [u8; 4]) -> Self {
        FontFeature { tag, value: 0 }
    }
}

impl fmt::Display for FontFeature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={}", String::from_utf8_lossy(&self.tag), self.value)
    }
}

/// The font of a text run: size, weight and OpenType features (the UI
/// family; the renderer picks the face).
#[derive(Clone, Debug, PartialEq)]
pub struct FontSpec {
    /// Size in logical pixels.
    pub size: f32,
    /// CSS weight, 1..=1000 (400 regular, 500 medium, 600 semibold).
    pub weight: f32,
    /// OpenType features.
    pub features: Vec<FontFeature>,
}

impl FontSpec {
    /// Regular weight.
    pub const REGULAR: f32 = 400.0;

    /// Regular text at `size` with no features.
    #[must_use]
    pub fn sized(size: f32) -> Self {
        FontSpec {
            size,
            weight: Self::REGULAR,
            features: Vec::new(),
        }
    }
}

/// Measures text for layout, in logical pixels.
pub trait TextMeasurer {
    /// The natural (unwrapped) size of `text` in `font`: the advance of its
    /// longest line and the height of all its lines.
    fn measure(&self, text: &str, font: &FontSpec) -> Size;

    /// The height of `text` wrapped to `width`.
    ///
    /// The default breaks greedily after spaces and hyphens, measuring each
    /// candidate line with [`TextMeasurer::measure`]; words wider than
    /// `width` keep a line to themselves, as the renderer lets them
    /// overflow. A measurer with a real line breaker should override it.
    fn wrapped_height(&self, text: &str, font: &FontSpec, width: f32) -> f32 {
        let height = |line: &str| {
            let line = line.trim_end();
            self.measure(if line.is_empty() { "x" } else { line }, font)
                .height
        };
        let mut total = 0.0;
        for paragraph in text.split('\n') {
            let mut line = String::new();
            for word in paragraph.split_inclusive([' ', '-']) {
                let before = line.len();
                line.push_str(word);
                if before > 0 && self.measure(line.trim_end(), font).width > width + 0.5 {
                    total += height(&line[..before]);
                    line.drain(..before);
                }
            }
            total += height(&line);
        }
        total
    }
}

/// The installed measurer.
#[derive(Clone, Default)]
pub(crate) struct MeasurerSlot(pub(crate) Option<Rc<dyn TextMeasurer>>);

impl fmt::Debug for MeasurerSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_some() {
            "TextMeasurer"
        } else {
            "estimate"
        })
    }
}

impl App {
    /// Installs a text measurer. Install it before the first frame: text
    /// already laid out is re-measured only when its view re-renders.
    pub fn set_text_measurer(&mut self, measurer: Rc<dyn TextMeasurer>) {
        self.text_measurer.0 = Some(measurer);
    }

    /// Whether a text measurer is installed.
    #[must_use]
    pub fn has_text_measurer(&self) -> bool {
        self.text_measurer.0.is_some()
    }
}

/// The layout measure of a text element: its natural size, and the wrapped
/// height when layout gives it less width than that.
pub(crate) fn text_measure(app: &App, t: &TextSpec) -> Measure {
    let font = t.font(app.theme());
    let size = font.size;
    match &app.text_measurer.0 {
        Some(m) => {
            let natural = m.measure(&t.text, &font);
            let mut key = DefaultHasher::new();
            t.text.hash(&mut key);
            font.size.to_bits().hash(&mut key);
            font.weight.to_bits().hash(&mut key);
            for f in &font.features {
                (f.tag, f.value).hash(&mut key);
            }
            let (m, text) = (m.clone(), t.text.clone());
            Measure::Text(TextBlock::new(natural, key.finish(), move |width| {
                m.wrapped_height(&text, &font, width)
            }))
        }
        None => Measure::TextPlaceholder {
            chars: u32::try_from(t.text.chars().count()).unwrap_or(u32::MAX),
            font_size: size,
            line_height: (size * 1.3).round(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Element, Render, TestHost, ViewContext, div, text};

    struct Fixed;

    impl TextMeasurer for Fixed {
        fn measure(&self, text: &str, font: &FontSpec) -> Size {
            // Bold text is wider; features are passed through.
            let w = if font.weight > 500.0 { 12.0 } else { 10.0 };
            let w = if font.features.contains(&FontFeature::TNUM) {
                w + 1.0
            } else {
                w
            };
            Size::new(text.len() as f32 * w, font.size * 2.0)
        }
    }

    struct Label;

    impl Render for Label {
        fn render(&mut self, _cx: &mut ViewContext<'_, Self>) -> Element {
            div()
                .flex_col()
                .style(|s| s.align_items = livspeil_ui_layout::style::AlignItems::START)
                .child(text("abcd").name("t").font_size(10.0))
                .child(text("abcd").name("bold").font_size(10.0).font_weight(600.0))
                .child(
                    text("12")
                        .name("tnum")
                        .font_size(10.0)
                        .font_feature(FontFeature::TNUM),
                )
        }
    }

    struct Paragraphs;

    impl Render for Paragraphs {
        fn render(&mut self, _cx: &mut ViewContext<'_, Self>) -> Element {
            // Columns stretch their children to 100 px; the card's height
            // follows the wrapped paragraph.
            div().flex_col().w(100.0).child(
                div()
                    .name("card")
                    .flex_col()
                    .p(5.0)
                    .child(text("aaaa bbbb cccc dddd").name("long").font_size(10.0))
                    .child(text("ab").name("short").font_size(10.0)),
            )
        }
    }

    #[test]
    fn text_narrower_than_its_natural_width_is_as_tall_as_its_lines() {
        let mut app = App::new();
        app.set_text_measurer(Rc::new(Fixed));
        let view = app.new_view(Paragraphs);
        app.set_root(view);
        let mut host = TestHost::new(app, Size::new(200.0, 200.0));
        host.run_until_idle(10);
        let b = |n: &str| host.app.bounds(host.app.find(n).unwrap()).unwrap();
        // 90 px hold nine 10 px glyphs: "aaaa bbbb" / "cccc dddd", 20 px
        // lines.
        assert_eq!((b("long").width, b("long").height), (90.0, 40.0));
        assert_eq!(b("short").height, 20.0);
        assert_eq!(b("card").height, 70.0);
        assert_eq!(b("short").y, b("long").bottom());
    }

    #[test]
    fn the_default_wrap_breaks_after_spaces_and_hyphens() {
        let font = FontSpec::sized(10.0);
        let h = |s: &str, w: f32| Fixed.wrapped_height(s, &font, w);
        assert_eq!(h("aaaa bbbb", 200.0), 20.0);
        assert_eq!(h("aaaa bbbb", 40.0), 40.0);
        assert_eq!(h("aaaa-bbbb", 50.0), 40.0);
        // A word wider than the line overflows on a line of its own.
        assert_eq!(h("aaaaaaaaaa b", 40.0), 40.0);
        assert_eq!(h("a\n\nb", 40.0), 60.0);
    }

    #[test]
    fn installed_measurer_sizes_text() {
        let mut app = App::new();
        assert!(!app.has_text_measurer());
        app.set_text_measurer(Rc::new(Fixed));
        let view = app.new_view(Label);
        app.set_root(view);
        let mut host = TestHost::new(app, Size::new(200.0, 100.0));
        host.run_until_idle(10);
        let id = host.app.find("t").unwrap();
        let b = host.app.bounds(id).unwrap();
        assert_eq!((b.width, b.height), (40.0, 20.0));
        let bold = host.app.bounds(host.app.find("bold").unwrap()).unwrap();
        assert_eq!(bold.width, 48.0);
        let tnum = host.app.bounds(host.app.find("tnum").unwrap()).unwrap();
        assert_eq!(tnum.width, 22.0);
        // The paint list carries weight and features to the renderer.
        let scene = host.scene().unwrap();
        let runs: Vec<&crate::TextRun> = scene
            .commands
            .iter()
            .filter_map(|c| match c {
                crate::PaintCmd::Text(t) => Some(t),
                _ => None,
            })
            .collect();
        assert_eq!(runs[1].font.weight, 600.0);
        assert_eq!(runs[2].font.features, vec![FontFeature::TNUM]);
        assert!(host.app.dump_scene(scene).contains("weight=600"));
        assert!(host.app.dump_scene(scene).contains("features=tnum=1"));
    }
}
