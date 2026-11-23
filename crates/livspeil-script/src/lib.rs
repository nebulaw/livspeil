// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Script workers: scripts (Python first) run **out of process** and talk
//! to the kernel over a line-based JSON protocol on their stdin and
//! stdout (milestone M0.14, decisions D5 and D13). The protocol is
//! specified in `docs/script-protocol.md`.
//!
//! - A script never gets in-process access. It reads the graph with
//!   queries (`graph.nodes`, `graph.node`, `graph.snapshot`), cooks with
//!   `graph.cook`, and changes the document only with `graph.apply`, whose
//!   payload is the serde form of [`livspeil_cmd::Command`]. Every applied
//!   command goes through [`livspeil_cmd::History`] with
//!   [`Origin::Script`](livspeil_cmd::Origin::Script), so it is
//!   validated, undoable and journaled ([`Scene::journal`]) like a UI edit.
//! - Each session has explicit [`Capabilities`] (`graph.read`,
//!   `graph.write`, `graph.cook`, `fs.read:<root>`), checked on every
//!   request; a missing one is a `capability_denied` error answer, not a
//!   crash. Capabilities scope the kernel API only: they are not an OS
//!   sandbox (see the protocol document).
//! - Limits ([`Limits`]): handshake and per-request timeouts (a timed-out
//!   host request is cancelled, then the worker killed), a total run time,
//!   a maximum message size, a cap on kept logs. The worker's environment
//!   is cleared except for an allow-list.
//! - [`ScriptSession`] is synchronous: three pipe threads per worker and
//!   channels; the caller's thread serves requests with bounded waits. No
//!   async runtime.
//!
//! ```no_run
//! # use livspeil_script::{Capabilities, Capability, Scene, ScriptConfig, ScriptSession};
//! # fn demo(graph: livspeil_graph::Graph) -> Result<(), livspeil_script::ScriptError> {
//! let mut scene = Scene::new("My Room", graph);
//! let caps = Capabilities::none()
//!     .with(Capability::GraphRead)
//!     .with(Capability::GraphWrite);
//! let mut session = ScriptSession::spawn(ScriptConfig::python("dim.py".as_ref(), caps))?;
//! session.run(&mut scene)?;
//! session.shutdown(&mut scene)?;
//! println!("{} commands applied", scene.journal.len());
//! # Ok(()) }
//! ```

mod capability;
mod error;
mod host;
mod process;
pub mod protocol;
mod session;

pub use capability::{Capabilities, Capability, CapabilityParseError};
pub use error::ScriptError;
pub use host::Scene;
pub use process::WorkerCommand;
pub use protocol::{PROTOCOL_VERSION, RemoteError};
pub use session::{
    Limits, LogEntry, LogLevel, Outcome, ScriptConfig, ScriptSession, SessionState, python_sdk_dir,
};
