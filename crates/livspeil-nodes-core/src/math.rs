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
