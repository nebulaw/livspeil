// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Geometry in logical pixels (y down). The frame's scale factor maps
//! logical to physical pixels.

/// An axis-aligned rectangle: origin and size, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl Rect {
    /// A rectangle from origin and size.
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    /// Right edge.
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    /// Bottom edge.
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// Grown by `d` on every side (shrunk for negative `d`).
    #[must_use]
    pub fn inflate(&self, d: f32) -> Rect {
        Rect::new(self.x - d, self.y - d, self.w + 2.0 * d, self.h + 2.0 * d)
    }

    /// Moved by `(dx, dy)`.
    #[must_use]
    pub fn translate(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// Whether all components are finite and the size is not negative.
    pub fn is_valid(&self) -> bool {
        [self.x, self.y, self.w, self.h]
            .iter()
            .all(|v| v.is_finite())
            && self.w >= 0.0
            && self.h >= 0.0
    }
}

/// A 2D affine transform: `x' = a*x + c*y + e`, `y' = b*x + d*y + f`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    /// Matrix entries in column-major order `[a, b, c, d, e, f]`.
    pub m: [f32; 6],
}

impl Default for Affine {
    fn default() -> Self {
        Affine::IDENTITY
    }
}

impl Affine {
    /// The identity.
    pub const IDENTITY: Affine = Affine {
        m: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    };

    /// A translation.
    pub fn translate(dx: f32, dy: f32) -> Affine {
        Affine {
            m: [1.0, 0.0, 0.0, 1.0, dx, dy],
        }
    }

    /// A scale about the origin.
    pub fn scale(sx: f32, sy: f32) -> Affine {
        Affine {
            m: [sx, 0.0, 0.0, sy, 0.0, 0.0],
        }
    }

    /// A rotation about the origin (radians, clockwise on screen since y is
    /// down).
    pub fn rotate(radians: f32) -> Affine {
        let (s, c) = radians.sin_cos();
        Affine {
            m: [c, s, -s, c, 0.0, 0.0],
        }
    }

    /// A rotation about the point `(cx, cy)`.
    pub fn rotate_about(radians: f32, cx: f32, cy: f32) -> Affine {
        Affine::translate(cx, cy)
            .then_after(Affine::rotate(radians))
            .then_after(Affine::translate(-cx, -cy))
    }

    /// `self * other`: applies `other` first, then `self`.
    #[must_use]
    pub fn then_after(self, other: Affine) -> Affine {
        let [a, b, c, d, e, f] = self.m;
        let [a2, b2, c2, d2, e2, f2] = other.m;
        Affine {
            m: [
                a * a2 + c * b2,
                b * a2 + d * b2,
                a * c2 + c * d2,
                b * c2 + d * d2,
                a * e2 + c * f2 + e,
                b * e2 + d * f2 + f,
            ],
        }
    }

    /// Transforms a point.
    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.m;
        (a * x + c * y + e, b * x + d * y + f)
    }

    /// Determinant of the linear part.
    pub fn det(&self) -> f32 {
        self.m[0] * self.m[3] - self.m[1] * self.m[2]
    }

    /// The inverse, or `None` if the transform is singular or not finite.
    pub fn inverse(&self) -> Option<Affine> {
        let det = self.det();
        if !det.is_finite() || det.abs() < 1e-12 || self.m.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let [a, b, c, d, e, f] = self.m;
        let inv = 1.0 / det;
        let (ia, ib, ic, id) = (d * inv, -b * inv, -c * inv, a * inv);
        Some(Affine {
            m: [ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)],
        })
    }

    /// Axis-aligned bounds of a transformed rectangle.
    pub fn bounds(&self, r: Rect) -> Rect {
        let pts = [
            self.apply(r.x, r.y),
            self.apply(r.right(), r.y),
            self.apply(r.x, r.bottom()),
            self.apply(r.right(), r.bottom()),
        ];
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (x, y) in pts {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_round_trips() {
        let t = Affine::rotate_about(0.3, 10.0, 20.0).then_after(Affine::scale(2.0, 1.5));
        let inv = t.inverse().unwrap();
        let (x, y) = t.apply(3.0, -4.0);
        let (bx, by) = inv.apply(x, y);
        assert!((bx - 3.0).abs() < 1e-4 && (by + 4.0).abs() < 1e-4);
    }
}
