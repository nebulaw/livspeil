// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Moving a node between networks with automatic rewiring (M0.16).
//!
//! [`Graph::move_node`] refuses a move that would leave a link crossing
//! networks other than through a subnet's interface. A *rewired* move
//! ([`Graph::move_node_rewired`]) keeps the data flow instead: every such
//! link is replaced by a chain through new interface ports. From the
//! source, the chain climbs out of each subnet up to the networks' common
//! ancestor through new interface *outputs* (`core.output@1`), then
//! descends into each subnet down to the target's network through new
//! interface *inputs* (`core.subnet.input@1`), all typed like the source
//! output. A source that is already an interface output skips its own
//! subnet. Ports are shared when several links leave the same output
//! through the same subnet.
//!
//! New ports are named `<source node>_<output port>` (`.` and `/`
//! replaced by `_`), with `_2`, `_3`, ... appended when the name is
//! taken; their ids are derived from a caller-chosen `seed`, so a journal
//! replays to the same ids. [`Graph::plan_rewired_move`] computes the
//! changes without making them: the links to remove, the ports to add and
//! the links to add, in order. The command bus applies a rewired move as
//! a transaction of those primitive edits, so its inverse restores the
//! exact previous structure. Event links are never rewired.

use std::collections::{BTreeMap, BTreeSet};

use livspeil_node_api::PortType;
use uuid::Uuid;

use crate::charm_defs::instance_node_id;
use crate::graph::{Graph, GraphError, NodeId, PortDirection};
use crate::network::{INPUT_TYPE, INTERFACE_PORT, OUTPUT_TYPE};
use crate::snapshot::NamedLink;

/// An interface port a rewired move adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPort {
    /// Id of the new boundary node.
    pub id: NodeId,
    /// The subnet that gets the port.
    pub subnet: NodeId,
    pub direction: PortDirection,
    /// Name of the port (and of its boundary node).
    pub name: String,
    pub port_type: PortType,
}

/// The edits of a rewired move, in order: remove `unlink`, move the node,
/// add `ports`, add `link`. See the [module docs](self).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RewirePlan {
    /// Links that would cross networks after the move.
    pub unlink: Vec<NamedLink>,
    /// New interface ports.
    pub ports: Vec<NewPort>,
    /// Links that replace them, through the new ports.
    pub link: Vec<NamedLink>,
}

impl RewirePlan {
    /// Whether the move needs no rewiring.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.unlink.is_empty()
    }
}

/// Expressions whose paths a move re-rendered: `(node, input port,
/// previous source)`.
type Rewritten = Vec<(NodeId, String, String)>;

/// The id of the `index`-th port a rewired move with `seed` creates.
fn port_id(seed: NodeId, index: usize) -> NodeId {
    instance_node_id(seed, NodeId::from_uuid(Uuid::from_u128(index as u128)))
}

fn port_name(node: &str, port: &str) -> String {
    format!("{node}_{port}")
        .chars()
        .map(|c| if c == '.' || c == '/' { '_' } else { c })
        .collect()
}

impl Graph {
    /// The edits that moving `id` into the network of `parent` with
    /// rewiring needs (see the [module docs](self)). Empty when the move
    /// needs no rewiring (or `id` is already there). The move itself is
    /// not checked here; [`Graph::move_node_rewired`] does.
    ///
    /// # Errors
    /// [`GraphError::UnknownNode`] for `id` or `parent`, and
    /// [`GraphError::CrossNetwork`] for an `Event` link that would cross
    /// networks.
    pub fn plan_rewired_move(
        &self,
        id: NodeId,
        parent: Option<NodeId>,
        seed: NodeId,
    ) -> Result<RewirePlan, GraphError> {
        if !self.contains(id) {
            return Err(GraphError::UnknownNode(id));
        }
        if let Some(p) = parent
            && !self.contains(p)
        {
            return Err(GraphError::UnknownNode(p));
        }
        let mut plan = RewirePlan::default();
        if self.parent(id) == parent {
            return Ok(plan);
        }
        let members: BTreeSet<NodeId> = self.subtree(id).into_iter().collect();
        if parent.is_some_and(|p| members.contains(&p)) {
            return Ok(plan); // Refused by the move itself.
        }
        let par = |n: NodeId| if n == id { parent } else { self.parent(n) };
        // Networks from the root down to `net` (inclusive), after the move.
        let chain = |net: Option<NodeId>| {
            let mut out = Vec::new();
            let mut current = net;
            while let Some(s) = current {
                out.push(s);
                current = par(s);
            }
            out.reverse();
            out
        };
        // Names taken per subnet by the plan so far.
        let mut taken: BTreeMap<NodeId, BTreeSet<String>> = BTreeMap::new();
        // (subnet, direction, source) -> the port carrying that source.
        let mut shared: BTreeMap<(NodeId, PortDirection, NodeId, String), NodeId> = BTreeMap::new();
        for link in self.links() {
            let (from, to) = (link.from.node, link.to.node);
            if members.contains(&from) == members.contains(&to)
                || self.link_visible(from, to, Some((id, parent)))
            {
                continue;
            }
            let Some(named) = self.named(link) else {
                continue;
            };
            let ty = self
                .node_spec(from)
                .and_then(|s| s.outputs.get(link.from.port))
                .map_or(PortType::Float, |p| p.ty);
            if ty == PortType::Event {
                return Err(GraphError::CrossNetwork { from, to });
            }
            let base = port_name(self.node_name(from).unwrap_or_default(), &named.from_port);
            let src_chain = chain(par(from));
            let to_net = if self.node_type(to) == Some(INPUT_TYPE) {
                par(to).and_then(par)
            } else {
                par(to)
            };
            let dst_chain = chain(to_net);
            let common = src_chain
                .iter()
                .zip(&dst_chain)
                .take_while(|(a, b)| a == b)
                .count();
            let mut current = (from, named.from_port.clone());
            let mut ups: Vec<NodeId> = src_chain[common..].iter().rev().copied().collect();
            if self.node_type(from) == Some(OUTPUT_TYPE) && !ups.is_empty() {
                ups.remove(0); // Its value is visible around its subnet.
            }
            let steps = ups.into_iter().map(|s| (s, PortDirection::Output)).chain(
                dst_chain[common..]
                    .iter()
                    .map(|&s| (s, PortDirection::Input)),
            );
            for (subnet, direction) in steps {
                let key = (subnet, direction, current.0, current.1.clone());
                if let Some(&port) = shared.get(&key) {
                    current = (port, INTERFACE_PORT.to_owned());
                    continue;
                }
                let names = taken.entry(subnet).or_default();
                let mut name = base.clone();
                let mut n = 1;
                while names.contains(&name) || self.find_in(Some(subnet), &name).is_some() {
                    n += 1;
                    name = format!("{base}_{n}");
                }
                names.insert(name.clone());
                let port = port_id(seed, plan.ports.len());
                plan.ports.push(NewPort {
                    id: port,
                    subnet,
                    direction,
                    name,
                    port_type: ty,
                });
                plan.link.push(NamedLink {
                    from: current.0,
                    from_port: current.1.clone(),
                    to: port,
                    to_port: INTERFACE_PORT.to_owned(),
                });
                shared.insert(key, port);
                current = (port, INTERFACE_PORT.to_owned());
            }
            plan.link.push(NamedLink {
                from: current.0,
                from_port: current.1,
                to,
                to_port: named.to_port.clone(),
            });
            plan.unlink.push(named);
        }
        Ok(plan)
    }

    /// Move `id` into the network of `parent` like [`Graph::move_node`],
    /// but rewire links that would cross networks through new interface
    /// ports instead of refusing (see the [module docs](self)). Returns the
    /// plan that was applied and the re-rendered expressions (as
    /// `move_node` does).
    ///
    /// # Errors
    /// The errors of [`Graph::plan_rewired_move`] and
    /// [`Graph::move_node`] (except [`GraphError::CrossNetwork`] for
    /// value links), and of adding the ports (for example
    /// [`GraphError::InInstance`] when a port would go into a Charm
    /// instance). The graph is unchanged on error.
    pub fn move_node_rewired(
        &mut self,
        id: NodeId,
        parent: Option<NodeId>,
        seed: NodeId,
    ) -> Result<(RewirePlan, Rewritten), GraphError> {
        let plan = self.plan_rewired_move(id, parent, seed)?;
        if plan.is_empty() {
            let rewritten = self.move_node(id, parent)?;
            return Ok((plan, rewritten));
        }
        let backup = self.duplicate();
        match self.apply_rewire(id, parent, &plan) {
            Ok(rewritten) => Ok((plan, rewritten)),
            Err(err) => {
                *self = backup;
                Err(err)
            }
        }
    }

    fn apply_rewire(
        &mut self,
        id: NodeId,
        parent: Option<NodeId>,
        plan: &RewirePlan,
    ) -> Result<Vec<(NodeId, String, String)>, GraphError> {
        for l in &plan.unlink {
            self.unlink(l.to, &l.to_port)?;
        }
        let rewritten = self.move_node(id, parent)?;
        for p in &plan.ports {
            let ty = match p.direction {
                PortDirection::Input => INPUT_TYPE,
                PortDirection::Output => OUTPUT_TYPE,
            };
            self.add_node_in(Some(p.subnet), p.id, ty, &p.name, Some(p.port_type))?;
        }
        for l in &plan.link {
            self.link(l.from, &l.from_port, l.to, &l.to_port)?;
        }
        Ok(rewritten)
    }
}
