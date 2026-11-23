// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! 2D geometry in logical pixels: vectors, sizes, rects, corner radii and
//! affine transforms.

use std::fmt;
use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

/// A 2D vector or point.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec2 {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
}

/// Points are vectors from the origin.
pub type Point = Vec2;

impl Vec2 {
    /// Zero.
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    /// A vector.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Vec2 { x, y }
    }

    /// Euclidean length.
    #[must_use]
    pub fn length(self) -> f32 {
        self.x.hypot(self.y)
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}

impl AddAssign for Vec2 {
    fn add_assign(&mut self, o: Vec2) {
        *self = *self + o;
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}

impl SubAssign for Vec2 {
    fn sub_assign(&mut self, o: Vec2) {
        *self = *self - o;
    }
}

impl Mul<f32> for Vec2 {
    type Output = Vec2;
    fn mul(self, s: f32) -> Vec2 {
        Vec2::new(self.x * s, self.y * s)
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

/// A width and height.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Size {
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

impl Size {
    /// Zero.
    pub const ZERO: Size = Size {
        width: 0.0,
        height: 0.0,
    };

    /// A size.
    #[must_use]
    pub const fn new(width: f32, height: f32) -> Self {
        Size { width, height }
    }
}

/// An axis-aligned rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    /// Left.
    pub x: f32,
    /// Top.
    pub y: f32,
    /// Width (>= 0).
    pub width: f32,
    /// Height (>= 0).
    pub height: f32,
}

impl Rect {
    /// Zero-sized at the origin.
    pub const ZERO: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    };

    /// A rect.
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    /// A rect at `origin` with `size`.
    #[must_use]
    pub const fn from_origin_size(origin: Point, size: Size) -> Self {
        Rect::new(origin.x, origin.y, size.width, size.height)
    }

    /// Top-left corner.
    #[must_use]
    pub fn origin(&self) -> Point {
        Vec2::new(self.x, self.y)
    }

    /// Size.
    #[must_use]
    pub fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    /// Right edge.
    #[must_use]
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    /// Bottom edge.
    #[must_use]
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    /// Center point.
    #[must_use]
    pub fn center(&self) -> Point {
        Vec2::new(self.x + self.width * 0.5, self.y + self.height * 0.5)
    }

    /// Whether `p` is inside (left/top inclusive, right/bottom exclusive).
    #[must_use]
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.right() && p.y < self.bottom()
    }

    /// Whether the rects overlap with positive area.
    #[must_use]
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }

    /// The overlapping part, or `None`.
    #[must_use]
    pub fn intersection(&self, o: &Rect) -> Option<Rect> {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        (r > x && b > y).then(|| Rect::new(x, y, r - x, b - y))
    }

    /// The smallest rect containing both.
    #[must_use]
    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect::new(
            x,
            y,
            self.right().max(o.right()) - x,
            self.bottom().max(o.bottom()) - y,
        )
    }

    /// Grow by `d` on every side (shrink if negative; never below zero size).
    #[must_use]
    pub fn inflate(&self, d: f32) -> Rect {
        Rect::new(
            self.x - d,
            self.y - d,
            (self.width + 2.0 * d).max(0.0),
            (self.height + 2.0 * d).max(0.0),
        )
    }

    /// Moved by `v`.
    #[must_use]
    pub fn translate(&self, v: Vec2) -> Rect {
        Rect::new(self.x + v.x, self.y + v.y, self.width, self.height)
    }
}

impl fmt::Display for Rect {
    /// `x,y wxh` with up to two decimals, trailing zeros trimmed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{},{} {}x{}",
            num(self.x),
            num(self.y),
            num(self.width),
            num(self.height)
        )
    }
}

/// Format a number with at most two decimals, trimming zeros (for dumps).
#[must_use]
pub fn num(v: f32) -> String {
    let v = if v == 0.0 { 0.0 } else { v };
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".to_owned() } else { s.to_owned() }
}

/// Per-corner radii.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Corners {
    /// Top-left.
    pub top_left: f32,
    /// Top-right.
    pub top_right: f32,
    /// Bottom-right.
    pub bottom_right: f32,
    /// Bottom-left.
    pub bottom_left: f32,
}

impl Corners {
    /// The same radius on every corner.
    #[must_use]
    pub const fn all(r: f32) -> Self {
        Corners {
            top_left: r,
            top_right: r,
            bottom_right: r,
            bottom_left: r,
        }
    }

    /// Every radius plus `d` (clamped at zero).
    #[must_use]
    pub fn grow(&self, d: f32) -> Self {
        Corners {
            top_left: (self.top_left + d).max(0.0),
            top_right: (self.top_right + d).max(0.0),
            bottom_right: (self.bottom_right + d).max(0.0),
            bottom_left: (self.bottom_left + d).max(0.0),
        }
    }

    /// Whether all radii are zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        *self == Corners::default()
    }
}

/// A 2D affine transform: `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform2D {
    /// Row 0, column 0.
    pub a: f32,
    /// Row 1, column 0.
    pub b: f32,
    /// Row 0, column 1.
    pub c: f32,
    /// Row 1, column 1.
    pub d: f32,
    /// X translation.
    pub e: f32,
    /// Y translation.
    pub f: f32,
}

impl Default for Transform2D {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform2D {
    /// Identity.
    pub const IDENTITY: Transform2D = Transform2D {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// A translation.
    #[must_use]
    pub const fn translate(v: Vec2) -> Self {
        Transform2D {
            e: v.x,
            f: v.y,
            ..Self::IDENTITY
        }
    }

    /// A scale about the origin.
    #[must_use]
    pub const fn scale(sx: f32, sy: f32) -> Self {
        Transform2D {
            a: sx,
            d: sy,
            ..Self::IDENTITY
        }
    }

    /// A rotation about the origin (radians, clockwise in y-down space).
    #[must_use]
    pub fn rotate(radians: f32) -> Self {
        let (s, c) = radians.sin_cos();
        Transform2D {
            a: c,
            b: s,
            c: -s,
            d: c,
            e: 0.0,
            f: 0.0,
        }
    }

    /// `self` applied about `origin` instead of (0, 0).
    #[must_use]
    pub fn about(self, origin: Point) -> Self {
        Transform2D::translate(-origin)
            .then(self)
            .then(Transform2D::translate(origin))
    }

    /// Apply `self`, then `next`.
    #[must_use]
    pub fn then(self, next: Transform2D) -> Self {
        let m = next;
        Transform2D {
            a: m.a * self.a + m.c * self.b,
            b: m.b * self.a + m.d * self.b,
            c: m.a * self.c + m.c * self.d,
            d: m.b * self.c + m.d * self.d,
            e: m.a * self.e + m.c * self.f + m.e,
            f: m.b * self.e + m.d * self.f + m.f,
        }
    }

    /// The inverse, or `None` if singular.
    #[must_use]
    pub fn inverse(&self) -> Option<Transform2D> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        let inv = 1.0 / det;
        Some(Transform2D {
            a: self.d * inv,
            b: -self.b * inv,
            c: -self.c * inv,
            d: self.a * inv,
            e: (self.c * self.f - self.d * self.e) * inv,
            f: (self.b * self.e - self.a * self.f) * inv,
        })
    }

    /// Transform a point.
    #[must_use]
    pub fn apply(&self, p: Point) -> Point {
        Vec2::new(
            self.a * p.x + self.c * p.y + self.e,
            self.b * p.x + self.d * p.y + self.f,
        )
    }

    /// The axis-aligned bounds of a transformed rect.
    #[must_use]
    pub fn apply_rect(&self, r: &Rect) -> Rect {
        let pts = [
            self.apply(Vec2::new(r.x, r.y)),
            self.apply(Vec2::new(r.right(), r.y)),
            self.apply(Vec2::new(r.x, r.bottom())),
            self.apply(Vec2::new(r.right(), r.bottom())),
        ];
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in pts {
            x0 = x0.min(p.x);
            y0 = y0.min(p.y);
            x1 = x1.max(p.x);
            y1 = y1.max(p.y);
        }
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// Whether this is (nearly) the identity.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        let t = Transform2D::IDENTITY;
        (self.a - t.a).abs() < 1e-6
            && self.b.abs() < 1e-6
            && self.c.abs() < 1e-6
            && (self.d - t.d).abs() < 1e-6
            && self.e.abs() < 1e-6
            && self.f.abs() < 1e-6
    }
}

impl fmt::Display for Transform2D {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{} {} {} {} {} {}]",
            num(self.a),
            num(self.b),
            num(self.c),
            num(self.d),
            num(self.e),
            num(self.f)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_compose_and_invert() {
        let t = Transform2D::scale(2.0, 2.0)
            .then(Transform2D::rotate(0.5))
            .then(Transform2D::translate(Vec2::new(10.0, -3.0)));
        let p = Vec2::new(3.0, 4.0);
        let q = t.apply(p);
        let back = t.inverse().unwrap().apply(q);
        assert!((back - p).length() < 1e-4);
        let about = Transform2D::scale(2.0, 2.0).about(Vec2::new(10.0, 10.0));
        assert_eq!(about.apply(Vec2::new(10.0, 10.0)), Vec2::new(10.0, 10.0));
        assert_eq!(about.apply(Vec2::new(11.0, 10.0)), Vec2::new(12.0, 10.0));
        assert!(Transform2D::scale(0.0, 1.0).inverse().is_none());
    }

    #[test]
    fn rect_ops() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(a.intersection(&b), Some(Rect::new(5.0, 5.0, 5.0, 5.0)));
        assert_eq!(a.union(&b), Rect::new(0.0, 0.0, 15.0, 15.0));
        assert!(a.contains(Vec2::new(0.0, 9.9)));
        assert!(!a.contains(Vec2::new(10.0, 5.0)));
        assert_eq!(format!("{}", Rect::new(1.5, -0.0, 2.25, 3.0)), "1.5,0 2.25x3");
    }
}
