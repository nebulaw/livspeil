// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! What changes the graph. Every change is a command in the history.

use livspeil_cmd::{Command, GroupId, Origin, delete_nodes};
use livspeil_graph::NodeId;

use super::Session;
use crate::catalog::short_name;
use crate::edit::Entry;
use crate::shown::{Port, links, root_nodes};

impl Session {
    /// Apply a command as part of the gesture under way, or on its own.
    fn run(&mut self, command: Command, in_gesture: bool) -> Result<(), String> {
        let group = if in_gesture {
            let groups = &mut self.groups;
            Some(*self.gesture.get_or_insert_with(|| {
                *groups += 1;
                GroupId(*groups)
            }))
        } else {
            self.end_gesture();
            None
        };
        self.history
            .apply(&mut self.graph, command, Origin::Ui, group)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// The gesture under way ended: later edits are a new undo step.
    pub fn end_gesture(&mut self) {
        if self.gesture.take().is_some() {
            self.history.seal();
            self.dirty = self.graph.snapshot() != self.saved;
        }
    }

    /// Place a node, as part of a drag. Positions change nothing the nodes
    /// compute, so nothing else is brought up to date.
    ///
    /// # Errors
    /// Why the node cannot be placed there.
    pub fn move_node(&mut self, node: NodeId, position: [f64; 2]) -> Result<(), String> {
        let command = Command::SetPosition {
            node,
            position: Some(position),
        };
        self.run(command, true)?;
        // Settled when the gesture ends.
        self.dirty = true;
        Ok(())
    }

    /// Whether output `from` may feed input `to`, replacing its link.
    #[must_use]
    pub fn can_connect(&self, from: &Port, to: &Port) -> bool {
        self.graph
            .check_link(from.node, &from.port, to.node, &to.port)
            .is_ok()
    }

    fn linked(&self, to: &Port) -> bool {
        matches!(self.graph.link_source(to.node, &to.port), Ok(Some(_)))
    }

    fn unlink(to: &Port) -> Command {
        Command::Unlink {
            to: to.node,
            to_port: to.port.clone(),
        }
    }

    /// Link output `from` to input `to`, replacing what fed it. Part of a
    /// wire drag.
    ///
    /// # Errors
    /// Why the link is not allowed.
    pub fn connect(&mut self, from: &Port, to: &Port) -> Result<(), String> {
        let link = Command::Link {
            from: from.node,
            from_port: from.port.clone(),
            to: to.node,
            to_port: to.port.clone(),
        };
        let command = if self.linked(to) {
            Command::Transaction(vec![Self::unlink(to), link])
        } else {
            link
        };
        self.run(command, true)?;
        self.refresh();
        Ok(())
    }

    /// Remove the link of input `to`. Part of a wire drag.
    ///
    /// # Errors
    /// Why it cannot be removed.
    pub fn disconnect(&mut self, to: &Port) -> Result<(), String> {
        self.run(Self::unlink(to), true)?;
        self.refresh();
        Ok(())
    }

    /// Give input `port` what a field asked for. One undo step.
    ///
    /// # Errors
    /// The kernel's reason for refusing it; nothing changes.
    pub fn enter(&mut self, port: &Port, entry: Entry) -> Result<(), String> {
        let (node, name) = (port.node, port.port.clone());
        let has_expression = matches!(self.graph.expression(node, &name), Ok(Some(_)));
        let has_value = matches!(self.graph.input_value(node, &name), Ok(Some(_)));
        let clear_expression = || Command::ClearExpression {
            node,
            port: name.clone(),
        };
        let clear_value = || Command::ClearInput {
            node,
            port: name.clone(),
        };
        let mut steps = Vec::new();
        match entry {
            Entry::Clear => {
                steps.extend(has_expression.then(clear_expression));
                steps.extend(has_value.then(clear_value));
            }
            Entry::Expression(source) => {
                if self.graph.expression(node, &name).ok().flatten() != Some(source.as_str()) {
                    steps.push(Command::SetExpression {
                        node,
                        port: name.clone(),
                        source,
                    });
                }
            }
            Entry::Value(value) => {
                // An expression would keep winning over the value.
                steps.extend(has_expression.then(clear_expression));
                if self.graph.input_value(node, &name).ok().flatten() != Some(&value) {
                    steps.push(Command::SetInput {
                        node,
                        port: name.clone(),
                        value,
                    });
                }
            }
        }
        if steps.is_empty() {
            return Ok(());
        }
        self.run(Command::Transaction(steps), false)?;
        self.refresh();
        Ok(())
    }

    /// Rename a node. One undo step.
    ///
    /// # Errors
    /// Why the name is refused.
    pub fn rename(&mut self, node: NodeId, name: &str) -> Result<(), String> {
        if self.graph.node_name(node) == Some(name) {
            return Ok(());
        }
        let command = Command::Rename {
            node,
            name: name.to_owned(),
        };
        self.run(command, false)?;
        self.refresh();
        Ok(())
    }

    /// Add a node of type `type_id` to the root network at `position`,
    /// under a name no other node there has. One undo step.
    ///
    /// # Errors
    /// Why it cannot be added.
    pub fn add(&mut self, type_id: &str, position: [f64; 2]) -> Result<NodeId, String> {
        let id = NodeId::new();
        let create = Command::CreateNode {
            id,
            type_id: type_id.to_owned(),
            name: self.free_name(short_name(type_id)),
        };
        let place = Command::SetPosition {
            node: id,
            position: Some(position),
        };
        self.run(Command::Transaction(vec![create, place]), false)?;
        self.refresh();
        Ok(id)
    }

    /// `base`, or `base2`, `base3`, ... if the root network has it.
    fn free_name(&self, base: &str) -> String {
        let taken = |name: &str| self.graph.find_in(None, name).is_some();
        if !taken(base) {
            return base.to_owned();
        }
        let mut n = 2_u64;
        loop {
            let name = format!("{base}{n}");
            if !taken(&name) {
                return name;
            }
            n += 1;
        }
    }

    /// Delete `nodes` and their links. One undo step.
    ///
    /// # Errors
    /// When a node that stays reads one of them in an expression; nothing
    /// is deleted then.
    pub fn delete(&mut self, nodes: &[NodeId]) -> Result<(), String> {
        if nodes.is_empty() {
            return Ok(());
        }
        if let Some(held) = self.held(nodes) {
            return Err(held);
        }
        let command = delete_nodes(&self.graph, nodes);
        self.run(command, false)?;
        if self.selected.is_some_and(|id| !self.graph.contains(id)) {
            self.selected = None;
        }
        self.refresh();
        Ok(())
    }

    /// Why `nodes` cannot be deleted, if an expression of a node that stays
    /// reads one of them.
    fn held(&self, nodes: &[NodeId]) -> Option<String> {
        let goes = |mut id: NodeId| loop {
            if nodes.contains(&id) {
                return true;
            }
            match self.graph.parent(id) {
                Some(parent) => id = parent,
                None => return false,
            }
        };
        let name = |id| {
            let path = self.graph.path(id).unwrap_or_default();
            path.strip_prefix('/').unwrap_or(&path).to_owned()
        };
        nodes.iter().find_map(|&node| {
            let (reader, port) = self
                .graph
                .expression_readers(node)
                .into_iter()
                .find(|&(reader, _)| !goes(reader))?;
            Some(format!(
                "`{}` cannot be deleted: the expression of `{}.{port}` reads it",
                name(node),
                name(reader),
            ))
        })
    }

    /// Undo the latest edit.
    ///
    /// # Errors
    /// When there is none.
    pub fn undo(&mut self) -> Result<(), String> {
        self.end_gesture();
        self.history
            .undo(&mut self.graph)
            .map_err(|e| e.to_string())?;
        self.refresh();
        Ok(())
    }

    /// Redo the latest undone edit.
    ///
    /// # Errors
    /// When there is none.
    pub fn redo(&mut self) -> Result<(), String> {
        self.end_gesture();
        self.history
            .redo(&mut self.graph)
            .map_err(|e| e.to_string())?;
        self.refresh();
        Ok(())
    }

    /// Bring everything derived from the graph up to date after an edit.
    fn refresh(&mut self) {
        self.nodes = root_nodes(&self.graph);
        self.links = links(&self.graph, &self.nodes);
        self.dirty = self.graph.snapshot() != self.saved;
        self.cook();
    }
}
