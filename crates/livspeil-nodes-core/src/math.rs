// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Scalar math nodes (milestone M0.6a).
//!
//! All nodes here are pure functions of `Float` inputs and follow IEEE 754
//! arithmetic: `NaN` and infinities propagate as the hardware computes them,
//! unless a node documents otherwise. Values the node cannot handle at all
//! (division by zero, an empty input range) are reported as
//! [`NodeError::InvalidInput`], so the scheduler keeps the last good output
//! instead of publishing an infinity or a `NaN`.

use livspeil_node_api::{CookContext, NodeError, NodeImpl, NodeSpec, PortType, Value};

use crate::float;

/// A `Float` input port with a default value.
fn float_in(spec: NodeSpec, name: &'static str, default: f64) -> NodeSpec {
    spec.input(name, PortType::Float, Some(Value::Float(default)))
}

/// Clamp `x` to `[lo, hi]` without panicking. Assumes `lo <= hi` when both
/// are numbers. A `NaN` `x` passes through unchanged; a `NaN` bound is
/// ignored (comparisons with `NaN` are false).
fn clamp_unchecked(x: f64, lo: f64, hi: f64) -> f64 {
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}

/// Deterministic minimum: a `NaN` operand is ignored (IEEE 754 `minNum`), and
/// `-0.0` is smaller than `+0.0` (IEEE 754-2019 `minimum`), so the result
/// never depends on the platform.
fn min_det(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        b
    } else if b.is_nan() || a < b {
        a
    } else if b < a {
        b
    } else if a.is_sign_negative() {
        // Equal values; only differ for signed zeros.
        a
    } else {
        b
    }
}

/// Deterministic maximum: the mirror of [`min_det`] (`+0.0` beats `-0.0`).
fn max_det(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        b
    } else if b.is_nan() || a > b {
        a
    } else if b > a {
        b
    } else if a.is_sign_positive() {
        a
    } else {
        b
    }
}

macro_rules! node_boilerplate {
    ($ty:ident) => {
        impl Default for $ty {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

/// `core.math.sub@1`: `out = a - b`.
#[derive(Debug)]
pub struct Sub {
    spec: NodeSpec,
}

impl Sub {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new("core.math.sub", 1, "Subtract b from a");
        let spec = float_in(float_in(spec, "a", 0.0), "b", 0.0);
        Sub {
            spec: spec.output("out", PortType::Float),
        }
    }
}
node_boilerplate!(Sub);

impl NodeImpl for Sub {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let a = float(inputs, 0, "a")?;
        let b = float(inputs, 1, "b")?;
        Ok(vec![Value::Float(a - b)])
    }
}

/// `core.math.mul@1`: `out = a * b`. `b` defaults to 1 so that linking only
/// `a` passes it through.
#[derive(Debug)]
pub struct Mul {
    spec: NodeSpec,
}

impl Mul {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new("core.math.mul", 1, "Multiply a by b");
        let spec = float_in(float_in(spec, "a", 0.0), "b", 1.0);
        Mul {
            spec: spec.output("out", PortType::Float),
        }
    }
}
node_boilerplate!(Mul);

impl NodeImpl for Mul {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let a = float(inputs, 0, "a")?;
        let b = float(inputs, 1, "b")?;
        Ok(vec![Value::Float(a * b)])
    }
}

/// `core.math.div@1`: `out = a / b`.
///
/// `b == 0` (either sign of zero) is an [`NodeError::InvalidInput`] on `b`,
/// so the scheduler keeps the last good output instead of publishing an
/// infinity or `NaN`. Any other divisor follows IEEE 754 (`a / inf == 0`,
/// a `NaN` operand gives `NaN`).
#[derive(Debug)]
pub struct Div {
    spec: NodeSpec,
}

impl Div {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new("core.math.div", 1, "Divide a by b (b must not be zero)");
        let spec = float_in(float_in(spec, "a", 0.0), "b", 1.0);
        Div {
            spec: spec.output("out", PortType::Float),
        }
    }
}
node_boilerplate!(Div);

impl NodeImpl for Div {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let a = float(inputs, 0, "a")?;
        let b = float(inputs, 1, "b")?;
        if b == 0.0 {
            return Err(NodeError::InvalidInput {
                port: "b",
                reason: "division by zero".to_owned(),
            });
        }
        Ok(vec![Value::Float(a / b)])
    }
}

/// `core.math.clamp@1`: `out = x` limited to `[min, max]`.
///
/// If `min > max` the bounds are swapped, so the node never fails and the
/// order in which a user drags the two sliders does not matter. A `NaN` `x`
/// passes through; a `NaN` bound is ignored (that side is unbounded).
#[derive(Debug)]
pub struct Clamp {
    spec: NodeSpec,
}

impl Clamp {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new(
            "core.math.clamp",
            1,
            "Limit x to the range [min, max] (bounds are swapped if reversed)",
        );
        let spec = float_in(float_in(float_in(spec, "x", 0.0), "min", 0.0), "max", 1.0);
        Clamp {
            spec: spec.output("out", PortType::Float),
        }
    }
}
node_boilerplate!(Clamp);

impl NodeImpl for Clamp {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let x = float(inputs, 0, "x")?;
        let min = float(inputs, 1, "min")?;
        let max = float(inputs, 2, "max")?;
        let (lo, hi) = if min > max { (max, min) } else { (min, max) };
        Ok(vec![Value::Float(clamp_unchecked(x, lo, hi))])
    }
}

/// `core.math.remap@1`: map `x` from `[in_min, in_max]` to
/// `[out_min, out_max]` linearly.
///
/// `out = out_min + (out_max - out_min) * t` with
/// `t = (x - in_min) / (in_max - in_min)`. With `clamp` set, `t` is limited
/// to `[0, 1]`, so the output stays between `out_min` and `out_max` (in
/// either order). Reversed ranges are allowed and invert the mapping.
/// `in_min == in_max` is an [`NodeError::InvalidInput`] on `in_max`.
#[derive(Debug)]
pub struct Remap {
    spec: NodeSpec,
}

impl Remap {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new(
            "core.math.remap",
            1,
            "Map x linearly from [in_min, in_max] to [out_min, out_max]",
        );
        let spec = float_in(spec, "x", 0.0);
        let spec = float_in(spec, "in_min", 0.0);
        let spec = float_in(spec, "in_max", 1.0);
        let spec = float_in(spec, "out_min", 0.0);
        let spec = float_in(spec, "out_max", 1.0);
        Remap {
            spec: spec
                .input("clamp", PortType::Bool, Some(Value::Bool(false)))
                .output("out", PortType::Float),
        }
    }
}
node_boilerplate!(Remap);

impl NodeImpl for Remap {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let x = float(inputs, 0, "x")?;
        let in_min = float(inputs, 1, "in_min")?;
        let in_max = float(inputs, 2, "in_max")?;
        let out_min = float(inputs, 3, "out_min")?;
        let out_max = float(inputs, 4, "out_max")?;
        let clamp = match inputs.get(5) {
            Some(Value::Bool(b)) => *b,
            _ => {
                return Err(NodeError::InvalidInput {
                    port: "clamp",
                    reason: "expected a Bool".to_owned(),
                });
            }
        };
        if in_min == in_max {
            return Err(NodeError::InvalidInput {
                port: "in_max",
                reason: "input range is empty (in_min == in_max)".to_owned(),
            });
        }
        let mut t = (x - in_min) / (in_max - in_min);
        if clamp {
            t = clamp_unchecked(t, 0.0, 1.0);
        }
        Ok(vec![Value::Float(out_min + (out_max - out_min) * t)])
    }
}

/// `core.math.min@1`: the smaller of `a` and `b`.
///
/// Deterministic on every platform: a `NaN` operand is ignored (the other
/// one is returned; `NaN` only if both are `NaN`), and `-0.0 < +0.0`.
#[derive(Debug)]
pub struct Min {
    spec: NodeSpec,
}

impl Min {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new("core.math.min", 1, "The smaller of a and b");
        let spec = float_in(float_in(spec, "a", 0.0), "b", 0.0);
        Min {
            spec: spec.output("out", PortType::Float),
        }
    }
}
node_boilerplate!(Min);

impl NodeImpl for Min {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let a = float(inputs, 0, "a")?;
        let b = float(inputs, 1, "b")?;
        Ok(vec![Value::Float(min_det(a, b))])
    }
}

/// `core.math.max@1`: the larger of `a` and `b`.
///
/// Deterministic on every platform: a `NaN` operand is ignored (the other
/// one is returned; `NaN` only if both are `NaN`), and `+0.0 > -0.0`.
#[derive(Debug)]
pub struct Max {
    spec: NodeSpec,
}

impl Max {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new("core.math.max", 1, "The larger of a and b");
        let spec = float_in(float_in(spec, "a", 0.0), "b", 0.0);
        Max {
            spec: spec.output("out", PortType::Float),
        }
    }
}
node_boilerplate!(Max);

impl NodeImpl for Max {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let a = float(inputs, 0, "a")?;
        let b = float(inputs, 1, "b")?;
        Ok(vec![Value::Float(max_det(a, b))])
    }
}

/// `core.math.smoothstep@1`: Hermite ease from 0 to 1 as `x` goes from
/// `edge0` to `edge1`.
///
/// `t = clamp((x - edge0) / (edge1 - edge0), 0, 1)`, `out = 3t² - 2t³`
/// (as in GLSL). `edge0 > edge1` is allowed and gives the falling curve.
/// When `edge0 == edge1` (undefined in GLSL) the node is a step:
/// `out = 0` for `x < edge0`, else `1`. A `NaN` `x` gives `NaN`.
#[derive(Debug)]
pub struct Smoothstep {
    spec: NodeSpec,
}

impl Smoothstep {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new(
            "core.math.smoothstep",
            1,
            "Smooth Hermite step from 0 to 1 as x goes from edge0 to edge1",
        );
        let spec = float_in(
            float_in(float_in(spec, "edge0", 0.0), "edge1", 1.0),
            "x",
            0.0,
        );
        Smoothstep {
            spec: spec.output("out", PortType::Float),
        }
    }
}
node_boilerplate!(Smoothstep);

impl NodeImpl for Smoothstep {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let edge0 = float(inputs, 0, "edge0")?;
        let edge1 = float(inputs, 1, "edge1")?;
        let x = float(inputs, 2, "x")?;
        let out = if x.is_nan() {
            f64::NAN
        } else if edge0 == edge1 {
            if x < edge0 { 0.0 } else { 1.0 }
        } else {
            let t = clamp_unchecked((x - edge0) / (edge1 - edge0), 0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        Ok(vec![Value::Float(out)])
    }
}

/// `core.math.abs@1`: `out = |x|`.
///
/// Clears the sign bit, so `abs(-0.0) == +0.0`, `abs(-inf) == inf` and a
/// `NaN` stays `NaN`.
#[derive(Debug)]
pub struct Abs {
    spec: NodeSpec,
}

impl Abs {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new("core.math.abs", 1, "Absolute value of x");
        Abs {
            spec: float_in(spec, "x", 0.0).output("out", PortType::Float),
        }
    }
}
node_boilerplate!(Abs);

impl NodeImpl for Abs {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let x = float(inputs, 0, "x")?;
        Ok(vec![Value::Float(x.abs())])
    }
}

/// `core.math.pow@1`: `out = base ^ exponent`.
///
/// Follows IEEE 754 / C99 Annex F `pow` and never fails: `x ^ 0 == 1` for
/// every `x` (even `NaN`), `1 ^ y == 1` for every `y` (even `NaN`), a
/// negative base with a non-integer exponent gives `NaN`, `0 ^ -1 == inf`.
/// `exponent` defaults to 1 so that linking only `base` passes it through.
///
/// Like `sin` and `cos`, `pow` comes from the platform math library: the
/// special cases above are exact everywhere, ordinary results may differ in
/// the last bit between platforms.
#[derive(Debug)]
pub struct Pow {
    spec: NodeSpec,
}

impl Pow {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new("core.math.pow", 1, "base raised to the power exponent");
        let spec = float_in(float_in(spec, "base", 0.0), "exponent", 1.0);
        Pow {
            spec: spec.output("out", PortType::Float),
        }
    }
}
node_boilerplate!(Pow);

impl NodeImpl for Pow {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let base = float(inputs, 0, "base")?;
        let exponent = float(inputs, 1, "exponent")?;
        Ok(vec![Value::Float(base.powf(exponent))])
    }
}

/// `core.math.sin@1`: `out = sin(x)`, `x` in radians.
///
/// `sin(±0) == ±0`; an infinite or `NaN` `x` gives `NaN`. Platform math
/// library (see [`Pow`] on last-bit differences).
#[derive(Debug)]
pub struct Sin {
    spec: NodeSpec,
}

impl Sin {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new("core.math.sin", 1, "Sine of x (radians)");
        Sin {
            spec: float_in(spec, "x", 0.0).output("out", PortType::Float),
        }
    }
}
node_boilerplate!(Sin);

impl NodeImpl for Sin {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let x = float(inputs, 0, "x")?;
        Ok(vec![Value::Float(x.sin())])
    }
}

/// `core.math.cos@1`: `out = cos(x)`, `x` in radians.
///
/// `cos(±0) == 1`; an infinite or `NaN` `x` gives `NaN`. Platform math
/// library (see [`Pow`] on last-bit differences).
#[derive(Debug)]
pub struct Cos {
    spec: NodeSpec,
}

impl Cos {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new("core.math.cos", 1, "Cosine of x (radians)");
        Cos {
            spec: float_in(spec, "x", 0.0).output("out", PortType::Float),
        }
    }
}
node_boilerplate!(Cos);

impl NodeImpl for Cos {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let x = float(inputs, 0, "x")?;
        Ok(vec![Value::Float(x.cos())])
    }
}

/// Reject a `NaN` or infinite bound on `port`.
fn finite_bound(value: f64, port: &'static str) -> Result<f64, NodeError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(NodeError::InvalidInput {
            port,
            reason: format!("expected a finite number, got {value}"),
        })
    }
}

/// `core.math.wrap@1`: `x` wrapped into the half-open range `[min, max)`.
///
/// `out = min + (x - min) mod (max - min)` with a Euclidean (never negative)
/// remainder, so values below `min` wrap from the top: with the defaults
/// `wrap(-0.25) == 0.75` and `wrap(1.0) == 0.0`. If `min > max` the bounds
/// are swapped, as in `core.math.clamp`. Errors
/// ([`NodeError::InvalidInput`]): `min == max` (on `max`), a `NaN` or
/// infinite bound (on that bound), and a range `max - min` too large to
/// represent (on `max`). A `NaN` or infinite `x` gives `NaN`, as does an `x`
/// so far from `min` that `x - min` overflows.
#[derive(Debug)]
pub struct Wrap {
    spec: NodeSpec,
}

impl Wrap {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new(
            "core.math.wrap",
            1,
            "Wrap x into [min, max) (bounds are swapped if reversed)",
        );
        let spec = float_in(float_in(float_in(spec, "x", 0.0), "min", 0.0), "max", 1.0);
        Wrap {
            spec: spec.output("out", PortType::Float),
        }
    }
}
node_boilerplate!(Wrap);

impl NodeImpl for Wrap {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let x = float(inputs, 0, "x")?;
        let min = finite_bound(float(inputs, 1, "min")?, "min")?;
        let max = finite_bound(float(inputs, 2, "max")?, "max")?;
        if min == max {
            return Err(NodeError::InvalidInput {
                port: "max",
                reason: "range is empty (min == max)".to_owned(),
            });
        }
        let (lo, hi) = if min > max { (max, min) } else { (min, max) };
        let range = hi - lo;
        if !range.is_finite() {
            return Err(NodeError::InvalidInput {
                port: "max",
                reason: "range max - min is too large".to_owned(),
            });
        }
        // rem_euclid is never negative but may round up to exactly `range`
        // for tiny negative offsets; fold that (and any rounding of the
        // final addition onto `hi`) back to `lo`.
        let out = lo + (x - lo).rem_euclid(range);
        let out = if out >= hi { lo } else { out };
        Ok(vec![Value::Float(out)])
    }
}

/// Quotients at or above this magnitude are already integers in `f64`, so
/// rounding them changes nothing (2^52).
const ROUND_LIMIT: f64 = 4_503_599_627_370_496.0;

/// `core.math.round@1`: `x` rounded to the nearest multiple of `step`, halves
/// away from zero (`round(2.5) == 3`, `round(-2.5) == -3`).
///
/// The sign of `step` does not matter. `step == 0` or a `NaN` or infinite
/// `step` is an [`NodeError::InvalidInput`] on `step`. A `NaN` or infinite
/// `x` passes through, as does any `x` whose quotient `x / step` reaches
/// 2^52 (it is already a whole number of steps as far as `f64` can tell).
/// Small negative values round to `-0.0`, as IEEE 754 rounding does.
///
/// When `1 / step` is a whole number (steps such as 0.1, 0.25 or 1), the
/// node computes `round(x * (1/step)) / (1/step)` instead of
/// `round(x / step) * step`: that way `round(0.3, 0.1)` is `0.3` rather than
/// `0.30000000000000004`, and `round(0.25, 0.1)` is `0.3` as on paper.
#[derive(Debug)]
pub struct Round {
    spec: NodeSpec,
}

impl Round {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new(
            "core.math.round",
            1,
            "Round x to the nearest multiple of step (halves away from zero)",
        );
        let spec = float_in(float_in(spec, "x", 0.0), "step", 1.0);
        Round {
            spec: spec.output("out", PortType::Float),
        }
    }
}
node_boilerplate!(Round);

/// `x` rounded to a multiple of `step`; `step` must be finite and non-zero.
fn round_to_step(x: f64, step: f64) -> f64 {
    let step = step.abs();
    let inv = 1.0 / step;
    let use_inv = inv.is_finite() && inv.fract() == 0.0 && inv < ROUND_LIMIT;
    let q = if use_inv { x * inv } else { x / step };
    if !q.is_finite() || q.abs() >= ROUND_LIMIT {
        return x;
    }
    if use_inv {
        q.round() / inv
    } else {
        q.round() * step
    }
}

impl NodeImpl for Round {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let x = float(inputs, 0, "x")?;
        let step = finite_bound(float(inputs, 1, "step")?, "step")?;
        if step == 0.0 {
            return Err(NodeError::InvalidInput {
                port: "step",
                reason: "step must not be zero".to_owned(),
            });
        }
        Ok(vec![Value::Float(round_to_step(x, step))])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cook(node: &dyn NodeImpl, inputs: &[f64]) -> Result<f64, NodeError> {
        let inputs: Vec<Value> = inputs.iter().map(|v| Value::Float(*v)).collect();
        let out = node.cook(&CookContext::default(), &inputs)?;
        assert_eq!(out.len(), 1);
        Ok(out[0].as_float().unwrap())
    }

    fn remap(x: f64, range: [f64; 4], clamp: bool) -> Result<f64, NodeError> {
        let mut inputs = vec![Value::Float(x)];
        inputs.extend(range.iter().map(|v| Value::Float(*v)));
        inputs.push(Value::Bool(clamp));
        let out = Remap::new().cook(&CookContext::default(), &inputs)?;
        Ok(out[0].as_float().unwrap())
    }

    /// Bitwise equality, so `-0.0` and `NaN` can be asserted exactly.
    fn same(a: f64, b: f64) -> bool {
        a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
    }

    #[test]
    fn sub_mul_div_basics() {
        assert_eq!(cook(&Sub::new(), &[5.0, 3.5]).unwrap(), 1.5);
        assert_eq!(cook(&Sub::new(), &[1.0, 3.0]).unwrap(), -2.0);
        assert_eq!(cook(&Mul::new(), &[2.5, -4.0]).unwrap(), -10.0);
        assert_eq!(cook(&Div::new(), &[7.0, 2.0]).unwrap(), 3.5);
        assert_eq!(cook(&Div::new(), &[1.0, f64::INFINITY]).unwrap(), 0.0);
    }

    #[test]
    fn ieee_specials_propagate() {
        assert!(
            cook(&Sub::new(), &[f64::INFINITY, f64::INFINITY])
                .unwrap()
                .is_nan()
        );
        assert!(cook(&Mul::new(), &[0.0, f64::INFINITY]).unwrap().is_nan());
        assert!(cook(&Mul::new(), &[f64::NAN, 1.0]).unwrap().is_nan());
        assert!(cook(&Div::new(), &[f64::NAN, 2.0]).unwrap().is_nan());
        assert!(cook(&Div::new(), &[1.0, f64::NAN]).unwrap().is_nan());
    }

    #[test]
    fn div_by_zero_is_invalid_input_on_b() {
        for zero in [0.0, -0.0] {
            let err = cook(&Div::new(), &[1.0, zero]).unwrap_err();
            assert!(matches!(err, NodeError::InvalidInput { port: "b", .. }));
        }
        let err = cook(&Div::new(), &[0.0, 0.0]).unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "b", .. }));
    }

    #[test]
    fn clamp_limits_and_swaps_reversed_bounds() {
        assert_eq!(cook(&Clamp::new(), &[0.5, 0.0, 1.0]).unwrap(), 0.5);
        assert_eq!(cook(&Clamp::new(), &[-2.0, 0.0, 1.0]).unwrap(), 0.0);
        assert_eq!(cook(&Clamp::new(), &[3.0, 0.0, 1.0]).unwrap(), 1.0);
        assert_eq!(cook(&Clamp::new(), &[3.0, 1.0, -1.0]).unwrap(), 1.0);
        assert_eq!(cook(&Clamp::new(), &[-3.0, 1.0, -1.0]).unwrap(), -1.0);
        assert_eq!(cook(&Clamp::new(), &[5.0, 2.0, 2.0]).unwrap(), 2.0);
        assert_eq!(
            cook(&Clamp::new(), &[f64::INFINITY, 0.0, 1.0]).unwrap(),
            1.0
        );
    }

    #[test]
    fn clamp_nan_rules() {
        assert!(cook(&Clamp::new(), &[f64::NAN, 0.0, 1.0]).unwrap().is_nan());
        // A NaN bound leaves that side open and never panics.
        assert_eq!(cook(&Clamp::new(), &[5.0, f64::NAN, 1.0]).unwrap(), 1.0);
        assert_eq!(cook(&Clamp::new(), &[-5.0, f64::NAN, 1.0]).unwrap(), -5.0);
        assert_eq!(cook(&Clamp::new(), &[-5.0, 0.0, f64::NAN]).unwrap(), 0.0);
    }

    #[test]
    fn remap_maps_linearly() {
        let r = [0.0, 10.0, 100.0, 200.0];
        assert_eq!(remap(2.5, r, false).unwrap(), 125.0);
        assert_eq!(remap(20.0, r, false).unwrap(), 300.0);
        assert_eq!(remap(20.0, r, true).unwrap(), 200.0);
        assert_eq!(remap(-5.0, r, true).unwrap(), 100.0);
        // Reversed output range: clamp keeps the value between the two ends.
        assert_eq!(remap(0.25, [0.0, 1.0, 1.0, 0.0], false).unwrap(), 0.75);
        assert_eq!(remap(2.0, [0.0, 1.0, 1.0, 0.0], true).unwrap(), 0.0);
        // Reversed input range.
        assert_eq!(remap(0.25, [1.0, 0.0, 0.0, 1.0], false).unwrap(), 0.75);
    }

    #[test]
    fn remap_empty_input_range_is_invalid() {
        let err = remap(1.0, [2.0, 2.0, 0.0, 1.0], false).unwrap_err();
        assert!(matches!(
            err,
            NodeError::InvalidInput { port: "in_max", .. }
        ));
    }

    #[test]
    fn remap_rejects_non_bool_clamp() {
        let inputs = [0.0, 0.0, 1.0, 0.0, 1.0, 0.0].map(Value::Float);
        let err = Remap::new()
            .cook(&CookContext::default(), &inputs)
            .unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "clamp", .. }));
    }

    #[test]
    fn min_max_basics_and_signed_zero() {
        assert_eq!(cook(&Min::new(), &[1.0, -2.0]).unwrap(), -2.0);
        assert_eq!(cook(&Max::new(), &[1.0, -2.0]).unwrap(), 1.0);
        for (a, b) in [(0.0, -0.0), (-0.0, 0.0)] {
            assert!(same(cook(&Min::new(), &[a, b]).unwrap(), -0.0));
            assert!(same(cook(&Max::new(), &[a, b]).unwrap(), 0.0));
        }
    }

    #[test]
    fn min_max_ignore_a_single_nan() {
        assert_eq!(cook(&Min::new(), &[f64::NAN, 3.0]).unwrap(), 3.0);
        assert_eq!(cook(&Min::new(), &[3.0, f64::NAN]).unwrap(), 3.0);
        assert_eq!(cook(&Max::new(), &[f64::NAN, 3.0]).unwrap(), 3.0);
        assert_eq!(cook(&Max::new(), &[3.0, f64::NAN]).unwrap(), 3.0);
        assert!(cook(&Min::new(), &[f64::NAN, f64::NAN]).unwrap().is_nan());
        assert!(cook(&Max::new(), &[f64::NAN, f64::NAN]).unwrap().is_nan());
        assert_eq!(
            cook(&Min::new(), &[f64::NEG_INFINITY, 0.0]).unwrap(),
            f64::NEG_INFINITY
        );
    }

    #[test]
    fn smoothstep_hermite_values() {
        let s = Smoothstep::new();
        assert_eq!(cook(&s, &[0.0, 1.0, -1.0]).unwrap(), 0.0);
        assert_eq!(cook(&s, &[0.0, 1.0, 0.0]).unwrap(), 0.0);
        assert_eq!(cook(&s, &[0.0, 1.0, 0.25]).unwrap(), 0.15625);
        assert_eq!(cook(&s, &[0.0, 1.0, 0.5]).unwrap(), 0.5);
        assert_eq!(cook(&s, &[0.0, 1.0, 0.75]).unwrap(), 0.84375);
        assert_eq!(cook(&s, &[0.0, 1.0, 1.0]).unwrap(), 1.0);
        assert_eq!(cook(&s, &[0.0, 1.0, 2.0]).unwrap(), 1.0);
        assert_eq!(cook(&s, &[2.0, 6.0, 3.0]).unwrap(), 0.15625);
        // Reversed edges give the falling curve.
        assert_eq!(cook(&s, &[1.0, 0.0, 0.25]).unwrap(), 0.84375);
    }

    #[test]
    fn smoothstep_equal_edges_is_a_step() {
        let s = Smoothstep::new();
        assert_eq!(cook(&s, &[0.5, 0.5, 0.4]).unwrap(), 0.0);
        assert_eq!(cook(&s, &[0.5, 0.5, 0.5]).unwrap(), 1.0);
        assert_eq!(cook(&s, &[0.5, 0.5, 0.6]).unwrap(), 1.0);
        assert!(cook(&s, &[0.5, 0.5, f64::NAN]).unwrap().is_nan());
        assert!(cook(&s, &[0.0, 1.0, f64::NAN]).unwrap().is_nan());
    }

    #[test]
    fn abs_clears_the_sign() {
        let n = Abs::new();
        assert_eq!(cook(&n, &[-2.5]).unwrap(), 2.5);
        assert_eq!(cook(&n, &[3.0]).unwrap(), 3.0);
        assert!(same(cook(&n, &[-0.0]).unwrap(), 0.0));
        assert_eq!(cook(&n, &[f64::NEG_INFINITY]).unwrap(), f64::INFINITY);
        assert!(cook(&n, &[f64::NAN]).unwrap().is_nan());
    }

    #[test]
    fn pow_values_and_ieee_special_cases() {
        let n = Pow::new();
        assert_eq!(cook(&n, &[2.0, 10.0]).unwrap(), 1024.0);
        assert_eq!(cook(&n, &[4.0, 0.5]).unwrap(), 2.0);
        assert_eq!(cook(&n, &[2.0, -2.0]).unwrap(), 0.25);
        assert_eq!(cook(&n, &[-2.0, 3.0]).unwrap(), -8.0);
        assert_eq!(cook(&n, &[7.5, 1.0]).unwrap(), 7.5);
        // x^0 == 1 and 1^y == 1, even for NaN.
        assert_eq!(cook(&n, &[f64::NAN, 0.0]).unwrap(), 1.0);
        assert_eq!(cook(&n, &[1.0, f64::NAN]).unwrap(), 1.0);
        assert!(cook(&n, &[-8.0, 1.0 / 3.0]).unwrap().is_nan());
        assert!(cook(&n, &[f64::NAN, 2.0]).unwrap().is_nan());
        assert_eq!(cook(&n, &[0.0, -1.0]).unwrap(), f64::INFINITY);
        assert_eq!(cook(&n, &[-0.0, -1.0]).unwrap(), f64::NEG_INFINITY);
        assert_eq!(cook(&n, &[0.5, f64::INFINITY]).unwrap(), 0.0);
        assert_eq!(cook(&n, &[2.0, f64::INFINITY]).unwrap(), f64::INFINITY);
    }

    #[test]
    fn sin_cos_values() {
        use std::f64::consts::{FRAC_PI_2, PI};
        assert!(same(cook(&Sin::new(), &[0.0]).unwrap(), 0.0));
        assert!(same(cook(&Sin::new(), &[-0.0]).unwrap(), -0.0));
        assert_eq!(cook(&Sin::new(), &[FRAC_PI_2]).unwrap(), 1.0);
        assert!(cook(&Sin::new(), &[PI]).unwrap().abs() < 1e-15);
        assert_eq!(cook(&Cos::new(), &[0.0]).unwrap(), 1.0);
        assert_eq!(cook(&Cos::new(), &[PI]).unwrap(), -1.0);
        assert!(cook(&Cos::new(), &[FRAC_PI_2]).unwrap().abs() < 1e-15);
        for x in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
            assert!(cook(&Sin::new(), &[x]).unwrap().is_nan());
            assert!(cook(&Cos::new(), &[x]).unwrap().is_nan());
        }
    }

    #[test]
    fn wrap_into_half_open_range() {
        let n = Wrap::new();
        assert_eq!(cook(&n, &[0.25, 0.0, 1.0]).unwrap(), 0.25);
        assert_eq!(cook(&n, &[1.25, 0.0, 1.0]).unwrap(), 0.25);
        assert_eq!(cook(&n, &[-0.25, 0.0, 1.0]).unwrap(), 0.75);
        assert_eq!(cook(&n, &[1.0, 0.0, 1.0]).unwrap(), 0.0);
        assert_eq!(cook(&n, &[-1.0, 0.0, 1.0]).unwrap(), 0.0);
        assert!(same(cook(&n, &[-0.0, 0.0, 1.0]).unwrap(), 0.0));
        assert_eq!(cook(&n, &[370.0, 0.0, 360.0]).unwrap(), 10.0);
        assert_eq!(cook(&n, &[-190.0, -180.0, 180.0]).unwrap(), 170.0);
        assert_eq!(cook(&n, &[180.0, -180.0, 180.0]).unwrap(), -180.0);
        // Reversed bounds are swapped.
        assert_eq!(cook(&n, &[1.25, 1.0, 0.0]).unwrap(), 0.25);
    }

    #[test]
    fn wrap_never_returns_max() {
        let n = Wrap::new();
        for x in [-1e-300, -f64::MIN_POSITIVE, -1e-17, -5e-324] {
            let out = cook(&n, &[x, 0.0, 1.0]).unwrap();
            assert!((0.0..1.0).contains(&out), "{x} -> {out}");
        }
        let out = cook(&n, &[-1e-17, 10.0, 11.0]).unwrap();
        assert!((10.0..11.0).contains(&out), "{out}");
    }

    #[test]
    fn wrap_errors_and_specials() {
        let n = Wrap::new();
        let err = cook(&n, &[0.5, 2.0, 2.0]).unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "max", .. }));
        let err = cook(&n, &[0.5, f64::NAN, 1.0]).unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "min", .. }));
        let err = cook(&n, &[0.5, 0.0, f64::INFINITY]).unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "max", .. }));
        let err = cook(&n, &[0.5, -f64::MAX, f64::MAX]).unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "max", .. }));
        assert!(cook(&n, &[f64::NAN, 0.0, 1.0]).unwrap().is_nan());
        assert!(cook(&n, &[f64::INFINITY, 0.0, 1.0]).unwrap().is_nan());
    }

    #[test]
    fn round_halves_away_from_zero() {
        let n = Round::new();
        assert_eq!(cook(&n, &[2.4, 1.0]).unwrap(), 2.0);
        assert_eq!(cook(&n, &[2.5, 1.0]).unwrap(), 3.0);
        assert_eq!(cook(&n, &[-2.5, 1.0]).unwrap(), -3.0);
        assert_eq!(cook(&n, &[0.5, 1.0]).unwrap(), 1.0);
        assert!(same(cook(&n, &[-0.4, 1.0]).unwrap(), -0.0));
        assert_eq!(cook(&n, &[7.0, 5.0]).unwrap(), 5.0);
        assert_eq!(cook(&n, &[7.5, 5.0]).unwrap(), 10.0);
        assert_eq!(cook(&n, &[7.5, -5.0]).unwrap(), 10.0);
        assert_eq!(cook(&n, &[10.0, 3.0]).unwrap(), 9.0);
    }

    #[test]
    fn round_to_decimal_steps_is_clean() {
        let n = Round::new();
        assert_eq!(cook(&n, &[0.3, 0.1]).unwrap(), 0.3);
        assert_eq!(cook(&n, &[0.25, 0.1]).unwrap(), 0.3);
        assert_eq!(cook(&n, &[-0.25, 0.1]).unwrap(), -0.3);
        assert_eq!(cook(&n, &[1.234_567, 0.01]).unwrap(), 1.23);
        assert_eq!(cook(&n, &[0.374, 0.25]).unwrap(), 0.25);
        assert_eq!(cook(&n, &[0.375, 0.25]).unwrap(), 0.5);
    }

    #[test]
    fn round_errors_and_specials() {
        let n = Round::new();
        for step in [0.0, -0.0, f64::NAN, f64::INFINITY] {
            let err = cook(&n, &[1.0, step]).unwrap_err();
            assert!(matches!(err, NodeError::InvalidInput { port: "step", .. }));
        }
        assert!(cook(&n, &[f64::NAN, 1.0]).unwrap().is_nan());
        assert_eq!(cook(&n, &[f64::INFINITY, 0.1]).unwrap(), f64::INFINITY);
        // Huge quotients pass through instead of overflowing to inf.
        assert_eq!(cook(&n, &[1e308, 0.01]).unwrap(), 1e308);
        assert_eq!(cook(&n, &[1e20, 1.0]).unwrap(), 1e20);
        // Subnormal step: 1/step overflows, x/step is huge, x is kept.
        assert_eq!(cook(&n, &[1.5, 5e-324]).unwrap(), 1.5);
    }

    #[test]
    fn math_nodes_reject_wrong_types() {
        let err = Mul::new()
            .cook(
                &CookContext::default(),
                &[Value::Float(1.0), Value::Bool(true)],
            )
            .unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "b", .. }));
        let err = Smoothstep::new()
            .cook(&CookContext::default(), &[Value::Float(1.0)])
            .unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "edge1", .. }));
    }
}
