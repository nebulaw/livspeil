// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared helpers and a test node with one input of every value type.

#![allow(dead_code)]

use std::sync::Arc;

use livspeil_graph::NodeId;
use livspeil_node_api::{
    CookContext, NodeError, NodeImpl, NodeRegistry, NodeSpec, PortType, Value,
};
use livspeil_nodes_core::register_all;
use uuid::Uuid;

/// `test.all@1`: one input per value type, one output per value type.
#[derive(Debug)]
pub struct AllTypes {
    spec: NodeSpec,
}

pub const ALL_TYPES: [(&str, PortType); 8] = [
    ("f", PortType::Float),
    ("i", PortType::Int),
    ("b", PortType::Bool),
    ("v2", PortType::Vec2),
    ("v3", PortType::Vec3),
    ("v4", PortType::Vec4),
    ("c", PortType::Color),
    ("s", PortType::String),
];

impl AllTypes {
    pub fn new() -> Self {
        let mut spec = NodeSpec::new("test.all", 1, "One port of every value type");
        for (name, ty) in ALL_TYPES {
            spec = spec.input(name, ty, Value::zero(ty));
        }
        for (name, ty) in ALL_TYPES {
            spec = spec.output(name, ty);
        }
        AllTypes { spec }
    }
}

impl NodeImpl for AllTypes {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        Ok(inputs.to_vec())
    }
}

/// Core nodes plus `test.all@1`.
pub fn registry() -> Arc<NodeRegistry> {
    let mut reg = NodeRegistry::new();
    register_all(&mut reg).unwrap();
    reg.register(Arc::new(AllTypes::new())).unwrap();
    Arc::new(reg)
}

/// A deterministic UUIDv7-shaped id.
pub fn id(n: u128) -> NodeId {
    NodeId::from_uuid(Uuid::from_u128(
        0x0192_0000_0000_7000_8000_0000_0000_0000 | n,
    ))
}
