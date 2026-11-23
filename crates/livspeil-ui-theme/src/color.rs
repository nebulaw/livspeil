// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Colors. Stored in OKLab (perceptual, linear for interpolation) with a
//! straight alpha; authored in OKLCH; converted to sRGB at the renderer.

use std::fmt;

/// A color in OKLab with straight (non-premultiplied) alpha.
///
/// OKLab is the interpolation space for animated colors: a straight line
/// between two OKLab colors looks even, without the muddy midpoints of sRGB.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Color {
    /// Perceptual lightness, 0..=1.
    pub l: f32,
    /// Green-red axis.
    pub a: f32,
    /// Blue-yellow axis.
    pub b: f32,
    /// Opacity, 0..=1.
    pub alpha: f32,
}

impl Color {
    /// Fully transparent black.
    pub const TRANSPARENT: Color = Color {
        l: 0.0,
        a: 0.0,
        b: 0.0,
        alpha: 0.0,
    };
    /// Opaque white.
    pub const WHITE: Color = Color {
        l: 1.0,
        a: 0.0,
        b: 0.0,
        alpha: 1.0,
    };
    /// Opaque black.
    pub const BLACK: Color = Color {
        l: 0.0,
        a: 0.0,
        b: 0.0,
        alpha: 1.0,
    };

    /// A color from OKLab components.
    #[must_use]
    pub const fn oklab(l: f32, a: f32, b: f32, alpha: f32) -> Self {
        Color { l, a, b, alpha }
    }

    /// A color from OKLCH: lightness 0..=1, chroma (0..~0.4), hue in degrees.
    #[must_use]
    pub fn oklch(l: f32, c: f32, h_deg: f32, alpha: f32) -> Self {
        let h = h_deg.to_radians();
        Color {
            l,
            a: c * h.cos(),
            b: c * h.sin(),
            alpha,
        }
    }

    /// A color from gamma-encoded sRGB components in 0..=1.
    #[must_use]
    pub fn srgb(r: f32, g: f32, b: f32, alpha: f32) -> Self {
        let [r, g, b] = [r, g, b].map(srgb_to_linear);
        Self::from_linear_srgb(r, g, b, alpha)
    }

    /// A color from 8-bit sRGB.
    #[must_use]
    pub fn srgb8(r: u8, g: u8, b: u8, alpha: u8) -> Self {
        let f = |v: u8| f32::from(v) / 255.0;
        Self::srgb(f(r), f(g), f(b), f(alpha))
    }

    /// A color from linear-light sRGB components.
    #[must_use]
    pub fn from_linear_srgb(r: f32, g: f32, b: f32, alpha: f32) -> Self {
        let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
        let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
        let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
        let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
        Color {
            l: 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
            a: 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
            b: 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
            alpha,
        }
    }

    /// Linear-light sRGB components (may fall outside 0..=1 for colors out
    /// of the sRGB gamut) and alpha.
    #[must_use]
    pub fn to_linear_srgb(self) -> [f32; 4] {
        let l = self.l + 0.396_337_78 * self.a + 0.215_803_76 * self.b;
        let m = self.l - 0.105_561_346 * self.a - 0.063_854_17 * self.b;
        let s = self.l - 0.089_484_18 * self.a - 1.291_485_5 * self.b;
        let (l, m, s) = (l * l * l, m * m * m, s * s * s);
        [
            4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
            -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
            -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
            self.alpha,
        ]
    }

    /// Gamma-encoded sRGB components clamped to 0..=1, and alpha.
    #[must_use]
    pub fn to_srgb(self) -> [f32; 4] {
        let [r, g, b, a] = self.to_linear_srgb();
        [
            linear_to_srgb(r.clamp(0.0, 1.0)),
            linear_to_srgb(g.clamp(0.0, 1.0)),
            linear_to_srgb(b.clamp(0.0, 1.0)),
            a.clamp(0.0, 1.0),
        ]
    }

    /// OKLCH components: lightness, chroma, hue in degrees (0..360).
    #[must_use]
    pub fn to_oklch(self) -> [f32; 3] {
        let c = self.a.hypot(self.b);
        let mut h = self.b.atan2(self.a).to_degrees();
        if h < 0.0 {
            h += 360.0;
        }
        if c < 1e-6 {
            h = 0.0;
        }
        [self.l, c, h]
    }

    /// The same color with a different alpha.
    #[must_use]
    pub const fn with_alpha(self, alpha: f32) -> Self {
        Color { alpha, ..self }
    }

    /// The same color with its alpha multiplied by `factor`.
    #[must_use]
    pub fn fade(self, factor: f32) -> Self {
        Color {
            alpha: self.alpha * factor,
            ..self
        }
    }

    /// Interpolate in OKLab.
    #[must_use]
    pub fn lerp(self, to: Color, t: f32) -> Color {
        Color {
            l: self.l + (to.l - self.l) * t,
            a: self.a + (to.a - self.a) * t,
            b: self.b + (to.b - self.b) * t,
            alpha: self.alpha + (to.alpha - self.alpha) * t,
        }
    }

    /// Parse `oklch(L C H)`, `oklch(L C H / A)`, `#rrggbb` or `#rrggbbaa`.
    /// `L` and `A` accept a trailing `%`.
    ///
    /// # Errors
    /// Returns a [`ColorParseError`] describing what is wrong.
    pub fn parse(text: &str) -> Result<Color, ColorParseError> {
        let text = text.trim();
        if let Some(hex) = text.strip_prefix('#') {
            return parse_hex(hex);
        }
        let inner = text
            .strip_prefix("oklch(")
            .and_then(|rest| rest.strip_suffix(')'))
            .ok_or(ColorParseError::Syntax)?;
        let (channels, alpha) = match inner.split_once('/') {
            Some((c, a)) => (c, Some(a.trim())),
            None => (inner, None),
        };
        let parts: Vec<&str> = channels.split_whitespace().collect();
        let [l, c, h] = parts.as_slice() else {
            return Err(ColorParseError::Syntax);
        };
        let l = parse_number(l, true)?;
        let c = parse_number(c, false)?;
        let h = parse_number(h.trim_end_matches("deg"), false)?;
        let alpha = match alpha {
            Some(a) => parse_number(a, true)?,
            None => 1.0,
        };
        if !(0.0..=1.0).contains(&l) || !(0.0..=1.0).contains(&alpha) || c < 0.0 {
            return Err(ColorParseError::OutOfRange);
        }
        Ok(Color::oklch(l, c, h, alpha))
    }
}

impl fmt::Display for Color {
    /// Formats as `oklch(L C H / A)` with three decimals.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [l, c, h] = self.to_oklch();
        write!(f, "oklch({l:.3} {c:.3} {h:.1} / {:.3})", self.alpha)
    }
}

/// Why a color string could not be parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorParseError {
    /// Not `oklch(...)` or `#hex`.
    Syntax,
    /// A component is not a number.
    Number,
    /// A component is outside its range.
    OutOfRange,
}

impl fmt::Display for ColorParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ColorParseError::Syntax => {
                "expected `oklch(L C H)`, `oklch(L C H / A)`, `#rrggbb` or `#rrggbbaa`"
            }
            ColorParseError::Number => "color component is not a number",
            ColorParseError::OutOfRange => "color component out of range (L and A in 0..1, C >= 0)",
        })
    }
}

impl std::error::Error for ColorParseError {}

fn parse_number(text: &str, allow_percent: bool) -> Result<f32, ColorParseError> {
    let text = text.trim();
    if let Some(p) = text.strip_suffix('%') {
        if !allow_percent {
            return Err(ColorParseError::Number);
        }
        return p
            .parse::<f32>()
            .map(|v| v / 100.0)
            .map_err(|_| ColorParseError::Number);
    }
    let v = text.parse::<f32>().map_err(|_| ColorParseError::Number)?;
    if v.is_finite() {
        Ok(v)
    } else {
        Err(ColorParseError::Number)
    }
}

fn parse_hex(hex: &str) -> Result<Color, ColorParseError> {
    if !(hex.len() == 6 || hex.len() == 8) || !hex.is_ascii() {
        return Err(ColorParseError::Syntax);
    }
    let byte = |i: usize| {
        hex.get(i..i + 2)
            .and_then(|s| u8::from_str_radix(s, 16).ok())
            .ok_or(ColorParseError::Number)
    };
    let alpha = if hex.len() == 8 { byte(6)? } else { 255 };
    Ok(Color::srgb8(byte(0)?, byte(2)?, byte(4)?, alpha))
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_and_black_round_trip() {
        let w = Color::srgb(1.0, 1.0, 1.0, 1.0);
        assert!((w.l - 1.0).abs() < 1e-3, "{w:?}");
        assert!(w.a.abs() < 1e-3 && w.b.abs() < 1e-3);
        let [r, g, b, _] = w.to_srgb();
        assert!((r - 1.0).abs() < 1e-3 && (g - 1.0).abs() < 1e-3 && (b - 1.0).abs() < 1e-3);
        let k = Color::srgb(0.0, 0.0, 0.0, 1.0);
        assert!(k.l.abs() < 1e-4);
    }

    #[test]
    fn srgb_round_trip() {
        for (r, g, b) in [(0.2, 0.4, 0.8), (0.9, 0.1, 0.3), (0.5, 0.5, 0.5)] {
            let [r2, g2, b2, _] = Color::srgb(r, g, b, 1.0).to_srgb();
            assert!((r - r2).abs() < 1e-3 && (g - g2).abs() < 1e-3 && (b - b2).abs() < 1e-3);
        }
    }

    #[test]
    fn parses_oklch_and_hex() {
        let c = Color::parse("oklch(0.7 0.15 250 / 50%)").unwrap();
        let [l, ch, h] = c.to_oklch();
        assert!((l - 0.7).abs() < 1e-5 && (ch - 0.15).abs() < 1e-5 && (h - 250.0).abs() < 1e-3);
        assert!((c.alpha - 0.5).abs() < 1e-6);
        let hex = Color::parse("#ff000080").unwrap();
        assert!((hex.alpha - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(Color::parse("rgb(1,2,3)"), Err(ColorParseError::Syntax));
        assert_eq!(
            Color::parse("oklch(1.5 0 0)"),
            Err(ColorParseError::OutOfRange)
        );
        assert_eq!(Color::parse("oklch(a 0 0)"), Err(ColorParseError::Number));
    }
}
