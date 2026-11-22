// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Color nodes.
//!
//! Colors are OKLCH plus alpha: `[L, C, h_degrees, alpha]` (see
//! [`Value::Color`]). Mixing happens in OKLab, the Cartesian form of OKLCH
//! (`a = C·cos h`, `b = C·sin h`): interpolating `a` and `b` linearly has no
//! "which way round the hue circle" ambiguity and passes through grey for
//! complementary colors instead of sweeping through unrelated hues.

use livspeil_node_api::{CookContext, NodeError, NodeImpl, NodeSpec, PortType, Value};

use crate::float;
use crate::time::{finite, wrap};

/// Chroma below which a color counts as achromatic; its hue is then reported
/// as 0 because the hue of a grey is undefined and would otherwise be
/// rounding noise.
pub const CHROMA_EPSILON: f64 = 1e-9;

/// OKLCH plus alpha to OKLab plus alpha: `[L, a, b, alpha]`.
#[must_use]
pub fn lch_to_lab([l, c, h, alpha]: [f64; 4]) -> [f64; 4] {
    let (sin, cos) = h.to_radians().sin_cos();
    [l, c * cos, c * sin, alpha]
}

/// OKLab plus alpha to OKLCH plus alpha, with the hue in `[0, 360)` and
/// hue 0 when the chroma is below [`CHROMA_EPSILON`].
#[must_use]
pub fn lab_to_lch([l, a, b, alpha]: [f64; 4]) -> [f64; 4] {
    let c = a.hypot(b);
    let h = if c < CHROMA_EPSILON {
        0.0
    } else {
        wrap(b.atan2(a).to_degrees(), 360.0)
    };
    [l, c, h, alpha]
}

/// Mix two OKLCH colors in OKLab; `t` is clamped to `[0, 1]`.
#[must_use]
pub fn mix_oklch(a: [f64; 4], b: [f64; 4], t: f64) -> [f64; 4] {
    let t = t.clamp(0.0, 1.0);
    let (la, lb) = (lch_to_lab(a), lch_to_lab(b));
    let mut out = [0.0; 4];
    for (o, (x, y)) in out.iter_mut().zip(la.iter().zip(lb.iter())) {
        *o = x + (y - x) * t;
    }
    lab_to_lch(out)
}

fn color(inputs: &[Value], index: usize, port: &'static str) -> Result<[f64; 4], NodeError> {
    match inputs.get(index) {
        Some(Value::Color(c)) if c.iter().all(|v| v.is_finite()) => Ok(*c),
        Some(Value::Color(c)) => Err(NodeError::InvalidInput {
            port,
            reason: format!("color components must be finite, got {c:?}"),
        }),
        _ => Err(NodeError::InvalidInput {
            port,
            reason: "expected a Color".to_owned(),
        }),
    }
}

/// `core.color.mix@1`: mix two OKLCH colors in OKLab.
///
/// Inputs: `a` (Color, default `[0.8, 0.12, 60, 1]`), `b` (Color, default
/// `[0.9, 0.08, 80, 1]`), `t` (Float, default 0.5, clamped to `[0, 1]`).
/// Output: `out` (Color): `L`, OKLab `a`/`b` and alpha are interpolated
/// linearly, then converted back to OKLCH with the hue in `[0, 360)`. When
/// the resulting chroma is below [`CHROMA_EPSILON`] the hue is 0.
#[derive(Debug)]
pub struct ColorMix {
    spec: NodeSpec,
}

impl ColorMix {
    #[must_use]
    pub fn new() -> Self {
        ColorMix {
            spec: NodeSpec::new(
                "core.color.mix",
                1,
                "Mix two OKLCH colors in OKLab space (t clamped to [0, 1])",
            )
            .input(
                "a",
                PortType::Color,
                Some(Value::Color([0.8, 0.12, 60.0, 1.0])),
            )
            .input(
                "b",
                PortType::Color,
                Some(Value::Color([0.9, 0.08, 80.0, 1.0])),
            )
            .input("t", PortType::Float, Some(Value::Float(0.5)))
            .output("out", PortType::Color),
        }
    }
}

impl Default for ColorMix {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeImpl for ColorMix {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let a = color(inputs, 0, "a")?;
        let b = color(inputs, 1, "b")?;
        let t = finite(float(inputs, 2, "t")?, "t")?;
        Ok(vec![Value::Color(mix_oklch(a, b, t))])
    }
}

/// `x` limited to `[0, 1]` (finite `x` only), with `-0.0` turned into `+0.0`.
fn unit(x: f64) -> f64 {
    x.clamp(0.0, 1.0) + 0.0
}

/// `core.color.compose@1`: build an OKLCH color from its components.
///
/// Inputs (Float): `l` (lightness, default 0.7), `c` (chroma, default 0.1),
/// `h` (hue in degrees, default 250), `alpha` (default 1).
/// Output: `out` (Color). Never fails on finite values; the components are
/// normalized so the result is always a valid color:
/// - `l` and `alpha` are clamped to `[0, 1]`;
/// - `c` is clamped to `>= 0` (no upper limit: how much chroma a display
///   can show depends on `l` and `h` and is the renderer's job to map);
/// - `h` is wrapped into `[0, 360)`, so `-30` becomes `330` and `720` is `0`.
///   The hue is kept even when `c == 0`, so animating chroma up from grey
///   keeps the chosen hue (unlike `core.color.mix`, which reports grey with
///   hue 0).
///
/// Negative zeros come out as `+0.0`. A `NaN` or infinite input is
/// [`NodeError::InvalidInput`] on that port.
#[derive(Debug)]
pub struct ColorCompose {
    spec: NodeSpec,
}

impl ColorCompose {
    #[must_use]
    pub fn new() -> Self {
        ColorCompose {
            spec: NodeSpec::new(
                "core.color.compose",
                1,
                "Build an OKLCH color (l, alpha clamped to [0, 1]; c >= 0; h wrapped to [0, 360))",
            )
            .input("l", PortType::Float, Some(Value::Float(0.7)))
            .input("c", PortType::Float, Some(Value::Float(0.1)))
            .input("h", PortType::Float, Some(Value::Float(250.0)))
            .input("alpha", PortType::Float, Some(Value::Float(1.0)))
            .output("out", PortType::Color),
        }
    }
}

impl Default for ColorCompose {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeImpl for ColorCompose {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let l = finite(float(inputs, 0, "l")?, "l")?;
        let c = finite(float(inputs, 1, "c")?, "c")?;
        let h = finite(float(inputs, 2, "h")?, "h")?;
        let alpha = finite(float(inputs, 3, "alpha")?, "alpha")?;
        let c = if c > 0.0 { c } else { 0.0 };
        Ok(vec![Value::Color([
            unit(l),
            c,
            wrap(h, 360.0) + 0.0,
            unit(alpha),
        ])])
    }
}

/// A three-stop OKLCH gradient at `t`: `c0` at 0, `c1` at `p1`, `c2` at 1.
///
/// `t` and `p1` should be finite; both are clamped to `[0, 1]`. Each segment
/// is mixed with [`mix_oklch`]. For `t < p1` the result mixes `c0` and `c1`,
/// otherwise `c1` and `c2`, so at `t == p1` it is `c1`. A stop squeezed to
/// zero width is never shown: with `p1 == 0` the gradient runs from `c1` to
/// `c2` (no `c0`), with `p1 == 1` from `c0` to `c1` (no `c2`).
#[must_use]
pub fn gradient3(c0: [f64; 4], c1: [f64; 4], c2: [f64; 4], p1: f64, t: f64) -> [f64; 4] {
    let t = t.clamp(0.0, 1.0);
    let p1 = p1.clamp(0.0, 1.0);
    if t < p1 {
        mix_oklch(c0, c1, t / p1)
    } else if p1 < 1.0 {
        mix_oklch(c1, c2, (t - p1) / (1.0 - p1))
    } else {
        // p1 == 1 and t == 1.
        mix_oklch(c1, c2, 0.0)
    }
}

/// `core.color.gradient@1`: sample a three-stop gradient, mixed in OKLab.
///
/// Inputs: `t` (Float, default 0.5, clamped to `[0, 1]`), the stops `c0`
/// (Color at position 0, default deep blue `[0.3, 0.1, 270, 1]`), `c1`
/// (Color at position `p1`, default orange `[0.7, 0.15, 45, 1]`), `c2`
/// (Color at position 1, default pale yellow `[0.95, 0.05, 95, 1]`), and
/// `p1` (Float, default 0.5, clamped to `[0, 1]`).
/// Output: `out` (Color), computed by [`gradient3`]: each segment is mixed
/// exactly like `core.color.mix`, so the hue normalization and grey rules
/// are the same. `NaN` or infinite values in any input are
/// [`NodeError::InvalidInput`].
///
/// The number of stops is fixed at three for now; variable-length gradients
/// and color ramps arrive once the node API has list parameters.
#[derive(Debug)]
pub struct ColorGradient {
    spec: NodeSpec,
}

impl ColorGradient {
    #[must_use]
    pub fn new() -> Self {
        ColorGradient {
            spec: NodeSpec::new(
                "core.color.gradient",
                1,
                "Three-stop OKLCH gradient (c0 at 0, c1 at p1, c2 at 1) sampled at t, mixed in OKLab",
            )
            .input("t", PortType::Float, Some(Value::Float(0.5)))
            .input(
                "c0",
                PortType::Color,
                Some(Value::Color([0.3, 0.1, 270.0, 1.0])),
            )
            .input(
                "c1",
                PortType::Color,
                Some(Value::Color([0.7, 0.15, 45.0, 1.0])),
            )
            .input(
                "c2",
                PortType::Color,
                Some(Value::Color([0.95, 0.05, 95.0, 1.0])),
            )
            .input("p1", PortType::Float, Some(Value::Float(0.5)))
            .output("out", PortType::Color),
        }
    }
}

impl Default for ColorGradient {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeImpl for ColorGradient {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let t = finite(float(inputs, 0, "t")?, "t")?;
        let c0 = color(inputs, 1, "c0")?;
        let c1 = color(inputs, 2, "c1")?;
        let c2 = color(inputs, 3, "c2")?;
        let p1 = finite(float(inputs, 4, "p1")?, "p1")?;
        Ok(vec![Value::Color(gradient3(c0, c1, c2, p1, t))])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: [f64; 4] = [0.8, 0.12, 60.0, 1.0];
    const B: [f64; 4] = [0.9, 0.08, 80.0, 1.0];

    fn cook(a: [f64; 4], b: [f64; 4], t: f64) -> [f64; 4] {
        let out = ColorMix::new()
            .cook(
                &CookContext::default(),
                &[Value::Color(a), Value::Color(b), Value::Float(t)],
            )
            .unwrap();
        let Value::Color(c) = out[0] else {
            panic!("not a color: {:?}", out[0]);
        };
        c
    }

    fn close4(actual: [f64; 4], expected: [f64; 4], tol: f64) {
        for (x, y) in actual.iter().zip(&expected) {
            assert!(
                (x - y).abs() <= tol,
                "expected {expected:?} ± {tol}, got {actual:?}"
            );
        }
    }

    #[test]
    fn endpoints_return_the_inputs() {
        close4(cook(A, B, 0.0), A, 1e-9);
        close4(cook(A, B, 1.0), B, 1e-9);
    }

    #[test]
    fn t_is_clamped() {
        close4(cook(A, B, -3.0), A, 1e-9);
        close4(cook(A, B, 7.0), B, 1e-9);
    }

    #[test]
    fn round_trip_is_stable() {
        for l in [0.0, 0.3, 0.7, 1.0] {
            for c in [0.01, 0.1, 0.25, 0.4] {
                for h in [0.0, 1.0, 45.0, 90.0, 179.9, 180.0, 270.0, 359.5] {
                    let lch = [l, c, h, 0.5];
                    let back = lab_to_lch(lch_to_lab(lch));
                    close4(back, lch, 1e-9);
                    // And again, to show the round trip does not drift.
                    close4(lab_to_lch(lch_to_lab(back)), lch, 1e-9);
                }
            }
        }
    }

    #[test]
    fn hue_is_normalized() {
        let out = lab_to_lch(lch_to_lab([0.5, 0.1, -30.0, 1.0]));
        close4(out, [0.5, 0.1, 330.0, 1.0], 1e-9);
        let out = lab_to_lch(lch_to_lab([0.5, 0.1, 720.0 + 10.0, 1.0]));
        close4(out, [0.5, 0.1, 10.0, 1.0], 1e-9);
    }

    #[test]
    fn mixing_across_zero_hue_takes_the_short_way() {
        // 350° and 10° meet at 0°, not at 180°.
        let out = cook([0.6, 0.1, 350.0, 1.0], [0.6, 0.1, 10.0, 1.0], 0.5);
        let h = if out[2] > 180.0 {
            out[2] - 360.0
        } else {
            out[2]
        };
        assert!(h.abs() < 1e-9, "{out:?}");
    }

    #[test]
    fn complementary_colors_meet_at_grey_with_hue_zero() {
        let out = cook([0.5, 0.1, 30.0, 1.0], [0.5, 0.1, 210.0, 0.0], 0.5);
        assert!(out[1] < CHROMA_EPSILON, "{out:?}");
        assert_eq!(out[2], 0.0);
        assert!((out[0] - 0.5).abs() < 1e-12);
        assert!((out[3] - 0.5).abs() < 1e-12);
    }

    #[test]
    fn midpoint_of_the_defaults() {
        let out = cook(A, B, 0.5);
        close4(out, [0.85, out[1], out[2], 1.0], 1e-12);
        // Hue lies between the two inputs; chroma dips slightly below the
        // average because the mix cuts the chord of the hue circle.
        assert!(out[2] > 60.0 && out[2] < 80.0, "{out:?}");
        assert!(out[1] < 0.1 && out[1] > 0.09, "{out:?}");
    }

    fn compose(l: f64, c: f64, h: f64, alpha: f64) -> Result<[f64; 4], NodeError> {
        let out = ColorCompose::new()
            .cook(&CookContext::default(), &[l, c, h, alpha].map(Value::Float))?;
        let Value::Color(c) = out[0] else {
            panic!("not a color: {:?}", out[0]);
        };
        Ok(c)
    }

    fn bits(c: [f64; 4]) -> [u64; 4] {
        c.map(f64::to_bits)
    }

    #[test]
    fn compose_passes_valid_components_through() {
        assert_eq!(
            compose(0.7, 0.1, 250.0, 1.0).unwrap(),
            [0.7, 0.1, 250.0, 1.0]
        );
        assert_eq!(compose(0.0, 0.0, 0.0, 0.0).unwrap(), [0.0; 4]);
        assert_eq!(
            compose(1.0, 0.37, 359.5, 0.25).unwrap(),
            [1.0, 0.37, 359.5, 0.25]
        );
    }

    #[test]
    fn compose_normalizes() {
        assert_eq!(
            compose(1.5, -0.2, -30.0, 2.0).unwrap(),
            [1.0, 0.0, 330.0, 1.0]
        );
        assert_eq!(
            compose(-0.5, 0.5, 720.0, -1.0).unwrap(),
            [0.0, 0.5, 0.0, 0.0]
        );
        assert_eq!(compose(0.5, 0.1, 360.0, 1.0).unwrap()[2], 0.0);
        assert_eq!(compose(0.5, 0.1, -360.0, 1.0).unwrap()[2], 0.0);
        assert_eq!(compose(0.5, 0.1, 725.5, 1.0).unwrap()[2], 5.5);
        // Tiny negative hue rounds to 360 in rem_euclid; it must fold to 0.
        let h = compose(0.5, 0.1, -1e-15, 1.0).unwrap()[2];
        assert!((0.0..360.0).contains(&h), "{h}");
        // Grey keeps its hue.
        assert_eq!(compose(0.5, 0.0, 120.0, 1.0).unwrap()[2], 120.0);
        // Negative zeros become positive.
        assert_eq!(
            bits(compose(-0.0, -0.0, -0.0, -0.0).unwrap()),
            bits([0.0; 4])
        );
    }

    #[test]
    fn compose_rejects_non_finite() {
        for (i, port) in ["l", "c", "h", "alpha"].into_iter().enumerate() {
            for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                let mut v = [0.5, 0.1, 10.0, 1.0];
                v[i] = bad;
                let err = compose(v[0], v[1], v[2], v[3]).unwrap_err();
                assert!(
                    matches!(err, NodeError::InvalidInput { port: p, .. } if p == port),
                    "{port}={bad}: {err}"
                );
            }
        }
    }

    const G0: [f64; 4] = [0.3, 0.1, 270.0, 1.0];
    const G1: [f64; 4] = [0.7, 0.15, 45.0, 1.0];
    const G2: [f64; 4] = [0.95, 0.05, 95.0, 1.0];

    fn gradient(t: f64, p1: f64) -> Result<[f64; 4], NodeError> {
        let out = ColorGradient::new().cook(
            &CookContext::default(),
            &[
                Value::Float(t),
                Value::Color(G0),
                Value::Color(G1),
                Value::Color(G2),
                Value::Float(p1),
            ],
        )?;
        let Value::Color(c) = out[0] else {
            panic!("not a color: {:?}", out[0]);
        };
        Ok(c)
    }

    #[test]
    fn gradient_hits_its_stops() {
        close4(gradient(0.0, 0.5).unwrap(), G0, 1e-12);
        close4(gradient(0.5, 0.5).unwrap(), G1, 1e-12);
        close4(gradient(1.0, 0.5).unwrap(), G2, 1e-12);
        close4(gradient(0.3, 0.3).unwrap(), G1, 1e-12);
        // t is clamped.
        close4(gradient(-2.0, 0.5).unwrap(), G0, 1e-12);
        close4(gradient(9.0, 0.5).unwrap(), G2, 1e-12);
    }

    #[test]
    fn gradient_segments_match_mix() {
        assert_eq!(gradient(0.25, 0.5).unwrap(), mix_oklch(G0, G1, 0.5));
        assert_eq!(gradient(0.75, 0.5).unwrap(), mix_oklch(G1, G2, 0.5));
        assert_eq!(gradient(0.1, 0.2).unwrap(), mix_oklch(G0, G1, 0.5));
        assert_eq!(gradient(0.6, 0.2).unwrap(), mix_oklch(G1, G2, 0.5));
    }

    #[test]
    fn gradient_with_squeezed_stops() {
        // p1 = 0: c0 never shows, the gradient runs c1 -> c2.
        close4(gradient(0.0, 0.0).unwrap(), G1, 1e-12);
        assert_eq!(gradient(0.5, 0.0).unwrap(), mix_oklch(G1, G2, 0.5));
        close4(gradient(1.0, 0.0).unwrap(), G2, 1e-12);
        // p1 = 1: c2 never shows, the gradient runs c0 -> c1.
        close4(gradient(0.0, 1.0).unwrap(), G0, 1e-12);
        assert_eq!(gradient(0.5, 1.0).unwrap(), mix_oklch(G0, G1, 0.5));
        close4(gradient(1.0, 1.0).unwrap(), G1, 1e-12);
        // p1 outside [0, 1] is clamped.
        assert_eq!(gradient(0.5, -3.0).unwrap(), gradient(0.5, 0.0).unwrap());
        assert_eq!(gradient(0.5, 4.0).unwrap(), gradient(0.5, 1.0).unwrap());
    }

    #[test]
    fn gradient_rejects_non_finite() {
        let err = gradient(f64::NAN, 0.5).unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "t", .. }));
        let err = gradient(0.5, f64::INFINITY).unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "p1", .. }));
        let err = ColorGradient::new()
            .cook(
                &CookContext::default(),
                &[
                    Value::Float(0.5),
                    Value::Color(G0),
                    Value::Color([0.5, f64::NAN, 0.0, 1.0]),
                    Value::Color(G2),
                    Value::Float(0.5),
                ],
            )
            .unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "c1", .. }));
        let err = ColorGradient::new()
            .cook(
                &CookContext::default(),
                &[
                    Value::Float(0.5),
                    Value::Color(G0),
                    Value::Color(G1),
                    Value::Float(1.0),
                    Value::Float(0.5),
                ],
            )
            .unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "c2", .. }));
    }

    #[test]
    fn rejects_non_color_and_non_finite_inputs() {
        let node = ColorMix::new();
        let err = node
            .cook(
                &CookContext::default(),
                &[Value::Float(1.0), Value::Color(B), Value::Float(0.5)],
            )
            .unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "a", .. }));
        let err = node
            .cook(
                &CookContext::default(),
                &[
                    Value::Color(A),
                    Value::Color([f64::NAN, 0.0, 0.0, 1.0]),
                    Value::Float(0.5),
                ],
            )
            .unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "b", .. }));
        let err = node
            .cook(
                &CookContext::default(),
                &[Value::Color(A), Value::Color(B), Value::Float(f64::NAN)],
            )
            .unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "t", .. }));
    }
}
