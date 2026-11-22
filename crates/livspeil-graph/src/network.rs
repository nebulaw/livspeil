// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Nested networks: subnets, Charm interfaces, promotion and moves
//! (milestone M0.7). See the crate documentation for the design.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use livspeil_node_api::{CookContext, NodeError, NodeImpl, NodeSpec, PortType, Value};
use livspeil_params::expr;

use crate::graph::{Graph, GraphError, NodeId, PortDirection};
use crate::sources::{Rewrite, render_path, resolve_path};

/// Qualified id of the subnet node type: a node that contains a network.
pub const SUBNET_TYPE: &str = "core.subnet@1";
/// Qualified id of a subnet's interface input (boundary) node.
pub const INPUT_TYPE: &str = "core.subnet.input@1";
/// Qualified id of a subnet's interface output (boundary) node; in the root
/// network it marks a sink.
pub const OUTPUT_TYPE: &str = "core.output@1";
/// Name of the input and output port of both boundary node types.
pub const INTERFACE_PORT: &str = "value";

/// Metadata that turns a subnet into a Charm, the unit of sharing (kernel
/// spec, section 13). It does not affect cooking.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct CharmMeta {
    /// Display name, for example `Daylight`.
    pub name: String,
    /// Version of the Charm, free-form (semver recommended).
    #[cfg_attr(feature = "serde", serde(default))]
    pub version: Option<String>,
    /// Icon name.
    #[cfg_attr(feature = "serde", serde(default))]
    pub icon: Option<String>,
    /// Capabilities the Charm needs (host I/O and the like), sorted.
    #[cfg_attr(feature = "serde", serde(default))]
    pub capabilities: Vec<String>,
    /// Minimum hardware tier (`T<n>`), 0 by default.
    #[cfg_attr(feature = "serde", serde(default))]
    pub min_tier: u8,
}

impl CharmMeta {
    /// Metadata with a name and nothing else.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        CharmMeta {
            name: name.into(),
            ..CharmMeta::default()
        }
    }
}

/// Per-subnet bookkeeping.
#[derive(Debug, Default)]
pub(crate) struct Subnet {
    /// Names of the nodes in the subnet's network.
    pub(crate) names: BTreeMap<String, NodeId>,
    pub(crate) charm: Option<CharmMeta>,
}

/// Whether `qualified_id` is one of the port-polymorphic boundary types.
pub(crate) fn is_boundary_type(qualified_id: &str) -> bool {
    qualified_id == INPUT_TYPE || qualified_id == OUTPUT_TYPE
}

/// Names used in paths (subnets and their contents) must be single path
/// segments.
pub(crate) fn check_name(name: &str, in_path: bool) -> Result<(), GraphError> {
    if name.is_empty() {
        return Err(GraphError::EmptyName);
    }
    if in_path && (name.contains('/') || name == "." || name == "..") {
        return Err(GraphError::InvalidName(name.to_owned()));
    }
    Ok(())
}

/// A pass-through node whose ports were retyped.
#[derive(Debug)]
struct Retyped {
    inner: Arc<dyn NodeImpl>,
    spec: NodeSpec,
}

impl NodeImpl for Retyped {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        self.inner.cook(ctx, inputs)
    }
}

/// `imp` with every port of type `ty` (defaults: the type's zero value).
/// Only used for the pass-through boundary types.
pub(crate) fn retype(imp: Arc<dyn NodeImpl>, ty: PortType) -> Arc<dyn NodeImpl> {
    let spec = imp.spec();
    if spec.inputs.iter().chain(&spec.outputs).all(|p| p.ty == ty) {
        return imp;
    }
    let mut spec = spec.clone();
    for port in &mut spec.inputs {
        port.ty = ty;
        port.default = Value::zero(ty);
    }
    for port in &mut spec.outputs {
        port.ty = ty;
    }
    Arc::new(Retyped { inner: imp, spec })
}

impl Graph {
    /// The subnet whose network holds `id`; `None` for a node of the root
    /// network (or an unknown node).
    #[must_use]
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).and_then(|n| n.parent)
    }

    /// Whether `id` is a subnet (a `core.subnet@1` node).
    #[must_use]
    pub fn is_subnet(&self, id: NodeId) -> bool {
        self.subnets.contains_key(&id)
    }

    /// The nodes of the network of subnet `network` (`None`: the root
    /// network), in id order. Empty for a node that is not a subnet.
    #[must_use]
    pub fn children(&self, network: Option<NodeId>) -> Vec<NodeId> {
        match network {
            None => self
                .node_ids()
                .filter(|&id| self.parent(id).is_none())
                .collect(),
            Some(s) => {
                let mut ids: Vec<NodeId> = self
                    .subnets
                    .get(&s)
                    .map(|sub| sub.names.values().copied().collect())
                    .unwrap_or_default();
                ids.sort_unstable();
                ids
            }
        }
    }

    /// The subnets enclosing network `network`, outermost first, `network`
    /// itself last (empty for the root).
    pub(crate) fn ancestors(&self, network: Option<NodeId>) -> Vec<NodeId> {
        let mut chain = Vec::new();
        let mut current = network;
        while let Some(s) = current {
            chain.push(s);
            current = self.parent(s);
        }
        chain.reverse();
        chain
    }

    /// The absolute path of a node, `/lamp/rig/bulb`.
    #[must_use]
    pub fn path(&self, id: NodeId) -> Option<String> {
        let name = self.node_name(id)?;
        let mut path = String::new();
        for s in self.ancestors(self.parent(id)) {
            path.push('/');
            path.push_str(self.node_name(s)?);
        }
        path.push('/');
        path.push_str(name);
        Some(path)
    }

    /// Find a node by path from the root network: `/lamp/rig/bulb` or
    /// `lamp/rig/bulb`. A root node whose name contains `/` is found by
    /// its whole name first (as [`Graph::find`] does).
    #[must_use]
    pub fn find_path(&self, path: &str) -> Option<NodeId> {
        let body = path.strip_prefix('/').unwrap_or(path);
        if let Some(id) = self.find(body) {
            return Some(id);
        }
        let mut current = None;
        let mut found = None;
        for segment in body.split('/') {
            if let Some(prev) = found {
                if !self.is_subnet(prev) {
                    return None;
                }
                current = Some(prev);
            }
            found = Some(self.find_in(current, segment)?);
        }
        found
    }

    /// The declared port type of a boundary node (`core.subnet.input@1`
    /// or `core.output@1`); `None` for other nodes.
    #[must_use]
    pub fn port_type(&self, id: NodeId) -> Option<PortType> {
        self.node(id).and_then(|n| n.port_type)
    }

    /// The interface of a subnet: its `core.subnet.input@1` (for
    /// [`PortDirection::Input`]) or `core.output@1` children, in id order.
    #[must_use]
    pub fn interface(&self, subnet: NodeId, direction: PortDirection) -> Vec<NodeId> {
        let ty = match direction {
            PortDirection::Input => INPUT_TYPE,
            PortDirection::Output => OUTPUT_TYPE,
        };
        if !self.is_subnet(subnet) {
            return Vec::new();
        }
        self.children(Some(subnet))
            .into_iter()
            .filter(|&c| self.node_type(c) == Some(ty))
            .collect()
    }

    /// The boundary node of a subnet's interface port called `name`.
    #[must_use]
    pub fn interface_port(
        &self,
        subnet: NodeId,
        direction: PortDirection,
        name: &str,
    ) -> Option<NodeId> {
        if !self.is_subnet(subnet) {
            return None;
        }
        let id = self.find_in(Some(subnet), name)?;
        let ty = match direction {
            PortDirection::Input => INPUT_TYPE,
            PortDirection::Output => OUTPUT_TYPE,
        };
        (self.node_type(id) == Some(ty)).then_some(id)
    }

    /// The Charm metadata of a subnet, if it is a Charm.
    #[must_use]
    pub fn charm(&self, subnet: NodeId) -> Option<&CharmMeta> {
        self.subnets.get(&subnet)?.charm.as_ref()
    }

    /// Set (or with `None` remove) the Charm metadata of a subnet. Returns
    /// the previous metadata. Does not invalidate cooked results.
    ///
    /// # Errors
    /// Fails for an unknown node or one that is not a subnet.
    pub fn set_charm(
        &mut self,
        subnet: NodeId,
        charm: Option<CharmMeta>,
    ) -> Result<Option<CharmMeta>, GraphError> {
        if !self.contains(subnet) {
            return Err(GraphError::UnknownNode(subnet));
        }
        let slot = self
            .charm_slot(subnet)
            .ok_or(GraphError::NotASubnet(subnet))?;
        Ok(std::mem::replace(slot, charm))
    }

    /// `id` and every node inside it (for a subnet), descendants before
    /// their subnet. Empty for an unknown node.
    pub(crate) fn subtree(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        if !self.contains(id) {
            return out;
        }
        // Iterative post-order.
        let mut stack = vec![(id, false)];
        while let Some((n, expanded)) = stack.pop() {
            if expanded {
                out.push(n);
                continue;
            }
            stack.push((n, true));
            if let Some(sub) = self.subnets.get(&n) {
                stack.extend(sub.names.values().map(|&c| (c, false)));
            }
        }
        out
    }

    /// Whether a link from an output of `from` to an input of `to` is
    /// allowed by the network rules, optionally pretending node
    /// `moved.0` were in network `moved.1`.
    ///
    /// An output is visible in its node's network, and an interface
    /// output's also in the network around its subnet. An input is visible
    /// in its node's network, except an interface input's, which is only
    /// visible in the network around its subnet.
    pub(crate) fn link_visible(
        &self,
        from: NodeId,
        to: NodeId,
        moved: Option<(NodeId, Option<NodeId>)>,
    ) -> bool {
        let parent = |n: NodeId| match moved {
            Some((m, p)) if m == n => p,
            _ => self.parent(n),
        };
        let to_net = if self.node_type(to) == Some(INPUT_TYPE) {
            match parent(to) {
                Some(s) => parent(s),
                None => return false,
            }
        } else {
            parent(to)
        };
        let from_net = parent(from);
        if from_net == to_net {
            return true;
        }
        self.node_type(from) == Some(OUTPUT_TYPE) && from_net.is_some_and(|s| parent(s) == to_net)
    }

    /// Move node `id` (a subnet with everything inside it) into the network
    /// of subnet `parent` (`None`: the root network). Returns the
    /// expressions whose `ch()` paths were re-rendered, as `(node, input
    /// port, previous source)`.
    ///
    /// Links are kept and must stay valid in the new place (see
    /// [`Graph::link`]): unlink what would cross networks first, or route
    /// it through the subnet's interface. `ch()` paths that would read
    /// another input after the move are re-rendered as the shortest
    /// relative path, so every expression reads exactly what it read
    /// before. Moving does not change what anything computes, so it does
    /// not invalidate cooked results.
    ///
    /// # Errors
    /// [`GraphError::UnknownNode`], [`GraphError::NotASubnet`],
    /// [`GraphError::WouldNest`] (into itself or a descendant),
    /// [`GraphError::NotInSubnet`] (an interface input to the root),
    /// [`GraphError::DuplicateName`] / [`GraphError::InvalidName`] in the
    /// new network, [`GraphError::CrossNetwork`] for a link that would
    /// become invalid, or an expression error when a path cannot be
    /// rendered. The graph is unchanged on error.
    pub fn move_node(
        &mut self,
        id: NodeId,
        parent: Option<NodeId>,
    ) -> Result<Vec<(NodeId, String, String)>, GraphError> {
        let node = self.node(id).ok_or(GraphError::UnknownNode(id))?;
        let (name, old_parent) = (node.name.clone(), node.parent);
        if old_parent == parent {
            return Ok(Vec::new());
        }
        if let Some(p) = parent {
            if !self.contains(p) {
                return Err(GraphError::UnknownNode(p));
            }
            if !self.is_subnet(p) {
                return Err(GraphError::NotASubnet(p));
            }
        }
        let subtree = self.subtree(id);
        if let Some(p) = parent {
            if subtree.contains(&p) {
                return Err(GraphError::WouldNest(id));
            }
        }
        if parent.is_none() && self.node_type(id) == Some(INPUT_TYPE) {
            return Err(GraphError::NotInSubnet(id));
        }
        check_name(&name, parent.is_some() || self.is_subnet(id))?;
        if self.find_in(parent, &name).is_some() {
            return Err(GraphError::DuplicateName(name));
        }
        let members: BTreeSet<NodeId> = subtree.iter().copied().collect();
        for link in self.links() {
            let (from, to) = (link.from.node, link.to.node);
            if (members.contains(&from) || members.contains(&to))
                && !self.link_visible(from, to, Some((id, parent)))
            {
                return Err(GraphError::CrossNetwork { from, to });
            }
        }

        // Expressions on the subtree and expressions reading it, with what
        // each path reads now.
        let mut affected: BTreeSet<(NodeId, usize)> = self.subtree_readers(id);
        for &n in &subtree {
            if let Some(node) = self.node(n) {
                for (index, e) in node.exprs.iter().enumerate() {
                    if e.is_some() {
                        affected.insert((n, index));
                    }
                }
            }
        }
        let mut before = Vec::new();
        for (reader, index) in affected {
            let Some(n) = self.node(reader) else {
                continue;
            };
            let Some(e) = n.exprs.get(index).and_then(Option::as_ref) else {
                continue;
            };
            let refs = expr::channel_refs(&e.source).unwrap_or_default();
            let reads: Vec<_> = refs
                .into_iter()
                .map(|r| {
                    let target = resolve_path(self, n.parent, &r.path).map(|x| (x.node, x.port));
                    (r, target)
                })
                .collect();
            before.push((reader, index, e.source.clone(), reads));
        }

        self.set_parent(id, parent);
        let mut rewrites: Vec<Rewrite> = Vec::new();
        let mut report = Vec::new();
        for (reader, index, source, reads) in before {
            let net = self.parent(reader);
            let mut out = source.clone();
            for (r, target) in reads.iter().rev() {
                let Some((node, port)) = target else {
                    continue;
                };
                let now = resolve_path(self, net, &r.path).map(|x| x.node);
                if now != Some(*node) {
                    let path = render_path(self, net, *node, port);
                    out.replace_range(r.span.start..r.span.end, &expr::quote(&path));
                }
            }
            if out != source {
                let port = self
                    .node(reader)
                    .and_then(|n| n.imp.spec().inputs.get(index))
                    .map_or_else(String::new, |p| p.name.to_owned());
                report.push((reader, port, source));
                rewrites.push((reader, index, out));
            }
        }
        if let Err(err) = self.apply_rewrites(rewrites) {
            self.set_parent(id, old_parent);
            return Err(err);
        }
        Ok(report)
    }

    /// Promote input `port` of `node` (inside a subnet) to an interface
    /// input of that subnet: add a `core.subnet.input@1` node `id` called
    /// `name` with the port's type, copy the port's static value (if set)
    /// to it, and link it to the port. The promoted parameter is then set
    /// on the subnet's interface (or linked from outside) and flows to the
    /// inner node; the inner node keeps its own static value underneath.
    ///
    /// # Errors
    /// [`GraphError::NotInSubnet`] for a node of the root network, the
    /// errors of [`Graph::add_node_in`] for the new node, and
    /// [`GraphError::InputOccupied`] when the port is linked. The graph is
    /// unchanged on error.
    pub fn promote(
        &mut self,
        node: NodeId,
        port: &str,
        id: NodeId,
        name: &str,
    ) -> Result<NodeId, GraphError> {
        let (_, ty) = self.input_port(node, port)?;
        let subnet = self.parent(node).ok_or(GraphError::NotInSubnet(node))?;
        if self.link_source(node, port)?.is_some() {
            return Err(GraphError::InputOccupied {
                node,
                port: port.to_owned(),
            });
        }
        let value = self.input_value(node, port)?.cloned();
        self.add_node_in(Some(subnet), id, INPUT_TYPE, name, Some(ty))?;
        let linked = value
            .map_or(Ok(()), |v| self.set_input(id, INTERFACE_PORT, v))
            .and_then(|()| self.link(id, INTERFACE_PORT, node, port));
        if let Err(err) = linked {
            self.remove_node(id)?;
            return Err(err);
        }
        Ok(id)
    }

    /// Undo a promotion of input `port` of `node`: unlink the interface
    /// input feeding it, copy the interface input's static value (if set)
    /// back to the port, and remove the interface input if it feeds nothing
    /// else (no other link, no `ch()` reader). Returns the interface input, or `None` when the port is not
    /// fed by one (then nothing changes).
    ///
    /// # Errors
    /// Fails for an unknown node or port.
    pub fn unpromote(&mut self, node: NodeId, port: &str) -> Result<Option<NodeId>, GraphError> {
        let Some((iface, _)) = self.link_source(node, port)? else {
            return Ok(None);
        };
        if self.node_type(iface) != Some(INPUT_TYPE) || self.parent(iface) != self.parent(node) {
            return Ok(None);
        }
        let value = self.input_value(iface, INTERFACE_PORT)?.cloned();
        self.unlink(node, port)?;
        if let Some(v) = value {
            self.set_input(node, port, v)?;
        }
        if self.outgoing_links(iface).is_empty() && self.referenced_by(iface).is_empty() {
            self.remove_node(iface)?;
        }
        Ok(Some(iface))
    }
}
