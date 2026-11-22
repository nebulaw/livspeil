// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The editable graph: nodes, links and the rules they must obey.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;

use livspeil_node_api::{NodeImpl, NodeRegistry, NodeSpec, PortType, Value};
use livspeil_params::AnimChannel;
use livspeil_params::expr::ExprError;
use uuid::Uuid;

use crate::snapshot::{GraphSnapshot, NamedLink, NodeSnapshot};
use crate::sources::{Expression, InputRef};

/// Stable identifier of a node (UUIDv7, so ids sort by creation time).
///
/// With the `serde` feature it serializes as its UUID string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(transparent)
)]
pub struct NodeId(Uuid);

impl NodeId {
    /// The smallest id (the nil UUID); handy for range queries.
    pub(crate) const MIN: NodeId = NodeId(Uuid::nil());

    /// A fresh, time-ordered id.
    #[must_use]
    pub fn new() -> Self {
        NodeId(Uuid::now_v7())
    }

    /// Wrap an existing UUID (used when loading documents and in tests).
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        NodeId(uuid)
    }

    /// The underlying UUID.
    #[must_use]
    pub const fn uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A port on a specific node, by index into the node spec's port list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Endpoint {
    pub node: NodeId,
    pub port: usize,
}

/// A connection from an output port to an input port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Link {
    pub from: Endpoint,
    pub to: Endpoint,
}

/// Which side of a node a port is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortDirection {
    Input,
    Output,
}

impl fmt::Display for PortDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PortDirection::Input => "input",
            PortDirection::Output => "output",
        })
    }
}

/// Why an edit to the graph was rejected. The graph is unchanged on error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    /// No node type with this qualified id is registered.
    UnknownNodeType(String),
    /// The name is empty.
    EmptyName,
    /// Another node already has this name.
    DuplicateName(String),
    /// Another node already has this id.
    DuplicateId(NodeId),
    /// No node with this id.
    UnknownNode(NodeId),
    /// The node has no port with this name.
    UnknownPort {
        node: NodeId,
        port: String,
        direction: PortDirection,
    },
    /// The value or link has the wrong type for the input port.
    TypeMismatch { expected: PortType, found: PortType },
    /// The input port already has a link; unlink it first.
    InputOccupied { node: NodeId, port: String },
    /// The link or `ch()` reference would make a node depend on itself.
    WouldCycle,
    /// An expression failed to parse, resolve its `ch()` paths or type
    /// check.
    Expression(ExprError),
    /// The node cannot be removed: input `port` of node `by` reads it
    /// through `ch()`.
    Referenced {
        node: NodeId,
        by: NodeId,
        port: String,
    },
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphError::UnknownNodeType(id) => write!(f, "unknown node type '{id}'"),
            GraphError::EmptyName => write!(f, "node names must not be empty"),
            GraphError::DuplicateName(name) => write!(f, "a node named '{name}' already exists"),
            GraphError::DuplicateId(id) => write!(f, "a node with id {id} already exists"),
            GraphError::UnknownNode(id) => write!(f, "no node with id {id}"),
            GraphError::UnknownPort {
                node,
                port,
                direction,
            } => write!(f, "node {node} has no {direction} port '{port}'"),
            GraphError::TypeMismatch { expected, found } => {
                write!(f, "type mismatch: expected {expected}, found {found}")
            }
            GraphError::InputOccupied { node, port } => {
                write!(f, "input '{port}' of node {node} is already linked")
            }
            GraphError::WouldCycle => write!(f, "the link would create a cycle"),
            GraphError::Expression(err) => write!(f, "expression error: {err}"),
            GraphError::Referenced { node, by, port } => write!(
                f,
                "node {node} is read by ch() in input '{port}' of node {by}"
            ),
        }
    }
}

impl std::error::Error for GraphError {}

pub(crate) struct Node {
    pub(crate) name: String,
    pub(crate) type_id: String,
    pub(crate) imp: Arc<dyn NodeImpl>,
    /// Explicitly set input values, by input port index. `None` means "use
    /// the spec default".
    pub(crate) values: Vec<Option<Value>>,
    /// Expressions, by input port index.
    pub(crate) exprs: Vec<Option<Expression>>,
    /// Animation channels, by input port index.
    pub(crate) anims: Vec<Option<AnimChannel>>,
    /// Bumped whenever this node's inputs or links change.
    pub(crate) rev: u64,
}

impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Node")
            .field("name", &self.name)
            .field("type_id", &self.type_id)
            .field("rev", &self.rev)
            .finish_non_exhaustive()
    }
}

/// A network of typed nodes (kernel spec, section 3).
#[derive(Debug)]
pub struct Graph {
    id: Uuid,
    registry: Arc<NodeRegistry>,
    pub(crate) nodes: BTreeMap<NodeId, Node>,
    /// Links keyed by their target input, which enforces one link per input.
    links: BTreeMap<(NodeId, usize), Endpoint>,
    /// The same links as `(source node, target node, target port)`, for fast
    /// downstream traversal.
    outgoing: BTreeSet<(NodeId, NodeId, usize)>,
    names: HashMap<String, NodeId>,
    /// `ch()` references as `(read node, reading node, reading input)`.
    pub(crate) ch_edges: BTreeSet<(NodeId, NodeId, usize)>,
    rev_counter: u64,
}

impl Graph {
    /// An empty graph that instantiates nodes from `registry`.
    #[must_use]
    pub fn new(registry: Arc<NodeRegistry>) -> Self {
        Graph {
            id: Uuid::now_v7(),
            registry,
            nodes: BTreeMap::new(),
            links: BTreeMap::new(),
            outgoing: BTreeSet::new(),
            names: HashMap::new(),
            ch_edges: BTreeSet::new(),
            rev_counter: 0,
        }
    }

    /// Identity of this graph instance (lets a cooker detect a different
    /// graph and drop stale caches).
    pub(crate) fn uid(&self) -> Uuid {
        self.id
    }

    /// Add a node of type `qualified_id` (for example `core.math.add@1`).
    ///
    /// # Errors
    /// Fails for an unknown node type, an empty name or a duplicate name.
    pub fn add_node(&mut self, qualified_id: &str, name: &str) -> Result<NodeId, GraphError> {
        self.add_node_with_id(NodeId::new(), qualified_id, name)
    }

    /// Like [`Graph::add_node`] with a caller-chosen id (document loading,
    /// deterministic tests).
    ///
    /// # Errors
    /// Same as [`Graph::add_node`], plus [`GraphError::DuplicateId`].
    pub fn add_node_with_id(
        &mut self,
        id: NodeId,
        qualified_id: &str,
        name: &str,
    ) -> Result<NodeId, GraphError> {
        if name.is_empty() {
            return Err(GraphError::EmptyName);
        }
        if self.names.contains_key(name) {
            return Err(GraphError::DuplicateName(name.to_owned()));
        }
        if self.nodes.contains_key(&id) {
            return Err(GraphError::DuplicateId(id));
        }
        let imp = self
            .registry
            .get(qualified_id)
            .ok_or_else(|| GraphError::UnknownNodeType(qualified_id.to_owned()))?
            .clone();
        let inputs = imp.spec().inputs.len();
        let values = vec![None; inputs];
        self.rev_counter += 1;
        self.nodes.insert(
            id,
            Node {
                name: name.to_owned(),
                type_id: qualified_id.to_owned(),
                imp,
                values,
                exprs: (0..inputs).map(|_| None).collect(),
                anims: vec![None; inputs],
                rev: self.rev_counter,
            },
        );
        self.names.insert(name.to_owned(), id);
        Ok(id)
    }

    /// Remove a node and every link attached to it. Inputs that were fed by
    /// the node fall back to their own sources, values or defaults.
    ///
    /// # Errors
    /// Fails if the node does not exist, or with [`GraphError::Referenced`]
    /// if another node's expression reads one of its inputs.
    pub fn remove_node(&mut self, id: NodeId) -> Result<(), GraphError> {
        if !self.nodes.contains_key(&id) {
            return Err(GraphError::UnknownNode(id));
        }
        if let Some((by, port)) = self.referenced_by(id).into_iter().next() {
            return Err(GraphError::Referenced { node: id, by, port });
        }
        let inputs = self.nodes.get(&id).map_or(0, |n| n.exprs.len());
        for index in 0..inputs {
            self.unindex_expression(id, index);
        }
        let node = self.nodes.remove(&id).ok_or(GraphError::UnknownNode(id))?;
        self.names.remove(&node.name);

        // Links into the removed node.
        let incoming: Vec<(NodeId, usize)> = self
            .links
            .range((id, 0)..=(id, usize::MAX))
            .map(|(key, _)| *key)
            .collect();
        for key in incoming {
            if let Some(src) = self.links.remove(&key) {
                self.outgoing.remove(&(src.node, key.0, key.1));
            }
        }

        // Links out of the removed node: their targets change.
        let outgoing: Vec<(NodeId, NodeId, usize)> = self
            .outgoing
            .range((id, NodeId(Uuid::nil()), 0)..)
            .take_while(|(from, _, _)| *from == id)
            .copied()
            .collect();
        for (from, to, port) in outgoing {
            self.outgoing.remove(&(from, to, port));
            self.links.remove(&(to, port));
            self.bump(to);
        }
        Ok(())
    }

    /// Set the value of an unlinked input. A link, if present, still wins.
    /// `Int` values are accepted for `Float` ports.
    ///
    /// # Errors
    /// Fails for an unknown node or port, or a value of the wrong type.
    pub fn set_input(&mut self, node: NodeId, port: &str, value: Value) -> Result<(), GraphError> {
        let (index, ty) = self.input_port(node, port)?;
        let found = value.port_type();
        let value = value.coerce(ty).ok_or(GraphError::TypeMismatch {
            expected: ty,
            found,
        })?;
        if let Some(n) = self.nodes.get_mut(&node) {
            n.values[index] = Some(value);
        }
        self.bump(node);
        Ok(())
    }

    /// Forget an explicitly set input value so the spec default applies.
    ///
    /// # Errors
    /// Fails for an unknown node or port.
    pub fn clear_input(&mut self, node: NodeId, port: &str) -> Result<(), GraphError> {
        let (index, _) = self.input_port(node, port)?;
        if let Some(n) = self.nodes.get_mut(&node) {
            n.values[index] = None;
        }
        self.bump(node);
        Ok(())
    }

    /// Connect `from.from_port` (an output) to `to.to_port` (an input).
    ///
    /// # Errors
    /// Fails for unknown nodes or ports, incompatible types, an input that
    /// already has a link, or a link that would create a cycle.
    pub fn link(
        &mut self,
        from: NodeId,
        from_port: &str,
        to: NodeId,
        to_port: &str,
    ) -> Result<(), GraphError> {
        let (out_index, out_ty) = self.output_port(from, from_port)?;
        let (in_index, in_ty) = self.input_port(to, to_port)?;
        if !in_ty.accepts(out_ty) {
            return Err(GraphError::TypeMismatch {
                expected: in_ty,
                found: out_ty,
            });
        }
        if self.links.contains_key(&(to, in_index)) {
            return Err(GraphError::InputOccupied {
                node: to,
                port: to_port.to_owned(),
            });
        }
        if self.depends_on(from, to) {
            return Err(GraphError::WouldCycle);
        }
        self.links.insert(
            (to, in_index),
            Endpoint {
                node: from,
                port: out_index,
            },
        );
        self.outgoing.insert((from, to, in_index));
        self.bump(to);
        Ok(())
    }

    /// Remove the link feeding `to.to_port`, if any. Returns whether a link
    /// was removed.
    ///
    /// # Errors
    /// Fails for an unknown node or port.
    pub fn unlink(&mut self, to: NodeId, to_port: &str) -> Result<bool, GraphError> {
        let (index, _) = self.input_port(to, to_port)?;
        let Some(src) = self.links.remove(&(to, index)) else {
            return Ok(false);
        };
        self.outgoing.remove(&(src.node, to, index));
        self.bump(to);
        Ok(true)
    }

    /// Rename a node. Returns the previous name. Renaming does not change
    /// what a node computes, so it does not invalidate cooked results.
    /// Renaming a node to its current name is a no-op.
    ///
    /// Every `ch()` path that reads the node is rewritten to the new name
    /// (see [`Graph::set_expression`]).
    ///
    /// # Errors
    /// Fails for an unknown node, an empty name or a name used by another
    /// node.
    pub fn rename(&mut self, id: NodeId, name: &str) -> Result<String, GraphError> {
        if name.is_empty() {
            return Err(GraphError::EmptyName);
        }
        let node = self.nodes.get_mut(&id).ok_or(GraphError::UnknownNode(id))?;
        if node.name == name {
            return Ok(name.to_owned());
        }
        if self.names.contains_key(name) {
            return Err(GraphError::DuplicateName(name.to_owned()));
        }
        let old = std::mem::replace(&mut node.name, name.to_owned());
        self.names.remove(&old);
        self.names.insert(name.to_owned(), id);
        if let Err(err) = self.rewrite_references(id, &old, name) {
            // Unreachable in practice; restore the old name.
            self.names.remove(name);
            self.names.insert(old.clone(), id);
            if let Some(node) = self.nodes.get_mut(&id) {
                node.name = old;
            }
            return Err(err);
        }
        Ok(old)
    }

    /// Whether a node with this id exists.
    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    /// The explicitly set value of an input, or `None` when the input uses
    /// its spec default. Links are not considered.
    ///
    /// # Errors
    /// Fails for an unknown node or port.
    pub fn input_value(&self, node: NodeId, port: &str) -> Result<Option<&Value>, GraphError> {
        let (index, _) = self.input_port(node, port)?;
        Ok(self
            .nodes
            .get(&node)
            .and_then(|n| n.values.get(index))
            .and_then(Option::as_ref))
    }

    /// Every explicitly set input value of a node as `(port name, value)`,
    /// in port order. Empty for an unknown node.
    pub fn input_values(&self, node: NodeId) -> impl Iterator<Item = (&str, &Value)> + '_ {
        self.nodes.get(&node).into_iter().flat_map(|n| {
            let spec = n.imp.spec();
            n.values
                .iter()
                .zip(&spec.inputs)
                .filter_map(|(value, port)| value.as_ref().map(|v| (port.name, v)))
        })
    }

    /// The output feeding input `to_port` of node `to`, as
    /// `(source node, output port name)`, or `None` when it is unlinked.
    ///
    /// # Errors
    /// Fails for an unknown node or port.
    pub fn link_source(
        &self,
        to: NodeId,
        to_port: &str,
    ) -> Result<Option<(NodeId, &str)>, GraphError> {
        let (index, _) = self.input_port(to, to_port)?;
        Ok(self.links.get(&(to, index)).and_then(|src| {
            let spec = self.nodes.get(&src.node)?.imp.spec();
            Some((src.node, spec.outputs.get(src.port)?.name))
        }))
    }

    /// The links feeding a node, by input port order, with port names.
    #[must_use]
    pub fn incoming_links(&self, id: NodeId) -> Vec<NamedLink> {
        self.links
            .range((id, 0)..=(id, usize::MAX))
            .filter_map(|(&(node, port), &from)| {
                self.named(Link {
                    from,
                    to: Endpoint { node, port },
                })
            })
            .collect()
    }

    /// The links leaving a node, ordered by target node and input port,
    /// with port names.
    #[must_use]
    pub fn outgoing_links(&self, id: NodeId) -> Vec<NamedLink> {
        self.outgoing
            .range((id, NodeId(Uuid::nil()), 0)..)
            .take_while(|(from, _, _)| *from == id)
            .filter_map(|&(_, to, port)| {
                let from = *self.links.get(&(to, port))?;
                self.named(Link {
                    from,
                    to: Endpoint { node: to, port },
                })
            })
            .collect()
    }

    /// A link with port names instead of indices.
    #[must_use]
    pub fn named(&self, link: Link) -> Option<NamedLink> {
        let from_spec = self.nodes.get(&link.from.node)?.imp.spec();
        let to_spec = self.nodes.get(&link.to.node)?.imp.spec();
        Some(NamedLink {
            from: link.from.node,
            from_port: from_spec.outputs.get(link.from.port)?.name.to_owned(),
            to: link.to.node,
            to_port: to_spec.inputs.get(link.to.port)?.name.to_owned(),
        })
    }

    /// A canonical, comparable copy of the editable state: node ids, names,
    /// types, explicitly set inputs, expressions, animation channels and
    /// links. Cook revisions and the graph
    /// instance id are not part of it, so two graphs built by the same
    /// edits have equal snapshots.
    #[must_use]
    pub fn snapshot(&self) -> GraphSnapshot {
        let nodes = self
            .nodes
            .iter()
            .map(|(&id, n)| NodeSnapshot {
                id,
                name: n.name.clone(),
                type_id: n.type_id.clone(),
                inputs: self
                    .input_values(id)
                    .map(|(port, value)| (port.to_owned(), value.clone()))
                    .collect(),
                expressions: self
                    .expressions(id)
                    .map(|(port, source)| (port.to_owned(), source.to_owned()))
                    .collect(),
                animations: self
                    .animations(id)
                    .map(|(port, channel)| (port.to_owned(), channel.clone()))
                    .collect(),
            })
            .collect();
        let links = self.links().filter_map(|l| self.named(l)).collect();
        GraphSnapshot { nodes, links }
    }

    /// Number of nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Number of links.
    #[must_use]
    pub fn link_count(&self) -> usize {
        self.links.len()
    }

    /// All node ids in id order.
    pub fn node_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes.keys().copied()
    }

    /// The name of a node.
    #[must_use]
    pub fn node_name(&self, id: NodeId) -> Option<&str> {
        self.nodes.get(&id).map(|n| n.name.as_str())
    }

    /// The qualified type id of a node.
    #[must_use]
    pub fn node_type(&self, id: NodeId) -> Option<&str> {
        self.nodes.get(&id).map(|n| n.type_id.as_str())
    }

    /// The spec of a node's type.
    #[must_use]
    pub fn node_spec(&self, id: NodeId) -> Option<&NodeSpec> {
        self.nodes.get(&id).map(|n| n.imp.spec())
    }

    /// Find a node by name.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<NodeId> {
        self.names.get(name).copied()
    }

    /// All links, ordered by target node and input port.
    pub fn links(&self) -> impl Iterator<Item = Link> + '_ {
        self.links.iter().map(|(&(node, port), &from)| Link {
            from,
            to: Endpoint { node, port },
        })
    }

    /// The links feeding a node, as `(input port index, source endpoint)`.
    pub(crate) fn upstream(&self, id: NodeId) -> impl Iterator<Item = (usize, Endpoint)> + '_ {
        self.links
            .range((id, 0)..=(id, usize::MAX))
            .map(|(&(_, port), &src)| (port, src))
    }

    /// Every node `id` directly depends on (link sources and nodes read by
    /// `ch()`), possibly with repeats.
    pub(crate) fn dependencies(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        let reads = self.nodes.get(&id).into_iter().flat_map(|n| {
            n.exprs
                .iter()
                .flatten()
                .flat_map(|e| e.targets.iter().map(|t: &InputRef| t.node))
        });
        self.upstream(id).map(|(_, src)| src.node).chain(reads)
    }

    pub(crate) fn bump(&mut self, id: NodeId) {
        self.rev_counter += 1;
        if let Some(node) = self.nodes.get_mut(&id) {
            node.rev = self.rev_counter;
        }
    }

    pub(crate) fn input_port(
        &self,
        node: NodeId,
        port: &str,
    ) -> Result<(usize, PortType), GraphError> {
        let n = self.nodes.get(&node).ok_or(GraphError::UnknownNode(node))?;
        let spec = n.imp.spec();
        spec.input_index(port)
            .map(|i| (i, spec.inputs[i].ty))
            .ok_or_else(|| GraphError::UnknownPort {
                node,
                port: port.to_owned(),
                direction: PortDirection::Input,
            })
    }

    fn output_port(&self, node: NodeId, port: &str) -> Result<(usize, PortType), GraphError> {
        let n = self.nodes.get(&node).ok_or(GraphError::UnknownNode(node))?;
        let spec = n.imp.spec();
        spec.output_index(port)
            .map(|i| (i, spec.outputs[i].ty))
            .ok_or_else(|| GraphError::UnknownPort {
                node,
                port: port.to_owned(),
                direction: PortDirection::Output,
            })
    }

    /// Whether `from` is `to` or already depends on `to`, i.e. whether
    /// adding the link (or `ch()` reference) `from -> to` would close a
    /// cycle. Walks downstream from `to` through links and `ch()`
    /// references, which is cheap when `to` is new.
    pub(crate) fn depends_on(&self, from: NodeId, to: NodeId) -> bool {
        if from == to {
            return true;
        }
        let mut seen = BTreeSet::new();
        let mut stack = vec![to];
        while let Some(current) = stack.pop() {
            if !seen.insert(current) {
                continue;
            }
            for &(_, next, _) in self
                .outgoing
                .range((current, NodeId(Uuid::nil()), 0)..)
                .take_while(|(f, _, _)| *f == current)
            {
                if next == from {
                    return true;
                }
                stack.push(next);
            }
            for (next, _) in self.ch_dependents(current) {
                if next == from {
                    return true;
                }
                stack.push(next);
            }
        }
        false
    }
}
