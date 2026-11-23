// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Text styles and their translation to Parley style properties.

use std::borrow::Cow;
use std::ops::Range;

use parley::{
    FontFamily, FontFamilyName, FontFeatures, FontStyle, FontWeight, GenericFamily, RangedBuilder,
    StyleProperty,
};

use crate::fonts::UI_FAMILY;

/// Which font family to use. Fallback for scripts the family does not
/// cover (Georgian, Arabic, CJK, ...) is automatic.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum FamilySpec {
    /// The UI face: vendored Inter (variable).
    #[default]
    Ui,
    /// A family by name (e.g. "Noto Sans Georgian").
    Named(String),
    SansSerif,
    Serif,
    Monospace,
}

impl FamilySpec {
    fn to_parley(&self) -> FontFamily<'static> {
        let names: Vec<FontFamilyName<'static>> = match self {
            // No generic sans-serif after Inter: a system sans that covers
            // more scripts (e.g. DejaVu Sans) would then win over the
            // bundled script fallbacks. Uncovered characters go through
            // script fallback instead.
            FamilySpec::Ui => vec![FontFamilyName::Named(Cow::Borrowed(UI_FAMILY))],
            FamilySpec::Named(name) => vec![
                FontFamilyName::Named(Cow::Owned(name.clone())),
                FontFamilyName::Named(Cow::Borrowed(UI_FAMILY)),
            ],
            FamilySpec::SansSerif => vec![FontFamilyName::Generic(GenericFamily::SansSerif)],
            FamilySpec::Serif => vec![FontFamilyName::Generic(GenericFamily::Serif)],
            FamilySpec::Monospace => vec![FontFamilyName::Generic(GenericFamily::Monospace)],
        };
        FontFamily::List(Cow::Owned(names))
    }
}

/// An OpenType feature setting, e.g. `tnum=1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontFeature {
    pub tag: [u8; 4],
    pub value: u16,
}

impl FontFeature {
    /// Tabular figures (parameter fields, UI spec §4).
    pub const TNUM: FontFeature = FontFeature::on(*b"tnum");
    /// Inter stylistic set 1 (open digits).
    pub const SS01: FontFeature = FontFeature::on(*b"ss01");
    /// Slashed zero.
    pub const ZERO: FontFeature = FontFeature::on(*b"zero");

    #[must_use]
    pub const fn on(tag: [u8; 4]) -> Self {
        Self { tag, value: 1 }
    }

    #[must_use]
    pub const fn off(tag: [u8; 4]) -> Self {
        Self { tag, value: 0 }
    }

    fn to_parley(self) -> parley::FontFeature {
        parley::FontFeature::new(parley::setting::Tag::new(&self.tag), self.value)
    }
}

fn features_to_parley(features: &[FontFeature]) -> FontFeatures<'static> {
    FontFeatures::List(Cow::Owned(features.iter().map(|f| f.to_parley()).collect()))
}

/// Line height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineHeight {
    /// Use the font's ascent + descent + line gap, times this factor.
    Metrics(f32),
    /// A multiple of the font size (CSS unitless `line-height`).
    FontSize(f32),
    /// Absolute height in logical pixels.
    Absolute(f32),
}

impl Default for LineHeight {
    fn default() -> Self {
        LineHeight::Metrics(1.0)
    }
}

impl LineHeight {
    fn to_parley(self) -> parley::LineHeight {
        match self {
            LineHeight::Metrics(f) => parley::LineHeight::MetricsRelative(f),
            LineHeight::FontSize(f) => parley::LineHeight::FontSizeRelative(f),
            LineHeight::Absolute(px) => parley::LineHeight::Absolute(px),
        }
    }
}

/// Semantic color of a span. Placeholder for the theme's color roles
/// (UI spec §7); resolved to actual colors by the renderer/theme. It is the
/// Parley "brush", so it is carried through layout per glyph run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ColorRole {
    #[default]
    TextPrimary,
    TextSecondary,
    TextTertiary,
    Accent,
    Danger,
    Warning,
    Success,
    /// Not themed: straight sRGB `0xRRGGBBAA`.
    Srgba(u32),
}

/// The base style of a text.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub family: FamilySpec,
    /// Font size in logical pixels.
    pub size: f32,
    /// CSS weight, 1..=1000 (Inter's variable axis spans 100..=900).
    pub weight: f32,
    pub italic: bool,
    pub features: Vec<FontFeature>,
    pub line_height: LineHeight,
    /// Extra advance after each character, in logical pixels.
    pub letter_spacing: f32,
    pub color: ColorRole,
    pub underline: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: FamilySpec::Ui,
            size: 13.0,
            weight: 400.0,
            italic: false,
            features: Vec::new(),
            line_height: LineHeight::default(),
            letter_spacing: 0.0,
            color: ColorRole::TextPrimary,
            underline: false,
        }
    }
}

impl TextStyle {
    #[must_use]
    pub fn sized(size: f32) -> Self {
        Self {
            size,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = weight;
        self
    }

    #[must_use]
    pub fn with_feature(mut self, feature: FontFeature) -> Self {
        self.features.push(feature);
        self
    }

    #[must_use]
    pub fn with_color(mut self, color: ColorRole) -> Self {
        self.color = color;
        self
    }

    pub(crate) fn push_defaults(&self, b: &mut RangedBuilder<'_, ColorRole>) {
        b.push_default(StyleProperty::FontFamily(self.family.to_parley()));
        b.push_default(StyleProperty::FontSize(self.size));
        b.push_default(StyleProperty::FontWeight(FontWeight::new(self.weight)));
        b.push_default(StyleProperty::FontStyle(if self.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        }));
        b.push_default(StyleProperty::FontFeatures(features_to_parley(
            &self.features,
        )));
        b.push_default(StyleProperty::LineHeight(self.line_height.to_parley()));
        b.push_default(StyleProperty::LetterSpacing(self.letter_spacing));
        b.push_default(StyleProperty::Brush(self.color));
        b.push_default(StyleProperty::Underline(self.underline));
    }
}

/// A rich-text override applied to a byte range of the text. Unset fields
/// inherit from the base [`TextStyle`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleSpan {
    pub range: Range<usize>,
    pub family: Option<FamilySpec>,
    pub size: Option<f32>,
    pub weight: Option<f32>,
    pub italic: Option<bool>,
    /// Replaces (does not merge with) the base features.
    pub features: Option<Vec<FontFeature>>,
    pub letter_spacing: Option<f32>,
    pub color: Option<ColorRole>,
    pub underline: Option<bool>,
}

impl StyleSpan {
    #[must_use]
    pub fn new(range: Range<usize>) -> Self {
        Self {
            range,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn weight(mut self, weight: f32) -> Self {
        self.weight = Some(weight);
        self
    }

    #[must_use]
    pub fn italic(mut self, italic: bool) -> Self {
        self.italic = Some(italic);
        self
    }

    #[must_use]
    pub fn size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    #[must_use]
    pub fn color(mut self, color: ColorRole) -> Self {
        self.color = Some(color);
        self
    }

    #[must_use]
    pub fn underline(mut self, underline: bool) -> Self {
        self.underline = Some(underline);
        self
    }

    #[must_use]
    pub fn features(mut self, features: Vec<FontFeature>) -> Self {
        self.features = Some(features);
        self
    }

    #[must_use]
    pub fn family(mut self, family: FamilySpec) -> Self {
        self.family = Some(family);
        self
    }

    pub(crate) fn push(&self, b: &mut RangedBuilder<'_, ColorRole>, range: Range<usize>) {
        if let Some(family) = &self.family {
            b.push(StyleProperty::FontFamily(family.to_parley()), range.clone());
        }
        if let Some(size) = self.size {
            b.push(StyleProperty::FontSize(size), range.clone());
        }
        if let Some(weight) = self.weight {
            b.push(
                StyleProperty::FontWeight(FontWeight::new(weight)),
                range.clone(),
            );
        }
        if let Some(italic) = self.italic {
            let style = if italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            };
            b.push(StyleProperty::FontStyle(style), range.clone());
        }
        if let Some(features) = &self.features {
            b.push(
                StyleProperty::FontFeatures(features_to_parley(features)),
                range.clone(),
            );
        }
        if let Some(spacing) = self.letter_spacing {
            b.push(StyleProperty::LetterSpacing(spacing), range.clone());
        }
        if let Some(color) = self.color {
            b.push(StyleProperty::Brush(color), range.clone());
        }
        if let Some(underline) = self.underline {
            b.push(StyleProperty::Underline(underline), range);
        }
    }
}
