// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Curve nodes (milestone M0.8).

use std::sync::Arc;

use livspeil_node_api::{CookContext, Curve, NodeError, NodeImpl, NodeSpec, PortType, Value};
use livspeil_params::sample_curve;

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

#[cfg(test)]
mod tests {
    use super::*;
    use livspeil_node_api::{CurveExtrapolation, CurveKey};

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
