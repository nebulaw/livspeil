// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Text measurement hook. The core does not shape text; a host installs a
//! [`TextMeasurer`] (for example `livspeil-ui-paint`'s, backed by
//! `livspeil-ui-text`) and text elements get their real size for layout.
//! Without one, text is sized by the character-count placeholder.

use std::fmt;
use std::rc::Rc;

use livspeil_ui_layout::{Measure, Size};

use crate::app::App;
use crate::element::TextSpec;

/// Measures text for layout, in logical pixels.
pub trait TextMeasurer {
    /// The natural (unwrapped) size of `text` at `font_size`: the advance of
    /// its longest line and the height of all its lines.
    fn measure(&self, text: &str, font_size: f32) -> Size;
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
    let size = t.size.unwrap_or_else(|| app.theme().metrics().body);
    match &app.text_measurer.0 {
        Some(m) => Measure::Fixed(m.measure(&t.text, size)),
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
        fn measure(&self, text: &str, font_size: f32) -> Size {
            Size::new(text.len() as f32 * 10.0, font_size * 2.0)
        }
    }

    struct Label;

    impl Render for Label {
        fn render(&mut self, _cx: &mut ViewContext<'_, Self>) -> Element {
            div()
                .items_center()
                .child(text("abcd").name("t").font_size(10.0))
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
    }
}
