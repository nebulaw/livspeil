// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Animation: [`Animatable`] values, springs (velocity-preserving
//! retargeting) and tweens (easings).
//!
//! Everything here is `Copy` and stored inline: stepping, retargeting and
//! velocity hand-off never allocate.

use std::time::Duration;

use livspeil_ui_layout::{Transform2D, Vec2};
use livspeil_ui_theme::{Color, MotionSpec, SpringSpec};

/// A value that can be animated: it maps to a small vector of floats in a
/// space where linear interpolation looks right (OKLab for colors).
pub trait Animatable: Copy + PartialEq + std::fmt::Debug + 'static {
    /// The float vector.
    type Repr: Copy + Default + std::fmt::Debug + PartialEq + AsRef<[f32]> + AsMut<[f32]>;
    /// Settle threshold for displacement, in `Repr` units.
    const EPSILON: f32;
    /// To floats.
    fn to_repr(self) -> Self::Repr;
    /// From floats.
    fn from_repr(r: Self::Repr) -> Self;
}

impl Animatable for f32 {
    type Repr = [f32; 1];
    const EPSILON: f32 = 1e-3;
    fn to_repr(self) -> [f32; 1] {
        [self]
    }
    fn from_repr(r: [f32; 1]) -> Self {
        r[0]
    }
}

impl Animatable for Vec2 {
    type Repr = [f32; 2];
    const EPSILON: f32 = 0.01;
    fn to_repr(self) -> [f32; 2] {
        [self.x, self.y]
    }
    fn from_repr(r: [f32; 2]) -> Self {
        Vec2::new(r[0], r[1])
    }
}

/// Colors animate in OKLab with straight alpha.
impl Animatable for Color {
    type Repr = [f32; 4];
    const EPSILON: f32 = 5e-4;
    fn to_repr(self) -> [f32; 4] {
        [self.l, self.a, self.b, self.alpha]
    }
    fn from_repr(r: [f32; 4]) -> Self {
        Color::oklab(r[0], r[1], r[2], r[3])
    }
}

/// A decomposed 2D transform (translate, scale, rotation) that animates
/// component-wise, so rotations turn instead of shearing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransformParts {
    /// Translation.
    pub translate: Vec2,
    /// Scale.
    pub scale: Vec2,
    /// Rotation in radians.
    pub rotation: f32,
}

impl Default for TransformParts {
    fn default() -> Self {
        TransformParts {
            translate: Vec2::ZERO,
            scale: Vec2::new(1.0, 1.0),
            rotation: 0.0,
        }
    }
}

impl TransformParts {
    /// The matrix: scale, then rotate, then translate (about the origin).
    #[must_use]
    pub fn to_transform(&self) -> Transform2D {
        Transform2D::scale(self.scale.x, self.scale.y)
            .then(Transform2D::rotate(self.rotation))
            .then(Transform2D::translate(self.translate))
    }
}

impl Animatable for TransformParts {
    type Repr = [f32; 5];
    const EPSILON: f32 = 1e-3;
    fn to_repr(self) -> [f32; 5] {
        [
            self.translate.x,
            self.translate.y,
            self.scale.x,
            self.scale.y,
            self.rotation,
        ]
    }
    fn from_repr(r: [f32; 5]) -> Self {
        TransformParts {
            translate: Vec2::new(r[0], r[1]),
            scale: Vec2::new(r[2], r[3]),
            rotation: r[4],
        }
    }
}

/// Tween easing curves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Easing {
    /// Constant speed.
    Linear,
    /// Cubic ease in.
    EaseIn,
    /// Cubic ease out.
    EaseOut,
    /// Cubic ease in-out.
    EaseInOut,
    /// CSS-style cubic Bézier `(x1, y1, x2, y2)`.
    CubicBezier(f32, f32, f32, f32),
}

impl Easing {
    /// Map progress `t` in 0..=1 to eased progress.
    #[must_use]
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::EaseIn => t * t * t,
            Easing::EaseOut => 1.0 - (1.0 - t).powi(3),
            Easing::EaseInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Easing::CubicBezier(x1, y1, x2, y2) => cubic_bezier(x1, y1, x2, y2, t),
        }
    }
}

fn bezier(a: f32, b: f32, s: f32) -> f32 {
    // B(s) with P0 = 0, P3 = 1.
    let u = 1.0 - s;
    3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
}

fn bezier_slope(a: f32, b: f32, s: f32) -> f32 {
    let u = 1.0 - s;
    3.0 * u * u * a + 6.0 * u * s * (b - a) + 3.0 * s * s * (1.0 - b)
}

fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    // Newton iterations, falling back to bisection.
    let mut s = x;
    for _ in 0..8 {
        let err = bezier(x1, x2, s) - x;
        if err.abs() < 1e-5 {
            return bezier(y1, y2, s);
        }
        let d = bezier_slope(x1, x2, s);
        if d.abs() < 1e-6 {
            break;
        }
        s = (s - err / d).clamp(0.0, 1.0);
    }
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    s = x;
    for _ in 0..30 {
        let v = bezier(x1, x2, s);
        if (v - x).abs() < 1e-5 {
            break;
        }
        if v < x {
            lo = s;
        } else {
            hi = s;
        }
        s = 0.5 * (lo + hi);
    }
    bezier(y1, y2, s)
}

/// Advance one spring component analytically by `dt` seconds.
///
/// `x` is the displacement from the target, `v` the velocity; `omega` is
/// the undamped angular frequency (`2π / response`) and `zeta` the damping
/// ratio. The closed form is exact for any `dt`, so results do not depend
/// on frame rate.
#[must_use]
pub fn spring_step(x: f32, v: f32, dt: f32, omega: f32, zeta: f32) -> (f32, f32) {
    if dt <= 0.0 {
        return (x, v);
    }
    if (zeta - 1.0).abs() < 1e-4 {
        let e = (-omega * dt).exp();
        let c = v + omega * x;
        ((x + c * dt) * e, (v - omega * c * dt) * e)
    } else if zeta < 1.0 {
        let wd = omega * (1.0 - zeta * zeta).sqrt();
        let a = zeta * omega;
        let e = (-a * dt).exp();
        let (s, c) = (wd * dt).sin_cos();
        let x1 = e * (x * c + (v + a * x) / wd * s);
        let v1 = e * (v * c - (a * v + omega * omega * x) / wd * s);
        (x1, v1)
    } else {
        let root = (zeta * zeta - 1.0).sqrt();
        let r1 = -omega * (zeta - root);
        let r2 = -omega * (zeta + root);
        let c1 = (v - r2 * x) / (r1 - r2);
        let c2 = x - c1;
        let (e1, e2) = ((r1 * dt).exp(), (r2 * dt).exp());
        (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Driver<T> {
    Idle,
    Spring {
        omega: f32,
        zeta: f32,
    },
    Tween {
        from: T,
        elapsed: f32,
        duration: f32,
        easing: Easing,
    },
}

/// An animated value: current value, velocity, target and driver, inline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Animated<T: Animatable> {
    value: T,
    velocity: T::Repr,
    target: T,
    driver: Driver<T>,
}

impl<T: Animatable> Animated<T> {
    /// At rest at `value`.
    pub fn new(value: T) -> Self {
        Animated {
            value,
            velocity: T::Repr::default(),
            target: value,
            driver: Driver::Idle,
        }
    }

    /// Current value.
    pub fn value(&self) -> T {
        self.value
    }

    /// Target value.
    pub fn target(&self) -> T {
        self.target
    }

    /// Current velocity (units per second).
    pub fn velocity(&self) -> T {
        T::from_repr(self.velocity)
    }

    /// Whether a velocity is set (for example handed off by a gesture).
    pub fn has_velocity(&self) -> bool {
        self.velocity.as_ref().iter().any(|v| *v != 0.0)
    }

    /// Whether it is moving.
    pub fn is_animating(&self) -> bool {
        !matches!(self.driver, Driver::Idle)
    }

    /// Jump to `value` and stop.
    pub fn set(&mut self, value: T) {
        *self = Animated::new(value);
    }

    /// Jump to the target and stop.
    pub fn finish(&mut self) {
        let target = self.target;
        self.set(target);
    }

    /// Animate to `target` as `spec` says. Springs keep the current value
    /// and velocity (retargeting is seamless); `Instant` jumps. Returns
    /// whether it is now animating.
    pub fn animate_to(&mut self, target: T, spec: MotionSpec) -> bool {
        match spec {
            MotionSpec::Instant => {
                self.set(target);
                false
            }
            MotionSpec::Spring(s) => self.spring_to(target, s),
        }
    }

    /// Spring to `target`, preserving velocity.
    pub fn spring_to(&mut self, target: T, spring: SpringSpec) -> bool {
        if spring.response <= 0.0 {
            self.set(target);
            return false;
        }
        self.target = target;
        self.driver = Driver::Spring {
            omega: std::f32::consts::TAU / spring.response,
            zeta: spring.damping.max(0.01),
        };
        if self.is_settled() {
            self.finish();
            return false;
        }
        true
    }

    /// Tween from the current value to `target`.
    pub fn tween_to(&mut self, target: T, duration: Duration, easing: Easing) -> bool {
        let d = duration.as_secs_f32();
        if d <= 0.0 {
            self.set(target);
            return false;
        }
        self.target = target;
        self.velocity = T::Repr::default();
        self.driver = Driver::Tween {
            from: self.value,
            elapsed: 0.0,
            duration: d,
            easing,
        };
        true
    }

    /// Hand a velocity to the animation (for example a gesture's release
    /// velocity). Takes effect on the next spring step; a resting value
    /// needs a spring retarget to move.
    pub fn set_velocity(&mut self, velocity: T) {
        self.velocity = velocity.to_repr();
    }

    fn is_settled(&self) -> bool {
        let v = self.value.to_repr();
        let t = self.target.to_repr();
        let eps = T::EPSILON;
        v.as_ref()
            .iter()
            .zip(t.as_ref())
            .all(|(a, b)| (a - b).abs() < eps)
            && self.velocity.as_ref().iter().all(|x| x.abs() < eps * 20.0)
    }

    /// Advance by `dt` seconds. Returns whether it is still moving.
    pub fn step(&mut self, dt: f32) -> bool {
        match self.driver {
            Driver::Idle => false,
            Driver::Spring { omega, zeta } => {
                let mut value = self.value.to_repr();
                let target = self.target.to_repr();
                for ((x, v), t) in value
                    .as_mut()
                    .iter_mut()
                    .zip(self.velocity.as_mut().iter_mut())
                    .zip(target.as_ref())
                {
                    let (nx, nv) = spring_step(*x - t, *v, dt, omega, zeta);
                    *x = t + nx;
                    *v = nv;
                }
                self.value = T::from_repr(value);
                if self.is_settled() {
                    self.finish();
                    return false;
                }
                true
            }
            Driver::Tween {
                from,
                elapsed,
                duration,
                easing,
            } => {
                let elapsed = elapsed + dt;
                if elapsed >= duration {
                    self.finish();
                    return false;
                }
                let k = easing.apply(elapsed / duration);
                let mut value = from.to_repr();
                let target = self.target.to_repr();
                for (x, t) in value.as_mut().iter_mut().zip(target.as_ref()) {
                    *x += (t - *x) * k;
                }
                self.value = T::from_repr(value);
                self.driver = Driver::Tween {
                    from,
                    elapsed,
                    duration,
                    easing,
                };
                true
            }
        }
    }
}

impl<T: Animatable + Default> Default for Animated<T> {
    fn default() -> Self {
        Animated::new(T::default())
    }
}
