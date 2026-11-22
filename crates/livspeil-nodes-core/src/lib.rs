// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The first built-in nodes.
//!
//! The core node set (milestones M0.2 and M0.6). Earliest nodes:
//! `core.math.add`, `core.math.lerp` and `core.time.clock`. The rest of the
//! set arrives in M0.6, one node per task (spec, implementation, golden
//! test); see `specs/livspeil-kernel-spec-v0.md`, sections 8 and 9. The specs
//! are written in Rust for now; `*.node.toml` generation is a later step.

use std::sync::Arc;

mod logic;
mod math;

pub use logic::{And, Compare, Not, Or, Select};
pub use math::{Clamp, Div, Max, Min, Mul, Remap, Smoothstep, Sub};
mod color;
mod sky;
mod time;

pub use color::{CHROMA_EPSILON, ColorMix, lab_to_lch, lch_to_lab, mix_oklch};
pub use sky::{
    AZIMUTH_EPSILON, AZIMUTH_SEAM_EPSILON, SunDirection, SunPosition, declination_degrees,
    sun_position,
};
pub use time::{SECONDS_PER_DAY, TimeOfDay, time_of_day};

use livspeil_node_api::{
    CookContext, NodeError, NodeImpl, NodeRegistry, NodeSpec, PortType, RegistryError, Value,
};

/// Register every core node into `registry`.
///
/// # Errors
/// Returns a [`RegistryError`] if one of the ids is already taken.
pub fn register_all(registry: &mut NodeRegistry) -> Result<(), RegistryError> {
    registry.register(Arc::new(Add::new()))?;
    registry.register(Arc::new(Lerp::new()))?;
    registry.register(Arc::new(Clock::new()))?;
    registry.register(Arc::new(Sub::new()))?;
    registry.register(Arc::new(Mul::new()))?;
    registry.register(Arc::new(Div::new()))?;
    registry.register(Arc::new(Clamp::new()))?;
    registry.register(Arc::new(Remap::new()))?;
    registry.register(Arc::new(Min::new()))?;
    registry.register(Arc::new(Max::new()))?;
    registry.register(Arc::new(Smoothstep::new()))?;
    registry.register(Arc::new(Select::new()))?;
    registry.register(Arc::new(Compare::new()))?;
    registry.register(Arc::new(And::new()))?;
    registry.register(Arc::new(Or::new()))?;
    registry.register(Arc::new(Not::new()))?;
    registry.register(Arc::new(TimeOfDay::new()))?;
    registry.register(Arc::new(SunDirection::new()))?;
    registry.register(Arc::new(ColorMix::new()))?;
    Ok(())
}

/// A registry holding exactly the core nodes.
#[must_use]
pub fn core_registry() -> NodeRegistry {
    let mut registry = NodeRegistry::new();
    // The ids below are unique by construction, so registration cannot fail.
    if let Err(err) = register_all(&mut registry) {
        unreachable!("core node ids are unique: {err}");
    }
    registry
}

fn float(inputs: &[Value], index: usize, port: &'static str) -> Result<f64, NodeError> {
    inputs
        .get(index)
        .and_then(Value::as_float)
        .ok_or_else(|| NodeError::InvalidInput {
            port,
            reason: "expected a Float".to_owned(),
        })
}

/// `core.math.add@1`: `out = a + b`.
#[derive(Debug)]
pub struct Add {
    spec: NodeSpec,
}

impl Add {
    #[must_use]
    pub fn new() -> Self {
        Add {
            spec: NodeSpec::new("core.math.add", 1, "Add two numbers")
                .input("a", PortType::Float, Some(Value::Float(0.0)))
                .input("b", PortType::Float, Some(Value::Float(0.0)))
                .output("out", PortType::Float),
        }
    }
}

impl Default for Add {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeImpl for Add {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let a = float(inputs, 0, "a")?;
        let b = float(inputs, 1, "b")?;
        Ok(vec![Value::Float(a + b)])
    }
}

/// `core.math.lerp@1`: `out = a + (b - a) * t` (not clamped).
#[derive(Debug)]
pub struct Lerp {
    spec: NodeSpec,
}

impl Lerp {
    #[must_use]
    pub fn new() -> Self {
        Lerp {
            spec: NodeSpec::new("core.math.lerp", 1, "Linear interpolation between a and b")
                .input("a", PortType::Float, Some(Value::Float(0.0)))
                .input("b", PortType::Float, Some(Value::Float(1.0)))
                .input("t", PortType::Float, Some(Value::Float(0.5)))
                .output("out", PortType::Float),
        }
    }
}

impl Default for Lerp {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeImpl for Lerp {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let a = float(inputs, 0, "a")?;
        let b = float(inputs, 1, "b")?;
        let t = float(inputs, 2, "t")?;
        Ok(vec![Value::Float(a + (b - a) * t)])
    }
}

/// `core.time.clock@1`: publishes the frame clock as `frame`, `time`, `dt`.
#[derive(Debug)]
pub struct Clock {
    spec: NodeSpec,
}

impl Clock {
    #[must_use]
    pub fn new() -> Self {
        Clock {
            spec: NodeSpec::new("core.time.clock", 1, "The frame clock")
                .time_dependent()
                .output("frame", PortType::Int)
                .output("time", PortType::Float)
                .output("dt", PortType::Float),
        }
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeImpl for Clock {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, ctx: &CookContext, _inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let frame = i64::try_from(ctx.frame)
            .map_err(|_| NodeError::Failed("frame counter exceeds i64".to_owned()))?;
        Ok(vec![
            Value::Int(frame),
            Value::Float(ctx.time),
            Value::Float(ctx.dt),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> CookContext {
        CookContext {
            frame: 12,
            time: 0.2,
            dt: 1.0 / 60.0,
        }
    }

    #[test]
    fn registry_contains_the_core_nodes() {
        let reg = core_registry();
        for id in [
            "core.math.add@1",
            "core.math.lerp@1",
            "core.time.clock@1",
            "core.math.sub@1",
            "core.math.mul@1",
            "core.math.div@1",
            "core.math.clamp@1",
            "core.math.remap@1",
            "core.math.min@1",
            "core.math.max@1",
            "core.math.smoothstep@1",
            "core.logic.select@1",
            "core.logic.compare@1",
            "core.logic.and@1",
            "core.logic.or@1",
            "core.logic.not@1",
            "core.time.timeofday@1",
            "core.sky.sun_direction@1",
            "core.color.mix@1",
        ] {
            assert!(reg.get(id).is_some(), "{id} is not registered");
        }
        assert!(register_all(&mut core_registry()).is_err());
    }

    #[test]
    fn add_adds() {
        let out = Add::new()
            .cook(&ctx(), &[Value::Float(1.5), Value::Float(2.0)])
            .unwrap();
        assert_eq!(out, vec![Value::Float(3.5)]);
    }

    #[test]
    fn lerp_matches_the_spec_example() {
        // From the kernel spec: a=0, b=10, t=0.25 -> 2.5
        let out = Lerp::new()
            .cook(
                &ctx(),
                &[Value::Float(0.0), Value::Float(10.0), Value::Float(0.25)],
            )
            .unwrap();
        assert_eq!(out, vec![Value::Float(2.5)]);
    }

    #[test]
    fn lerp_is_not_clamped() {
        let out = Lerp::new()
            .cook(
                &ctx(),
                &[Value::Float(0.0), Value::Float(10.0), Value::Float(2.0)],
            )
            .unwrap();
        assert_eq!(out, vec![Value::Float(20.0)]);
    }

    #[test]
    fn math_nodes_reject_non_float_inputs() {
        let err = Add::new()
            .cook(&ctx(), &[Value::Bool(true), Value::Float(1.0)])
            .unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "a", .. }));
    }

    #[test]
    fn clock_reports_the_context() {
        let out = Clock::new().cook(&ctx(), &[]).unwrap();
        assert_eq!(
            out,
            vec![Value::Int(12), Value::Float(0.2), Value::Float(1.0 / 60.0)]
        );
        assert!(Clock::new().spec().time_dependent);
    }

    #[test]
    fn specs_declare_matching_output_types() {
        let reg = core_registry();
        for id in reg.ids() {
            let node = reg.get(id).unwrap();
            let spec = node.spec();
            let inputs: Vec<Value> = spec
                .inputs
                .iter()
                .map(|p| p.default.clone().unwrap())
                .collect();
            let outputs = node.cook(&ctx(), &inputs).unwrap();
            assert_eq!(outputs.len(), spec.outputs.len(), "{id}");
            for (value, port) in outputs.iter().zip(&spec.outputs) {
                assert_eq!(value.port_type(), port.ty, "{id}.{}", port.name);
            }
        }
    }
}
