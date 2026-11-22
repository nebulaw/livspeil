// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Networks, nodes, links, type checking and the cook scheduler.
//!
//! Milestone M0.2; see `specs/livspeil-kernel-spec-v0.md`, sections 3 to 5.
//!
//! - [`Graph`] holds nodes and links and enforces the structural rules at
//!   edit time: known node types, unique names, type-checked links, one link
//!   per input, no cycles.
//! - [`Cooker`] evaluates a graph lazily. It cooks only what the requested
//!   node depends on, caches per node, skips nodes whose inputs did not
//!   change, and stops propagating when a re-cooked node produced the same
//!   outputs as before (early cutoff). Failures never panic the host: a
//!   failing node keeps its last good output and carries an error.
//!   Invalidation is push-based: edits mark the affected subgraph dirty,
//!   so a cook with nothing dirty upstream costs `O(1)` (see the `cook`
//!   module source for the design).
//!
//! - Inputs can also be driven by an expression or an animation channel
//!   ([`Graph::set_expression`], [`Graph::set_animation`], M0.5b). Priority:
//!   link, expression, animation, static value, default. `ch("node/port")`
//!   in an expression reads another node's resolved input and counts as a
//!   dependency edge for cycles and caching; renaming a node rewrites the
//!   paths that name it, and removing a node that is read is refused. See
//!   [`Graph::set_expression`] for the details.
//!
//! - System bindings (M0.8): [`Cooker::set_system`] installs a read-only
//!   host (`livspeil_node_api::SystemHost`) for nodes that read the system,
//!   such as `core.fs.list_dir`; they are re-cooked when the host's
//!   revision changes (see `cook/system.rs` for the caching rules).
//!
//! - [`GraphSnapshot`] is a canonical, comparable copy of the editable state
//!   (used by the command bus tests to prove undo and replay exact).
//!
//! Cargo feature `serde` (off by default) derives `Serialize` and
//! `Deserialize` for [`NodeId`] (as a UUID string), [`Endpoint`], [`Link`]
//! and the snapshot types, and turns on the same feature in
//! `livspeil-node-api` and `livspeil-params`. The command journal in `livspeil-cmd` enables it.
//!
//! Edits made through the command bus (`livspeil-cmd`, M0.3) get undo,
//! change events and journaling; the methods here are the primitive edits.
//!
//! Not yet implemented (later milestones): subnets and Charms, the
//! document format (M0.4), event ports, and parallel evaluation.

mod cook;
mod graph;
mod snapshot;
mod sources;

pub use cook::{CookError, CookStats, Cooker};
pub use graph::{Endpoint, Graph, GraphError, Link, NodeId, PortDirection};
pub use snapshot::{GraphSnapshot, NamedLink, NodeSnapshot};
