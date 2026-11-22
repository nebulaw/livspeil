// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Sampling of [`Curve`] values (milestone M0.8).
//!
//! A [`Curve`] (defined in `livspeil-node-api`, which cannot depend on this
//! crate) is a `Float` animation channel whose "time" axis is a position.
//! [`AnimChannel::from_curve`] converts it so that curves and animation
//! channels share one sampling implementation; [`AnimChannel::to_curve`]
//! goes back (for `Float` channels without time offset). [`sample_curve`]
//! is the shortcut used by nodes.

use livspeil_node_api::{
    Curve, CurveError, CurveExtrapolation, CurveInterpolation, CurveKey, CurveTangent, PortType,
    Value,
};

use crate::anim::{AnimChannel, AnimError, Extrapolation, Interpolation, Keyframe, Tangent};

fn extrapolation(e: CurveExtrapolation) -> Extrapolation {
    match e {
        CurveExtrapolation::Hold => Extrapolation::Hold,
        CurveExtrapolation::Loop => Extrapolation::Loop,
        CurveExtrapolation::PingPong => Extrapolation::PingPong,
    }
}

fn curve_extrapolation(e: Extrapolation) -> CurveExtrapolation {
    match e {
        Extrapolation::Hold => CurveExtrapolation::Hold,
        Extrapolation::Loop => CurveExtrapolation::Loop,
        Extrapolation::PingPong => CurveExtrapolation::PingPong,
    }
}

fn keyframe(key: &CurveKey) -> Keyframe {
    Keyframe {
        time: key.position,
        value: Value::Float(key.value),
        interpolation: match key.interpolation {
            CurveInterpolation::Step => Interpolation::Step,
            CurveInterpolation::Linear => Interpolation::Linear,
            CurveInterpolation::Bezier => Interpolation::Bezier,
            CurveInterpolation::Spring { frequency, damping } => {
                Interpolation::Spring { frequency, damping }
            }
        },
        tangent: match key.tangent {
            CurveTangent::Auto => Tangent::Auto,
            CurveTangent::Flat => Tangent::Flat,
            CurveTangent::Explicit { incoming, outgoing } => Tangent::Explicit {
                incoming: Value::Float(incoming),
                outgoing: Value::Float(outgoing),
            },
        },
    }
}

/// Why a channel cannot become a [`Curve`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToCurveError {
    /// Curves hold `Float` values only.
    NotFloat(PortType),
    /// Curves have no time offset.
    HasOffset,
    /// The keys do not form a valid curve.
    Curve(CurveError),
}

impl std::fmt::Display for ToCurveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToCurveError::NotFloat(ty) => write!(f, "curves hold Float values, not {ty}"),
            ToCurveError::HasOffset => f.write_str("curves have no offset"),
            ToCurveError::Curve(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ToCurveError {}

impl AnimChannel {
    /// The `Float` channel with the curve's keys (positions as times),
    /// interpolations, tangents and extrapolation, and no offset.
    ///
    /// # Errors
    /// None for a validated curve; an [`AnimError`] is returned only if
    /// the two validations ever disagree.
    pub fn from_curve(curve: &Curve) -> Result<AnimChannel, AnimError> {
        Ok(
            AnimChannel::new(curve.keys().iter().map(keyframe).collect())?
                .with_extrapolation(extrapolation(curve.pre()), extrapolation(curve.post())),
        )
    }

    /// The curve of a `Float` channel without offset.
    ///
    /// # Errors
    /// [`ToCurveError`] for other value types or a non-zero offset.
    pub fn to_curve(&self) -> Result<Curve, ToCurveError> {
        if self.value_type() != PortType::Float {
            return Err(ToCurveError::NotFloat(self.value_type()));
        }
        if self.offset() != 0.0 {
            return Err(ToCurveError::HasOffset);
        }
        let float = |v: &Value| v.as_float().unwrap_or(0.0);
        let keys = self
            .keys()
            .iter()
            .map(|k| CurveKey {
                position: k.time,
                value: float(&k.value),
                interpolation: match k.interpolation {
                    Interpolation::Step => CurveInterpolation::Step,
                    Interpolation::Linear => CurveInterpolation::Linear,
                    Interpolation::Bezier => CurveInterpolation::Bezier,
                    Interpolation::Spring { frequency, damping } => {
                        CurveInterpolation::Spring { frequency, damping }
                    }
                },
                tangent: match &k.tangent {
                    Tangent::Auto => CurveTangent::Auto,
                    Tangent::Flat => CurveTangent::Flat,
                    Tangent::Explicit { incoming, outgoing } => CurveTangent::Explicit {
                        incoming: float(incoming),
                        outgoing: float(outgoing),
                    },
                },
            })
            .collect();
        Ok(Curve::new(keys)
            .map_err(ToCurveError::Curve)?
            .with_extrapolation(
                curve_extrapolation(self.pre()),
                curve_extrapolation(self.post()),
            ))
    }
}

/// The value of `curve` at `position`, with exactly the semantics of a
/// `Float` [`AnimChannel`] sampled at that time (see [`crate::anim`]).
///
/// # Errors
/// Only if the curve is rejected as a channel (never for a validated
/// curve).
pub fn sample_curve(curve: &Curve, position: f64) -> Result<f64, AnimError> {
    let channel = AnimChannel::from_curve(curve)?;
    Ok(channel.sample(position).as_float().unwrap_or(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(keys: Vec<CurveKey>) -> Curve {
        Curve::new(keys).unwrap()
    }

    #[test]
    fn samples_like_a_channel() {
        let c = curve(vec![CurveKey::new(0.0, 0.0), CurveKey::new(1.0, 2.0)]);
        assert_eq!(sample_curve(&c, 0.25), Ok(0.5));
        assert_eq!(sample_curve(&c, -1.0), Ok(0.0));
        assert_eq!(sample_curve(&c, 5.0), Ok(2.0));
        let looped = c
            .clone()
            .with_extrapolation(CurveExtrapolation::Loop, CurveExtrapolation::PingPong);
        assert_eq!(sample_curve(&looped, 1.25), Ok(1.5));
        assert_eq!(sample_curve(&looped, -0.75), Ok(0.5));
        let step = curve(vec![
            CurveKey::new(0.0, 1.0).with_interpolation(CurveInterpolation::Step),
            CurveKey::new(1.0, 3.0),
        ]);
        assert_eq!(sample_curve(&step, 0.99), Ok(1.0));
        assert_eq!(sample_curve(&Curve::constant(4.0), 100.0), Ok(4.0));
    }

    #[test]
    fn round_trips_through_channels() {
        let c = curve(vec![
            CurveKey::new(0.0, 0.0)
                .with_interpolation(CurveInterpolation::Bezier)
                .with_tangent(CurveTangent::Explicit {
                    incoming: 1.0,
                    outgoing: -2.0,
                }),
            CurveKey::new(0.5, 1.0)
                .with_interpolation(CurveInterpolation::Spring {
                    frequency: 2.0,
                    damping: 0.5,
                })
                .with_tangent(CurveTangent::Flat),
            CurveKey::new(1.0, 0.25),
        ])
        .with_extrapolation(CurveExtrapolation::PingPong, CurveExtrapolation::Loop);
        let channel = AnimChannel::from_curve(&c).unwrap();
        assert_eq!(channel.to_curve(), Ok(c));
    }

    #[test]
    fn only_float_channels_without_offset_become_curves() {
        let vec2 = AnimChannel::new(vec![Keyframe::new(0.0, Value::Vec2([0.0; 2]))]).unwrap();
        assert_eq!(vec2.to_curve(), Err(ToCurveError::NotFloat(PortType::Vec2)));
        let shifted = AnimChannel::constant(1.0)
            .unwrap()
            .with_offset(1.0)
            .unwrap();
        assert_eq!(shifted.to_curve(), Err(ToCurveError::HasOffset));
    }
}
