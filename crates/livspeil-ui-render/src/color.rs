// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Colors. Stored in OKLab (plus straight alpha); converted to gamma-encoded
//! sRGB for compositing (UI chrome is composited in gamma sRGB, see the UI
//! spec, section 3). Out-of-gamut colors are clamped per channel.

/// A color in OKLab with straight (not premultiplied) alpha.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    /// Lightness, 0..1.
    pub l: f32,
    /// Green-red axis.
    pub a: f32,
    /// Blue-yellow axis.
    pub b: f32,
    /// Alpha, 0..1.
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
    /// Opaque black.
    pub const BLACK: Color = Color {
        l: 0.0,
        a: 0.0,
        b: 0.0,
        alpha: 1.0,
    };
    /// Opaque white.
    pub const WHITE: Color = Color {
        l: 1.0,
        a: 0.0,
        b: 0.0,
        alpha: 1.0,
    };

    /// OKLCH: lightness 0..1, chroma (0..~0.37), hue in degrees.
    pub fn oklch(l: f32, c: f32, h_deg: f32) -> Color {
        let h = h_deg.to_radians();
        Color {
            l,
            a: c * h.cos(),
            b: c * h.sin(),
            alpha: 1.0,
        }
    }

    /// OKLab components.
    pub fn oklab(l: f32, a: f32, b: f32) -> Color {
        Color {
            l,
            a,
            b,
            alpha: 1.0,
        }
    }

    /// Gamma-encoded sRGB components, 0..1.
    pub fn srgb(r: f32, g: f32, b: f32) -> Color {
        let lin = [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b)];
        let [l, a, b] = linear_srgb_to_oklab(lin);
        Color {
            l,
            a,
            b,
            alpha: 1.0,
        }
    }

    /// 8-bit gamma-encoded sRGB.
    pub fn srgb8(r: u8, g: u8, b: u8) -> Color {
        Color::srgb(
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
        )
    }

    /// The same color with another alpha.
    #[must_use]
    pub fn with_alpha(self, alpha: f32) -> Color {
        Color { alpha, ..self }
    }

    /// Gamma-encoded sRGB with straight alpha, each channel clamped to 0..1.
    pub fn to_srgb(self) -> [f32; 4] {
        let [r, g, b] = oklab_to_srgb([self.l, self.a, self.b]);
        [r, g, b, self.alpha.clamp(0.0, 1.0)]
    }

    /// Gamma-encoded sRGB, premultiplied by alpha.
    pub fn to_srgb_premul(self) -> [f32; 4] {
        let [r, g, b, a] = self.to_srgb();
        [r * a, g * a, b * a, a]
    }

    /// `[L, a, b, alpha]`.
    pub fn to_array(self) -> [f32; 4] {
        [self.l, self.a, self.b, self.alpha]
    }
}

/// sRGB transfer function, decode (gamma -> linear).
pub fn srgb_to_linear(x: f32) -> f32 {
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB transfer function, encode (linear -> gamma). Input clamped to 0..1.
/// Mirrors `linear_to_srgb` in `primitives.wgsl`.
pub fn linear_to_srgb(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    if x <= 0.003_130_8 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

/// OKLab -> gamma sRGB, clamped. Mirrors `oklab_to_srgb` in `primitives.wgsl`.
pub fn oklab_to_srgb(lab: [f32; 3]) -> [f32; 3] {
    let [l, a, b] = lab;
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    let r = 4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3;
    let g = -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3;
    let bl = -0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3;
    [linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(bl)]
}

fn linear_srgb_to_oklab(rgb: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = rgb;
    let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
    let (l_, m_, s_) = (l.cbrt(), m.cbrt(), s.cbrt());
    [
        0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
        1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
        0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_round_trips_through_oklab() {
        for &(r, g, b) in &[(0u8, 0u8, 0u8), (255, 255, 255), (255, 0, 0), (12, 200, 99)] {
            let c = Color::srgb8(r, g, b).to_srgb();
            let back = [c[0] * 255.0, c[1] * 255.0, c[2] * 255.0];
            for (x, y) in back.iter().zip([r, g, b]) {
                assert!((x - f32::from(y)).abs() < 0.05, "{back:?} vs {r},{g},{b}");
            }
        }
    }

    #[test]
    fn white_is_l1() {
        let w = Color::srgb(1.0, 1.0, 1.0);
        assert!((w.l - 1.0).abs() < 1e-4 && w.a.abs() < 1e-4 && w.b.abs() < 1e-4);
    }
}
