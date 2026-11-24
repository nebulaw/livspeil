// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`Label`]: static text using the core text element.

use livspeil_ui_core::{Element, SharedString, text};
use livspeil_ui_theme::Theme;

use crate::semantics::{Role, Semantics};
use crate::style::{self, metrics};

/// Text emphasis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Tone {
    /// `text.primary`.
    #[default]
    Primary,
    /// `text.secondary`.
    Secondary,
    /// `text.tertiary`.
    Tertiary,
}

/// Static text in the density's body size (or `large`), colored by tone.
#[derive(Clone, Debug)]
pub struct Label {
    text: SharedString,
    tone: Tone,
    large: bool,
    disabled: bool,
}

impl Label {
    /// A primary label.
    #[must_use]
    pub fn new(text: impl Into<SharedString>) -> Self {
        Label {
            text: text.into(),
            tone: Tone::Primary,
            large: false,
            disabled: false,
        }
    }

    /// Set the tone.
    #[must_use]
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// Secondary tone.
    #[must_use]
    pub fn secondary(self) -> Self {
        self.tone(Tone::Secondary)
    }

    /// The density's large body size.
    #[must_use]
    pub fn large(mut self) -> Self {
        self.large = true;
        self
    }

    /// Dim as disabled.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Semantics: `Label` with the text.
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        let mut s = Semantics::new(Role::Label).label(&*self.text);
        s.disabled = self.disabled;
        s
    }

    /// Build the element (name `label`; not hit-testable).
    #[must_use]
    pub fn render(self, theme: &Theme) -> Element {
        let sem = self.semantics();
        let c = theme.colors();
        let color = match self.tone {
            Tone::Primary => c.text_primary,
            Tone::Secondary => c.text_secondary,
            Tone::Tertiary => c.text_tertiary,
        };
        let size = if self.large {
            theme.metrics().body_large
        } else {
            metrics(theme).font
        };
        style::disabled(
            sem.apply(text(self.text))
                .name("label")
                .font_size(size)
                .text_color(color)
                .hit_test(false),
            self.disabled,
        )
    }
}
