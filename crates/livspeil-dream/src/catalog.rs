// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The node types of a room, and which of them a person can add.

use std::collections::BTreeMap;
use std::sync::Arc;

use livspeil_graph::{INPUT_TYPE, SUBNET_TYPE};
use livspeil_node_api::NodeRegistry;
use livspeil_nodes_core::register_all;

use crate::session::OpenError;

/// Charm instances are made from a definition, not added by type.
const CHARM_TYPE: &str = "core.charm@1";

/// A node type that can be added to the root network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeType {
    /// The qualified id, `core.math.add@1`.
    pub id: String,
    /// What the list calls it, `math.add`.
    pub label: String,
    /// What it does.
    pub summary: &'static str,
}

/// The built-in node types.
///
/// # Errors
/// [`OpenError::Registry`] if two of them claim one id.
pub fn registry() -> Result<Arc<NodeRegistry>, OpenError> {
    let mut registry = NodeRegistry::new();
    register_all(&mut registry).map_err(OpenError::Registry)?;
    Ok(Arc::new(registry))
}

/// The types that can be added, in the order of their labels: the latest
/// version of each. Left out are what cannot be in the root network
/// (subnet inputs), instances, and empty subnets, which cannot be entered
/// yet.
#[must_use]
pub fn node_types(registry: &NodeRegistry) -> Vec<NodeType> {
    let mut latest = BTreeMap::new();
    for id in registry.ids() {
        if [SUBNET_TYPE, INPUT_TYPE, CHARM_TYPE].contains(&id) {
            continue;
        }
        let Some(node) = registry.get(id) else {
            continue;
        };
        let spec = node.spec();
        let found = latest.entry(spec.id).or_insert(spec);
        if spec.version > found.version {
            *found = spec;
        }
    }
    latest
        .into_values()
        .map(|spec| NodeType {
            id: spec.qualified_id(),
            label: spec.id.strip_prefix("core.").unwrap_or(spec.id).to_owned(),
            summary: spec.summary,
        })
        .collect()
}

/// What a new node of type `type_id` is called, before a number makes it
/// unique: `add` for `core.math.add@1`.
#[must_use]
pub fn short_name(type_id: &str) -> &str {
    let id = type_id.split_once('@').map_or(type_id, |(id, _)| id);
    id.rsplit_once('.').map_or(id, |(_, last)| last)
}
