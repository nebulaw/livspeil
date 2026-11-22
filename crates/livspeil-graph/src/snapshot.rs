// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Canonical, comparable copies of a graph's editable state.

use livspeil_node_api::{PortType, Value};
use livspeil_params::AnimChannel;

use crate::{CharmMeta, NodeId};

/// A link described by port names rather than port indices.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NamedLink {
    pub from: NodeId,
    pub from_port: String,
    pub to: NodeId,
    pub to_port: String,
}

/// One node of a [`GraphSnapshot`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NodeSnapshot {
    pub id: NodeId,
    pub name: String,
    /// Qualified type id, for example `core.math.add@1`.
    pub type_id: String,
    /// Explicitly set inputs as `(port name, value)`, in port order.
    pub inputs: Vec<(String, Value)>,
    /// Expressions as `(port name, source)`, in port order.
    #[cfg_attr(feature = "serde", serde(default))]
    pub expressions: Vec<(String, String)>,
    /// Animation channels as `(port name, channel)`, in port order.
    #[cfg_attr(feature = "serde", serde(default))]
    pub animations: Vec<(String, AnimChannel)>,
    /// The subnet holding the node; `None` in the root network.
    #[cfg_attr(feature = "serde", serde(default))]
    pub parent: Option<NodeId>,
    /// Declared port type of a boundary node (`core.subnet.input@1`,
    /// `core.output@1`).
    #[cfg_attr(feature = "serde", serde(default))]
    pub port_type: Option<PortType>,
    /// Charm metadata of a subnet.
    #[cfg_attr(feature = "serde", serde(default))]
    pub charm: Option<CharmMeta>,
}

/// The editable state of a graph: nodes of every network in id order and links ordered by
/// target node and input port. Produced by [`crate::Graph::snapshot`].
///
/// Equality uses IEEE float equality, so a snapshot holding `NaN` never
/// equals itself.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GraphSnapshot {
    pub nodes: Vec<NodeSnapshot>,
    pub links: Vec<NamedLink>,
}
