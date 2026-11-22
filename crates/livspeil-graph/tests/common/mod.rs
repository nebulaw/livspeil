// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared helpers and test-only node types.

#![allow(dead_code)]

use std::sync::Arc;

use livspeil_graph::Graph;
use livspeil_node_api::{
    CookContext, NodeError, NodeImpl, NodeRegistry, NodeSpec, PortType, Value,
};
use livspeil_nodes_core::register_all;

/// Doubles `x`; fails for negative input and panics for `x > 1000`.
#[derive(Debug)]
pub struct Fragile {
    spec: NodeSpec,
}

impl Fragile {
    pub fn new() -> Self {
        Fragile {
            spec: NodeSpec::new("test.fragile", 1, "Fails on demand")
                .input("x", PortType::Float, Some(Value::Float(0.0)))
                .output("out", PortType::Float),
        }
    }
}

impl NodeImpl for Fragile {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let x = inputs[0].as_float().expect("coerced to Float");
        if x < 0.0 {
            return Err(NodeError::InvalidInput {
                port: "x",
                reason: "must not be negative".to_owned(),
            });
        }
        assert!(x <= 1000.0, "x is far too large");
        Ok(vec![Value::Float(x * 2.0)])
    }
}

/// Declares one Float output but returns a Bool.
#[derive(Debug)]
pub struct Liar {
    spec: NodeSpec,
}

impl Liar {
    pub fn new() -> Self {
        Liar {
            spec: NodeSpec::new("test.liar", 1, "Returns the wrong type")
                .output("out", PortType::Float),
        }
    }
}

impl NodeImpl for Liar {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _: &CookContext, _: &[Value]) -> Result<Vec<Value>, NodeError> {
        Ok(vec![Value::Bool(true)])
    }
}

/// Accepts a Bool and an Int; useful for type-mismatch tests.
#[derive(Debug)]
pub struct Sink {
    spec: NodeSpec,
}

impl Sink {
    pub fn new() -> Self {
        Sink {
            spec: NodeSpec::new("test.sink", 1, "Takes a Bool and an Int")
                .input("flag", PortType::Bool, None)
                .input("count", PortType::Int, None)
                .output("flag", PortType::Bool),
        }
    }
}

impl NodeImpl for Sink {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        Ok(vec![inputs[0].clone()])
    }
}

/// Core nodes plus the test-only nodes above.
pub fn registry() -> Arc<NodeRegistry> {
    let mut registry = NodeRegistry::new();
    register_all(&mut registry).expect("core nodes register");
    registry
        .register(Arc::new(Fragile::new()))
        .expect("fragile registers");
    registry
        .register(Arc::new(Liar::new()))
        .expect("liar registers");
    registry
        .register(Arc::new(Sink::new()))
        .expect("sink registers");
    Arc::new(registry)
}

pub fn graph() -> Graph {
    Graph::new(registry())
}

pub fn float(v: f64) -> Value {
    Value::Float(v)
}
