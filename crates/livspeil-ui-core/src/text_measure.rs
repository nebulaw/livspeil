// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Text measurement hook and font attributes. The core does not shape
//! text; a host installs a [`TextMeasurer`] (for example
//! `livspeil-ui-paint`'s, backed by `livspeil-ui-text`) and text elements
//! get their real size for layout. Without one (headless tests), text is
//! sized by a character-count estimate.

use std::fmt;
use std::rc::Rc;

use livspeil_ui_layout::{Measure, Size};

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
}

/// The installed measurer.
#[derive(Clone, Default)]
pub(crate) struct MeasurerSlot(pub(crate) Option<Rc<dyn TextMeasurer>>);

impl fmt::Debug for MeasurerSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_some() {
            "TextMeasurer"
        } else {
            "placeholder"
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

/// The layout measure of a text element. Measured text is one fixed box
/// (wrapping by the available width needs a measure callback in
/// `livspeil-ui-layout`; until then wrapped paragraphs get an explicit
/// size from their view).
pub(crate) fn text_measure(app: &App, t: &TextSpec) -> Measure {
    let font = t.font(app.theme());
    let size = font.size;
    match &app.text_measurer.0 {
        Some(m) => Measure::Fixed(m.measure(&t.text, &font)),
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
