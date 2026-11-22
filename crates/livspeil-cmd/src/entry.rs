// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Metadata carried by every applied command.

use serde::{Deserialize, Serialize};

use crate::Command;

/// Who issued a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// The user, through Dream Mode or another UI.
    Ui,
    /// A script.
    Script,
    /// An AI assistant.
    Ai,
    /// A document or asset import.
    Import,
    /// A collaborator's edit arriving over the network.
    Collab,
}

/// Coalescing key: consecutive commands with the same group form one undo
/// step (for example every value of one slider drag). Callers choose the
/// ids; a fresh id per gesture is enough.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupId(pub u64);

/// Whether a journal entry records a new command or an undo or redo.
/// Replay treats them alike: it applies the recorded command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    #[default]
    Do,
    Undo,
    Redo,
}

/// One applied command with its metadata: a line of the journal.
///
/// JSON form, one per line:
/// `{"kind":"do","origin":"ui","group":7,"command":{"rename":{...}}}`.
/// `kind` defaults to `do` and `group` to none when absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalEntry {
    #[serde(default)]
    pub kind: EntryKind,
    pub origin: Origin,
    #[serde(default)]
    pub group: Option<GroupId>,
    pub command: Command,
}
