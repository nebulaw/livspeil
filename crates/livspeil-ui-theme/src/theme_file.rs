// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! `theme.toml`: user overrides on top of the built-in tokens.
//!
//! ```toml
//! scheme = "dark"             # dark | light
//! density = "pro"             # simple | pro
//! tier = "t1"                 # t0 | t1 | t2
//! reduce_transparency = false
//!
//! spacing = [2, 4, 8, 12, 16, 24, 32]
//! type_sizes = [11, 12, 13, 15, 17, 22, 28]
//! icon_sizes = [14, 16, 20, 24]
//!
//! [radii]
//! md = 9
//!
//! [metrics.pro]
//! row_height = 22
//!
//! [elevation]
//! e2 = ["0 4 12 / 0.24", "0 1 2 / 0.2"]
//!
//! [colors.dark]
//! accent = "oklch(0.70 0.15 300)"
//! "surface.0" = "#101014"
//! ```

use std::collections::BTreeMap;
use std::ops::Range;

use serde::Deserialize;
use toml::Spanned;

use crate::color::Color;
use crate::error::ConfigError;
use crate::tokens::{
    COLOR_ROLE_NAMES, ColorRoles, Density, DensityMetrics, Scheme, Shadow, ShadowLayer, Theme, Tier,
};

/// File name used in error messages.
pub const THEME_FILE: &str = "theme.toml";

impl Theme {
    /// Parse `theme.toml` source, merging it over the default theme.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] located at the offending value.
    pub fn parse(source: &str) -> Result<Theme, ConfigError> {
        let raw: RawTheme =
            toml::from_str(source).map_err(|e| ConfigError::from_toml(THEME_FILE, source, &e))?;
        let err =
            |span: Range<usize>, msg: String| ConfigError::at(THEME_FILE, source, Some(span), msg);
        let mut theme = Theme::default();

        if let Some(s) = raw.scheme {
            theme.scheme = match s.get_ref().as_str() {
                "dark" => Scheme::Dark,
                "light" => Scheme::Light,
                other => {
                    return Err(err(
                        s.span(),
                        format!("unknown scheme `{other}` (dark, light)"),
                    ));
                }
            };
        }
        if let Some(d) = raw.density {
            theme.density = match d.get_ref().as_str() {
                "simple" => Density::Simple,
                "pro" => Density::Pro,
                other => {
                    return Err(err(
                        d.span(),
                        format!("unknown density `{other}` (simple, pro)"),
                    ));
                }
            };
        }
        if let Some(t) = raw.tier {
            theme.tier = match t.get_ref().to_ascii_lowercase().as_str() {
                "t0" => Tier::T0,
                "t1" => Tier::T1,
                "t2" => Tier::T2,
                other => {
                    return Err(err(
                        t.span(),
                        format!("unknown tier `{other}` (t0, t1, t2)"),
                    ));
                }
            };
        }
        if let Some(r) = raw.reduce_transparency {
            theme.reduce_transparency = r;
        }
        let tokens = &mut theme.tokens;
        if let Some(v) = raw.spacing {
            tokens.spacing.0 = sizes::<7>(v, &err)?;
        }
        if let Some(v) = raw.type_sizes {
            tokens.type_scale.sizes = sizes::<7>(v, &err)?;
        }
        if let Some(v) = raw.icon_sizes {
            tokens.icons.0 = sizes::<4>(v, &err)?;
        }
        if let Some(r) = raw.radii {
            let radii = &mut tokens.radii;
            for (slot, value) in [
                (&mut radii.xs, r.xs),
                (&mut radii.sm, r.sm),
                (&mut radii.md, r.md),
                (&mut radii.lg, r.lg),
                (&mut radii.window, r.window),
            ] {
                if let Some(v) = value {
                    *slot = size(&v, &err)?;
                }
            }
        }
        if let Some(d) = raw.metrics {
            if let Some(m) = d.simple {
                apply_metrics(&mut tokens.simple, m, &err)?;
            }
            if let Some(m) = d.pro {
                apply_metrics(&mut tokens.pro, m, &err)?;
            }
        }
        if let Some(e) = raw.elevation {
            for (name, layers) in e {
                let span = name.span();
                let index = match name.get_ref().as_str() {
                    "e1" => 0,
                    "e2" => 1,
                    "e3" => 2,
                    "e4" => 3,
                    other => {
                        return Err(err(span, format!("unknown elevation `{other}` (e1..e4)")));
                    }
                };
                tokens.elevations[index] = shadow(layers, &err)?;
            }
        }
        if let Some(c) = raw.colors {
            if let Some(map) = c.dark {
                apply_colors(&mut tokens.dark, map, &err)?;
            }
            if let Some(map) = c.light {
                apply_colors(&mut tokens.light, map, &err)?;
            }
        }
        Ok(theme)
    }
}

type ErrFn<'a> = dyn Fn(Range<usize>, String) -> ConfigError + 'a;

#[allow(clippy::cast_possible_truncation)]
fn size(v: &Spanned<f64>, err: &ErrFn<'_>) -> Result<f32, ConfigError> {
    let x = *v.get_ref();
    if !x.is_finite() || !(0.0..=1000.0).contains(&x) {
        return Err(err(
            v.span(),
            format!("size must be between 0 and 1000, got {x}"),
        ));
    }
    Ok(x as f32)
}

fn sizes<const N: usize>(
    v: Spanned<Vec<Spanned<f64>>>,
    err: &ErrFn<'_>,
) -> Result<[f32; N], ConfigError> {
    let span = v.span();
    let values = v.into_inner();
    if values.len() != N {
        return Err(err(
            span,
            format!("expected {N} values, got {}", values.len()),
        ));
    }
    let mut out = [0.0; N];
    for (slot, value) in out.iter_mut().zip(&values) {
        *slot = size(value, err)?;
    }
    Ok(out)
}

fn apply_metrics(
    m: &mut DensityMetrics,
    raw: RawMetrics,
    err: &ErrFn<'_>,
) -> Result<(), ConfigError> {
    if let Some(v) = raw.body {
        m.body = size(&v, err)?;
    }
    if let Some(v) = raw.body_large {
        m.body_large = size(&v, err)?;
    }
    if let Some(v) = raw.row_height {
        m.row_height = size(&v, err)?;
    }
    Ok(())
}

fn shadow(layers: Spanned<Vec<Spanned<String>>>, err: &ErrFn<'_>) -> Result<Shadow, ConfigError> {
    let span = layers.span();
    let layers = layers.into_inner();
    let parsed = layers
        .iter()
        .map(|l| parse_layer(l.get_ref()).map_err(|m| err(l.span(), m)))
        .collect::<Result<Vec<_>, _>>()?;
    match parsed.as_slice() {
        [key] => Ok(Shadow {
            key: *key,
            contact: None,
        }),
        [key, contact] => Ok(Shadow {
            key: *key,
            contact: Some(*contact),
        }),
        _ => Err(err(span, "a shadow has one or two layers".to_owned())),
    }
}

/// `"x y blur [spread] [/ alpha]"`.
fn parse_layer(text: &str) -> Result<ShadowLayer, String> {
    let (geometry, alpha) = match text.split_once('/') {
        Some((g, a)) => (g, Some(a.trim())),
        None => (text, None),
    };
    let nums = geometry
        .split_whitespace()
        .map(str::parse::<f32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            format!("shadow layer `{text}`: expected numbers `x y blur [spread] [/ alpha]`")
        })?;
    let alpha = match alpha {
        Some(a) => a
            .parse::<f32>()
            .ok()
            .filter(|a| (0.0..=1.0).contains(a))
            .ok_or_else(|| format!("shadow layer `{text}`: alpha must be in 0..1"))?,
        None => 0.25,
    };
    match nums.as_slice() {
        [x, y, blur] => Ok(ShadowLayer::new(*x, *y, *blur, alpha)),
        [x, y, blur, spread] => Ok(ShadowLayer {
            spread: *spread,
            ..ShadowLayer::new(*x, *y, *blur, alpha)
        }),
        _ => Err(format!(
            "shadow layer `{text}`: expected `x y blur [spread] [/ alpha]`"
        )),
    }
}

fn apply_colors(
    roles: &mut ColorRoles,
    map: BTreeMap<Spanned<String>, Spanned<String>>,
    err: &ErrFn<'_>,
) -> Result<(), ConfigError> {
    for (name, value) in map {
        let Some(slot) = roles.role_mut(name.get_ref()) else {
            return Err(err(
                name.span(),
                format!(
                    "unknown color role `{}` (known: {})",
                    name.get_ref(),
                    COLOR_ROLE_NAMES.join(", ")
                ),
            ));
        };
        *slot = Color::parse(value.get_ref()).map_err(|e| err(value.span(), e.to_string()))?;
    }
    Ok(())
}

type RawElevations = BTreeMap<Spanned<String>, Spanned<Vec<Spanned<String>>>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTheme {
    scheme: Option<Spanned<String>>,
    density: Option<Spanned<String>>,
    tier: Option<Spanned<String>>,
    reduce_transparency: Option<bool>,
    spacing: Option<Spanned<Vec<Spanned<f64>>>>,
    type_sizes: Option<Spanned<Vec<Spanned<f64>>>>,
    icon_sizes: Option<Spanned<Vec<Spanned<f64>>>>,
    radii: Option<RawRadii>,
    elevation: Option<RawElevations>,
    colors: Option<RawColors>,
    metrics: Option<RawDensities>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDensities {
    simple: Option<RawMetrics>,
    pro: Option<RawMetrics>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMetrics {
    body: Option<Spanned<f64>>,
    body_large: Option<Spanned<f64>>,
    row_height: Option<Spanned<f64>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRadii {
    xs: Option<Spanned<f64>>,
    sm: Option<Spanned<f64>>,
    md: Option<Spanned<f64>>,
    lg: Option<Spanned<f64>>,
    window: Option<Spanned<f64>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawColors {
    dark: Option<BTreeMap<Spanned<String>, Spanned<String>>>,
    light: Option<BTreeMap<Spanned<String>, Spanned<String>>>,
}
