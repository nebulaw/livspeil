// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Commands, their inverses and the changes they report.

use livspeil_graph::{Graph, NamedLink, NodeId};
use livspeil_node_api::Value;
use serde::{Deserialize, Serialize};

use crate::CmdError;

/// An edit to a graph (kernel spec, section 7).
///
/// JSON form (serde's externally tagged enums, snake case):
/// `{"set_input":{"node":"…","port":"a","value":{"Float":1.0}}}`,
/// `{"transaction":[…]}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Command {
    /// Add a node of type `type_id` (qualified, e.g. `core.math.add@1`).
    CreateNode {
        id: NodeId,
        type_id: String,
        name: String,
    },
    /// Remove a node and every link attached to it.
    DeleteNode { id: NodeId },
    /// Set an input's value (stand-in for `SetParam` until M0.5). `Int`
    /// values are accepted for `Float` inputs and stored as `Float`.
    SetInput {
        node: NodeId,
        port: String,
        value: Value,
    },
    /// Forget an input's explicit value so the spec default applies.
    ClearInput { node: NodeId, port: String },
    /// Connect an output to an unlinked input.
    Link {
        from: NodeId,
        from_port: String,
        to: NodeId,
        to_port: String,
    },
    /// Remove the link feeding an input. Unlinking an unlinked input is a
    /// successful no-op.
    Unlink { to: NodeId, to_port: String },
    /// Rename a node. Does not invalidate cooked results.
    Rename { node: NodeId, name: String },
    /// Apply the commands in order, all or nothing. An empty transaction is
    /// a no-op.
    Transaction(Vec<Command>),
}

/// A change to the document, reported by every applied command so a UI can
/// invalidate precisely.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DocChange {
    NodeAdded {
        id: NodeId,
    },
    /// Reported after the `Unlinked` changes of the node's links.
    NodeRemoved {
        id: NodeId,
    },
    /// An input's explicit value was set or cleared.
    InputChanged {
        node: NodeId,
        port: String,
    },
    Linked(NamedLink),
    Unlinked(NamedLink),
    Renamed {
        node: NodeId,
        old: String,
        new: String,
    },
}

/// The result of applying a command.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    /// Applying this command to the edited graph restores the previous
    /// state.
    pub inverse: Command,
    /// What changed, in order. Empty for a no-op.
    pub changes: Vec<DocChange>,
}

impl Command {
    /// A `CreateNode` with a fresh id.
    #[must_use]
    pub fn create_node(type_id: &str, name: &str) -> Self {
        Command::CreateNode {
            id: NodeId::new(),
            type_id: type_id.to_owned(),
            name: name.to_owned(),
        }
    }

    /// A command that changes nothing (an empty transaction).
    #[must_use]
    pub const fn noop() -> Self {
        Command::Transaction(Vec::new())
    }

    /// Apply the command to `graph`.
    ///
    /// # Errors
    /// Returns a [`CmdError`] and leaves the graph unchanged when the edit
    /// is invalid. For a transaction, steps applied before the failing one
    /// are rolled back.
    pub fn apply(&self, graph: &mut Graph) -> Result<Applied, CmdError> {
        let mut changes = Vec::new();
        let inverse = self.apply_into(graph, &mut changes)?;
        Ok(Applied { inverse, changes })
    }

    /// The command as a list of steps: a transaction's steps, or itself.
    pub(crate) fn into_steps(self) -> Vec<Command> {
        match self {
            Command::Transaction(steps) => steps,
            other => vec![other],
        }
    }

    /// Whether any value in the command is NaN or infinite.
    pub(crate) fn has_non_finite(&self) -> bool {
        match self {
            Command::SetInput { value, .. } => !value_is_finite(value),
            Command::Transaction(steps) => steps.iter().any(Command::has_non_finite),
            _ => false,
        }
    }

    fn apply_into(
        &self,
        graph: &mut Graph,
        changes: &mut Vec<DocChange>,
    ) -> Result<Command, CmdError> {
        match self {
            Command::CreateNode { id, type_id, name } => {
                graph.add_node_with_id(*id, type_id, name)?;
                changes.push(DocChange::NodeAdded { id: *id });
                Ok(Command::DeleteNode { id: *id })
            }
            Command::DeleteNode { id } => {
                let id = *id;
                let (Some(name), Some(type_id)) = (graph.node_name(id), graph.node_type(id)) else {
                    return Err(livspeil_graph::GraphError::UnknownNode(id).into());
                };
                let mut restore = vec![Command::CreateNode {
                    id,
                    type_id: type_id.to_owned(),
                    name: name.to_owned(),
                }];
                restore.extend(
                    graph
                        .input_values(id)
                        .map(|(port, value)| Command::SetInput {
                            node: id,
                            port: port.to_owned(),
                            value: value.clone(),
                        }),
                );
                let mut links = graph.incoming_links(id);
                links.extend(graph.outgoing_links(id));
                restore.extend(links.iter().map(link_command));
                graph.remove_node(id)?;
                changes.extend(links.into_iter().map(DocChange::Unlinked));
                changes.push(DocChange::NodeRemoved { id });
                Ok(Command::Transaction(restore))
            }
            Command::SetInput { node, port, value } => {
                let previous = graph.input_value(*node, port)?.cloned();
                graph.set_input(*node, port, value.clone())?;
                changes.push(DocChange::InputChanged {
                    node: *node,
                    port: port.clone(),
                });
                Ok(restore_input(*node, port, previous))
            }
            Command::ClearInput { node, port } => {
                let previous = graph.input_value(*node, port)?.cloned();
                graph.clear_input(*node, port)?;
                if previous.is_some() {
                    changes.push(DocChange::InputChanged {
                        node: *node,
                        port: port.clone(),
                    });
                }
                Ok(restore_input(*node, port, previous))
            }
            Command::Link {
                from,
                from_port,
                to,
                to_port,
            } => {
                graph.link(*from, from_port, *to, to_port)?;
                changes.push(DocChange::Linked(NamedLink {
                    from: *from,
                    from_port: from_port.clone(),
                    to: *to,
                    to_port: to_port.clone(),
                }));
                Ok(Command::Unlink {
                    to: *to,
                    to_port: to_port.clone(),
                })
            }
            Command::Unlink { to, to_port } => {
                let source = graph
                    .link_source(*to, to_port)?
                    .map(|(node, port)| (node, port.to_owned()));
                graph.unlink(*to, to_port)?;
                let Some((from, from_port)) = source else {
                    return Ok(Command::noop());
                };
                let link = NamedLink {
                    from,
                    from_port,
                    to: *to,
                    to_port: to_port.clone(),
                };
                let inverse = link_command(&link);
                changes.push(DocChange::Unlinked(link));
                Ok(inverse)
            }
            Command::Rename { node, name } => {
                let old = graph.rename(*node, name)?;
                if old != *name {
                    changes.push(DocChange::Renamed {
                        node: *node,
                        old: old.clone(),
                        new: name.clone(),
                    });
                }
                Ok(Command::Rename {
                    node: *node,
                    name: old,
                })
            }
            Command::Transaction(steps) => {
                let mark = changes.len();
                let mut inverses = Vec::with_capacity(steps.len());
                for (index, step) in steps.iter().enumerate() {
                    match step.apply_into(graph, changes) {
                        Ok(inverse) => inverses.push(inverse),
                        Err(source) => {
                            changes.truncate(mark);
                            let source = Box::new(source);
                            let mut scratch = Vec::new();
                            for inverse in inverses.iter().rev() {
                                if let Err(err) = inverse.apply_into(graph, &mut scratch) {
                                    return Err(CmdError::RollbackFailed {
                                        index,
                                        source,
                                        rollback: Box::new(err),
                                    });
                                }
                            }
                            return Err(CmdError::InTransaction { index, source });
                        }
                    }
                }
                inverses.reverse();
                Ok(Command::Transaction(inverses))
            }
        }
    }
}

fn restore_input(node: NodeId, port: &str, previous: Option<Value>) -> Command {
    match previous {
        Some(value) => Command::SetInput {
            node,
            port: port.to_owned(),
            value,
        },
        None => Command::ClearInput {
            node,
            port: port.to_owned(),
        },
    }
}

fn link_command(link: &NamedLink) -> Command {
    Command::Link {
        from: link.from,
        from_port: link.from_port.clone(),
        to: link.to,
        to_port: link.to_port.clone(),
    }
}

fn value_is_finite(value: &Value) -> bool {
    match value {
        Value::Float(v) => v.is_finite(),
        Value::Vec2(v) => v.iter().all(|x| x.is_finite()),
        Value::Vec3(v) => v.iter().all(|x| x.is_finite()),
        Value::Vec4(v) | Value::Color(v) => v.iter().all(|x| x.is_finite()),
        _ => true,
    }
}
