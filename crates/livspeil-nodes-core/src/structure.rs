// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Structural nodes (milestone M0.7): subnets and their interface.
//!
//! - `core.subnet@1`: a node that contains a network. It has no ports of
//!   its own; its interface is made of the boundary nodes inside it.
//! - `core.subnet.input@1`: one input port of the enclosing subnet. Its
//!   input `value` is fed from *outside* the subnet (a link in the outer
//!   network, or its own static value, expression or animation: the
//!   promoted parameter); its output `value` is read *inside*.
//! - `core.charm@1`: an instance of a shared Charm definition (M0.16).
//!   Like a subnet it has no ports and contains a network, but that
//!   network is the definition's (see `livspeil_graph::Graph::instantiate`).
//! - `core.output@1`: one output port of the enclosing subnet (its input
//!   `value` is fed inside, its output `value` is read outside). In the
//!   root network it marks a sink: `livspeil cook` prints the root
//!   `core.output` nodes when there are any.
//!
//! The boundary nodes are pass-through and *port-polymorphic*: the
//! registered specs use `Float`, and `livspeil-graph` retypes both ports
//! of a node to the interface port's declared type when it is created
//! (see `livspeil_graph::Graph::add_node_in`). Their cook copies the input
//! to the output, which is correct for every type.

use livspeil_node_api::{CookContext, NodeError, NodeImpl, NodeSpec, PortType, Value};

/// Qualified id of the subnet node.
pub const SUBNET: &str = "core.subnet@1";
/// Qualified id of a Charm instance node.
pub const CHARM: &str = "core.charm@1";
/// Qualified id of the subnet input boundary node.
pub const SUBNET_INPUT: &str = "core.subnet.input@1";
/// Qualified id of the output boundary node.
pub const OUTPUT: &str = "core.output@1";

/// `core.subnet@1`: a container for a network; no ports, cooks to nothing.
#[derive(Debug, Clone)]
pub struct Subnet {
    spec: NodeSpec,
}

impl Subnet {
    #[must_use]
    pub fn new() -> Self {
        Subnet {
            spec: NodeSpec::new(
                "core.subnet",
                1,
                "A node that contains a network (a Charm when it has metadata)",
            ),
        }
    }
}

impl Default for Subnet {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeImpl for Subnet {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _: &CookContext, _: &[Value]) -> Result<Vec<Value>, NodeError> {
        Ok(Vec::new())
    }
}

/// `core.charm@1`: an instance of a shared Charm definition; no ports,
/// cooks to nothing (its contents are the definition's nodes).
#[derive(Debug, Clone)]
pub struct Charm {
    spec: NodeSpec,
}

impl Charm {
    #[must_use]
    pub fn new() -> Self {
        Charm {
            spec: NodeSpec::new("core.charm", 1, "An instance of a shared Charm definition"),
        }
    }
}

impl Default for Charm {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeImpl for Charm {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _: &CookContext, _: &[Value]) -> Result<Vec<Value>, NodeError> {
        Ok(Vec::new())
    }
}

/// A pass-through node with one input and one output called `value`.
fn pass_through(id: &'static str, summary: &'static str, ty: PortType) -> NodeSpec {
    NodeSpec::new(id, 1, summary)
        .input("value", ty, Value::zero(ty))
        .output("value", ty)
}

fn copy(inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
    match inputs {
        [value] => Ok(vec![value.clone()]),
        _ => Err(NodeError::Failed(format!(
            "expected 1 input, got {}",
            inputs.len()
        ))),
    }
}

/// `core.subnet.input@1`: an input port of the enclosing subnet.
#[derive(Debug, Clone)]
pub struct SubnetInput {
    spec: NodeSpec,
}

impl SubnetInput {
    /// The registered form, with `Float` ports.
    #[must_use]
    pub fn new() -> Self {
        Self::with_type(PortType::Float)
    }

    /// The node with ports of type `ty`.
    #[must_use]
    pub fn with_type(ty: PortType) -> Self {
        SubnetInput {
            spec: pass_through(
                "core.subnet.input",
                "An input port of the enclosing subnet",
                ty,
            ),
        }
    }
}

impl Default for SubnetInput {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeImpl for SubnetInput {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        copy(inputs)
    }
}

/// `core.output@1`: an output port of the enclosing subnet, or a sink of
/// the root network.
#[derive(Debug, Clone)]
pub struct Output {
    spec: NodeSpec,
}

impl Output {
    /// The registered form, with `Float` ports.
    #[must_use]
    pub fn new() -> Self {
        Self::with_type(PortType::Float)
    }

    /// The node with ports of type `ty`.
    #[must_use]
    pub fn with_type(ty: PortType) -> Self {
        Output {
            spec: pass_through(
                "core.output",
                "An output of the enclosing subnet, or a sink of the root network",
                ty,
            ),
        }
    }
}

impl Default for Output {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeImpl for Output {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        copy(inputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_match_specs() {
        assert_eq!(Subnet::new().spec().qualified_id(), SUBNET);
        assert_eq!(Charm::new().spec().qualified_id(), CHARM);
        assert_eq!(SubnetInput::new().spec().qualified_id(), SUBNET_INPUT);
        assert_eq!(Output::new().spec().qualified_id(), OUTPUT);
    }

    #[test]
    fn boundary_nodes_pass_values_through() {
        let ctx = CookContext::default();
        let color = Value::Color([0.5, 0.1, 30.0, 1.0]);
        let out = Output::with_type(PortType::Color);
        assert_eq!(out.spec().inputs[0].ty, PortType::Color);
        assert_eq!(
            out.cook(&ctx, std::slice::from_ref(&color)).unwrap(),
            [color]
        );
        let input = SubnetInput::new();
        assert_eq!(
            input.cook(&ctx, &[Value::Float(2.0)]).unwrap(),
            [Value::Float(2.0)]
        );
        assert!(input.cook(&ctx, &[]).is_err());
        assert!(Subnet::new().cook(&ctx, &[]).unwrap().is_empty());
        assert!(Charm::new().cook(&ctx, &[]).unwrap().is_empty());
    }
}
