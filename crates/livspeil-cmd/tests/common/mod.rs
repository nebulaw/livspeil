// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Test-only node types and helpers. The command bus tests define their own
//! nodes so they do not depend on the core node set.

#![allow(dead_code)]

use std::sync::Arc;

use livspeil_graph::{Graph, NodeId};
use livspeil_node_api::{
    CookContext, NodeError, NodeImpl, NodeRegistry, NodeSpec, PortType, Value,
};
use uuid::Uuid;

pub const NUM: &str = "t.num@1";
pub const FLAG: &str = "t.flag@1";

/// Two Float inputs, one Float output.
#[derive(Debug)]
struct Num(NodeSpec);

impl NodeImpl for Num {
    fn spec(&self) -> &NodeSpec {
        &self.0
    }
    fn cook(&self, _: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let a = inputs[0].as_float().unwrap_or(0.0);
        let b = inputs[1].as_float().unwrap_or(0.0);
        Ok(vec![Value::Float(a + b)])
    }
}

/// Bool and Int inputs and outputs.
#[derive(Debug)]
struct Flag(NodeSpec);

impl NodeImpl for Flag {
    fn spec(&self) -> &NodeSpec {
        &self.0
    }
    fn cook(&self, _: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        Ok(inputs.to_vec())
    }
}

pub fn registry() -> Arc<NodeRegistry> {
    let mut registry = NodeRegistry::new();
    registry
        .register(Arc::new(Num(NodeSpec::new("t.num", 1, "Adds")
            .input("a", PortType::Float, Some(Value::Float(0.0)))
            .input("b", PortType::Float, Some(Value::Float(0.0)))
            .output("out", PortType::Float))))
        .expect("num registers");
    registry
        .register(Arc::new(Flag(
            NodeSpec::new("t.flag", 1, "Passes through")
                .input("flag", PortType::Bool, None)
                .input("count", PortType::Int, None)
                .output("flag", PortType::Bool)
                .output("n", PortType::Int),
        )))
        .expect("flag registers");
    Arc::new(registry)
}

pub fn graph() -> Graph {
    Graph::new(registry())
}

pub fn id(n: u128) -> NodeId {
    NodeId::from_uuid(Uuid::from_u128(n))
}
