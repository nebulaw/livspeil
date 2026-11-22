// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Command bus, transactions, undo/redo and the journal.
//!
//! Milestone M0.3; see `specs/livspeil-kernel-spec-v0.md`, section 7.
//!
//! - Every edit to a [`Graph`](livspeil_graph::Graph) is a [`Command`].
//!   [`Command::apply`] either changes the graph and returns the inverse
//!   command plus the [`DocChange`]s it caused, or fails and leaves the graph
//!   exactly as it was. A [`Command::Transaction`] is all-or-nothing: if a
//!   step fails, the steps already applied are rolled back with their
//!   inverses.
//! - [`History`] keeps undo and redo stacks of inverses. Commands sharing a
//!   [`GroupId`] in a row (a slider drag) coalesce into one undo step.
//! - The journal is append-only JSON Lines: one [`JournalEntry`] (command,
//!   [`Origin`], group, and whether it was a do, undo or redo) per line.
//!   [`replay`] applies a journal to a graph; replaying onto the same
//!   starting graph reproduces the same state.
//!
//! Input values are edited with `SetInput`/`ClearInput` (static values),
//! `SetExpression`/`ClearExpression` and `SetAnimation`/`ClearAnimation`
//! (M0.5b); every one has an exact inverse. Animation channels are
//! validated finite, so they always journal. Ports are addressed by name, never by index, so commands
//! stay valid when a node type gains ports.

mod command;
mod entry;
mod error;
mod history;
mod journal;

pub use command::{Applied, Command, DocChange};
pub use entry::{EntryKind, GroupId, JournalEntry, Origin};
pub use error::{CmdError, JournalError};
pub use history::{History, Record};
pub use journal::{JournalWriter, read_journal, replay};
