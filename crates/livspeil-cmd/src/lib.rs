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
//!
//! Subnets and Charms (M0.7): `CreateNodeIn` (a node in any network, with
//! the port type of a boundary node), `CreateSubnet`, `DeleteNode` (a
//! subnet is deleted with everything inside it), `AddInterfacePort` /
//! `RemoveInterfacePort`, `MoveNode` (between networks; its inverse
//! restores the exact text of re-rendered `ch()` paths), `Promote` /
//! `Unpromote` and `SetCharm`. Composite commands (promotion, interface
//! ports, subnet deletion) apply as transactions of primitive edits, so
//! their inverses are exact by construction.
//!
//! Shared Charm definitions (M0.16): `AddDefinition` / `RemoveDefinition`,
//! `UpdateDefinition` (its inverse restores the previous definition and
//! then every instance input's exact sources, the links dropped with
//! removed interface ports and the re-rendered expressions),
//! `Instantiate`, `Detach` / `Attach` (each other's inverse),
//! `MakeDefinition` (a transaction: carry outside readers, delete the
//! subnet, add the definition, instantiate with the subnet's id, relink),
//! and `DeleteNode` on an instance (its inverse instantiates it again and
//! restores its interface input sources and links). `MoveNodeRewired`
//! applies a rewired move as a transaction of unlinks, the move, new
//! interface ports and links, with port ids derived from its `seed` so
//! journals replay exactly. Edits inside an instance are refused by the
//! graph (`GraphError::InInstance`).

mod command;
mod delete;
mod entry;
mod error;
mod history;
mod journal;

pub use command::{Applied, Command, DocChange};
pub use delete::delete_nodes;
pub use entry::{EntryKind, GroupId, JournalEntry, Origin};
pub use error::{CmdError, JournalError};
pub use history::{History, Record};
pub use journal::{JournalWriter, read_journal, replay};
