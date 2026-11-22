// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! 3D vector nodes (milestone M0.6c).
//!
//! `Vec3` values are `[x, y, z]`; in the scene frame +y is up, +x is east
//! and +z is south (see `core.sky.sun_direction`). The nodes use only IEEE
//! 754 basic operations and `sqrt`, which are correctly rounded, so their
//! results are bit-identical on every platform. `NaN` and infinities
//! propagate unless a node documents otherwise; no node here fails on
//! values, only on wrongly typed inputs.

use livspeil_node_api::{CookContext, NodeError, NodeImpl, NodeSpec, PortType, Value};

use crate::float;

fn vec3(inputs: &[Value], index: usize, port: &'static str) -> Result<[f64; 3], NodeError> {
    match inputs.get(index) {
        Some(Value::Vec3(v)) => Ok(*v),
        _ => Err(NodeError::InvalidInput {
            port,
            reason: "expected a Vec3".to_owned(),
        }),
    }
}

fn vec3_in(spec: NodeSpec, name: &'static str, default: [f64; 3]) -> NodeSpec {
    spec.input(name, PortType::Vec3, Some(Value::Vec3(default)))
}

fn float_in(spec: NodeSpec, name: &'static str, default: f64) -> NodeSpec {
    spec.input(name, PortType::Float, Some(Value::Float(default)))
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

/// `a · b`, summed left to right (`x`, then `y`, then `z`).
#[must_use]
pub fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// 2^600 and 2^-600: multiplying by a power of two is exact, so these
/// rescale a vector whose squared length would overflow or underflow without
/// adding rounding error.
const SCALE_UP: f64 = f64::from_bits(0x6570_0000_0000_0000);
const SCALE_DOWN: f64 = f64::from_bits(0x1A70_0000_0000_0000);

/// A finite, non-`NaN` `v` rescaled by a power of two so that its squared
/// length is a normal finite number, with the factor used. Zero stays zero.
fn rescale(v: [f64; 3]) -> ([f64; 3], f64) {
    let sum = dot3(v, v);
    let k = if sum.is_infinite() {
        SCALE_DOWN
    } else if sum < f64::MIN_POSITIVE && v.iter().any(|c| *c != 0.0) {
        SCALE_UP
    } else {
        1.0
    };
    (v.map(|c| c * k), k)
}

/// Euclidean length of `v`.
///
/// `sqrt(x² + y² + z²)`; when that sum would overflow past `f64::MAX` or
/// underflow into subnormals, the components are first scaled by an exact
/// power of two, so lengths up to `f64::MAX` and down to the smallest
/// subnormal come out right. `NaN` if any component is `NaN`, else `inf` if
/// any component is infinite.
#[must_use]
pub fn length3(v: [f64; 3]) -> f64 {
    if v.iter().any(|c| c.is_nan()) {
        return f64::NAN;
    }
    if v.iter().any(|c| c.is_infinite()) {
        return f64::INFINITY;
    }
    let (w, k) = rescale(v);
    dot3(w, w).sqrt() / k
}

/// `v` scaled to unit length.
///
/// - The zero vector (either sign of zero) maps to `[0, 0, 0]`, so a
///   degenerate input gives a harmless "no direction" instead of `NaN`.
/// - Any `NaN` component makes every component `NaN`.
/// - Infinite components define the direction on their own (the limit of
///   growing that component): `[inf, 5, 0]` gives `[1, 0, 0]`,
///   `[inf, -inf, 0]` gives `[1/√2, -1/√2, 0]`.
/// - Very large and very small vectors are rescaled exactly first (see
///   [`length3`]), so they normalize as well as ordinary ones.
#[must_use]
pub fn normalize3(v: [f64; 3]) -> [f64; 3] {
    if v.iter().any(|c| c.is_nan()) {
        return [f64::NAN; 3];
    }
    let v = if v.iter().any(|c| c.is_infinite()) {
        v.map(|c| if c.is_infinite() { c.signum() } else { 0.0 })
    } else {
        v
    };
    let (w, _) = rescale(v);
    let len = dot3(w, w).sqrt();
    if len == 0.0 {
        return [0.0; 3];
    }
    w.map(|c| c / len)
}

/// `core.vector.compose3@1`: build a `Vec3` from `x`, `y`, `z` (Floats,
/// default 0).
#[derive(Debug)]
pub struct Compose3 {
    spec: NodeSpec,
}

impl Compose3 {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new("core.vector.compose3", 1, "Build a Vec3 from x, y and z");
        let spec = float_in(float_in(float_in(spec, "x", 0.0), "y", 0.0), "z", 0.0);
        Compose3 {
            spec: spec.output("out", PortType::Vec3),
        }
    }
}
node_boilerplate!(Compose3);

impl NodeImpl for Compose3 {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let x = float(inputs, 0, "x")?;
        let y = float(inputs, 1, "y")?;
        let z = float(inputs, 2, "z")?;
        Ok(vec![Value::Vec3([x, y, z])])
    }
}

/// `core.vector.decompose3@1`: split `v` (Vec3, default `[0, 0, 0]`) into
/// the Floats `x`, `y`, `z`.
#[derive(Debug)]
pub struct Decompose3 {
    spec: NodeSpec,
}

impl Decompose3 {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new("core.vector.decompose3", 1, "Split a Vec3 into x, y and z");
        Decompose3 {
            spec: vec3_in(spec, "v", [0.0; 3])
                .output("x", PortType::Float)
                .output("y", PortType::Float)
                .output("z", PortType::Float),
        }
    }
}
node_boilerplate!(Decompose3);

impl NodeImpl for Decompose3 {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let [x, y, z] = vec3(inputs, 0, "v")?;
        Ok(vec![Value::Float(x), Value::Float(y), Value::Float(z)])
    }
}

/// `core.vector.length3@1`: Euclidean length of `v` (Vec3, default
/// `[0, 0, 0]`) as the Float `out`. Overflow- and underflow-safe; see
/// [`length3`].
#[derive(Debug)]
pub struct Length3 {
    spec: NodeSpec,
}

impl Length3 {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new("core.vector.length3", 1, "Euclidean length of a Vec3");
        Length3 {
            spec: vec3_in(spec, "v", [0.0; 3]).output("out", PortType::Float),
        }
    }
}
node_boilerplate!(Length3);

impl NodeImpl for Length3 {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let v = vec3(inputs, 0, "v")?;
        Ok(vec![Value::Float(length3(v))])
    }
}

/// `core.vector.normalize3@1`: `v` (Vec3, default `[0, 0, 0]`) scaled to
/// unit length as the Vec3 `out`. The zero vector gives the zero vector;
/// see [`normalize3`] for `NaN` and infinities.
#[derive(Debug)]
pub struct Normalize3 {
    spec: NodeSpec,
}

impl Normalize3 {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new(
            "core.vector.normalize3",
            1,
            "Scale a Vec3 to unit length (the zero vector stays zero)",
        );
        Normalize3 {
            spec: vec3_in(spec, "v", [0.0; 3]).output("out", PortType::Vec3),
        }
    }
}
node_boilerplate!(Normalize3);

impl NodeImpl for Normalize3 {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let v = vec3(inputs, 0, "v")?;
        Ok(vec![Value::Vec3(normalize3(v))])
    }
}

/// `core.vector.dot3@1`: dot product of `a` and `b` (Vec3s, default
/// `[0, 0, 0]`) as the Float `out`, summed in the order x, y, z. IEEE 754
/// throughout: `inf · 0` gives `NaN`.
#[derive(Debug)]
pub struct Dot3 {
    spec: NodeSpec,
}

impl Dot3 {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new("core.vector.dot3", 1, "Dot product of two Vec3s");
        Dot3 {
            spec: vec3_in(vec3_in(spec, "a", [0.0; 3]), "b", [0.0; 3])
                .output("out", PortType::Float),
        }
    }
}
node_boilerplate!(Dot3);

impl NodeImpl for Dot3 {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let a = vec3(inputs, 0, "a")?;
        let b = vec3(inputs, 1, "b")?;
        Ok(vec![Value::Float(dot3(a, b))])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2^-1022 and 2^1000: exact powers of two, so small multiples of them
    /// are exact and their squares underflow or overflow.
    const TINY: f64 = f64::MIN_POSITIVE;
    const HUGE: f64 = f64::from_bits((1023 + 1000) << 52);

    fn run(node: &dyn NodeImpl, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        node.cook(&CookContext::default(), inputs)
    }

    fn out_vec3(values: &[Value]) -> [f64; 3] {
        let Value::Vec3(v) = values[0] else {
            panic!("not a Vec3: {:?}", values[0]);
        };
        v
    }

    /// Bitwise equality, so `-0.0` and `NaN` can be asserted exactly.
    fn same3(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter()
            .zip(&b)
            .all(|(x, y)| x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()))
    }

    #[test]
    fn compose_and_decompose_round_trip() {
        let out = run(
            &Compose3::new(),
            &[
                Value::Float(1.5),
                Value::Float(-0.0),
                Value::Float(f64::NAN),
            ],
        )
        .unwrap();
        let v = out_vec3(&out);
        assert!(same3(v, [1.5, -0.0, f64::NAN]));
        let parts = run(&Decompose3::new(), &out).unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], Value::Float(1.5));
        assert!(parts[1].as_float().unwrap().is_sign_negative());
        assert!(parts[2].as_float().unwrap().is_nan());
    }

    #[test]
    fn length_exact_values() {
        assert_eq!(length3([3.0, 4.0, 0.0]), 5.0);
        assert_eq!(length3([2.0, 3.0, 6.0]), 7.0);
        assert_eq!(length3([-1.0, -2.0, 2.0]), 3.0);
        assert_eq!(length3([0.0, 0.0, 0.0]), 0.0);
        assert_eq!(length3([-0.0, 0.0, -0.0]), 0.0);
        assert_eq!(length3([0.0, 0.0, -2.5]), 2.5);
    }

    #[test]
    fn length_survives_overflow_and_underflow() {
        // Exactly representable triples scale exactly.
        assert_eq!(length3([3.0 * HUGE, 4.0 * HUGE, 0.0]), 5.0 * HUGE);
        assert_eq!(length3([3.0 * TINY, 4.0 * TINY, 0.0]), 5.0 * TINY);
        assert_eq!(length3([f64::MAX, 0.0, 0.0]), f64::MAX);
        assert_eq!(length3([-f64::MAX, 0.0, 0.0]), f64::MAX);
        assert_eq!(length3([5e-324, 0.0, 0.0]), 5e-324);
        assert_eq!(length3([0.0, f64::MIN_POSITIVE, 0.0]), f64::MIN_POSITIVE);
        let l = length3([3e200, 4e200, 0.0]);
        assert!((l / 5e200 - 1.0).abs() <= f64::EPSILON, "{l}");
        let l = length3([3e-200, 4e-200, 0.0]);
        assert!((l / 5e-200 - 1.0).abs() <= f64::EPSILON, "{l}");
    }

    #[test]
    fn length_specials() {
        assert!(length3([f64::NAN, 1.0, 0.0]).is_nan());
        assert!(length3([f64::INFINITY, f64::NAN, 0.0]).is_nan());
        assert_eq!(length3([f64::NEG_INFINITY, 1.0, 0.0]), f64::INFINITY);
    }

    #[test]
    fn normalize_values() {
        assert_eq!(normalize3([3.0, 0.0, 4.0]), [0.6, 0.0, 0.8]);
        assert_eq!(normalize3([0.0, -7.0, 0.0]), [0.0, -1.0, 0.0]);
        assert_eq!(
            normalize3([2.0, 3.0, 6.0]),
            [2.0 / 7.0, 3.0 / 7.0, 6.0 / 7.0]
        );
        assert_eq!(normalize3([3.0 * TINY, 0.0, 4.0 * TINY]), [0.6, 0.0, 0.8]);
        assert_eq!(normalize3([3.0 * HUGE, 0.0, 4.0 * HUGE]), [0.6, 0.0, 0.8]);
        for v in [[3e-200, 4e-200, 0.0], [3e200, 4e200, 0.0]] {
            let n = normalize3(v);
            assert!((n[0] - 0.6).abs() <= f64::EPSILON, "{n:?}");
            assert!((n[1] - 0.8).abs() <= f64::EPSILON, "{n:?}");
        }
        let n = normalize3([1.0, 1.0, 1.0]);
        assert!((length3(n) - 1.0).abs() <= f64::EPSILON);
    }

    #[test]
    fn normalize_zero_is_zero() {
        assert!(same3(normalize3([0.0; 3]), [0.0; 3]));
        assert!(same3(normalize3([-0.0, 0.0, -0.0]), [0.0; 3]));
        let out = run(&Normalize3::new(), &[Value::Vec3([0.0; 3])]).unwrap();
        assert!(same3(out_vec3(&out), [0.0; 3]));
    }

    #[test]
    fn normalize_specials() {
        assert!(same3(normalize3([f64::NAN, 1.0, 0.0]), [f64::NAN; 3]));
        assert!(same3(
            normalize3([f64::INFINITY, 5.0, 0.0]),
            [1.0, 0.0, 0.0]
        ));
        let h = std::f64::consts::FRAC_1_SQRT_2;
        let v = normalize3([f64::INFINITY, f64::NEG_INFINITY, 1.0]);
        assert!((v[0] - h).abs() <= f64::EPSILON, "{v:?}");
        assert!((v[1] + h).abs() <= f64::EPSILON, "{v:?}");
        assert_eq!(v[2], 0.0);
    }

    #[test]
    fn dot_values() {
        assert_eq!(dot3([1.0, 2.0, 3.0], [4.0, -5.0, 6.0]), 12.0);
        assert_eq!(dot3([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), 0.0);
        assert!(dot3([f64::INFINITY, 0.0, 0.0], [0.0, 1.0, 0.0]).is_nan());
        let out = run(
            &Dot3::new(),
            &[Value::Vec3([0.5, 0.5, 0.0]), Value::Vec3([2.0, 4.0, 9.0])],
        )
        .unwrap();
        assert_eq!(out, vec![Value::Float(3.0)]);
    }

    #[test]
    fn vector_nodes_reject_wrong_types() {
        let err = run(&Length3::new(), &[Value::Float(1.0)]).unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "v", .. }));
        let err = run(
            &Dot3::new(),
            &[Value::Vec3([0.0; 3]), Value::Color([0.0; 4])],
        )
        .unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "b", .. }));
        let err = run(
            &Compose3::new(),
            &[Value::Float(0.0), Value::Bool(true), Value::Float(0.0)],
        )
        .unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "y", .. }));
        let err = run(&Decompose3::new(), &[]).unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "v", .. }));
        let err = run(&Normalize3::new(), &[Value::Vec2([1.0, 0.0])]).unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "v", .. }));
    }
}
