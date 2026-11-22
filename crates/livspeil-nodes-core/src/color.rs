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
