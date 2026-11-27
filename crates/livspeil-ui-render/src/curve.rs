// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Cubic Bezier math of the curve primitive.
//!
//! A curve is drawn as a polyline of `segments` pieces, equal steps of the
//! curve parameter. The count is chosen when the frame is encoded so that
//! the polyline stays within [`CURVE_TOLERANCE_PX`] of the true curve on
//! screen (Wang's bound), up to [`MAX_CURVE_SEGMENTS`].
//!
//! The shader draws one box per segment, each large enough for every pixel
//! the stroke touches near that segment. Boxes of neighbouring (or crossing)
//! segments overlap, so each pixel is claimed by exactly one of them: the
//! segment nearest to it, the lowest index among equals. The stroke is
//! therefore blended once wherever it covers, with no gaps, however the
//! curve bends or crosses itself. The coverage is that of the distance to
//! the whole polyline, which makes joints and caps round.
//!
//! Finding that nearest segment need not look at all of them: a curve that
//! keeps advancing along some direction can only come back within the
//! stroke's width of itself a few segments away. [`search_window`] bounds
//! how few, so a long wire costs a handful of distance tests per pixel; a
//! curve with no such direction (it may double back) is searched whole.
//!
//! [`point_at`] and [`segment_distance`] are mirrored in
//! `shaders/primitives.wgsl`.

use crate::geom::Affine;

/// Most straight pieces a curve is drawn with. A shader loop bound: must
/// match `primitives.wgsl`.
pub const MAX_CURVE_SEGMENTS: u32 = 64;

/// How far the drawn polyline may be from the curve, in physical pixels
/// (reached by every curve that needs at most [`MAX_CURVE_SEGMENTS`]).
pub const CURVE_TOLERANCE_PX: f32 = 0.2;

pub(crate) type Points = [[f32; 2]; 4];

/// Euclidean length of `(x, y)`. Plain `sqrt`: curve encoding needs several
/// per wire, and `hypot`'s protection against overflow is not worth its cost
/// for screen coordinates.
#[inline]
pub(crate) fn length(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

/// The point at parameter `t`; exactly the end points at 0 and 1.
#[inline]
pub(crate) fn point_at(p: Points, t: f32) -> [f32; 2] {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    [
        a * p[0][0] + b * p[1][0] + c * p[2][0] + d * p[3][0],
        a * p[0][1] + b * p[1][1] + c * p[2][1] + d * p[3][1],
    ]
}

/// The two ends of segment `i` of `segments`.
#[inline]
pub(crate) fn segment(p: Points, i: u32, segments: u32) -> [[f32; 2]; 2] {
    let n = segments as f32;
    [point_at(p, i as f32 / n), point_at(p, (i + 1) as f32 / n)]
}

/// Distance from `q` to the segment from `a` to `b`.
#[inline]
pub(crate) fn segment_distance(q: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let aq = [q[0] - a[0], q[1] - a[1]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if len2 > 0.0 {
        ((aq[0] * ab[0] + aq[1] * ab[1]) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let d = [aq[0] - ab[0] * t, aq[1] - ab[1] * t];
    (d[0] * d[0] + d[1] * d[1]).sqrt()
}

/// The largest factor by which `t` stretches a length (the larger singular
/// value of its linear part).
pub(crate) fn max_stretch(t: Affine) -> f32 {
    let [a, b, c, d, _, _] = t.m;
    let sum = a * a + b * b + c * c + d * d;
    let det = a * d - b * c;
    let disc = (sum * sum - 4.0 * det * det).max(0.0).sqrt();
    ((sum + disc) * 0.5).sqrt()
}

/// Straight pieces for a curve of which one local unit covers at most
/// `px_per_unit` physical pixels (Wang's bound for a cubic:
/// `n^2 >= 3/4 * M / tolerance`, `M` the largest second difference).
pub(crate) fn segment_count(p: Points, px_per_unit: f32) -> u32 {
    let second = |i: usize| {
        let x = p[i][0] - 2.0 * p[i + 1][0] + p[i + 2][0];
        let y = p[i][1] - 2.0 * p[i + 1][1] + p[i + 2][1];
        length(x, y)
    };
    let m = second(0).max(second(1)) * px_per_unit;
    let n = (0.75 * m / CURVE_TOLERANCE_PX).sqrt().ceil();
    // NaN and negative values saturate to 0 and are raised to 1.
    (n as u32).clamp(1, MAX_CURVE_SEGMENTS)
}

/// The smallest value on `0..=1` of the quadratic Bezier with control
/// values `a`, `b`, `c`.
fn quadratic_min(a: f32, b: f32, c: f32) -> f32 {
    let curvature = a - 2.0 * b + c;
    if curvature > 0.0 {
        let t = (a - b) / curvature;
        if t > 0.0 && t < 1.0 {
            return (a * c - b * b) / curvature;
        }
    }
    a.min(c)
}

/// How many segments to either side of a box's own segment must be searched
/// for a segment nearer than it, for pixels within `reach` of their own
/// segment (local units). `segments` when any might be.
///
/// If the curve's derivative has at least `m > 0` along a unit direction
/// everywhere, every segment advances at least `3 m / segments` along it.
/// Segments `i` and `k` then have `(|k - i| - 1)` such steps between them,
/// and can only both be within `reach` of one pixel if that is at most
/// `2 reach`. The direction tried: both axes, both diagonals and the chord,
/// which covers links running forward and steep links running back.
pub(crate) fn search_window(p: Points, segments: u32, reach: f32) -> u32 {
    let edge = |i: usize| [p[i + 1][0] - p[i][0], p[i + 1][1] - p[i][1]];
    let edges = [edge(0), edge(1), edge(2)];
    let diagonal = std::f32::consts::FRAC_1_SQRT_2;
    let chord = [p[3][0] - p[0][0], p[3][1] - p[0][1]];
    let norm = length(chord[0], chord[1]);
    let chord = if norm > 0.0 {
        [chord[0] / norm, chord[1] / norm]
    } else {
        [1.0, 0.0]
    };
    let mut advance = 0.0f32;
    for u in [
        [1.0, 0.0],
        [0.0, 1.0],
        [diagonal, diagonal],
        [diagonal, -diagonal],
        chord,
    ] {
        let [a, b, c] = edges.map(|e| e[0] * u[0] + e[1] * u[1]);
        advance = advance
            .max(quadratic_min(a, b, c))
            .max(quadratic_min(-a, -b, -c));
    }
    // The least any segment advances.
    let step = 3.0 * advance / segments as f32;
    if step > 0.0 {
        // One more than the bound, against rounding.
        let window = (2.0 * reach / step).floor() + 2.0;
        // NaN saturates to 0: search everything instead.
        if window >= 0.0 && window < segments as f32 {
            return window as u32;
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Points = [[0.0, 0.0], [150.0, 0.0], [150.0, 120.0], [300.0, 120.0]];

    /// Largest distance from the curve to its polyline.
    fn error(p: Points, n: u32) -> f32 {
        (0..2000)
            .map(|k| {
                let q = point_at(p, k as f32 / 1999.0);
                (0..n)
                    .map(|i| {
                        let [a, b] = segment(p, i, n);
                        segment_distance(q, a, b)
                    })
                    .fold(f32::MAX, f32::min)
            })
            .fold(0.0, f32::max)
    }

    #[test]
    fn ends_are_exact() {
        assert_eq!(point_at(S, 0.0), S[0]);
        assert_eq!(point_at(S, 1.0), S[3]);
        assert_eq!(segment(S, 6, 7)[1], S[3]);
    }

    #[test]
    fn the_polyline_stays_within_the_tolerance() {
        let loops = [[0.0, 0.0], [100.0, 80.0], [-60.0, 80.0], [40.0, 0.0]];
        let cusp = [[0.0, 0.0], [100.0, 60.0], [0.0, 60.0], [100.0, 0.0]];
        for p in [S, loops, cusp] {
            for scale in [0.25, 1.0, 2.0] {
                let q = p.map(|[x, y]| [x * scale, y * scale]);
                let n = segment_count(p, scale);
                assert!(n < MAX_CURVE_SEGMENTS, "{n}");
                assert!(error(q, n) <= CURVE_TOLERANCE_PX * 1.01, "{p:?} x{scale}");
            }
        }
    }

    #[test]
    fn segment_counts_follow_size_and_shape() {
        // A straight line with even control points needs one piece.
        let line = [[0.0, 0.0], [10.0, 10.0], [20.0, 20.0], [30.0, 30.0]];
        assert_eq!(segment_count(line, 4.0), 1);
        assert!(segment_count(S, 2.0) > segment_count(S, 1.0));
        assert_eq!(segment_count(S, 1.0e9), MAX_CURVE_SEGMENTS);
        assert_eq!(segment_count(S, 0.0), 1);
    }

    /// Wherever the window says a pixel's nearest segment is, it is: for
    /// points around every segment, searching the window finds the same
    /// nearest segment as searching all.
    #[test]
    fn the_window_never_misses_a_nearer_segment() {
        let mut state = 7u32;
        let mut random = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state % 4001) as f32 / 4000.0
        };
        let mut narrow = 0;
        for case in 0..400 {
            let p: Points = if case % 2 == 0 {
                // Node links, forward and back.
                let (dx, dy) = (random() * 600.0 - 300.0, random() * 600.0 - 300.0);
                let r = (dx.abs() * 0.5).max(24.0);
                [[0.0, 0.0], [r, 0.0], [dx - r, dy], [dx, dy]]
            } else {
                std::array::from_fn(|_| [random() * 200.0, random() * 200.0])
            };
            let n = segment_count(p, 1.0);
            let reach = 0.5 + random() * 6.0;
            let window = search_window(p, n, reach);
            narrow += u32::from(window < n);
            let nearest = |q: [f32; 2], range: std::ops::Range<u32>| {
                range.fold((f32::MAX, 0), |best, k| {
                    let [a, b] = segment(p, k, n);
                    let d = segment_distance(q, a, b);
                    if d < best.0 { (d, k) } else { best }
                })
            };
            for _ in 0..200 {
                let i = (random() * (n - 1) as f32) as u32;
                let [a, b] = segment(p, i, n);
                let t = random();
                let q = [
                    a[0] + (b[0] - a[0]) * t + (random() * 2.0 - 1.0) * reach,
                    a[1] + (b[1] - a[1]) * t + (random() * 2.0 - 1.0) * reach,
                ];
                if segment_distance(q, a, b) >= reach {
                    continue;
                }
                let all = nearest(q, 0..n);
                let windowed = nearest(q, i.saturating_sub(window)..(i + window + 1).min(n));
                // The shader's question: is `i` the owner?
                assert_eq!(all.1 == i, windowed.1 == i, "{p:?} segment {i} at {q:?}");
            }
        }
        assert!(narrow > 100, "{narrow} curves got a narrow window");
    }

    #[test]
    fn forward_links_search_few_segments_and_folds_search_all() {
        let n = segment_count(S, 1.0);
        let window = search_window(S, n, 2.0);
        assert!(window <= 4, "{window} of {n}");
        // A level link running back lies on itself.
        let back = [[0.0, 0.0], [24.0, 0.0], [-124.0, 0.0], [-100.0, 0.0]];
        let n = segment_count(back, 1.0);
        assert_eq!(search_window(back, n, 2.0), n);
        assert_eq!(search_window([[1.0, 1.0]; 4], 1, 2.0), 1);
    }

    #[test]
    fn stretch_is_the_larger_singular_value() {
        assert_eq!(max_stretch(Affine::IDENTITY), 1.0);
        assert!((max_stretch(Affine::scale(0.5, 3.0)) - 3.0).abs() < 1e-6);
        assert!((max_stretch(Affine::rotate(0.7)) - 1.0).abs() < 1e-3);
    }
}
