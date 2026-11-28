// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Deleting several nodes as one edit.

use std::collections::BTreeSet;

use livspeil_graph::{Graph, NodeId};

use crate::Command;

/// The command that deletes `ids` together: one transaction, so one undo
/// step and all or nothing.
///
/// A node cannot be deleted while an expression reads it, so expressions
/// by which the listed nodes read each other are cleared first (the
/// inverse restores them). An expression of a node that stays still keeps
/// what it reads from being deleted: the command then fails and changes
/// nothing. Nodes listed twice, or inside a listed subnet, are covered by
/// the one deletion.
///
/// The command is made for `graph` as it is now: apply it before editing
/// the graph further.
#[must_use]
pub fn delete_nodes(graph: &Graph, ids: &[NodeId]) -> Command {
    let listed: BTreeSet<NodeId> = ids.iter().copied().collect();
    // The listed node that `id` goes with: itself, or a subnet around it.
    let goes_with = |id: NodeId| {
        let mut found = None;
        let mut at = Some(id);
        while let Some(node) = at {
            if listed.contains(&node) {
                found = Some(node);
            }
            at = graph.parent(node);
        }
        found
    };
    let nodes: Vec<NodeId> = ids
        .iter()
        .copied()
        .enumerate()
        .filter(|&(i, id)| !ids[..i].contains(&id) && goes_with(id).is_none_or(|g| g == id))
        .map(|(_, id)| id)
        .collect();
    let mut cleared = BTreeSet::new();
    for &node in &nodes {
        for (reader, port) in graph.expression_readers(node) {
            if goes_with(reader).is_some_and(|other| other != node) {
                cleared.insert((reader, port.to_owned()));
            }
        }
    }
    let clear = cleared
        .into_iter()
        .map(|(node, port)| Command::ClearExpression { node, port });
    let delete = nodes.into_iter().map(|id| Command::DeleteNode { id });
    Command::Transaction(clear.chain(delete).collect())
}
