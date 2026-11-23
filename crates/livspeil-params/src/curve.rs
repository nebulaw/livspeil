// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Sampling of [`Curve`] values (milestones M0.8 and M0.17).
//!
//! A [`Curve`] (defined in `livspeil-node-api`, which cannot depend on this
//! crate) is an animation channel whose "time" axis is a position.
//! [`AnimChannel::from_curve`] converts it so that curves and animation
//! channels share one sampling implementation; [`AnimChannel::to_curve`]
//! goes back (for channels without time offset). Both conversions are
//! lossless and keep the key type (`Float`, `Vec2`, `Vec3`, `Vec4` or
//! `Color`).
//!
//! [`sample_curve_value`] samples a curve of any key type, and
//! [`sample_curve`] a float curve (the shortcut used by float nodes). They
//! sample exactly like the channel, except for **color curves**, which
//! interpolate in OKLab plus alpha instead of component by component in
//! OKLCH (see the `livspeil_node_api::curve` module): the keys are
//! converted to OKLab (explicit color tangents are already OKLab slopes),
//! sampled as a `Vec4` channel and converted back with the hue in
//! `[0, 360)`, hue 0 below chroma [`CHROMA_EPSILON`]; a sample equal to a
//! key's OKLab value (on a key, in a step or hold) is that key's original
//! color, so keys come back bit for bit.

use livspeil_node_api::{
    Curve, CurveExtrapolation, CurveInterpolation, CurveKey, CurveTangent, PortType, TypedCurveKey,
    TypedCurveTangent, Value,
};

use livspeil_node_api::CurveError;

/// Below this OKLab chroma a sampled color is grey and gets hue 0 (the
/// same threshold as `core.color.mix`).
pub const CHROMA_EPSILON: f64 = 1e-9;

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

fn interpolation(i: CurveInterpolation) -> Interpolation {
    match i {
        CurveInterpolation::Step => Interpolation::Step,
        CurveInterpolation::Linear => Interpolation::Linear,
        CurveInterpolation::Bezier => Interpolation::Bezier,
        CurveInterpolation::Spring { frequency, damping } => {
            Interpolation::Spring { frequency, damping }
        }
    }
}

fn curve_interpolation(i: Interpolation) -> CurveInterpolation {
    match i {
        Interpolation::Step => CurveInterpolation::Step,
        Interpolation::Linear => CurveInterpolation::Linear,
        Interpolation::Bezier => CurveInterpolation::Bezier,
        Interpolation::Spring { frequency, damping } => {
            CurveInterpolation::Spring { frequency, damping }
        }
    }
}

fn keyframe(key: &CurveKey) -> Keyframe {
    Keyframe {
        time: key.position,
        value: Value::Float(key.value),
        interpolation: interpolation(key.interpolation),
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

/// A typed key as a keyframe; `lab` converts color values to OKLab
/// `Vec4`s (tangents of color keys already are OKLab slopes).
fn typed_keyframe(key: &TypedCurveKey, lab: bool) -> Keyframe {
    let convert = |v: &Value, is_value: bool| match v {
        Value::Color(c) if lab && is_value => Value::Vec4(lch_to_lab(*c)),
        Value::Color(c) if lab => Value::Vec4(*c),
        other => other.clone(),
    };
    Keyframe {
        time: key.position,
        value: convert(&key.value, true),
        interpolation: interpolation(key.interpolation),
        tangent: match &key.tangent {
            TypedCurveTangent::Auto => Tangent::Auto,
            TypedCurveTangent::Flat => Tangent::Flat,
            TypedCurveTangent::Explicit { incoming, outgoing } => Tangent::Explicit {
                incoming: convert(incoming, false),
                outgoing: convert(outgoing, false),
            },
        },
    }
}

/// OKLCH plus alpha to OKLab plus alpha.
fn lch_to_lab([l, c, h, alpha]: [f64; 4]) -> [f64; 4] {
    let (sin, cos) = h.to_radians().sin_cos();
    [l, c * cos, c * sin, alpha]
}

/// OKLab plus alpha to OKLCH plus alpha, hue in `[0, 360)`, hue 0 for
/// greys.
fn lab_to_lch([l, a, b, alpha]: [f64; 4]) -> [f64; 4] {
    let c = a.hypot(b);
    let h = if c < CHROMA_EPSILON {
        0.0
    } else {
        let h = b.atan2(a).to_degrees().rem_euclid(360.0);
        // rem_euclid can round up to exactly 360.
        // `+ 0.0` turns -0 into 0.
        if h >= 360.0 { 0.0 } else { h + 0.0 }
    };
    [l, c, h, alpha]
}

/// Why a channel cannot become a [`Curve`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToCurveError {
    /// The value type cannot be held by a curve (no longer produced since
    /// curves hold every animatable type, M0.17).
    NotFloat(PortType),
    /// Curves have no time offset.
    HasOffset,
    /// The keys do not form a valid curve.
    Curve(CurveError),
}

impl std::fmt::Display for ToCurveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToCurveError::NotFloat(ty) => write!(f, "curves cannot hold {ty} values"),
            ToCurveError::HasOffset => f.write_str("curves have no offset"),
            ToCurveError::Curve(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ToCurveError {}

impl AnimChannel {
    /// The channel with the curve's keys (positions as times, values of
    /// the curve's key type), interpolations, tangents and extrapolation,
    /// and no offset. Sampling it gives the curve's values, except that
    /// color channels interpolate component by component (use
    /// [`sample_curve_value`] for the OKLab semantics of color curves).
    ///
    /// # Errors
    /// None for a validated curve; an [`AnimError`] is returned only if
    /// the two validations ever disagree.
    pub fn from_curve(curve: &Curve) -> Result<AnimChannel, AnimError> {
        let keys = if curve.value_type() == PortType::Float {
            curve.keys().iter().map(keyframe).collect()
        } else {
            curve
                .typed_keys()
                .iter()
                .map(|k| typed_keyframe(k, false))
                .collect()
        };
        Ok(AnimChannel::new(keys)?
            .with_extrapolation(extrapolation(curve.pre()), extrapolation(curve.post())))
    }

    /// The curve of a channel without offset (any value type).
    ///
    /// # Errors
    /// [`ToCurveError`] for a non-zero offset.
    pub fn to_curve(&self) -> Result<Curve, ToCurveError> {
        if self.offset() != 0.0 {
            return Err(ToCurveError::HasOffset);
        }
        let keys = self
            .keys()
            .iter()
            .map(|k| TypedCurveKey {
                position: k.time,
                value: k.value.clone(),
                interpolation: curve_interpolation(k.interpolation),
                tangent: match &k.tangent {
                    Tangent::Auto => TypedCurveTangent::Auto,
                    Tangent::Flat => TypedCurveTangent::Flat,
                    Tangent::Explicit { incoming, outgoing } => TypedCurveTangent::Explicit {
                        incoming: incoming.clone(),
                        outgoing: outgoing.clone(),
                    },
                },
            })
            .collect();
        Ok(Curve::typed(keys)
            .map_err(ToCurveError::Curve)?
            .with_extrapolation(
                curve_extrapolation(self.pre()),
                curve_extrapolation(self.post()),
            ))
    }
}

/// The value of a float `curve` at `position`, with exactly the semantics
/// of a `Float` [`AnimChannel`] sampled at that time (see [`crate::anim`]).
/// For typed curves, the first component of [`sample_curve_value`].
///
/// # Errors
/// Only if the curve is rejected as a channel (never for a validated
/// curve).
pub fn sample_curve(curve: &Curve, position: f64) -> Result<f64, AnimError> {
    if curve.value_type() != PortType::Float {
        return Ok(match sample_curve_value(curve, position)? {
            Value::Vec2([x, ..]) => x,
            Value::Vec3([x, ..]) => x,
            Value::Vec4([x, ..]) | Value::Color([x, ..]) => x,
            other => other.as_float().unwrap_or(0.0),
        });
    }
    let channel = AnimChannel::from_curve(curve)?;
    Ok(channel.sample(position).as_float().unwrap_or(0.0))
}

/// The value of `curve` at `position`, of the curve's key type
/// ([`Curve::value_type`]); colors interpolate in OKLab (see the module
/// documentation).
///
/// # Errors
/// Only if the curve is rejected as a channel (never for a validated
/// curve).
pub fn sample_curve_value(curve: &Curve, position: f64) -> Result<Value, AnimError> {
    if curve.value_type() != PortType::Color {
        return Ok(AnimChannel::from_curve(curve)?.sample(position));
    }
    let keys = curve.typed_keys();
    let channel = AnimChannel::new(keys.iter().map(|k| typed_keyframe(k, true)).collect())?
        .with_extrapolation(extrapolation(curve.pre()), extrapolation(curve.post()));
    let Value::Vec4(lab) = channel.sample(position) else {
        return Ok(Value::Color([0.0, 0.0, 0.0, 1.0]));
    };
    // On a key (or holding one): that key's color, bit for bit.
    for key in keys.iter() {
        if let Value::Color(c) = key.value {
            if lch_to_lab(c) == lab {
                return Ok(key.value.clone());
            }
        }
    }
    Ok(Value::Color(lab_to_lch(lab)))
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
    fn typed_channels_become_typed_curves() {
        let vec2 = AnimChannel::new(vec![
            Keyframe::new(0.0, Value::Vec2([0.0; 2])).with_tangent(Tangent::Explicit {
                incoming: Value::Vec2([1.0, 2.0]),
                outgoing: Value::Vec2([3.0, 4.0]),
            }),
            Keyframe::new(1.0, Value::Vec2([2.0, 4.0])),
        ])
        .unwrap();
        let curve = vec2.to_curve().unwrap();
        assert_eq!(curve.value_type(), PortType::Vec2);
        assert_eq!(AnimChannel::from_curve(&curve), Ok(vec2));
        assert_eq!(sample_curve_value(&curve, 0.5), Ok(Value::Vec2([1.0, 2.0])));
        assert_eq!(sample_curve(&curve, 0.5), Ok(1.0));
    }

    #[test]
    fn colors_interpolate_in_oklab() {
        let red = [0.6, 0.2, 30.0, 1.0];
        let blue = [0.6, 0.2, 330.0, 0.0];
        let grey = [0.9, 0.0, 200.0, 1.0];
        let color = |keys: Vec<[f64; 4]>| {
            Curve::typed(
                keys.into_iter()
                    .enumerate()
                    .map(|(i, c)| TypedCurveKey::new(i as f64, Value::Color(c)))
                    .collect(),
            )
            .unwrap()
        };
        let c = color(vec![red, blue]);
        // Keys and holds come back exactly.
        assert_eq!(sample_curve_value(&c, 0.0), Ok(Value::Color(red)));
        assert_eq!(sample_curve_value(&c, 1.0), Ok(Value::Color(blue)));
        assert_eq!(sample_curve_value(&c, 7.0), Ok(Value::Color(blue)));
        // The short way round: hue 0 (= 360), not 180.
        let Ok(Value::Color(mid)) = sample_curve_value(&c, 0.5) else {
            panic!()
        };
        assert!(mid[2] < 1e-9 || mid[2] > 359.0, "{mid:?}");
        assert!((mid[3] - 0.5).abs() < 1e-12);
        assert!((0.0..360.0).contains(&mid[2]));
        // A grey key does not drag the hue: towards red the hue stays red's.
        let g = color(vec![grey, red]);
        let Ok(Value::Color(towards)) = sample_curve_value(&g, 0.5) else {
            panic!()
        };
        assert!((towards[2] - 30.0).abs() < 1e-9, "{towards:?}");
        assert!((towards[1] - 0.1).abs() < 1e-12, "{towards:?}");
        // Pure greys get hue 0.
        let greys = color(vec![grey, [0.1, 0.0, 10.0, 1.0]]);
        let Ok(Value::Color(mid)) = sample_curve_value(&greys, 0.5) else {
            panic!()
        };
        assert_eq!(mid[2], 0.0);
        assert!((mid[0] - 0.5).abs() < 1e-12);
        assert_eq!(lab_to_lch(lch_to_lab([0.5, 0.1, 0.0, 1.0]))[2], 0.0);
        // Channels keep color keys as they are.
        let channel = AnimChannel::from_curve(&c).unwrap();
        assert_eq!(channel.value_type(), PortType::Color);
        assert_eq!(channel.to_curve(), Ok(c));
    }

    #[test]
    fn channels_without_offset_become_curves() {
        let shifted = AnimChannel::constant(1.0)
            .unwrap()
            .with_offset(1.0)
            .unwrap();
        assert_eq!(shifted.to_curve(), Err(ToCurveError::HasOffset));
    }
}
