// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Wires: stroked cubic Bézier curves (node editor links, connectors).
//!
//! An element carries wires with [`Element::wires`] in coordinates
//! relative to its top-left corner; they are painted (offset to window
//! space, under any enclosing transform such as a canvas's pan/zoom) after
//! its background and before its children, as [`PaintCmd::Wire`]s. The paint
//! bridge turns each into one curve primitive of the renderer;
//! [`Wire::flatten`] serves hit testing ([`Wire::distance_to`]).
//!
//! [`PaintCmd::Wire`]: crate::PaintCmd::Wire

use livspeil_ui_layout::{Point, Rect, Vec2};
use livspeil_ui_theme::Color;

use crate::element::{Element, ElementId};

/// A stroked cubic Bézier curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wire {
    /// The element that painted it (set when painted).
    pub element: ElementId,
    /// Start point.
    pub from: Point,
    /// First control point.
    pub c1: Point,
    /// Second control point.
    pub c2: Point,
    /// End point.
    pub to: Point,
    /// Stroke width in the element's logical pixels.
    pub width: f32,
    /// Color.
    pub color: Color,
}

impl Wire {
    /// A curve with explicit control points.
    #[must_use]
    pub fn bezier(from: Point, c1: Point, c2: Point, to: Point, width: f32, color: Color) -> Wire {
        Wire {
            element: ElementId::ROOT,
            from,
            c1,
            c2,
            to,
            width,
            color,
        }
    }

    /// A node-editor link: leaves `from` to the right and enters `to` from
    /// the left, with horizontal tangents of half the horizontal distance
    /// (at least 24 px, so backward links loop around).
    #[must_use]
    pub fn link(from: Point, to: Point, width: f32, color: Color) -> Wire {
        let reach = ((to.x - from.x).abs() * 0.5).max(24.0);
        Wire::bezier(
            from,
            from + Vec2::new(reach, 0.0),
            to - Vec2::new(reach, 0.0),
            to,
            width,
            color,
        )
    }

    /// The point at parameter `t` in `0..=1`.
    #[must_use]
    pub fn point_at(&self, t: f32) -> Point {
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        Point::new(
            a * self.from.x + b * self.c1.x + c * self.c2.x + d * self.to.x,
            a * self.from.y + b * self.c1.y + c * self.c2.y + d * self.to.y,
        )
    }

    /// Length of the control polygon (an upper bound of the curve length).
    #[must_use]
    pub fn hull_length(&self) -> f32 {
        (self.c1 - self.from).length() + (self.c2 - self.c1).length() + (self.to - self.c2).length()
    }

    /// Points along the curve, ends included, with segments about
    /// `max_segment` long (4 to 64 segments; a straight wire is one).
    #[must_use]
    pub fn flatten(&self, max_segment: f32) -> Vec<Point> {
        let straight = {
            let d = self.to - self.from;
            let off = |p: Point| {
                let v = p - self.from;
                (v.x * d.y - v.y * d.x).abs() / d.length().max(1e-6)
            };
            off(self.c1) < 0.05 && off(self.c2) < 0.05
        };
        if straight {
            return vec![self.from, self.to];
        }
        let len = self.hull_length();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let n = (len / max_segment.max(0.5)).ceil().clamp(4.0, 64.0) as usize;
        #[allow(clippy::cast_precision_loss)]
        (0..=n)
            .map(|i| self.point_at(i as f32 / n as f32))
            .collect()
    }

    /// Bounds of the stroke (control hull grown by half the width).
    #[must_use]
    pub fn bounds(&self) -> Rect {
        let pts = [self.from, self.c1, self.c2, self.to];
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in pts {
            x0 = x0.min(p.x);
            y0 = y0.min(p.y);
            x1 = x1.max(p.x);
            y1 = y1.max(p.y);
        }
        Rect::new(x0, y0, x1 - x0, y1 - y0).inflate(self.width * 0.5)
    }

    /// Distance from `p` to the curve (approximate: to its flattening).
    #[must_use]
    pub fn distance_to(&self, p: Point) -> f32 {
        let pts = self.flatten(8.0);
        pts.windows(2)
            .map(|w| segment_distance(p, w[0], w[1]))
            .fold(f32::MAX, f32::min)
    }
}

fn segment_distance(p: Point, a: Point, b: Point) -> f32 {
    let ab = b - a;
    let len2 = ab.x * ab.x + ab.y * ab.y;
    let t = if len2 > 0.0 {
        (((p - a).x * ab.x + (p - a).y * ab.y) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p - (a + ab * t)).length()
}

impl Element {
    /// Wires to paint, in coordinates relative to this element's top-left,
    /// above its background and below its children. Replaces earlier
    /// wires.
    #[must_use]
    pub fn wires(mut self, wires: Vec<Wire>) -> Self {
        self.wires = wires;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_tangents_and_endpoints() {
        let w = Wire::link(
            Point::new(0.0, 0.0),
            Point::new(100.0, 50.0),
            2.0,
            Color::WHITE,
        );
        assert_eq!(w.point_at(0.0), w.from);
        assert_eq!(w.point_at(1.0), w.to);
        assert_eq!(w.c1, Point::new(50.0, 0.0));
        let back = Wire::link(Point::ZERO, Point::new(-10.0, 0.0), 2.0, Color::WHITE);
        assert_eq!(back.c1.x, 24.0);
    }

    #[test]
    fn flatten_and_distance() {
        let w = Wire::link(Point::ZERO, Point::new(200.0, 100.0), 2.0, Color::WHITE);
        let pts = w.flatten(10.0);
        assert!(pts.len() > 10 && pts.len() <= 65);
        assert_eq!(pts[0], w.from);
        assert_eq!(*pts.last().unwrap(), w.to);
        assert!(w.distance_to(w.point_at(0.5)) < 0.5);
        assert!(w.distance_to(Point::new(0.0, 100.0)) > 20.0);
        let s = Wire::bezier(
            Point::ZERO,
            Point::new(1.0, 1.0),
            Point::new(2.0, 2.0),
            Point::new(3.0, 3.0),
            1.0,
            Color::WHITE,
        );
        assert_eq!(s.flatten(1.0).len(), 2);
        assert!(w.bounds().contains(w.point_at(0.3)));
    }
}
