// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Design tokens (UI spec section 7) and materials (section 8).

use crate::color::Color;

/// Light or dark color scheme. Dark is the default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Scheme {
    /// Dark-first default.
    #[default]
    Dark,
    /// Light scheme.
    Light,
}

/// Information density.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Density {
    /// Roomier: 13/15 body, 32 px rows.
    #[default]
    Simple,
    /// Denser tools: 12/13 body, 24 px rows.
    Pro,
}

/// Material quality tier (GPU budget).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub enum Tier {
    /// No blur: opaque tints.
    T0,
    /// Reduced blur, no saturation boost.
    T1,
    /// Full recipe.
    #[default]
    T2,
}

/// Spacing scale: 2, 4, 8, 12, 16, 24, 32.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing(pub [f32; 7]);

impl Spacing {
    /// Step `i` of the scale (clamped to the last step).
    #[must_use]
    pub fn step(&self, i: usize) -> f32 {
        self.0[i.min(self.0.len() - 1)]
    }
}

/// Corner radii (continuous corners).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Radii {
    /// 4
    pub xs: f32,
    /// 6
    pub sm: f32,
    /// 10
    pub md: f32,
    /// 14
    pub lg: f32,
    /// 12
    pub window: f32,
}

/// Type scale (Inter).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeScale {
    /// 11, 12, 13, 15, 17, 22, 28.
    pub sizes: [f32; 7],
    /// 400, 500, 600.
    pub weights: [u16; 3],
}

/// Metrics that depend on [`Density`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DensityMetrics {
    /// Body text size.
    pub body: f32,
    /// Emphasized body / control label size.
    pub body_large: f32,
    /// List, table and control row height.
    pub row_height: f32,
}

/// Icon sizes: 14, 16, 20, 24.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconSizes(pub [f32; 4]);

/// Semantic color roles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorRoles {
    /// `surface.0..3`, from the window background up.
    pub surface: [Color; 4],
    /// High-emphasis text.
    pub text_primary: Color,
    /// 62% text.
    pub text_secondary: Color,
    /// 42% text.
    pub text_tertiary: Color,
    /// Accent before contrast clamping (see [`Theme::accent`]).
    pub accent: Color,
    /// Destructive actions.
    pub danger: Color,
    /// Warnings.
    pub warning: Color,
    /// Success.
    pub success: Color,
    /// Keyboard focus ring.
    pub focus_ring: Color,
}

/// Role names as used in `theme.toml`.
pub const COLOR_ROLE_NAMES: [&str; 12] = [
    "surface.0",
    "surface.1",
    "surface.2",
    "surface.3",
    "text.primary",
    "text.secondary",
    "text.tertiary",
    "accent",
    "danger",
    "warning",
    "success",
    "focus-ring",
];

impl ColorRoles {
    /// The role called `name` (see [`COLOR_ROLE_NAMES`]).
    pub fn role_mut(&mut self, name: &str) -> Option<&mut Color> {
        Some(match name {
            "surface.0" => &mut self.surface[0],
            "surface.1" => &mut self.surface[1],
            "surface.2" => &mut self.surface[2],
            "surface.3" => &mut self.surface[3],
            "text.primary" => &mut self.text_primary,
            "text.secondary" => &mut self.text_secondary,
            "text.tertiary" => &mut self.text_tertiary,
            "accent" => &mut self.accent,
            "danger" => &mut self.danger,
            "warning" => &mut self.warning,
            "success" => &mut self.success,
            "focus-ring" => &mut self.focus_ring,
            _ => return None,
        })
    }

    /// Dark-first defaults.
    #[must_use]
    pub fn dark() -> Self {
        let text = Color::oklch(0.97, 0.0, 0.0, 1.0);
        ColorRoles {
            surface: [0.14, 0.17, 0.20, 0.24].map(|l| Color::oklch(l, 0.008, 265.0, 1.0)),
            text_primary: text.with_alpha(0.92),
            text_secondary: text.with_alpha(0.62),
            text_tertiary: text.with_alpha(0.42),
            accent: Color::oklch(0.68, 0.16, 255.0, 1.0),
            danger: Color::oklch(0.66, 0.20, 25.0, 1.0),
            warning: Color::oklch(0.80, 0.15, 80.0, 1.0),
            success: Color::oklch(0.72, 0.15, 150.0, 1.0),
            focus_ring: Color::oklch(0.74, 0.15, 255.0, 0.9),
        }
    }

    /// Light defaults.
    #[must_use]
    pub fn light() -> Self {
        let text = Color::oklch(0.16, 0.0, 0.0, 1.0);
        ColorRoles {
            surface: [0.97, 0.95, 0.92, 0.89].map(|l| Color::oklch(l, 0.004, 265.0, 1.0)),
            text_primary: text.with_alpha(0.90),
            text_secondary: text.with_alpha(0.62),
            text_tertiary: text.with_alpha(0.42),
            accent: Color::oklch(0.56, 0.18, 255.0, 1.0),
            danger: Color::oklch(0.56, 0.21, 25.0, 1.0),
            warning: Color::oklch(0.68, 0.16, 70.0, 1.0),
            success: Color::oklch(0.58, 0.15, 150.0, 1.0),
            focus_ring: Color::oklch(0.58, 0.17, 255.0, 0.9),
        }
    }
}

/// One shadow layer: offset, blur radius, spread and black at `alpha`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowLayer {
    /// Horizontal offset.
    pub x: f32,
    /// Vertical offset.
    pub y: f32,
    /// Blur radius.
    pub blur: f32,
    /// Spread.
    pub spread: f32,
    /// Opacity of the (black) shadow.
    pub alpha: f32,
}

impl ShadowLayer {
    /// A layer.
    #[must_use]
    pub const fn new(x: f32, y: f32, blur: f32, alpha: f32) -> Self {
        ShadowLayer {
            x,
            y,
            blur,
            spread: 0.0,
            alpha,
        }
    }
}

/// A two-layer shadow: a large soft key layer and a tight contact layer.
/// `e1` has only one layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    /// Key (large) layer.
    pub key: ShadowLayer,
    /// Contact (small) layer.
    pub contact: Option<ShadowLayer>,
}

/// Elevation levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Elevation {
    /// `0 1 2`
    E1,
    /// `0 4 12` + `0 1 2`
    E2,
    /// `0 12 32` + `0 2 6`
    E3,
    /// `0 24 64` + `0 4 12`
    E4,
}

impl Elevation {
    /// 0-based index.
    #[must_use]
    pub fn index(self) -> usize {
        self as usize
    }
}

/// Surface materials.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Material {
    /// Dense tools, inspectors.
    Solid,
    /// Menubar, docks, sidebars.
    Frost,
    /// Tooltips, menus.
    Glass,
    /// Text over the scene: no surface.
    Ink,
}

/// A concrete material recipe for one tier, ready for the renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialRecipe {
    /// Backdrop blur radius in px; 0 = no backdrop sampling.
    pub blur: f32,
    /// Tint fill (alpha = tint strength); transparent for Ink.
    pub tint: Color,
    /// Backdrop saturation multiplier.
    pub saturate: f32,
    /// Inner highlight (white) alpha, whole hairline.
    pub highlight: f32,
    /// Top rim highlight (white) alpha.
    pub rim: f32,
    /// Hairline border (white) alpha.
    pub hairline: f32,
    /// Shadow.
    pub elevation: Option<Elevation>,
    /// Subtle text shadow (Ink).
    pub text_shadow: bool,
}

/// All tokens.
#[derive(Clone, Debug, PartialEq)]
pub struct Tokens {
    /// Spacing scale.
    pub spacing: Spacing,
    /// Radii.
    pub radii: Radii,
    /// Type scale.
    pub type_scale: TypeScale,
    /// Metrics for Simple density.
    pub simple: DensityMetrics,
    /// Metrics for Pro density.
    pub pro: DensityMetrics,
    /// Icon sizes.
    pub icons: IconSizes,
    /// Shadows for e1..e4.
    pub elevations: [Shadow; 4],
    /// Dark scheme colors.
    pub dark: ColorRoles,
    /// Light scheme colors.
    pub light: ColorRoles,
}

impl Default for Tokens {
    fn default() -> Self {
        Tokens {
            spacing: Spacing([2.0, 4.0, 8.0, 12.0, 16.0, 24.0, 32.0]),
            radii: Radii {
                xs: 4.0,
                sm: 6.0,
                md: 10.0,
                lg: 14.0,
                window: 12.0,
            },
            type_scale: TypeScale {
                sizes: [11.0, 12.0, 13.0, 15.0, 17.0, 22.0, 28.0],
                weights: [400, 500, 600],
            },
            simple: DensityMetrics {
                body: 13.0,
                body_large: 15.0,
                row_height: 32.0,
            },
            pro: DensityMetrics {
                body: 12.0,
                body_large: 13.0,
                row_height: 24.0,
            },
            icons: IconSizes([14.0, 16.0, 20.0, 24.0]),
            elevations: [
                Shadow {
                    key: ShadowLayer::new(0.0, 1.0, 2.0, 0.30),
                    contact: None,
                },
                Shadow {
                    key: ShadowLayer::new(0.0, 4.0, 12.0, 0.24),
                    contact: Some(ShadowLayer::new(0.0, 1.0, 2.0, 0.20)),
                },
                Shadow {
                    key: ShadowLayer::new(0.0, 12.0, 32.0, 0.28),
                    contact: Some(ShadowLayer::new(0.0, 2.0, 6.0, 0.20)),
                },
                Shadow {
                    key: ShadowLayer::new(0.0, 24.0, 64.0, 0.32),
                    contact: Some(ShadowLayer::new(0.0, 4.0, 12.0, 0.22)),
                },
            ],
            dark: ColorRoles::dark(),
            light: ColorRoles::light(),
        }
    }
}

/// The resolved theme: tokens plus the user's choices.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Theme {
    /// Token values.
    pub tokens: Tokens,
    /// Color scheme.
    pub scheme: Scheme,
    /// Density.
    pub density: Density,
    /// Material tier.
    pub tier: Tier,
    /// Reduce transparency: materials use opaque tints.
    pub reduce_transparency: bool,
}

impl Theme {
    /// Colors for the active scheme.
    #[must_use]
    pub fn colors(&self) -> &ColorRoles {
        match self.scheme {
            Scheme::Dark => &self.tokens.dark,
            Scheme::Light => &self.tokens.light,
        }
    }

    /// Density metrics for the active density.
    #[must_use]
    pub fn metrics(&self) -> DensityMetrics {
        match self.density {
            Density::Simple => self.tokens.simple,
            Density::Pro => self.tokens.pro,
        }
    }

    /// The shadow for `elevation`.
    #[must_use]
    pub fn shadow(&self, elevation: Elevation) -> Shadow {
        self.tokens.elevations[elevation.index()]
    }

    /// The accent, with its lightness pushed away from `surface.1` until the
    /// difference is at least 0.30 (adaptive accent, clamped for contrast).
    #[must_use]
    pub fn accent(&self) -> Color {
        clamp_contrast(self.colors().accent, self.colors().surface[1], 0.30)
    }

    /// The recipe for `material` at the active tier.
    #[must_use]
    pub fn material(&self, material: Material) -> MaterialRecipe {
        material_recipe(material, self.tier, self.reduce_transparency, self.colors())
    }
}

/// Move `color`'s lightness away from `background` so they differ by at
/// least `min_delta` (keeping chroma and hue).
#[must_use]
pub fn clamp_contrast(color: Color, background: Color, min_delta: f32) -> Color {
    let delta = color.l - background.l;
    if delta.abs() >= min_delta {
        return color;
    }
    let target = if background.l < 0.5 {
        (background.l + min_delta).min(1.0)
    } else {
        (background.l - min_delta).max(0.0)
    };
    Color { l: target, ..color }
}

/// The recipe for `material` at `tier`. "Reduce transparency" and T0 use an
/// opaque tint; T1 halves the blur and drops the saturation boost.
#[must_use]
pub fn material_recipe(
    material: Material,
    tier: Tier,
    reduce_transparency: bool,
    colors: &ColorRoles,
) -> MaterialRecipe {
    let opaque = reduce_transparency || tier == Tier::T0;
    let mut r = match material {
        Material::Solid => MaterialRecipe {
            blur: 0.0,
            tint: colors.surface[1],
            saturate: 1.0,
            highlight: 0.0,
            rim: 0.0,
            hairline: 0.06,
            elevation: Some(Elevation::E1),
            text_shadow: false,
        },
        Material::Frost => MaterialRecipe {
            blur: 24.0,
            tint: colors.surface[1].with_alpha(0.72),
            saturate: 1.3,
            highlight: 0.08,
            rim: 0.0,
            hairline: 0.06,
            elevation: Some(Elevation::E2),
            text_shadow: false,
        },
        Material::Glass => MaterialRecipe {
            blur: 16.0,
            tint: colors.surface[2].with_alpha(0.5),
            saturate: 1.0,
            highlight: 0.0,
            rim: 0.12,
            hairline: 0.06,
            elevation: Some(Elevation::E3),
            text_shadow: false,
        },
        Material::Ink => MaterialRecipe {
            blur: 0.0,
            tint: Color::TRANSPARENT,
            saturate: 1.0,
            highlight: 0.0,
            rim: 0.0,
            hairline: 0.0,
            elevation: None,
            text_shadow: true,
        },
    };
    if material == Material::Ink {
        return r;
    }
    if opaque {
        r.blur = 0.0;
        r.saturate = 1.0;
        r.tint = r.tint.with_alpha(1.0);
    } else if tier == Tier::T1 {
        r.blur *= 0.5;
        r.saturate = 1.0;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_tiers_degrade() {
        let colors = ColorRoles::dark();
        let t2 = material_recipe(Material::Frost, Tier::T2, false, &colors);
        assert_eq!(t2.blur, 24.0);
        assert!((t2.saturate - 1.3).abs() < 1e-6);
        let t1 = material_recipe(Material::Frost, Tier::T1, false, &colors);
        assert_eq!(t1.blur, 12.0);
        assert_eq!(t1.saturate, 1.0);
        let t0 = material_recipe(Material::Frost, Tier::T0, false, &colors);
        assert_eq!(t0.blur, 0.0);
        assert_eq!(t0.tint.alpha, 1.0);
        let rt = material_recipe(Material::Glass, Tier::T2, true, &colors);
        assert_eq!(rt.blur, 0.0);
        assert_eq!(rt.tint.alpha, 1.0);
    }

    #[test]
    fn accent_is_clamped_for_contrast() {
        let mut theme = Theme::default();
        theme.tokens.dark.accent = Color::oklch(0.25, 0.1, 250.0, 1.0);
        let a = theme.accent();
        assert!((a.l - theme.colors().surface[1].l).abs() >= 0.299);
    }

    #[test]
    fn density_metrics() {
        let mut theme = Theme::default();
        assert_eq!(theme.metrics().row_height, 32.0);
        theme.density = Density::Pro;
        assert_eq!(theme.metrics().row_height, 24.0);
        assert_eq!(theme.metrics().body, 12.0);
    }
}
