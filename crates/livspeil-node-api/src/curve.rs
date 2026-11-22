// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Float curves ([`Value::Curve`](crate::Value::Curve)), the data of ramp
//! parameters such as `core.curve.ramp`.
//!
//! A [`Curve`] maps a position to a number through keys, exactly like a
//! `Float` animation channel of `livspeil-params` (`AnimChannel`) maps time
//! to a value: same interpolations (step, linear, bezier with auto, flat or
//! explicit tangents, spring), same pre and post extrapolation (hold, loop,
//! ping-pong), no time offset. This crate only holds and validates the
//! data, because it cannot depend on `livspeil-params`; sampling lives
//! there (`AnimChannel::from_curve`), so curves and animation channels
//! share one implementation and one set of semantics.
//!
//! Curves are immutable and validated: at least one key, finite numbers,
//! strictly increasing positions, valid spring settings. They sit behind
//! an [`Arc`] inside [`Value`](crate::Value), so cloning is cheap. With the
//! `serde` feature, deserializing validates like [`Curve::new`].

use std::fmt;

/// How the value moves from a key to the next one. Stored on the left key.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum CurveInterpolation {
    /// Hold the left value until the next key.
    Step,
    /// Straight line.
    #[default]
    Linear,
    /// Cubic curve shaped by the keys' tangents.
    Bezier,
    /// A damped spring from the left value toward the right one; the
    /// position plays the role of time.
    Spring { frequency: f64, damping: f64 },
}

/// Slopes of a key for `Bezier` segments.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum CurveTangent {
    /// Auto-clamped: smooth, flat at extremes and ends, never overshoots.
    #[default]
    Auto,
    /// Zero slope on both sides.
    Flat,
    /// Explicit slopes, in value units per position unit.
    Explicit { incoming: f64, outgoing: f64 },
}

/// What happens outside the keyed position range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum CurveExtrapolation {
    /// Keep the value of the nearest key.
    #[default]
    Hold,
    /// Repeat the keyed range.
    Loop,
    /// Repeat the keyed range, alternately forward and backward.
    PingPong,
}

/// One key of a [`Curve`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CurveKey {
    pub position: f64,
    pub value: f64,
    #[cfg_attr(feature = "serde", serde(default))]
    pub interpolation: CurveInterpolation,
    #[cfg_attr(feature = "serde", serde(default))]
    pub tangent: CurveTangent,
}

impl CurveKey {
    /// A `Linear` key with `Auto` tangents.
    #[must_use]
    pub fn new(position: f64, value: f64) -> Self {
        CurveKey {
            position,
            value,
            interpolation: CurveInterpolation::Linear,
            tangent: CurveTangent::Auto,
        }
    }

    /// The same key with another interpolation.
    #[must_use]
    pub fn with_interpolation(mut self, interpolation: CurveInterpolation) -> Self {
        self.interpolation = interpolation;
        self
    }

    /// The same key with another tangent.
    #[must_use]
    pub fn with_tangent(mut self, tangent: CurveTangent) -> Self {
        self.tangent = tangent;
        self
    }
}

/// Why a curve is invalid. Key indices are 0-based.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CurveError {
    /// A curve needs at least one key.
    NoKeys,
    /// Positions must be strictly increasing.
    NotIncreasing { index: usize },
    /// A position, value or tangent is NaN or infinite.
    NonFinite { index: usize },
    /// A spring's frequency must be finite and positive and its damping
    /// finite and not negative.
    InvalidSpring { index: usize },
}

impl fmt::Display for CurveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CurveError::NoKeys => f.write_str("a curve needs at least one key"),
            CurveError::NotIncreasing { index } => {
                write!(f, "key {index}: key positions must be strictly increasing")
            }
            CurveError::NonFinite { index } => write!(
                f,
                "key {index}: positions, values and tangents must be finite"
            ),
            CurveError::InvalidSpring { index } => write!(
                f,
                "key {index}: spring frequency must be positive and damping not negative"
            ),
        }
    }
}

impl std::error::Error for CurveError {}

/// A validated float curve. See the [module documentation](self).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "RawCurve")
)]
pub struct Curve {
    keys: Vec<CurveKey>,
    #[cfg_attr(feature = "serde", serde(default))]
    pre: CurveExtrapolation,
    #[cfg_attr(feature = "serde", serde(default))]
    post: CurveExtrapolation,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct RawCurve {
    keys: Vec<CurveKey>,
    #[serde(default)]
    pre: CurveExtrapolation,
    #[serde(default)]
    post: CurveExtrapolation,
}

#[cfg(feature = "serde")]
impl TryFrom<RawCurve> for Curve {
    type Error = CurveError;

    fn try_from(raw: RawCurve) -> Result<Self, CurveError> {
        Ok(Curve::new(raw.keys)?.with_extrapolation(raw.pre, raw.post))
    }
}

impl Curve {
    /// A curve through `keys`, holding before the first and after the last.
    ///
    /// # Errors
    /// A [`CurveError`] for no keys, non-finite numbers, positions that do
    /// not strictly increase, or invalid spring settings.
    pub fn new(keys: Vec<CurveKey>) -> Result<Self, CurveError> {
        if keys.is_empty() {
            return Err(CurveError::NoKeys);
        }
        for (index, key) in keys.iter().enumerate() {
            let slopes_finite = match key.tangent {
                CurveTangent::Explicit { incoming, outgoing } => {
                    incoming.is_finite() && outgoing.is_finite()
                }
                _ => true,
            };
            if !key.position.is_finite() || !key.value.is_finite() || !slopes_finite {
                return Err(CurveError::NonFinite { index });
            }
            if index > 0 && key.position <= keys[index - 1].position {
                return Err(CurveError::NotIncreasing { index });
            }
            if let CurveInterpolation::Spring { frequency, damping } = key.interpolation {
                let ok = frequency.is_finite()
                    && frequency > 0.0
                    && damping.is_finite()
                    && damping >= 0.0;
                if !ok {
                    return Err(CurveError::InvalidSpring { index });
                }
            }
        }
        Ok(Curve {
            keys,
            pre: CurveExtrapolation::Hold,
            post: CurveExtrapolation::Hold,
        })
    }

    /// The constant curve `value` (one key at position 0).
    #[must_use]
    pub fn constant(value: f64) -> Self {
        let value = if value.is_finite() { value } else { 0.0 };
        Curve {
            keys: vec![CurveKey::new(0.0, value)],
            pre: CurveExtrapolation::Hold,
            post: CurveExtrapolation::Hold,
        }
    }

    /// The linear ramp from 0 at position 0 to 1 at position 1.
    #[must_use]
    pub fn identity() -> Self {
        Curve {
            keys: vec![CurveKey::new(0.0, 0.0), CurveKey::new(1.0, 1.0)],
            pre: CurveExtrapolation::Hold,
            post: CurveExtrapolation::Hold,
        }
    }

    /// The same curve with other extrapolation before the first and after
    /// the last key.
    #[must_use]
    pub fn with_extrapolation(mut self, pre: CurveExtrapolation, post: CurveExtrapolation) -> Self {
        self.pre = pre;
        self.post = post;
        self
    }

    /// The keys, sorted by position.
    #[must_use]
    pub fn keys(&self) -> &[CurveKey] {
        &self.keys
    }

    /// Extrapolation before the first key.
    #[must_use]
    pub fn pre(&self) -> CurveExtrapolation {
        self.pre
    }

    /// Extrapolation after the last key.
    #[must_use]
    pub fn post(&self) -> CurveExtrapolation {
        self.post
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_keys() {
        assert_eq!(Curve::new(vec![]), Err(CurveError::NoKeys));
        assert_eq!(
            Curve::new(vec![CurveKey::new(1.0, 0.0), CurveKey::new(1.0, 1.0)]),
            Err(CurveError::NotIncreasing { index: 1 })
        );
        assert_eq!(
            Curve::new(vec![CurveKey::new(0.0, f64::NAN)]),
            Err(CurveError::NonFinite { index: 0 })
        );
        assert_eq!(
            Curve::new(vec![CurveKey::new(0.0, 0.0).with_tangent(
                CurveTangent::Explicit {
                    incoming: 0.0,
                    outgoing: f64::INFINITY
                }
            )]),
            Err(CurveError::NonFinite { index: 0 })
        );
        assert_eq!(
            Curve::new(vec![CurveKey::new(0.0, 0.0).with_interpolation(
                CurveInterpolation::Spring {
                    frequency: 0.0,
                    damping: 1.0
                }
            )]),
            Err(CurveError::InvalidSpring { index: 0 })
        );
        let c = Curve::identity()
            .with_extrapolation(CurveExtrapolation::Loop, CurveExtrapolation::PingPong);
        assert_eq!(c.keys().len(), 2);
        assert_eq!(c.pre(), CurveExtrapolation::Loop);
        assert_eq!(c.post(), CurveExtrapolation::PingPong);
        assert_eq!(Curve::constant(f64::NAN).keys()[0].value, 0.0);
    }
}
