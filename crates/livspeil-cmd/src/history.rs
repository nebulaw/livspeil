// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Linear undo and redo over the command bus.

use std::collections::BTreeMap;

use livspeil_graph::{Graph, NodeId};

use crate::{CmdError, Command, DocChange, EntryKind, GroupId, JournalEntry, Origin};

/// What [`History::apply`], [`History::undo`] or [`History::redo`] did.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// The command that was applied, ready to append to a journal. For an
    /// undo or redo it is the transaction that was actually applied.
    pub entry: JournalEntry,
    /// The document changes it caused.
    pub changes: Vec<DocChange>,
}

/// One undo step: commands applied in order and, index for index, their
/// inverses.
#[derive(Debug, Clone)]
struct Step {
    origin: Origin,
    group: Option<GroupId>,
    forward: Vec<Command>,
    backward: Vec<Command>,
    /// The entry of each node placed since the last command of another kind.
    /// A node placed again replaces its entry instead of adding one, so
    /// dragging costs as much to remember as the nodes dragged, not as the
    /// frames it took.
    layout: BTreeMap<NodeId, usize>,
}

/// The nodes `command` places, if placing nodes is all it does.
fn placed(command: &Command) -> Option<Vec<NodeId>> {
    match command {
        Command::SetPosition { node, .. } => Some(vec![*node]),
        Command::Transaction(steps) => steps
            .iter()
            .map(|step| match step {
                Command::SetPosition { node, .. } => Some(*node),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

impl Step {
    fn new(origin: Origin, group: Option<GroupId>) -> Self {
        Step {
            origin,
            group,
            forward: Vec::new(),
            backward: Vec::new(),
            layout: BTreeMap::new(),
        }
    }

    /// Add an applied command and its inverse to the step.
    fn push(&mut self, command: Command, inverse: Command) {
        let Some(nodes) = placed(&command) else {
            // Later placements may not merge across this command.
            self.layout.clear();
            self.forward.push(command);
            self.backward.push(inverse);
            return;
        };
        // A transaction's inverse lists its steps' inverses last to first.
        let mut inverses = inverse.into_steps();
        if nodes.len() > 1 {
            inverses.reverse();
        }
        for ((node, forward), backward) in nodes.into_iter().zip(command.into_steps()).zip(inverses)
        {
            match self.layout.get(&node) {
                // The first inverse already goes back to before the step.
                Some(&at) => self.forward[at] = forward,
                None => {
                    self.layout.insert(node, self.forward.len());
                    self.forward.push(forward);
                    self.backward.push(backward);
                }
            }
        }
    }
}

/// Linear undo/redo history (branching history is out of scope).
///
/// The history must see every edit to its graph; an edit made around it can
/// make an undo fail (with the graph left unchanged).
#[derive(Debug, Clone, Default)]
pub struct History {
    done: Vec<Step>,
    undone: Vec<Step>,
    /// The group the last applied command belonged to, while it is still
    /// open for coalescing.
    open_group: Option<GroupId>,
}

impl History {
    /// An empty history.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply `command` and record it for undo. Clears the redo stack. A
    /// command that only places nodes where they already are is not recorded
    /// and changes neither stack.
    ///
    /// When `group` is `Some` and equals the group of the previous command
    /// applied through this history (with no undo, redo or [`seal`] in
    /// between), the command joins that undo step.
    ///
    /// [`seal`]: History::seal
    ///
    /// # Errors
    /// Returns the command's error; the graph and the history are unchanged.
    pub fn apply(
        &mut self,
        graph: &mut Graph,
        command: Command,
        origin: Origin,
        group: Option<GroupId>,
    ) -> Result<Record, CmdError> {
        let applied = command.apply(graph)?;
        // Putting nodes where they already are is not an edit: it keeps redo
        // (an editor may report a click as a move).
        let unmoved = applied.changes.is_empty() && placed(&command).is_some_and(|n| !n.is_empty());
        let entry = JournalEntry {
            kind: EntryKind::Do,
            origin,
            group,
            command: command.clone(),
        };
        if unmoved {
            return Ok(Record {
                entry,
                changes: applied.changes,
            });
        }
        self.undone.clear();
        match self.done.last_mut() {
            Some(step) if group.is_some() && group == self.open_group => {
                step.push(command, applied.inverse);
            }
            _ => {
                let mut step = Step::new(origin, group);
                step.push(command, applied.inverse);
                self.done.push(step);
            }
        }
        self.open_group = group;
        Ok(Record {
            entry,
            changes: applied.changes,
        })
    }

    /// Undo the most recent step.
    ///
    /// # Errors
    /// [`CmdError::NothingToUndo`] on an empty stack, or the error of the
    /// inverse (graph and history unchanged).
    pub fn undo(&mut self, graph: &mut Graph) -> Result<Record, CmdError> {
        let step = self.done.last().ok_or(CmdError::NothingToUndo)?;
        let command = Command::Transaction(step.backward.iter().rev().cloned().collect());
        let applied = command.apply(graph)?;
        let Some(mut step) = self.done.pop() else {
            return Err(CmdError::NothingToUndo);
        };
        // The inverse of the undo transaction lists, in order, commands that
        // redo each original command.
        step.forward = applied.inverse.into_steps();
        let record = Record {
            entry: JournalEntry {
                kind: EntryKind::Undo,
                origin: step.origin,
                group: step.group,
                command,
            },
            changes: applied.changes,
        };
        self.undone.push(step);
        self.open_group = None;
        Ok(record)
    }

    /// Redo the most recently undone step.
    ///
    /// # Errors
    /// [`CmdError::NothingToRedo`] on an empty stack, or the error of the
    /// command (graph and history unchanged).
    pub fn redo(&mut self, graph: &mut Graph) -> Result<Record, CmdError> {
        let step = self.undone.last().ok_or(CmdError::NothingToRedo)?;
        let command = Command::Transaction(step.forward.clone());
        let applied = command.apply(graph)?;
        let Some(mut step) = self.undone.pop() else {
            return Err(CmdError::NothingToRedo);
        };
        // The inverse lists the per-command inverses last to first.
        let mut backward = applied.inverse.into_steps();
        backward.reverse();
        step.backward = backward;
        let record = Record {
            entry: JournalEntry {
                kind: EntryKind::Redo,
                origin: step.origin,
                group: step.group,
                command,
            },
            changes: applied.changes,
        };
        self.done.push(step);
        self.open_group = None;
        Ok(record)
    }

    /// End the current coalescing group (for example when a slider drag
    /// ends), so the next command starts a new undo step even if it reuses
    /// the group id.
    pub fn seal(&mut self) {
        self.open_group = None;
    }

    /// Whether there is a step to undo.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    /// Whether there is a step to redo.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// Number of undo steps.
    #[must_use]
    pub fn undo_len(&self) -> usize {
        self.done.len()
    }

    /// Number of redo steps.
    #[must_use]
    pub fn redo_len(&self) -> usize {
        self.undone.len()
    }

    /// Forget all steps.
    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
        self.open_group = None;
    }
}
