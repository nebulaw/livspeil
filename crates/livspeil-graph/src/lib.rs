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
//!   paths that name it, and removing a node that is read is refused.
//!   `out("node/port")` (M0.11) reads another node's output the same way.
//!   See [`Graph::set_expression`] for the details and
//!   [`Graph::expression_refs`] for listing the references.
//!
//! - System bindings (M0.8): [`Cooker::set_system`] installs a read-only
//!   host (`livspeil_node_api::SystemHost`) for nodes that read the system,
//!   such as `core.fs.list_dir`; they are re-cooked when the host's
//!   revision changes (see `cook/system.rs` for the caching rules).
//!
//! - Events (M0.10): links between `Event` ports are stored like other
//!   links but are not value dependencies (they may form cycles).
//!   [`Cooker::tick`] runs the input and events phases of a tick (external
//!   events, polls, bounded FIFO delivery in a stable order); event nodes
//!   keep runtime state per cooker, and a state change invalidates the
//!   node's values and their downstream for that cooker. See
//!   `cook/events.rs` for the design.
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
//! # Networks, subnets and Charms (M0.7)
//!
//! Networks nest, but storage stays **flat**: one [`Graph`] holds every
//! node of every network in the same slab, and every node records its
//! *parent*, the subnet whose network holds it (`None` for the root
//! network `/`). A **subnet** is a node of type [`SUBNET_TYPE`]
//! (`core.subnet@1`, no ports of its own) that owns a child network.
//! Names are unique per network; [`Graph::path`] and [`Graph::find_path`]
//! address nodes as `/lamp/rig/bulb`. Names of subnets and of nodes inside
//! them are single path segments (no `/`, not `.` or `..`); root nodes keep
//! the pre-M0.7 freedom.
//!
//! A subnet's **interface** is made of boundary nodes inside it, one per
//! port, named after the port: [`INPUT_TYPE`] (`core.subnet.input@1`) for
//! each input and [`OUTPUT_TYPE`] (`core.output@1`) for each output. Both
//! are pass-through nodes with one input and one output called
//! [`INTERFACE_PORT`] (`value`), and both are *port-polymorphic*:
//! [`Graph::add_node_in`] retypes their ports to the declared type, which
//! [`Graph::port_type`] reports. Interface inputs exist only inside
//! subnets; `core.output` nodes in the root network mark sinks.
//!
//! **Links** connect nodes of the same network, or cross exactly one
//! subnet boundary through the interface (see [`Graph::link`]):
//!
//! * the input `value` of an interface input is fed from the network
//!   *around* its subnet (never from inside); its output is read inside;
//! * the output `value` of an interface output can be read inside its
//!   subnet and in the network around it.
//!
//! So a link from `/hour.out` into `/lamp/hour.value` (where `/lamp/hour`
//! is an interface input) and one from `/lamp/color.value` (an interface
//! output) to `/mix.a` are valid; documents write them as `lamp.hour` and
//! `lamp.color`, as if they were ports of the subnet.
//!
//! **Cooking** needs nothing new: boundary nodes are ordinary nodes and
//! cross-boundary links are ordinary links in the one flat dependency
//! graph, so the scheduler's push-based invalidation, caching, early
//! cutoff, time dependence and error containment apply across subnet
//! boundaries unchanged, with the same costs. Cooking a subnet node itself
//! yields no outputs; cook its interface outputs (or what reads them).
//!
//! **Promotion**: [`Graph::promote`] exposes an inner input as an input of
//! the subnet: it adds an interface input with the port's type and value
//! and links it to the inner port, so the value set on (or linked into)
//! the interface flows to the inner node. [`Graph::unpromote`] reverses it,
//! keeping the promoted value on the inner node.
//!
//! A **Charm** is a subnet with metadata ([`CharmMeta`]: name, version,
//! icon, required capabilities, minimum tier; [`Graph::set_charm`]) and its
//! declared interface. Removing a subnet removes its whole subtree;
//! [`Graph::move_node`] moves a node (or subnet) between networks.
//!
//! `ch()` paths are relative to the network of the node holding the
//! expression (`"x/port"` sibling, `"../x/port"` parent network,
//! `"/a/b/port"` absolute); see [`Graph::set_expression`].
//!
//! Not yet implemented (later milestones): Charm definitions shared by
//! several instances (each instance is a copy today) and parallel
//! evaluation.

mod cook;
mod graph;
mod network;
mod snapshot;
mod sources;

pub use cook::{
    CookError, CookStats, Cooker, DEFAULT_EVENT_LIMIT, EventError, ExternalEvent, FiredEvent,
    TickReport,
};
pub use graph::{Endpoint, Graph, GraphError, Link, NodeId, PortDirection};
pub use livspeil_params::expr::RefKind;
pub use network::{CharmMeta, INPUT_TYPE, INTERFACE_PORT, OUTPUT_TYPE, SUBNET_TYPE};
pub use snapshot::{GraphSnapshot, NamedLink, NodeSnapshot};
pub use sources::ExprRef;
