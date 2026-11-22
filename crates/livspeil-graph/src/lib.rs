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
//!
//! Not yet implemented (later milestones): subnets and Charms, parameters
//! and expressions (M0.5), the command bus (M0.3), the document format
//! (M0.4), event ports, and parallel evaluation.

mod cook;
mod graph;

pub use cook::{CookError, CookStats, Cooker};
pub use graph::{Endpoint, Graph, GraphError, Link, NodeId, PortDirection};
