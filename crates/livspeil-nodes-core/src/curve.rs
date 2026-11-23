// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Curve nodes (milestones M0.8 and M0.17).

use std::sync::Arc;

use livspeil_node_api::{
    CookContext, Curve, NodeError, NodeImpl, NodeSpec, PortType, TypeClass, TypeVar, Value,
    resolve_spec, resolved_default,
};
use livspeil_params::{sample_curve, sample_curve_value};

use crate::float;

/// `core.curve.ramp@1`: `out` is `curve` sampled at `position`.
///
/// Inputs: `curve` (Curve, default the linear ramp from 0 at 0 to 1 at 1),
/// `position` (Float, default 0). Output: `out` (Float).
///
/// Sampling is exactly that of a `Float` animation channel with the
/// position as time (`livspeil_params::anim`): step, linear, bezier and
/// spring segments, hold/loop/ping-pong outside the keys. A `NaN` position
/// gives the first key's value, infinities the end keys' values.
#[derive(Debug)]
pub struct Ramp {
    spec: NodeSpec,
}

impl Ramp {
    #[must_use]
    pub fn new() -> Self {
        Ramp {
            spec: NodeSpec::new("core.curve.ramp", 1, "Sample a curve at a position")
                .input(
                    "curve",
                    PortType::Curve,
                    Some(Value::Curve(Arc::new(Curve::identity()))),
                )
                .input("position", PortType::Float, Some(Value::Float(0.0)))
                .output("out", PortType::Float),
        }
    }
}

impl Default for Ramp {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeImpl for Ramp {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let curve =
            inputs
                .first()
                .and_then(Value::as_curve)
                .ok_or_else(|| NodeError::InvalidInput {
                    port: "curve",
                    reason: "expected a Curve".to_owned(),
                })?;
        let position = float(inputs, 1, "position")?;
        let out = sample_curve(curve, position).map_err(|e| NodeError::InvalidInput {
            port: "curve",
            reason: e.to_string(),
        })?;
        Ok(vec![Value::Float(out)])
    }
}

/// `core.curve.ramp@2`: `out` is `curve` sampled at `position`, for
/// curves of any key type: `<T: Interpolable = Float>(curve: Curve =
/// identity, position: Float = 0) -> out: T`.
///
/// The key type of a curve is data (the port type is always `Curve`), so
/// `T` resolves from where `out` is linked or from the type pinned on the
/// node (`port-type=` in documents), never from the curve. A curve whose
/// keys have type `T` is sampled as such (vectors component by component,
/// colors in OKLab; see `livspeil_params::curve`); a `Float` curve is
/// broadcast (`[x, x, x]` for `Vec3`, the grey `oklch(x 0 0 1)` for
/// colors), so the default identity curve works for every `T`; any other
/// key type is an invalid-input error on `curve`. Unresolved `T` is
/// `Float`, where the node is exactly `@1`.
#[derive(Debug)]
pub struct Ramp2 {
    spec: NodeSpec,
    vars: Vec<TypeVar>,
    ty: PortType,
}

impl Ramp2 {
    #[must_use]
    pub fn new() -> Self {
        Ramp2 {
            spec: NodeSpec::new(
                "core.curve.ramp",
                2,
                "Sample a curve of numbers, vectors or colors at a position",
            )
            .input(
                "curve",
                PortType::Curve,
                Some(Value::Curve(Arc::new(Curve::identity()))),
            )
            .input("position", PortType::Float, Some(Value::Float(0.0)))
            .output("out", PortType::Float),
            vars: vec![TypeVar::new("T", TypeClass::Interpolable, PortType::Float).output("out")],
            ty: PortType::Float,
        }
    }
}

impl Default for Ramp2 {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeImpl for Ramp2 {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn type_vars(&self) -> &[TypeVar] {
        &self.vars
    }

    fn with_types(&self, types: &[PortType]) -> Option<Arc<dyn NodeImpl>> {
        let &[ty] = types else { return None };
        let spec = resolve_spec(&self.spec, &self.vars, types).ok()?;
        Some(Arc::new(Ramp2 {
            spec,
            vars: Vec::new(),
            ty,
        }))
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let curve =
            inputs
                .first()
                .and_then(Value::as_curve)
                .ok_or_else(|| NodeError::InvalidInput {
                    port: "curve",
                    reason: "expected a Curve".to_owned(),
                })?;
        let position = float(inputs, 1, "position")?;
        let bad = |reason: String| NodeError::InvalidInput {
            port: "curve",
            reason,
        };
        let keys = curve.value_type();
        let out = if keys == self.ty {
            sample_curve_value(curve, position).map_err(|e| bad(e.to_string()))?
        } else if keys == PortType::Float {
            let x = sample_curve(curve, position).map_err(|e| bad(e.to_string()))?;
            resolved_default(Some(&Value::Float(x)), self.ty)
                .ok_or_else(|| bad(format!("cannot broadcast to {}", self.ty)))?
        } else {
            return Err(bad(format!(
                "a {keys} curve cannot give {} values (expected {} or Float keys)",
                self.ty, self.ty
            )));
        };
        Ok(vec![out])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use livspeil_node_api::{CurveExtrapolation, CurveKey, TypedCurveKey, instantiate};

    fn ramp2(ty: PortType, curve: Curve, x: f64) -> Result<Value, NodeError> {
        let base: Arc<dyn NodeImpl> = Arc::new(Ramp2::new());
        let node = instantiate(&base, &[ty]).unwrap();
        assert_eq!(node.spec().outputs[0].ty, ty);
        node.cook(
            &CookContext::default(),
            &[Value::Curve(Arc::new(curve)), Value::Float(x)],
        )
        .map(|mut v| v.remove(0))
    }

    #[test]
    fn typed_ramps() {
        assert_eq!(
            ramp2(PortType::Float, Curve::identity(), 0.25),
            Ok(Value::Float(0.25))
        );
        assert_eq!(
            ramp2(PortType::Vec3, Curve::identity(), 0.25),
            Ok(Value::Vec3([0.25; 3]))
        );
        assert_eq!(
            ramp2(PortType::Color, Curve::identity(), 0.5),
            Ok(Value::Color([0.5, 0.0, 0.0, 1.0]))
        );
        let v2 = Curve::typed(vec![
            TypedCurveKey::new(0.0, Value::Vec2([0.0, 0.0])),
            TypedCurveKey::new(1.0, Value::Vec2([2.0, 4.0])),
        ])
        .unwrap();
        assert_eq!(
            ramp2(PortType::Vec2, v2.clone(), 0.5),
            Ok(Value::Vec2([1.0, 2.0]))
        );
        assert!(matches!(
            ramp2(PortType::Vec3, v2.clone(), 0.5),
            Err(NodeError::InvalidInput { port: "curve", .. })
        ));
        assert!(matches!(
            ramp2(PortType::Float, v2, 0.5),
            Err(NodeError::InvalidInput { port: "curve", .. })
        ));
        assert!(Ramp2::new().with_types(&[]).is_none());
    }

    fn ramp(curve: Curve, x: f64) -> f64 {
        Ramp::new()
            .cook(
                &CookContext::default(),
                &[Value::Curve(Arc::new(curve)), Value::Float(x)],
            )
            .unwrap()[0]
            .as_float()
            .unwrap()
    }

    #[test]
    fn samples() {
        assert_eq!(ramp(Curve::identity(), 0.25), 0.25);
        assert_eq!(ramp(Curve::identity(), 2.0), 1.0);
        assert_eq!(ramp(Curve::identity(), f64::NAN), 0.0);
        let looped = Curve::new(vec![CurveKey::new(0.0, 0.0), CurveKey::new(2.0, 4.0)])
            .unwrap()
            .with_extrapolation(CurveExtrapolation::Hold, CurveExtrapolation::Loop);
        assert_eq!(ramp(looped, 3.0), 2.0);
    }
}
