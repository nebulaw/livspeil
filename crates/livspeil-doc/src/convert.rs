// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Conversion between [`Document`] and [`Graph`].

use std::collections::BTreeMap;
use std::sync::Arc;

use livspeil_graph::{
    CharmDef, Graph, GraphError, INPUT_TYPE, INTERFACE_PORT, NamedLink, NodeId, NodeSnapshot,
    OUTPUT_TYPE, PortDirection, SUBNET_TYPE, instance_node_id,
};
use livspeil_node_api::{NodeRegistry, Value};

use crate::error::DocError;
use crate::model::{
    DocAnim, DocCharmDef, DocDefRef, DocExpr, DocLink, DocNode, Document, Network, SourceLoc,
};
use crate::value::is_finite;

impl Document {
    /// Capture the editable state of `graph` as a document named `name`.
    /// The result is in canonical order, has [`crate::CURRENT_FORMAT`],
    /// tier `T0` and no meta (graphs carry no editor positions yet).
    ///
    /// Subnets get a nested network (always `Some`, possibly empty). Each
    /// link is written in the network where both ends are visible: plainly
    /// between siblings, and as `subnet.port` for an end on a subnet's
    /// interface seen from the network around it.
    ///
    /// # Errors
    /// [`DocError::NonFinite`] if an input value is `NaN` or infinite.
    ///
    /// Charm definitions (M0.16) become the `charms` block; an instance is
    /// written as a `core.charm@1` node with `def="id@version"` and only
    /// the interface input sources that differ from the definition's
    /// defaults (its contents are not written).
    pub fn from_graph(graph: &Graph, name: impl Into<String>) -> Result<Document, DocError> {
        let snapshot = graph.snapshot();
        let mut charms = Vec::new();
        for def in &snapshot.definitions {
            charms.push(DocCharmDef {
                id: def.id.clone(),
                version: def.version.clone(),
                meta: def.meta.clone(),
                network: build_network(&def.nodes, &def.links, |_| true, plain_node)?,
                at: SourceLoc::default(),
            });
        }
        let inside = |n: &NodeSnapshot| graph.instance_holding(n.id).is_some();
        let network = build_network(
            &snapshot.nodes,
            &snapshot.links,
            |n| !inside(n),
            |n| match &n.def {
                Some(def) => instance_node(graph, n, def),
                None => plain_node(n),
            },
        )?;
        let mut doc = Document::new(name);
        doc.charms = charms;
        doc.network = network;
        Ok(doc)
    }

    /// Build a graph from the document, preserving node ids.
    ///
    /// Steps, in order: add every node (in document order, each subnet
    /// before the nodes inside it); set static inputs; set animation
    /// channels; make links; set expressions. So the graph applies every
    /// rule it enforces at edit time: known node types, unique names (per
    /// network) and ids, known ports, type checks (an `Int` value is
    /// accepted for a `Float` input and stored as a float), one link per
    /// input, network boundaries and no cycles. Expressions come last
    /// because their `ch()` paths name other nodes; a `ch()` reference that
    /// closes a cycle (with links or other expressions) is reported at the
    /// expression. Meta is not part of the graph.
    ///
    /// # Errors
    /// [`DocError::Graph`] naming the node, input, link or expression at
    /// fault, with its location when the document was parsed from text.
    pub fn to_graph(&self, registry: Arc<NodeRegistry>) -> Result<Graph, DocError> {
        let mut graph = Graph::new(Arc::clone(&registry));
        for def in &self.charms {
            let built = definition_to_graph(def, &registry)?;
            graph
                .add_definition(built)
                .map_err(|source| graph_err(def.at.0, format!("def '{}'", def.id), source))?;
        }
        load_network(&mut graph, None, &self.network)?;
        Ok(graph)
    }
}

/// Container of a definition's network while building it.
const SCRATCH: NodeId = NodeId::from_uuid(uuid::Uuid::max());

/// Build a definition by loading its network into an empty subnet of a
/// scratch graph, so every rule (and error location) of
/// [`Document::to_graph`] applies, then capturing it.
fn definition_to_graph(
    def: &DocCharmDef,
    registry: &Arc<NodeRegistry>,
) -> Result<CharmDef, DocError> {
    let context = || format!("def '{}'", def.id);
    let mut scratch = Graph::new(Arc::clone(registry));
    scratch
        .add_node_in(None, SCRATCH, SUBNET_TYPE, &def.id, None)
        .map_err(|source| graph_err(def.at.0, context(), source))?;
    scratch
        .set_charm(SCRATCH, Some(def.meta.clone()))
        .map_err(|source| graph_err(def.at.0, context(), source))?;
    load_network(&mut scratch, Some(SCRATCH), &def.network)?;
    scratch
        .definition_from_subnet(SCRATCH, &def.id, &def.version)
        .map_err(|source| graph_err(def.at.0, context(), source))
}

/// The node and port a source of input `port` of `node` is set on: the
/// port itself, or for a Charm instance the `value` input of its
/// interface input `port`.
fn source_target(graph: &Graph, node: &DocNode, port: &str) -> Result<(NodeId, String), DocError> {
    if !node.is_instance() {
        return Ok((node.id, port.to_owned()));
    }
    graph
        .interface_port(node.id, PortDirection::Input, port)
        .map(|id| (id, INTERFACE_PORT.to_owned()))
        .ok_or_else(|| {
            DocError::invalid(
                node.at.0,
                format!(
                    "node '{}': its definition has no interface input '{port}'",
                    node.name
                ),
            )
        })
}

/// Add `network` (and the networks inside it) to `graph` in the network of
/// `root`; see [`Document::to_graph`] for the steps.
fn load_network(
    graph: &mut Graph,
    root: Option<NodeId>,
    network: &Network,
) -> Result<(), DocError> {
    // Every network with the subnet holding it, parents first.
    let mut networks: Vec<(Option<NodeId>, &Network)> = vec![(root, network)];
    let mut index = 0;
    while let Some(&(parent, network)) = networks.get(index) {
        index += 1;
        for node in &network.nodes {
            let context = || format!("node '{}'", node.name);
            if node.is_instance() {
                let def = node.def.as_ref().ok_or_else(|| {
                    DocError::invalid(
                        node.at.0,
                        format!("{}: a Charm instance needs def=\"id@version\"", context()),
                    )
                })?;
                graph
                    .instantiate(parent, node.id, &node.name, &def.id)
                    .map_err(|source| graph_err(def.at.0.or(node.at.0), context(), source))?;
                continue;
            }
            graph
                .add_node_in(parent, node.id, &node.type_id, &node.name, node.port_type)
                .map_err(|source| graph_err(node.at.0, context(), source))?;
            if let Some(charm) = &node.charm {
                graph
                    .set_charm(node.id, Some(charm.clone()))
                    .map_err(|source| graph_err(node.at.0, context(), source))?;
            }
            match &node.network {
                Some(inner) if node.is_subnet() => networks.push((Some(node.id), inner)),
                Some(_) => {
                    return Err(graph_err(
                        node.at.0,
                        context(),
                        GraphError::NotASubnet(node.id),
                    ));
                }
                None => {}
            }
        }
    }
    let nodes = || networks.iter().flat_map(|(_, n)| &n.nodes);
    for node in nodes() {
        for (port, value) in &node.inputs {
            let (target, target_port) = source_target(graph, node, port)?;
            graph
                .set_input(target, &target_port, value.clone())
                .map_err(|source| {
                    graph_err(
                        node.at.0,
                        format!("node '{}' input '{port}'", node.name),
                        source,
                    )
                })?;
        }
    }
    for node in nodes() {
        for (port, anim) in &node.animations {
            let (target, target_port) = source_target(graph, node, port)?;
            graph
                .set_animation(target, &target_port, anim.channel.clone())
                .map_err(|source| {
                    graph_err(
                        anim.at.0.or(node.at.0),
                        format!("node '{}' input '{port}' anim", node.name),
                        source,
                    )
                })?;
        }
    }
    for &(parent, network) in &networks {
        for link in &network.links {
            let context = format!(
                "link {}.{} -> {}.{}",
                link.from_node, link.from_port, link.to_node, link.to_port
            );
            let endpoint = |name: &str,
                            port: &str,
                            direction: PortDirection|
             -> Result<(NodeId, String), DocError> {
                let id = graph
                    .find_in(parent, name)
                    .ok_or_else(|| DocError::Invalid {
                        at: link.at.0,
                        message: format!("{context}: no node named '{name}'"),
                    })?;
                if !graph.is_subnet(id) {
                    return Ok((id, port.to_owned()));
                }
                let iface =
                    graph
                        .interface_port(id, direction, port)
                        .ok_or_else(|| DocError::Invalid {
                            at: link.at.0,
                            message: format!(
                                "{context}: subnet '{name}' has no {direction} port '{port}'"
                            ),
                        })?;
                Ok((iface, INTERFACE_PORT.to_owned()))
            };
            let (from, from_port) =
                endpoint(&link.from_node, &link.from_port, PortDirection::Output)?;
            let (to, to_port) = endpoint(&link.to_node, &link.to_port, PortDirection::Input)?;
            graph
                .link(from, &from_port, to, &to_port)
                .map_err(|source| graph_err(link.at.0, context.clone(), source))?;
        }
    }
    for node in nodes() {
        for (port, expr) in &node.expressions {
            let (target, target_port) = source_target(graph, node, port)?;
            graph
                .set_expression(target, &target_port, &expr.source)
                .map_err(|source| {
                    graph_err(
                        expr.at.0.or(node.at.0),
                        format!("node '{}' input '{port}' expr", node.name),
                        source,
                    )
                })?;
        }
    }
    Ok(())
}

/// The document form of `nodes` (a whole graph, or a definition's
/// network) with `links`: the nodes for which `include` holds, made by
/// `make`, nested by parent, each link in the network where both of its
/// ends are visible, in canonical order.
fn build_network(
    nodes: &[NodeSnapshot],
    links: &[NamedLink],
    include: impl Fn(&NodeSnapshot) -> bool,
    make: impl Fn(&NodeSnapshot) -> Result<DocNode, DocError>,
) -> Result<Network, DocError> {
    let by_id: BTreeMap<NodeId, &NodeSnapshot> = nodes.iter().map(|n| (n.id, n)).collect();
    let mut doc_nodes: BTreeMap<NodeId, DocNode> = BTreeMap::new();
    for node in nodes.iter().filter(|n| include(n)) {
        doc_nodes.insert(node.id, make(node)?);
    }

    // Links, by the network they are written in.
    let mut doc_links: BTreeMap<Option<NodeId>, Vec<DocLink>> = BTreeMap::new();
    for link in links {
        let (Some(from), Some(to)) = (by_id.get(&link.from), by_id.get(&link.to)) else {
            continue; // A snapshot only links existing nodes.
        };
        let mut from_forms = vec![(from.parent, from.name.clone(), link.from_port.clone())];
        if from.type_id == OUTPUT_TYPE {
            if let Some(s) = from.parent.and_then(|s| by_id.get(&s)) {
                from_forms.push((s.parent, s.name.clone(), from.name.clone()));
            }
        }
        let to_form = if to.type_id == INPUT_TYPE {
            let s = to.parent.and_then(|s| by_id.get(&s));
            s.map(|s| (s.parent, s.name.clone(), to.name.clone()))
        } else {
            Some((to.parent, to.name.clone(), link.to_port.clone()))
        };
        let Some((net, to_node, to_port)) = to_form else {
            continue;
        };
        let Some((_, from_node, from_port)) = from_forms.into_iter().find(|(n, _, _)| *n == net)
        else {
            continue; // The graph only allows visible links.
        };
        doc_links
            .entry(net)
            .or_default()
            .push(DocLink::new(from_node, from_port, to_node, to_port));
    }

    // Assemble the tree, innermost subnets first.
    let mut by_parent: BTreeMap<Option<NodeId>, Vec<NodeId>> = BTreeMap::new();
    for node in nodes.iter().filter(|n| include(n)) {
        by_parent.entry(node.parent).or_default().push(node.id);
    }
    let mut order: Vec<NodeId> = Vec::new();
    let mut stack: Vec<Option<NodeId>> = vec![None];
    while let Some(net) = stack.pop() {
        for &child in by_parent.get(&net).into_iter().flatten() {
            order.push(child);
            stack.push(Some(child));
        }
    }
    let mut network = Network::default();
    for id in order.into_iter().rev() {
        let Some(mut doc_node) = doc_nodes.remove(&id) else {
            continue;
        };
        if let Some(inner) = &mut doc_node.network {
            inner.links = doc_links.remove(&Some(id)).unwrap_or_default();
        }
        let parent = by_id.get(&id).and_then(|n| n.parent);
        let target = match parent.and_then(|p| doc_nodes.get_mut(&p)) {
            Some(p) => p.network.get_or_insert_with(Network::default),
            None => &mut network,
        };
        target.nodes.push(doc_node);
    }
    network.links = doc_links.remove(&None).unwrap_or_default();
    network.sort();
    Ok(network)
}

/// A node with its sources (a subnet with an empty network to fill).
fn plain_node(node: &NodeSnapshot) -> Result<DocNode, DocError> {
    let mut doc_node = DocNode::new(node.id, node.name.clone(), node.type_id.clone());
    for (port, value) in &node.inputs {
        if !is_finite(value) {
            return Err(DocError::NonFinite {
                node: node.name.clone(),
                port: port.clone(),
            });
        }
        doc_node.inputs.insert(port.clone(), value.clone());
    }
    for (port, source) in &node.expressions {
        doc_node
            .expressions
            .insert(port.clone(), DocExpr::new(source.clone()));
    }
    for (port, channel) in &node.animations {
        // Channels are validated, so their numbers are finite.
        doc_node
            .animations
            .insert(port.clone(), DocAnim::new(channel.clone()));
    }
    doc_node.port_type = node.port_type;
    doc_node.charm.clone_from(&node.charm);
    if doc_node.is_subnet() {
        doc_node.network = Some(Network::default());
    }
    Ok(doc_node)
}

/// A Charm instance: its definition reference and the sources of its
/// interface inputs that differ from the definition's defaults, by
/// interface input name.
fn instance_node(graph: &Graph, node: &NodeSnapshot, def_id: &str) -> Result<DocNode, DocError> {
    let mut doc_node = DocNode::new(node.id, node.name.clone(), node.type_id.clone());
    let Some(def) = graph.definition(def_id) else {
        return Err(DocError::invalid(
            None,
            format!(
                "instance '{}' uses unknown definition '{def_id}'",
                node.name
            ),
        ));
    };
    doc_node.def = Some(DocDefRef::new(def.id.clone(), def.version.clone()));
    for t in def.interface_inputs() {
        let id = instance_node_id(node.id, t.id);
        let port = INTERFACE_PORT;
        let value = graph.input_value(id, port).ok().flatten();
        let default = find_value(&t.inputs, port);
        if value != default {
            if let Some(value) = value {
                if !is_finite(value) {
                    return Err(DocError::NonFinite {
                        node: node.name.clone(),
                        port: t.name.clone(),
                    });
                }
                doc_node.inputs.insert(t.name.clone(), value.clone());
            }
        }
        let expr = graph.expression(id, port).ok().flatten();
        let default = t
            .expressions
            .iter()
            .find(|(p, _)| p == port)
            .map(|(_, s)| s.as_str());
        if let (Some(source), true) = (expr, expr != default) {
            doc_node
                .expressions
                .insert(t.name.clone(), DocExpr::new(source));
        }
        let anim = graph.animation(id, port).ok().flatten();
        let default = t.animations.iter().find(|(p, _)| p == port).map(|(_, a)| a);
        if let (Some(channel), true) = (anim, anim != default) {
            doc_node
                .animations
                .insert(t.name.clone(), DocAnim::new(channel.clone()));
        }
    }
    Ok(doc_node)
}

fn find_value<'a>(list: &'a [(String, Value)], port: &str) -> Option<&'a Value> {
    list.iter().find(|(p, _)| p == port).map(|(_, v)| v)
}

fn graph_err(at: Option<crate::Location>, context: String, source: GraphError) -> DocError {
    DocError::Graph {
        at,
        context,
        source,
    }
}
