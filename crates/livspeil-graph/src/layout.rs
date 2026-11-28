// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Editor layout: where each node sits in the node editor.
//!
//! Layout is edited like any other part of the graph (through commands, with
//! undo and a journal) and saved in the document, but it takes **no part in
//! evaluation**: changing it dirties nothing and re-cooks nothing.
//!
//! A node has a position or has none (an editor then places it itself).
//! Nodes inside a Charm instance never have one: an instance's contents come
//! from its definition and are not saved per instance. Definitions carry no
//! layout ([`crate::CharmDef`] has nowhere to hold it).

use crate::{Graph, GraphError, NodeId};

impl Graph {
    /// Where the editor shows `node`; `None` if it was never placed or does
    /// not exist.
    #[must_use]
    pub fn position(&self, node: NodeId) -> Option<[f64; 2]> {
        self.node(node)?.position
    }

    /// Every placed node and its position, in id order.
    pub fn positions(&self) -> impl Iterator<Item = (NodeId, [f64; 2])> + '_ {
        self.node_ids()
            .filter_map(|id| Some((id, self.position(id)?)))
    }

    /// Place `node` (`None`: unplace it). Returns the previous position.
    ///
    /// # Errors
    /// [`GraphError::UnknownNode`]; [`GraphError::NonFinitePosition`] for a
    /// `NaN` or infinite coordinate; [`GraphError::InInstance`] for a node
    /// inside a Charm instance (the instance itself can be placed). The
    /// graph is unchanged on error.
    pub fn set_position(
        &mut self,
        node: NodeId,
        position: Option<[f64; 2]>,
    ) -> Result<Option<[f64; 2]>, GraphError> {
        if !self.contains(node) {
            return Err(GraphError::UnknownNode(node));
        }
        if position.is_some_and(|p| !p.iter().all(|v| v.is_finite())) {
            return Err(GraphError::NonFinitePosition(node));
        }
        self.check_structure_editable(node)?;
        let slot = self.node_mut(node).ok_or(GraphError::UnknownNode(node))?;
        Ok(std::mem::replace(&mut slot.position, position))
    }

    /// Forget the positions of everything inside `container`.
    pub(crate) fn clear_layout_inside(&mut self, container: NodeId) {
        let mut stack = self.children(Some(container));
        while let Some(id) = stack.pop() {
            stack.extend(self.children(Some(id)));
            if let Some(node) = self.node_mut(id) {
                node.position = None;
            }
        }
    }
}
