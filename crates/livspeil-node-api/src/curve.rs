// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Curves ([`Value::Curve`](crate::Value::Curve)), the data of ramp
//! parameters such as `core.curve.ramp`.
//!
//! A [`Curve`] maps a position to a value through keys, exactly like an
//! animation channel of `livspeil-params` (`AnimChannel`) maps time to a
//! value: same interpolations (step, linear, bezier with auto, flat or
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
//!
//! # Typed curves (M0.17)
//!
//! Key values are `Float` ([`Curve::new`], [`CurveKey`]) or, through
//! [`Curve::typed`] and [`TypedCurveKey`], all `Vec2`, all `Vec3`, all
//! `Vec4` or all `Color` ([`Curve::value_type`]). Vectors interpolate
//! component by component. Colors (OKLCH plus alpha) interpolate in
//! **OKLab** plus alpha, so hues never wrap the long way round and grey
//! keys (chroma 0, hue meaningless) do not drag the hue; the result is
//! converted back with the hue in `[0, 360)` (0 below chroma `1e-9`), and
//! a sample that lands exactly on a key gives that key's value unchanged.
//! Explicit tangents of a color curve are slopes in OKLab plus alpha
//! (`[L, a, b, alpha]` per position unit), stored as a `Color` value.
//! The port type stays [`PortType::Curve`](crate::PortType::Curve) for
//! every key type: the key type is data, like a table's column types.
//! [`Curve::keys`] still lists every key, holding the first component of
//! typed values, so code written for float curves keeps working.

use std::borrow::Cow;

use crate::{PortType, Value};

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
    /// Key values must be `Float`, `Vec2`, `Vec3`, `Vec4` or `Color`.
    UnsupportedType { index: usize, ty: PortType },
    /// A key's value (or explicit tangent) has another type than key 0.
    MixedTypes {
        index: usize,
        expected: PortType,
        found: PortType,
    },
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
            CurveError::UnsupportedType { index, ty } => {
                write!(f, "key {index}: curves cannot hold {ty} values")
            }
            CurveError::MixedTypes {
                index,
                expected,
                found,
            } => write!(f, "key {index}: expected {expected}, found {found}"),
        }
    }
}

impl std::error::Error for CurveError {}

/// Slopes of a [`TypedCurveKey`] for `Bezier` segments.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum TypedCurveTangent {
    /// Auto-clamped (per component): smooth, flat at extremes and ends.
    #[default]
    Auto,
    /// Zero slope on both sides.
    Flat,
    /// Explicit slopes per position unit, of the key's value type (for
    /// colors: OKLab plus alpha, see the [module documentation](self)).
    Explicit { incoming: Value, outgoing: Value },
}

/// One key of a [`Curve`] of any key type.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TypedCurveKey {
    pub position: f64,
    /// `Float`, `Vec2`, `Vec3`, `Vec4` or `Color`.
    pub value: Value,
    #[cfg_attr(feature = "serde", serde(default))]
    pub interpolation: CurveInterpolation,
    #[cfg_attr(feature = "serde", serde(default))]
    pub tangent: TypedCurveTangent,
}

impl TypedCurveKey {
    /// A `Linear` key with `Auto` tangents.
    #[must_use]
    pub fn new(position: f64, value: Value) -> Self {
        TypedCurveKey {
            position,
            value,
            interpolation: CurveInterpolation::Linear,
            tangent: TypedCurveTangent::Auto,
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
    pub fn with_tangent(mut self, tangent: TypedCurveTangent) -> Self {
        self.tangent = tangent;
        self
    }
}

impl From<CurveKey> for TypedCurveKey {
    fn from(key: CurveKey) -> Self {
        TypedCurveKey {
            position: key.position,
            value: Value::Float(key.value),
            interpolation: key.interpolation,
            tangent: match key.tangent {
                CurveTangent::Auto => TypedCurveTangent::Auto,
                CurveTangent::Flat => TypedCurveTangent::Flat,
                CurveTangent::Explicit { incoming, outgoing } => TypedCurveTangent::Explicit {
                    incoming: Value::Float(incoming),
                    outgoing: Value::Float(outgoing),
                },
            },
        }
    }
}

/// The components of a curve key value, or `None` for other types.
fn components(value: &Value) -> Option<&[f64]> {
    Some(match value {
        Value::Float(v) => std::slice::from_ref(v),
        Value::Vec2(v) => v,
        Value::Vec3(v) => v,
        Value::Vec4(v) | Value::Color(v) => v,
        _ => return None,
    })
}

/// The first component of a key value (0 for unsupported types).
fn first(value: &Value) -> f64 {
    components(value)
        .and_then(|c| c.first().copied())
        .unwrap_or(0.0)
}

/// A validated curve. See the [module documentation](self).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "RawCurve")
)]
pub struct Curve {
    /// Every key; for typed curves the first component of each value.
    keys: Vec<CurveKey>,
    #[cfg_attr(feature = "serde", serde(default))]
    pre: CurveExtrapolation,
    #[cfg_attr(feature = "serde", serde(default))]
    post: CurveExtrapolation,
    /// The full keys of a curve whose values are not `Float`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    typed: Option<Vec<TypedCurveKey>>,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct RawCurve {
    #[serde(default)]
    keys: Vec<CurveKey>,
    #[serde(default)]
    pre: CurveExtrapolation,
    #[serde(default)]
    post: CurveExtrapolation,
    #[serde(default)]
    typed: Option<Vec<TypedCurveKey>>,
}

#[cfg(feature = "serde")]
impl TryFrom<RawCurve> for Curve {
    type Error = CurveError;

    fn try_from(raw: RawCurve) -> Result<Self, CurveError> {
        let curve = match raw.typed {
            Some(typed) => Curve::typed(typed)?,
            None => Curve::new(raw.keys)?,
        };
        Ok(curve.with_extrapolation(raw.pre, raw.post))
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
            typed: None,
        })
    }

    /// A curve through keys of any supported value type (see the [module
    /// documentation](self)), holding before the first and after the last.
    /// Keys holding `Float` values give the same curve as [`Curve::new`].
    ///
    /// # Errors
    /// A [`CurveError`] as for [`Curve::new`], plus unsupported or mixed
    /// value (and explicit tangent) types.
    pub fn typed(keys: Vec<TypedCurveKey>) -> Result<Self, CurveError> {
        let ty = keys.first().ok_or(CurveError::NoKeys)?.value.port_type();
        for (index, key) in keys.iter().enumerate() {
            let found = key.value.port_type();
            if components(&key.value).is_none() {
                return Err(CurveError::UnsupportedType { index, ty: found });
            }
            if found != ty {
                return Err(CurveError::MixedTypes {
                    index,
                    expected: ty,
                    found,
                });
            }
            if let TypedCurveTangent::Explicit { incoming, outgoing } = &key.tangent {
                for slope in [incoming, outgoing] {
                    if slope.port_type() != ty {
                        return Err(CurveError::MixedTypes {
                            index,
                            expected: ty,
                            found: slope.port_type(),
                        });
                    }
                }
            }
            let finite = |v: &Value| components(v).is_some_and(|c| c.iter().all(|x| x.is_finite()));
            let tangents_finite = match &key.tangent {
                TypedCurveTangent::Explicit { incoming, outgoing } => {
                    finite(incoming) && finite(outgoing)
                }
                _ => true,
            };
            if !finite(&key.value) || !tangents_finite {
                return Err(CurveError::NonFinite { index });
            }
        }
        let projected = keys
            .iter()
            .map(|k| CurveKey {
                position: k.position,
                value: first(&k.value),
                interpolation: k.interpolation,
                tangent: match &k.tangent {
                    TypedCurveTangent::Auto => CurveTangent::Auto,
                    TypedCurveTangent::Flat => CurveTangent::Flat,
                    TypedCurveTangent::Explicit { incoming, outgoing } => CurveTangent::Explicit {
                        incoming: first(incoming),
                        outgoing: first(outgoing),
                    },
                },
            })
            .collect();
        // Positions, the first components and springs.
        let mut curve = Curve::new(projected)?;
        if ty != PortType::Float {
            curve.typed = Some(keys);
        }
        Ok(curve)
    }

    /// The constant curve `value` (one key at position 0).
    #[must_use]
    pub fn constant(value: f64) -> Self {
        let value = if value.is_finite() { value } else { 0.0 };
        Curve {
            keys: vec![CurveKey::new(0.0, value)],
            pre: CurveExtrapolation::Hold,
            post: CurveExtrapolation::Hold,
            typed: None,
        }
    }

    /// The linear ramp from 0 at position 0 to 1 at position 1.
    #[must_use]
    pub fn identity() -> Self {
        Curve {
            keys: vec![CurveKey::new(0.0, 0.0), CurveKey::new(1.0, 1.0)],
            pre: CurveExtrapolation::Hold,
            post: CurveExtrapolation::Hold,
            typed: None,
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

    /// The keys, sorted by position. For a typed curve each key holds the
    /// first component of its value (see [`Curve::typed_keys`]).
    #[must_use]
    pub fn keys(&self) -> &[CurveKey] {
        &self.keys
    }

    /// The type of every key value: `Float`, `Vec2`, `Vec3`, `Vec4` or
    /// `Color`.
    #[must_use]
    pub fn value_type(&self) -> PortType {
        match self.typed.as_deref().and_then(<[TypedCurveKey]>::first) {
            Some(key) => key.value.port_type(),
            None => PortType::Float,
        }
    }

    /// The keys with their full values, for curves of every value type
    /// (borrowed for typed curves, built for float curves).
    #[must_use]
    pub fn typed_keys(&self) -> Cow<'_, [TypedCurveKey]> {
        match &self.typed {
            Some(keys) => Cow::Borrowed(keys),
            None => Cow::Owned(self.keys.iter().copied().map(TypedCurveKey::from).collect()),
        }
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

    #[test]
    fn typed_curves() {
        let v3 = |x: f64| Value::Vec3([x, x + 1.0, x + 2.0]);
        let c = Curve::typed(vec![
            TypedCurveKey::new(0.0, v3(0.0)).with_interpolation(CurveInterpolation::Bezier),
            TypedCurveKey::new(1.0, v3(5.0)).with_tangent(TypedCurveTangent::Explicit {
                incoming: v3(1.0),
                outgoing: v3(2.0),
            }),
        ])
        .unwrap()
        .with_extrapolation(CurveExtrapolation::Loop, CurveExtrapolation::Hold);
        assert_eq!(c.value_type(), PortType::Vec3);
        assert_eq!(c.keys()[1].value, 5.0);
        assert_eq!(
            c.keys()[1].tangent,
            CurveTangent::Explicit {
                incoming: 1.0,
                outgoing: 2.0
            }
        );
        assert_eq!(c.typed_keys()[1].value, v3(5.0));
        assert_eq!(c.pre(), CurveExtrapolation::Loop);
        // Float keys give the plain float curve.
        let f = Curve::typed(vec![
            TypedCurveKey::new(0.0, Value::Float(0.0)),
            TypedCurveKey::new(1.0, Value::Float(1.0)),
        ])
        .unwrap();
        assert_eq!(f, Curve::identity());
        assert_eq!(f.value_type(), PortType::Float);
        assert_eq!(f.typed_keys()[1].value, Value::Float(1.0));
        let err = |keys| Curve::typed(keys).unwrap_err();
        assert_eq!(err(vec![]), CurveError::NoKeys);
        assert_eq!(
            err(vec![TypedCurveKey::new(0.0, Value::Int(1))]),
            CurveError::UnsupportedType {
                index: 0,
                ty: PortType::Int
            }
        );
        assert_eq!(
            err(vec![
                TypedCurveKey::new(0.0, v3(0.0)),
                TypedCurveKey::new(1.0, Value::Float(0.0))
            ]),
            CurveError::MixedTypes {
                index: 1,
                expected: PortType::Vec3,
                found: PortType::Float
            }
        );
        assert_eq!(
            err(vec![TypedCurveKey::new(0.0, v3(0.0)).with_tangent(
                TypedCurveTangent::Explicit {
                    incoming: v3(0.0),
                    outgoing: Value::Float(0.0)
                }
            )]),
            CurveError::MixedTypes {
                index: 0,
                expected: PortType::Vec3,
                found: PortType::Float
            }
        );
        assert_eq!(
            err(vec![TypedCurveKey::new(
                0.0,
                Value::Color([0.5, f64::NAN, 0.0, 1.0])
            )]),
            CurveError::NonFinite { index: 0 }
        );
        assert_eq!(
            err(vec![
                TypedCurveKey::new(1.0, v3(0.0)),
                TypedCurveKey::new(0.0, v3(0.0))
            ]),
            CurveError::NotIncreasing { index: 1 }
        );
        assert!(
            !CurveError::UnsupportedType {
                index: 0,
                ty: PortType::Int
            }
            .to_string()
            .is_empty()
        );
    }
}
