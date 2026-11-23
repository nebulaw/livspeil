// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Test-only node types and helpers. The command bus tests define their own
//! nodes so they do not depend on the core node set.

#![allow(dead_code)]

use std::sync::Arc;

use livspeil_graph::{CharmDef, Graph, NodeId};
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
    // Subnets and their boundary nodes (structural, M0.7).
    registry
        .register(Arc::new(livspeil_nodes_core::Subnet::new()))
        .expect("subnet registers");
    registry
        .register(Arc::new(livspeil_nodes_core::SubnetInput::new()))
        .expect("subnet input registers");
    registry
        .register(Arc::new(livspeil_nodes_core::Output::new()))
        .expect("output registers");
    // Charm instances (M0.16).
    registry
        .register(Arc::new(livspeil_nodes_core::Charm::new()))
        .expect("charm registers");
    Arc::new(registry)
}

pub fn graph() -> Graph {
    Graph::new(registry())
}

pub fn id(n: u128) -> NodeId {
    NodeId::from_uuid(Uuid::from_u128(n))
}

/// Local ids of the test definitions.
pub const D_IN: u128 = 0x101;
pub const D_GAIN: u128 = 0x102;
pub const D_NUM: u128 = 0x103;
pub const D_OUT: u128 = 0x104;
pub const D_EXTRA: u128 = 0x105;

/// Definition `def_id` at `version`: interface inputs `in` (default 1.0)
/// and `gain` (default `gain`, absent when `variant == 2`), a `t.num`
/// adding them (`sum`, renamed `total` in variant 1, which also adds an
/// `extra` node with an expression), and output `out`.
pub fn definition(def_id: &str, version: &str, gain: f64, variant: u8) -> CharmDef {
    use livspeil_graph::{INPUT_TYPE, INTERFACE_PORT, OUTPUT_TYPE, SUBNET_TYPE};
    let mut g = graph();
    let s = g
        .add_node_in(None, id(0x100), SUBNET_TYPE, "scratch", None)
        .unwrap();
    let input = g
        .add_node_in(Some(s), id(D_IN), INPUT_TYPE, "in", Some(PortType::Float))
        .unwrap();
    g.set_input(input, INTERFACE_PORT, Value::Float(1.0))
        .unwrap();
    let name = if variant == 1 { "total" } else { "sum" };
    let num = g.add_node_in(Some(s), id(D_NUM), NUM, name, None).unwrap();
    let out = g
        .add_node_in(
            Some(s),
            id(D_OUT),
            OUTPUT_TYPE,
            "out",
            Some(PortType::Float),
        )
        .unwrap();
    g.link(input, INTERFACE_PORT, num, "a").unwrap();
    if variant != 2 {
        let gain_in = g
            .add_node_in(
                Some(s),
                id(D_GAIN),
                INPUT_TYPE,
                "gain",
                Some(PortType::Float),
            )
            .unwrap();
        g.set_input(gain_in, INTERFACE_PORT, Value::Float(gain))
            .unwrap();
        g.link(gain_in, INTERFACE_PORT, num, "b").unwrap();
    }
    if variant == 1 {
        let extra = g
            .add_node_in(Some(s), id(D_EXTRA), NUM, "extra", None)
            .unwrap();
        g.set_expression(extra, "b", "ch(\"total/b\") + 1").unwrap();
        g.link(num, "out", extra, "a").unwrap();
        g.link(extra, "out", out, INTERFACE_PORT).unwrap();
    } else {
        g.link(num, "out", out, INTERFACE_PORT).unwrap();
    }
    g.definition_from_subnet(s, def_id, version).unwrap()
}
