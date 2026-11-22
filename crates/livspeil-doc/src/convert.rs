// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Conversion between [`Document`] and [`Graph`].

use std::collections::BTreeMap;
use std::sync::Arc;

use livspeil_graph::{Graph, GraphError, NodeId};
use livspeil_node_api::NodeRegistry;

use crate::error::DocError;
use crate::model::{DocLink, DocNode, Document, Network};
use crate::value::is_finite;

impl Document {
    /// Capture the editable state of `graph` as a document named `name`.
    /// The result is in canonical order, has [`crate::CURRENT_FORMAT`],
    /// tier `T0` and no meta (graphs carry no editor positions yet).
    ///
    /// # Errors
    /// [`DocError::NonFinite`] if an input value is `NaN` or infinite.
    pub fn from_graph(graph: &Graph, name: impl Into<String>) -> Result<Document, DocError> {
        let snapshot = graph.snapshot();
        let names: BTreeMap<NodeId, &str> = snapshot
            .nodes
            .iter()
            .map(|n| (n.id, n.name.as_str()))
            .collect();
        let mut network = Network::default();
        for node in &snapshot.nodes {
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
            network.nodes.push(doc_node);
        }
        for link in &snapshot.links {
            let (Some(from), Some(to)) = (names.get(&link.from), names.get(&link.to)) else {
                continue; // A snapshot only links existing nodes.
            };
            network.links.push(DocLink::new(
                *from,
                link.from_port.clone(),
                *to,
                link.to_port.clone(),
            ));
        }
        network.sort();
        let mut doc = Document::new(name);
        doc.network = network;
        Ok(doc)
    }

    /// Build a graph from the document, preserving node ids.
    ///
    /// Nodes are added first (in document order), then inputs are set, then
    /// links are made, so the graph applies every rule it enforces at edit
    /// time: known node types, unique names and ids, known ports, type
    /// checks (an `Int` value is accepted for a `Float` input and stored as
    /// a float), one link per input and no cycles. Meta is not part of the
    /// graph.
    ///
    /// # Errors
    /// [`DocError::Graph`] naming the node, input or link at fault, with
    /// its location when the document was parsed from text.
    pub fn to_graph(&self, registry: Arc<NodeRegistry>) -> Result<Graph, DocError> {
        let mut graph = Graph::new(registry);
        let mut ids = BTreeMap::new();
        for node in &self.network.nodes {
            let context = || format!("node '{}'", node.name);
            graph
                .add_node_with_id(node.id, &node.type_id, &node.name)
                .map_err(|source| graph_err(node.at.0, context(), source))?;
            ids.insert(node.name.as_str(), node.id);
        }
        for node in &self.network.nodes {
            for (port, value) in &node.inputs {
                graph
                    .set_input(node.id, port, value.clone())
                    .map_err(|source| {
                        graph_err(
                            node.at.0,
                            format!("node '{}' input '{port}'", node.name),
                            source,
                        )
                    })?;
            }
        }
        for link in &self.network.links {
            let context = format!(
                "link {}.{} -> {}.{}",
                link.from_node, link.from_port, link.to_node, link.to_port
            );
            let lookup = |name: &str| {
                ids.get(name).copied().ok_or_else(|| DocError::Invalid {
                    at: link.at.0,
                    message: format!("{context}: no node named '{name}'"),
                })
            };
            let from = lookup(&link.from_node)?;
            let to = lookup(&link.to_node)?;
            graph
                .link(from, &link.from_port, to, &link.to_port)
                .map_err(|source| graph_err(link.at.0, context.clone(), source))?;
        }
        Ok(graph)
    }
}

fn graph_err(at: Option<crate::Location>, context: String, source: GraphError) -> DocError {
    DocError::Graph {
        at,
        context,
        source,
    }
}
