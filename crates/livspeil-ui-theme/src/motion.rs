// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Motion profiles and `motion.toml`.
//!
//! ```toml
//! profile = "snappy"          # snappy | fluid | calm | off (or a custom profile)
//! speed = 1.0                 # global multiplier; 0 = instant
//! reduce_motion = false       # replaces movement with fades
//!
//! [profiles.snappy]
//! micro    = { response = 0.10, damping = 0.90 }
//! standard = { response = 0.16, damping = 0.86 }
//! large    = { response = 0.26, damping = 0.82 }
//!
//! [overrides.windows]
//! speed = 1.2
//! ```

use std::collections::BTreeMap;

use serde::Deserialize;
use toml::Spanned;

use crate::error::ConfigError;

/// File name used in error messages.
pub const MOTION_FILE: &str = "motion.toml";

/// The three motion classes of a profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MotionClass {
    /// Hover, press, toggle.
    Micro,
    /// Menus, popovers, selection.
    Standard,
    /// Panels, window tokens, camera moves. At most two run at once.
    Large,
}

/// What an animated property does visually; decides the Reduce Motion path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PropertyKind {
    /// Position, scale, rotation: replaced by an instant change under Reduce Motion.
    Movement,
    /// Color and opacity: kept, but without overshoot, under Reduce Motion.
    Appearance,
}

/// Spring parameters: `response` is the period in seconds of the undamped
/// oscillation (how quickly it gets there), `damping` the damping ratio
/// (1 = critically damped, < 1 overshoots).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringSpec {
    /// Seconds; 0 means instant.
    pub response: f32,
    /// Damping ratio, > 0.
    pub damping: f32,
}

impl SpringSpec {
    /// A spring spec.
    #[must_use]
    pub const fn new(response: f32, damping: f32) -> Self {
        SpringSpec { response, damping }
    }

    /// The same spring without overshoot (damping at least 1). Shared by the
    /// Calm profile and Reduce Motion.
    #[must_use]
    pub fn without_overshoot(self) -> Self {
        SpringSpec {
            response: self.response,
            damping: self.damping.max(1.0),
        }
    }
}

/// Springs for the three motion classes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionProfile {
    /// Hover, press, toggle.
    pub micro: SpringSpec,
    /// Menus, popovers, selection.
    pub standard: SpringSpec,
    /// Panels, window tokens, camera moves.
    pub large: SpringSpec,
}

impl MotionProfile {
    /// The spring for `class`.
    #[must_use]
    pub fn spring(&self, class: MotionClass) -> SpringSpec {
        match class {
            MotionClass::Micro => self.micro,
            MotionClass::Standard => self.standard,
            MotionClass::Large => self.large,
        }
    }

    /// Exact values from the UI spec (section 6).
    pub const SNAPPY: MotionProfile = MotionProfile {
        micro: SpringSpec::new(0.10, 0.90),
        standard: SpringSpec::new(0.16, 0.86),
        large: SpringSpec::new(0.26, 0.82),
    };
    /// Slower and slightly bouncier.
    pub const FLUID: MotionProfile = MotionProfile {
        micro: SpringSpec::new(0.14, 0.80),
        standard: SpringSpec::new(0.22, 0.78),
        large: SpringSpec::new(0.34, 0.76),
    };
    /// Slow and critically damped: never overshoots.
    pub const CALM: MotionProfile = MotionProfile {
        micro: SpringSpec::new(0.18, 1.0),
        standard: SpringSpec::new(0.28, 1.0),
        large: SpringSpec::new(0.42, 1.0),
    };
    /// Everything instant.
    pub const OFF: MotionProfile = MotionProfile {
        micro: SpringSpec::new(0.0, 1.0),
        standard: SpringSpec::new(0.0, 1.0),
        large: SpringSpec::new(0.0, 1.0),
    };

    /// The built-in profile called `name`, if any.
    #[must_use]
    pub fn builtin(name: &str) -> Option<MotionProfile> {
        match name {
            "snappy" => Some(Self::SNAPPY),
            "fluid" => Some(Self::FLUID),
            "calm" => Some(Self::CALM),
            "off" => Some(Self::OFF),
            _ => None,
        }
    }
}

/// Per-scope adjustments (`[overrides.<scope>]`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ScopeOverride {
    /// Use another profile in this scope.
    pub profile: Option<String>,
    /// Multiplies the global speed in this scope.
    pub speed: Option<f32>,
    /// Replaces the global `reduce_motion` in this scope.
    pub reduce_motion: Option<bool>,
}

/// How a property should move, after profile, speed and Reduce Motion are applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MotionSpec {
    /// Jump to the target.
    Instant,
    /// Animate with this spring (`response` already divided by speed).
    Spring(SpringSpec),
}

/// The parsed `motion.toml`, merged over the built-in defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct MotionConfig {
    /// Active profile name.
    pub profile: String,
    /// Global speed multiplier; 0 = instant.
    pub speed: f32,
    /// Replace movement with fades.
    pub reduce_motion: bool,
    /// All profiles: built-ins (possibly overridden) and custom ones.
    pub profiles: BTreeMap<String, MotionProfile>,
    /// Per-scope overrides.
    pub overrides: BTreeMap<String, ScopeOverride>,
}

impl Default for MotionConfig {
    fn default() -> Self {
        let profiles = ["snappy", "fluid", "calm", "off"]
            .into_iter()
            .filter_map(|n| MotionProfile::builtin(n).map(|p| (n.to_owned(), p)))
            .collect();
        MotionConfig {
            profile: "snappy".to_owned(),
            speed: 1.0,
            reduce_motion: false,
            profiles,
            overrides: BTreeMap::new(),
        }
    }
}

impl MotionConfig {
    /// The active profile (falls back to Snappy if the name is unknown,
    /// which parsing never allows).
    #[must_use]
    pub fn active_profile(&self) -> MotionProfile {
        self.profile_named(&self.profile)
    }

    fn profile_named(&self, name: &str) -> MotionProfile {
        self.profiles
            .get(name)
            .copied()
            .or_else(|| MotionProfile::builtin(name))
            .unwrap_or(MotionProfile::SNAPPY)
    }

    /// Effective speed in `scope` (global speed times the scope's override).
    #[must_use]
    pub fn speed(&self, scope: Option<&str>) -> f32 {
        let factor = scope
            .and_then(|s| self.overrides.get(s))
            .and_then(|o| o.speed)
            .unwrap_or(1.0);
        self.speed * factor
    }

    /// Whether Reduce Motion is on in `scope`.
    #[must_use]
    pub fn reduce_motion(&self, scope: Option<&str>) -> bool {
        scope
            .and_then(|s| self.overrides.get(s))
            .and_then(|o| o.reduce_motion)
            .unwrap_or(self.reduce_motion)
    }

    /// Resolve how a property of `kind` in motion `class` animates in `scope`.
    ///
    /// - speed 0 (global or scoped) or the `off` profile: instant;
    /// - Reduce Motion: movement is instant; appearance uses the Calm
    ///   profile's spring for `class`, without overshoot (the same path the
    ///   Calm profile takes);
    /// - otherwise the profile's spring with `response / speed`.
    #[must_use]
    pub fn resolve(
        &self,
        scope: Option<&str>,
        class: MotionClass,
        kind: PropertyKind,
    ) -> MotionSpec {
        let over = scope.and_then(|s| self.overrides.get(s));
        let profile_name = over
            .and_then(|o| o.profile.as_deref())
            .unwrap_or(&self.profile);
        let speed = self.speed(scope);
        if speed <= 0.0 || !speed.is_finite() {
            return MotionSpec::Instant;
        }
        let spring = if self.reduce_motion(scope) {
            if kind == PropertyKind::Movement {
                return MotionSpec::Instant;
            }
            self.profile_named("calm").spring(class).without_overshoot()
        } else {
            self.profile_named(profile_name).spring(class)
        };
        if spring.response <= 0.0 {
            return MotionSpec::Instant;
        }
        MotionSpec::Spring(SpringSpec {
            response: spring.response / speed,
            damping: spring.damping,
        })
    }

    /// Parse `motion.toml` source, merging it over the defaults.
    ///
    /// Parsing is strict: unknown keys, wrong types, unknown profile names
    /// and out-of-range numbers are errors with a line and column.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] located at the offending value.
    pub fn parse(source: &str) -> Result<MotionConfig, ConfigError> {
        let raw: RawMotion =
            toml::from_str(source).map_err(|e| ConfigError::from_toml(MOTION_FILE, source, &e))?;
        let err = |span: std::ops::Range<usize>, msg: String| {
            ConfigError::at(MOTION_FILE, source, Some(span), msg)
        };
        let mut config = MotionConfig::default();

        for (name, raw_profile) in raw.profiles {
            let span = raw_profile.span();
            let raw_profile = raw_profile.into_inner();
            let base = MotionProfile::builtin(&name);
            let pick = |field: Option<Spanned<RawSpring>>,
                        fallback: Option<SpringSpec>,
                        class: &str|
             -> Result<SpringSpec, ConfigError> {
                match field {
                    Some(s) => spring_from_raw(s, &err),
                    None => fallback.ok_or_else(|| {
                        err(
                            span.clone(),
                            format!("custom profile `{name}` must define `{class}`"),
                        )
                    }),
                }
            };
            let profile = MotionProfile {
                micro: pick(raw_profile.micro, base.map(|b| b.micro), "micro")?,
                standard: pick(raw_profile.standard, base.map(|b| b.standard), "standard")?,
                large: pick(raw_profile.large, base.map(|b| b.large), "large")?,
            };
            config.profiles.insert(name, profile);
        }

        if let Some(profile) = raw.profile {
            let span = profile.span();
            let name = profile.into_inner();
            if !config.profiles.contains_key(&name) {
                return Err(err(span, unknown_profile(&name, &config)));
            }
            config.profile = name;
        }
        if let Some(speed) = raw.speed {
            config.speed = speed_from_raw(speed, &err)?;
        }
        if let Some(reduce) = raw.reduce_motion {
            config.reduce_motion = reduce;
        }
        for (scope, raw_over) in raw.overrides {
            let raw_over = raw_over.into_inner();
            let mut over = ScopeOverride {
                reduce_motion: raw_over.reduce_motion,
                ..ScopeOverride::default()
            };
            if let Some(profile) = raw_over.profile {
                let span = profile.span();
                let name = profile.into_inner();
                if !config.profiles.contains_key(&name) {
                    return Err(err(span, unknown_profile(&name, &config)));
                }
                over.profile = Some(name);
            }
            if let Some(speed) = raw_over.speed {
                over.speed = Some(speed_from_raw(speed, &err)?);
            }
            config.overrides.insert(scope, over);
        }
        Ok(config)
    }
}

fn unknown_profile(name: &str, config: &MotionConfig) -> String {
    let known: Vec<&str> = config.profiles.keys().map(String::as_str).collect();
    format!("unknown profile `{name}` (known: {})", known.join(", "))
}

type ErrFn<'a> = dyn Fn(std::ops::Range<usize>, String) -> ConfigError + 'a;

fn speed_from_raw(speed: Spanned<f64>, err: &ErrFn<'_>) -> Result<f32, ConfigError> {
    let span = speed.span();
    let v = *speed.get_ref();
    if !v.is_finite() || !(0.0..=100.0).contains(&v) {
        return Err(err(
            span,
            format!("speed must be between 0 and 100, got {v}"),
        ));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(v as f32)
}

fn spring_from_raw(raw: Spanned<RawSpring>, err: &ErrFn<'_>) -> Result<SpringSpec, ConfigError> {
    let raw = raw.into_inner();
    let (rs, r) = (raw.response.span(), *raw.response.get_ref());
    if !r.is_finite() || !(0.0..=10.0).contains(&r) {
        return Err(err(
            rs,
            format!("response must be between 0 and 10 seconds, got {r}"),
        ));
    }
    let (ds, d) = (raw.damping.span(), *raw.damping.get_ref());
    if !d.is_finite() || d <= 0.0 || d > 10.0 {
        return Err(err(ds, format!("damping must be in (0, 10], got {d}")));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(SpringSpec::new(r as f32, d as f32))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMotion {
    profile: Option<Spanned<String>>,
    speed: Option<Spanned<f64>>,
    reduce_motion: Option<bool>,
    #[serde(default)]
    profiles: BTreeMap<String, Spanned<RawProfile>>,
    #[serde(default)]
    overrides: BTreeMap<String, Spanned<RawOverride>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProfile {
    micro: Option<Spanned<RawSpring>>,
    standard: Option<Spanned<RawSpring>>,
    large: Option<Spanned<RawSpring>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSpring {
    response: Spanned<f64>,
    damping: Spanned<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOverride {
    profile: Option<Spanned<String>>,
    speed: Option<Spanned<f64>>,
    reduce_motion: Option<bool>,
}
