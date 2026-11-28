// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The root network as an editor shows it.
//!
//! A plain node owns its ports. A subnet or Charm instance has none: its
//! interface is a boundary node inside it per port, and a link into or out
//! of the subnet ends on that boundary node. [`Port`] hides the difference.

use std::collections::HashMap;

use livspeil_graph::{Graph, INTERFACE_PORT, NodeId, PortDirection};
use livspeil_node_api::PortType;

/// One port of a shown node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    /// The name shown.
    pub name: String,
    /// Its type.
    pub ty: PortType,
    /// The graph node that has the port.
    pub node: NodeId,
    /// The port's name there.
    pub port: String,
}

/// A node of the root network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// The node.
    pub id: NodeId,
    /// Its name.
    pub name: String,
    /// Inputs, top to bottom.
    pub inputs: Vec<Port>,
    /// Outputs, top to bottom.
    pub outputs: Vec<Port>,
}

fn ports(graph: &Graph, id: NodeId, direction: PortDirection) -> Vec<Port> {
    if graph.is_subnet(id) || graph.is_instance(id) {
        return graph
            .interface(id, direction)
            .into_iter()
            .filter_map(|boundary| {
                let spec = graph.node_spec(boundary)?;
                let ty = match direction {
                    PortDirection::Input => spec.inputs.first(),
                    PortDirection::Output => spec.outputs.first(),
                }?
                .ty;
                Some(Port {
                    name: graph.node_name(boundary)?.to_owned(),
                    ty,
                    node: boundary,
                    port: INTERFACE_PORT.to_owned(),
                })
            })
            .collect();
    }
    let Some(spec) = graph.node_spec(id) else {
        return Vec::new();
    };
    let list = match direction {
        PortDirection::Input => &spec.inputs,
        PortDirection::Output => &spec.outputs,
    };
    list.iter()
        .map(|p| Port {
            name: p.name.to_owned(),
            ty: p.ty,
            node: id,
            port: p.name.to_owned(),
        })
        .collect()
}

/// The shown form of `id`, or `None` if there is no such node.
#[must_use]
pub fn shown(graph: &Graph, id: NodeId) -> Option<Shown> {
    Some(Shown {
        id,
        name: graph.node_name(id)?.to_owned(),
        inputs: ports(graph, id, PortDirection::Input),
        outputs: ports(graph, id, PortDirection::Output),
    })
}

/// Every node of the root network, in id order.
#[must_use]
pub fn root_nodes(graph: &Graph) -> Vec<Shown> {
    graph
        .children(None)
        .into_iter()
        .filter_map(|id| shown(graph, id))
        .collect()
}

/// A link between shown nodes: `(node index, port index)` at both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ShownLink {
    /// The output.
    pub from: (usize, usize),
    /// The input.
    pub to: (usize, usize),
}

/// The links between the `nodes`. Links inside a subnet are not among them.
#[must_use]
pub fn links(graph: &Graph, nodes: &[Shown]) -> Vec<ShownLink> {
    let outputs: HashMap<(NodeId, &str), (usize, usize)> = nodes
        .iter()
        .enumerate()
        .flat_map(|(n, node)| {
            node.outputs
                .iter()
                .enumerate()
                .map(move |(p, port)| ((port.node, port.port.as_str()), (n, p)))
        })
        .collect();
    let mut out = Vec::new();
    for (n, node) in nodes.iter().enumerate() {
        for (p, port) in node.inputs.iter().enumerate() {
            if let Ok(Some((from, from_port))) = graph.link_source(port.node, &port.port)
                && let Some(&from) = outputs.get(&(from, from_port))
            {
                out.push(ShownLink { from, to: (n, p) });
            }
        }
    }
    out
}
