// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Animation channels: keyframed values sampled over time (kernel spec,
//! section 6).
//!
//! An [`AnimChannel`] holds at least one [`Keyframe`], sorted by strictly
//! increasing time, all of one value type: `Float`, `Vec2`, `Vec3`, `Vec4`
//! or `Color`. Vectors and colors are interpolated component by component
//! (a color's hue is interpolated as a plain number, without wrapping
//! around 360 degrees).
//!
//! # Sampling
//!
//! [`AnimChannel::sample`] first subtracts the channel's time offset, so a
//! channel with offset `2.0` reaches its first key two seconds later. Before
//! the first key and after the last one the pre and post [`Extrapolation`]
//! applies: `Hold` keeps the end value, `Loop` repeats the keyed range and
//! `PingPong` repeats it alternately forward and backward. A channel with a
//! single key is constant. Non-finite times hold: `-inf` and `NaN` give the
//! first key's value, `+inf` the last key's.
//!
//! Between two keys the left key's [`Interpolation`] applies:
//!
//! * `Step` holds the left value until the right key's time.
//! * `Linear` interpolates linearly.
//! * `Bezier` is a cubic Hermite curve (equivalently, a cubic Bezier with
//!   handles at one third of the segment) whose slopes come from each key's
//!   [`Tangent`]: `Auto` is *auto-clamped* (the average of the two adjacent
//!   secant slopes, zero at local extremes and at the first and last key, and
//!   limited to three times the smaller secant so the curve never overshoots
//!   between keys), `Flat` is zero, and `Explicit` gives the incoming and
//!   outgoing slopes in value units per second.
//! * `Spring { frequency, damping }` moves from the left value toward the
//!   right one like a damped spring released at rest: `frequency` is the
//!   undamped natural frequency in Hz and `damping` the damping ratio (`1`
//!   is critically damped, below `1` overshoots and rings). The segment
//!   reaches the right key's value exactly at its time, which is a jump if
//!   the spring has not settled by then.
//!
//! Sampling is deterministic (plain IEEE arithmetic and the pure-Rust `libm`
//! for `exp`, `sin` and `cos`), allocation-free, and `O(log n)` in the
//! number of keys.

use std::fmt;

use livspeil_node_api::{PortType, Value};

/// How the value moves from a key to the next one. Stored on the left key.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Interpolation {
    /// Hold the left value until the next key.
    Step,
    /// Straight line.
    #[default]
    Linear,
    /// Cubic curve shaped by the keys' tangents.
    Bezier,
    /// A damped spring from the left value toward the right one.
    Spring {
        /// Undamped natural frequency in Hz; finite and positive.
        frequency: f64,
        /// Damping ratio; finite and not negative.
        damping: f64,
    },
}

/// Slopes of a key for `Bezier` segments.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Tangent {
    /// Auto-clamped: smooth, flat at extremes and ends, never overshoots.
    #[default]
    Auto,
    /// Zero slope on both sides.
    Flat,
    /// Explicit slopes in value units per second, of the key's value type.
    Explicit {
        /// Slope arriving at the key.
        incoming: Value,
        /// Slope leaving the key.
        outgoing: Value,
    },
}

/// What happens outside the keyed time range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Extrapolation {
    /// Keep the value of the nearest key.
    #[default]
    Hold,
    /// Repeat the keyed range.
    Loop,
    /// Repeat the keyed range, alternately forward and backward.
    PingPong,
}

/// One key of an [`AnimChannel`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Keyframe {
    /// Seconds, before the channel's offset is applied.
    pub time: f64,
    /// The value at `time`.
    pub value: Value,
    /// How the segment starting at this key is interpolated.
    #[cfg_attr(feature = "serde", serde(default))]
    pub interpolation: Interpolation,
    /// Slopes used by adjacent `Bezier` segments.
    #[cfg_attr(feature = "serde", serde(default))]
    pub tangent: Tangent,
}

impl Keyframe {
    /// A `Linear` key with `Auto` tangents.
    #[must_use]
    pub fn new(time: f64, value: Value) -> Self {
        Keyframe {
            time,
            value,
            interpolation: Interpolation::Linear,
            tangent: Tangent::Auto,
        }
    }

    /// A `Float` key.
    #[must_use]
    pub fn float(time: f64, value: f64) -> Self {
        Keyframe::new(time, Value::Float(value))
    }

    /// The same key with another interpolation.
    #[must_use]
    pub fn with_interpolation(mut self, interpolation: Interpolation) -> Self {
        self.interpolation = interpolation;
        self
    }

    /// The same key with another tangent.
    #[must_use]
    pub fn with_tangent(mut self, tangent: Tangent) -> Self {
        self.tangent = tangent;
        self
    }
}

/// Why a channel is invalid. Key indices are 0-based.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AnimError {
    /// A channel needs at least one key.
    NoKeys,
    /// Keys must hold `Float`, `Vec2`, `Vec3`, `Vec4` or `Color`.
    UnsupportedType { index: usize, ty: PortType },
    /// A key's value (or explicit tangent) has another type than key 0.
    MixedTypes {
        index: usize,
        expected: PortType,
        found: PortType,
    },
    /// Key times must be finite and strictly increasing.
    NotIncreasing { index: usize },
    /// A key's time, value or tangent is NaN or infinite.
    NonFinite { index: usize },
    /// A spring's frequency must be finite and positive and its damping
    /// finite and not negative.
    InvalidSpring { index: usize },
    /// The time offset must be finite.
    NonFiniteOffset,
}

impl fmt::Display for AnimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnimError::NoKeys => f.write_str("an animation channel needs at least one key"),
            AnimError::UnsupportedType { index, ty } => {
                write!(f, "key {index}: {ty} values cannot be animated")
            }
            AnimError::MixedTypes {
                index,
                expected,
                found,
            } => write!(f, "key {index}: expected {expected}, found {found}"),
            AnimError::NotIncreasing { index } => {
                write!(f, "key {index}: key times must be strictly increasing")
            }
            AnimError::NonFinite { index } => {
                write!(f, "key {index}: times, values and tangents must be finite")
            }
            AnimError::InvalidSpring { index } => write!(
                f,
                "key {index}: spring frequency must be positive and damping not negative"
            ),
            AnimError::NonFiniteOffset => f.write_str("the time offset must be finite"),
        }
    }
}

impl std::error::Error for AnimError {}

/// A validated, keyframed animation of one value. See the [module
/// documentation](self) for the sampling rules.
///
/// With the `serde` feature, deserializing validates like
/// [`AnimChannel::new`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "RawChannel")
)]
pub struct AnimChannel {
    keys: Vec<Keyframe>,
    #[cfg_attr(feature = "serde", serde(default))]
    pre: Extrapolation,
    #[cfg_attr(feature = "serde", serde(default))]
    post: Extrapolation,
    #[cfg_attr(feature = "serde", serde(default))]
    offset: f64,
}

/// Unvalidated mirror of [`AnimChannel`] for deserialization.
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct RawChannel {
    keys: Vec<Keyframe>,
    #[serde(default)]
    pre: Extrapolation,
    #[serde(default)]
    post: Extrapolation,
    #[serde(default)]
    offset: f64,
}

#[cfg(feature = "serde")]
impl TryFrom<RawChannel> for AnimChannel {
    type Error = AnimError;

    fn try_from(raw: RawChannel) -> Result<Self, AnimError> {
        AnimChannel::new(raw.keys)?
            .with_extrapolation(raw.pre, raw.post)
            .with_offset(raw.offset)
    }
}

/// Up to four components and how many are used.
#[derive(Clone, Copy)]
struct Comps {
    v: [f64; 4],
    n: usize,
}

fn comps(value: &Value) -> Option<Comps> {
    Some(match value {
        Value::Float(x) => Comps {
            v: [*x, 0.0, 0.0, 0.0],
            n: 1,
        },
        Value::Vec2([a, b]) => Comps {
            v: [*a, *b, 0.0, 0.0],
            n: 2,
        },
        Value::Vec3([a, b, c]) => Comps {
            v: [*a, *b, *c, 0.0],
            n: 3,
        },
        Value::Vec4(v) | Value::Color(v) => Comps { v: *v, n: 4 },
        _ => return None,
    })
}

fn from_comps(ty: PortType, v: [f64; 4]) -> Value {
    match ty {
        PortType::Vec2 => Value::Vec2([v[0], v[1]]),
        PortType::Vec3 => Value::Vec3([v[0], v[1], v[2]]),
        PortType::Vec4 => Value::Vec4(v),
        PortType::Color => Value::Color(v),
        _ => Value::Float(v[0]),
    }
}

fn finite(value: &Value) -> bool {
    comps(value).is_some_and(|c| c.v.iter().all(|x| x.is_finite()))
}

impl AnimChannel {
    /// A channel through `keys`, holding before the first and after the
    /// last key, without time offset.
    ///
    /// # Errors
    /// Returns an [`AnimError`] for no keys, unsupported or mixed value
    /// types, non-finite numbers, times that do not strictly increase, or
    /// invalid spring parameters.
    pub fn new(keys: Vec<Keyframe>) -> Result<Self, AnimError> {
        let first = keys.first().ok_or(AnimError::NoKeys)?;
        let ty = first.value.port_type();
        for (index, key) in keys.iter().enumerate() {
            let found = key.value.port_type();
            if comps(&key.value).is_none() {
                return Err(AnimError::UnsupportedType { index, ty: found });
            }
            if found != ty {
                return Err(AnimError::MixedTypes {
                    index,
                    expected: ty,
                    found,
                });
            }
            if !key.time.is_finite() || !finite(&key.value) {
                return Err(AnimError::NonFinite { index });
            }
            if index > 0 && key.time <= keys[index - 1].time {
                return Err(AnimError::NotIncreasing { index });
            }
            if let Tangent::Explicit { incoming, outgoing } = &key.tangent {
                for slope in [incoming, outgoing] {
                    if slope.port_type() != ty {
                        return Err(AnimError::MixedTypes {
                            index,
                            expected: ty,
                            found: slope.port_type(),
                        });
                    }
                    if !finite(slope) {
                        return Err(AnimError::NonFinite { index });
                    }
                }
            }
            if let Interpolation::Spring { frequency, damping } = key.interpolation {
                let ok = frequency.is_finite()
                    && frequency > 0.0
                    && damping.is_finite()
                    && damping >= 0.0;
                if !ok {
                    return Err(AnimError::InvalidSpring { index });
                }
            }
        }
        Ok(AnimChannel {
            keys,
            pre: Extrapolation::Hold,
            post: Extrapolation::Hold,
            offset: 0.0,
        })
    }

    /// A constant `Float` channel; handy in tests.
    #[must_use]
    pub fn constant(value: f64) -> Option<Self> {
        AnimChannel::new(vec![Keyframe::float(0.0, value)]).ok()
    }

    /// The same channel with other extrapolation before the first and after
    /// the last key.
    #[must_use]
    pub fn with_extrapolation(mut self, pre: Extrapolation, post: Extrapolation) -> Self {
        self.pre = pre;
        self.post = post;
        self
    }

    /// The same channel shifted later in time by `offset` seconds.
    ///
    /// # Errors
    /// [`AnimError::NonFiniteOffset`] for NaN or infinite offsets.
    pub fn with_offset(mut self, offset: f64) -> Result<Self, AnimError> {
        if !offset.is_finite() {
            return Err(AnimError::NonFiniteOffset);
        }
        self.offset = offset;
        Ok(self)
    }

    /// The keys, sorted by time.
    #[must_use]
    pub fn keys(&self) -> &[Keyframe] {
        &self.keys
    }

    /// Extrapolation before the first key.
    #[must_use]
    pub fn pre(&self) -> Extrapolation {
        self.pre
    }

    /// Extrapolation after the last key.
    #[must_use]
    pub fn post(&self) -> Extrapolation {
        self.post
    }

    /// Time offset in seconds.
    #[must_use]
    pub fn offset(&self) -> f64 {
        self.offset
    }

    /// The type of every value [`AnimChannel::sample`] returns.
    #[must_use]
    pub fn value_type(&self) -> PortType {
        self.keys
            .first()
            .map_or(PortType::Float, |k| k.value.port_type())
    }

    /// The value at `time` seconds. Deterministic and allocation-free.
    #[must_use]
    // Component loops index several parallel arrays.
    #[allow(clippy::needless_range_loop)]
    pub fn sample(&self, time: f64) -> Value {
        let ty = self.value_type();
        let (Some(first), Some(last)) = (self.keys.first(), self.keys.last()) else {
            return Value::Float(0.0);
        };
        let t = time - self.offset;
        if t.is_nan() || t == f64::NEG_INFINITY {
            return first.value.clone();
        }
        if t == f64::INFINITY {
            return last.value.clone();
        }
        let (start, end) = (first.time, last.time);
        let duration = end - start;
        let t = if t < start {
            match self.pre {
                Extrapolation::Hold => return first.value.clone(),
                mode => wrap(t, start, duration, mode),
            }
        } else if t > end {
            match self.post {
                Extrapolation::Hold => return last.value.clone(),
                mode => wrap(t, start, duration, mode),
            }
        } else {
            t
        };
        if self.keys.len() == 1 {
            return first.value.clone();
        }

        // Segment [i, i + 1] containing t.
        let n = self.keys.len();
        let i = self
            .keys
            .partition_point(|k| k.time <= t)
            .saturating_sub(1)
            .min(n - 2);
        let (k0, k1) = (&self.keys[i], &self.keys[i + 1]);
        let (Some(a), Some(b)) = (comps(&k0.value), comps(&k1.value)) else {
            return k0.value.clone();
        };
        if t >= k1.time {
            return k1.value.clone();
        }
        let h = k1.time - k0.time;
        let s = ((t - k0.time) / h).clamp(0.0, 1.0);
        let mut out = [0.0; 4];
        match k0.interpolation {
            Interpolation::Step => return k0.value.clone(),
            Interpolation::Linear => {
                for c in 0..a.n {
                    out[c] = a.v[c] + (b.v[c] - a.v[c]) * s;
                }
            }
            Interpolation::Bezier => {
                let m0 = self.slope(i, false);
                let m1 = self.slope(i + 1, true);
                let s2 = s * s;
                let s3 = s2 * s;
                let h00 = 2.0 * s3 - 3.0 * s2 + 1.0;
                let h10 = s3 - 2.0 * s2 + s;
                let h01 = -2.0 * s3 + 3.0 * s2;
                let h11 = s3 - s2;
                for c in 0..a.n {
                    out[c] = h00 * a.v[c] + h10 * h * m0[c] + h01 * b.v[c] + h11 * h * m1[c];
                }
            }
            Interpolation::Spring { frequency, damping } => {
                let g = spring_decay(t - k0.time, frequency, damping);
                for c in 0..a.n {
                    out[c] = b.v[c] + (a.v[c] - b.v[c]) * g;
                }
            }
        }
        from_comps(ty, out)
    }

    /// The incoming (`incoming == true`) or outgoing slope of key `i`, per
    /// component.
    #[allow(clippy::needless_range_loop)]
    fn slope(&self, i: usize, incoming: bool) -> [f64; 4] {
        let mut out = [0.0; 4];
        let Some(key) = self.keys.get(i) else {
            return out;
        };
        match &key.tangent {
            Tangent::Flat => out,
            Tangent::Explicit {
                incoming: inc,
                outgoing: outg,
            } => comps(if incoming { inc } else { outg }).map_or(out, |c| c.v),
            Tangent::Auto => {
                if i == 0 || i + 1 >= self.keys.len() {
                    return out;
                }
                let (p, k, q) = (&self.keys[i - 1], key, &self.keys[i + 1]);
                let (Some(pv), Some(kv), Some(qv)) =
                    (comps(&p.value), comps(&k.value), comps(&q.value))
                else {
                    return out;
                };
                for c in 0..kv.n {
                    let d0 = (kv.v[c] - pv.v[c]) / (k.time - p.time);
                    let d1 = (qv.v[c] - kv.v[c]) / (q.time - k.time);
                    out[c] = if d0 * d1 <= 0.0 {
                        0.0
                    } else {
                        let m = 0.5 * (d0 + d1);
                        let limit = 3.0 * d0.abs().min(d1.abs());
                        m.clamp(-limit, limit)
                    };
                }
                out
            }
        }
    }
}

/// Map `t` outside `[start, start + duration]` back into it.
fn wrap(t: f64, start: f64, duration: f64, mode: Extrapolation) -> f64 {
    if duration <= 0.0 {
        return start;
    }
    match mode {
        Extrapolation::Hold => t.clamp(start, start + duration),
        Extrapolation::Loop => start + (t - start).rem_euclid(duration),
        Extrapolation::PingPong => {
            let u = (t - start).rem_euclid(2.0 * duration);
            if u <= duration {
                start + u
            } else {
                start + 2.0 * duration - u
            }
        }
    }
}

/// Remaining fraction of the initial displacement of a damped spring
/// released at rest, after `tau` seconds: 1 at `tau == 0`, tending to 0.
fn spring_decay(tau: f64, frequency: f64, damping: f64) -> f64 {
    let omega = std::f64::consts::TAU * frequency;
    if damping < 1.0 {
        let wd = omega * libm::sqrt(1.0 - damping * damping);
        libm::exp(-damping * omega * tau)
            * (libm::cos(wd * tau) + damping * omega / wd * libm::sin(wd * tau))
    } else if damping == 1.0 {
        libm::exp(-omega * tau) * (1.0 + omega * tau)
    } else {
        let root = libm::sqrt(damping * damping - 1.0);
        let r1 = -omega * (damping - root);
        let r2 = -omega * (damping + root);
        (r2 * libm::exp(r1 * tau) - r1 * libm::exp(r2 * tau)) / (r2 - r1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(v: &Value) -> f64 {
        v.as_float().unwrap()
    }

    fn ramp(interp: Interpolation) -> AnimChannel {
        AnimChannel::new(vec![
            Keyframe::float(0.0, 0.0).with_interpolation(interp),
            Keyframe::float(1.0, 10.0).with_interpolation(interp),
            Keyframe::float(2.0, 0.0),
        ])
        .unwrap()
    }

    #[test]
    fn validation() {
        assert_eq!(AnimChannel::new(vec![]), Err(AnimError::NoKeys));
        assert_eq!(
            AnimChannel::new(vec![Keyframe::float(1.0, 0.0), Keyframe::float(1.0, 1.0)]),
            Err(AnimError::NotIncreasing { index: 1 })
        );
        assert_eq!(
            AnimChannel::new(vec![Keyframe::float(f64::NAN, 0.0)]),
            Err(AnimError::NonFinite { index: 0 })
        );
        assert_eq!(
            AnimChannel::new(vec![Keyframe::new(0.0, Value::Int(1))]),
            Err(AnimError::UnsupportedType {
                index: 0,
                ty: PortType::Int
            })
        );
        assert_eq!(
            AnimChannel::new(vec![
                Keyframe::float(0.0, 0.0),
                Keyframe::new(1.0, Value::Vec2([0.0; 2]))
            ]),
            Err(AnimError::MixedTypes {
                index: 1,
                expected: PortType::Float,
                found: PortType::Vec2
            })
        );
        assert_eq!(
            AnimChannel::new(vec![Keyframe::float(0.0, 0.0).with_interpolation(
                Interpolation::Spring {
                    frequency: 0.0,
                    damping: 1.0
                }
            )]),
            Err(AnimError::InvalidSpring { index: 0 })
        );
        assert_eq!(
            AnimChannel::new(vec![Keyframe::float(0.0, 0.0).with_tangent(
                Tangent::Explicit {
                    incoming: Value::Float(0.0),
                    outgoing: Value::Vec2([0.0; 2]),
                }
            )]),
            Err(AnimError::MixedTypes {
                index: 0,
                expected: PortType::Float,
                found: PortType::Vec2
            })
        );
        assert_eq!(
            AnimChannel::constant(1.0)
                .unwrap()
                .with_offset(f64::INFINITY),
            Err(AnimError::NonFiniteOffset)
        );
    }

    #[test]
    fn step_and_linear() {
        let step = ramp(Interpolation::Step);
        assert_eq!(f(&step.sample(0.99)), 0.0);
        assert_eq!(f(&step.sample(1.0)), 10.0);
        let lin = ramp(Interpolation::Linear);
        assert_eq!(f(&lin.sample(0.25)), 2.5);
        assert_eq!(f(&lin.sample(1.5)), 5.0);
        assert_eq!(f(&lin.sample(2.0)), 0.0);
    }

    #[test]
    fn bezier_hits_keys_and_auto_clamps() {
        let ch = ramp(Interpolation::Bezier);
        assert_eq!(f(&ch.sample(0.0)), 0.0);
        assert_eq!(f(&ch.sample(1.0)), 10.0);
        // Key 1 is a maximum: flat tangent, so no overshoot.
        for i in 0..=200 {
            let v = f(&ch.sample(f64::from(i) * 0.01));
            assert!((0.0..=10.0).contains(&v), "{v}");
        }
        // Symmetric ease with flat ends.
        assert!((f(&ch.sample(0.5)) - 5.0).abs() < 1e-12);
        // Monotone data stays monotone.
        let mono = AnimChannel::new(vec![
            Keyframe::float(0.0, 0.0).with_interpolation(Interpolation::Bezier),
            Keyframe::float(1.0, 1.0).with_interpolation(Interpolation::Bezier),
            Keyframe::float(1.1, 9.0).with_interpolation(Interpolation::Bezier),
            Keyframe::float(3.0, 10.0),
        ])
        .unwrap();
        let mut prev = f64::NEG_INFINITY;
        for i in 0..=300 {
            let v = f(&mono.sample(f64::from(i) * 0.01));
            assert!(v >= prev - 1e-12, "not monotone at {i}: {v} < {prev}");
            prev = v;
        }
    }

    #[test]
    fn explicit_tangents() {
        let slope = |s: f64| Tangent::Explicit {
            incoming: Value::Float(s),
            outgoing: Value::Float(s),
        };
        let ch = AnimChannel::new(vec![
            Keyframe::float(0.0, 0.0)
                .with_interpolation(Interpolation::Bezier)
                .with_tangent(slope(1.0)),
            Keyframe::float(1.0, 1.0).with_tangent(slope(1.0)),
        ])
        .unwrap();
        // Slopes equal to the secant reproduce the straight line.
        for i in 0..=10 {
            let t = f64::from(i) * 0.1;
            assert!((f(&ch.sample(t)) - t).abs() < 1e-12);
        }
    }

    #[test]
    fn spring_settles_and_ends_on_key() {
        let spring = |damping| {
            AnimChannel::new(vec![
                Keyframe::float(0.0, 0.0).with_interpolation(Interpolation::Spring {
                    frequency: 2.0,
                    damping,
                }),
                Keyframe::float(5.0, 1.0),
            ])
            .unwrap()
        };
        for damping in [0.0, 0.3, 1.0, 2.5] {
            let ch = spring(damping);
            assert_eq!(f(&ch.sample(0.0)), 0.0);
            assert_eq!(f(&ch.sample(5.0)), 1.0);
        }
        // Under-damped overshoots, critically damped does not.
        let under = spring(0.3);
        assert!((0..500).any(|i| f(&under.sample(f64::from(i) * 0.01)) > 1.0));
        let critical = spring(1.0);
        assert!((0..500).all(|i| f(&critical.sample(f64::from(i) * 0.01)) <= 1.0));
        assert!((f(&spring(2.5).sample(4.99)) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn extrapolation_and_offset() {
        let ch =
            AnimChannel::new(vec![Keyframe::float(1.0, 0.0), Keyframe::float(3.0, 2.0)]).unwrap();
        assert_eq!(f(&ch.sample(-5.0)), 0.0);
        assert_eq!(f(&ch.sample(9.0)), 2.0);
        let looped = ch
            .clone()
            .with_extrapolation(Extrapolation::Loop, Extrapolation::Loop);
        assert_eq!(f(&looped.sample(3.5)), 0.5);
        assert_eq!(f(&looped.sample(0.5)), 1.5);
        let pp = ch
            .clone()
            .with_extrapolation(Extrapolation::PingPong, Extrapolation::PingPong);
        assert_eq!(f(&pp.sample(3.5)), 1.5);
        assert_eq!(f(&pp.sample(5.5)), 0.5);
        assert_eq!(f(&pp.sample(0.5)), 0.5);
        let shifted = ch.with_offset(10.0).unwrap();
        assert_eq!(f(&shifted.sample(12.0)), 1.0);
        assert_eq!(f(&looped.sample(f64::INFINITY)), 2.0);
        assert_eq!(f(&looped.sample(f64::NAN)), 0.0);
    }

    #[test]
    fn vectors_are_component_wise() {
        let ch = AnimChannel::new(vec![
            Keyframe::new(0.0, Value::Color([0.5, 0.1, 0.0, 1.0])),
            Keyframe::new(2.0, Value::Color([0.9, 0.3, 360.0, 0.0])),
        ])
        .unwrap();
        assert_eq!(ch.value_type(), PortType::Color);
        assert_eq!(ch.sample(1.0), Value::Color([0.7, 0.2, 180.0, 0.5]));
        let single = AnimChannel::new(vec![Keyframe::new(4.0, Value::Vec2([1.0, 2.0]))])
            .unwrap()
            .with_extrapolation(Extrapolation::Loop, Extrapolation::PingPong);
        assert_eq!(single.sample(-3.0), Value::Vec2([1.0, 2.0]));
        assert_eq!(single.sample(30.0), Value::Vec2([1.0, 2.0]));
    }
}
