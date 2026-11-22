// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Document model, KDL serialization and migrations (milestone M0.4; see
//! `specs/livspeil-kernel-spec-v0.md`, section 10).
//!
//! A document is a directory package `Name.room/` holding `room.kdl` (later
//! also `assets/`, `assets.lock` and `thumbnails/`). `room.kdl` is KDL v2:
//!
//! ```kdl
//! room "My Room" format=1 min-tier="T1" {
//!   network "/" {
//!     node "sun" type="core.time.timeofday@1" id="01920000-0000-7000-8000-000000000001" {
//!       input "offset" 0.0
//!     }
//!     node "lamp" type="core.color.mix@1" id="01920000-0000-7000-8000-000000000002" {
//!       input "a" (color)"oklch(0.8 0.12 60.0 1.0)"
//!     }
//!     link from="sun.phase" to="lamp.t"
//!   }
//!   meta {
//!     node "sun" x=120.0 y=40.0
//!   }
//! }
//! ```
//!
//! - **Header**: `room "<name>" format=<int> min-tier="T<n>"`; the only
//!   top-level node. `format` is required; `min-tier` defaults to `T0`.
//! - **Network**: exactly one, `network "/"` (subnets come later). It holds
//!   `node "<name>" type="<id@version>" id="<uuid>"` entries, each with
//!   optional `input "<port>" <value>` children (explicitly set inputs only;
//!   unset inputs use the type's default), then
//!   `link from="<node>.<output>" to="<node>.<input>"`. The port is the text
//!   after the last dot, so node names may contain dots.
//! - **Meta**: optional, last, editor-only data: `node "<name>" x=.. y=..`.
//! - **Values**: see the table in the `value` module docs: `1.0` is a
//!   Float and `1` an Int, `#true`/`#false`, quoted strings, and annotated
//!   strings for `(vec2)`, `(vec3)`, `(vec4)` and `(color)"oklch(L C H A)"`.
//!   NaN and infinities are rejected.
//! - **Reserved**: `input "x" expr="..."` (expressions, M0.5) is rejected
//!   with a clear error until the format supports it.
//!
//! Canonical form ([`format`]): two-space indent, nodes sorted by id
//! (UUIDv7, so creation order; renames do not move nodes), inputs sorted by
//! port name, links sorted by `(from, to)` text, meta sorted by node name,
//! floats in Rust's shortest round-trip form (`1.0`, `0.1`, `1e-7`),
//! strings always quoted with KDL escapes. `parse(format(doc)) == doc`
//! and formatting is idempotent. [`canonicalize`] additionally keeps a
//! leading block of `//` comments (license headers); other comments are
//! dropped.
//!
//! [`parse`] is syntactic and strict (unknown nodes and properties are
//! errors); [`Document::to_graph`] checks the document against a node
//! registry (node types, ports, value types, links).

mod convert;
mod error;
mod format;
pub mod migrate;
mod model;
mod package;
mod parse;
mod value;

pub use error::{DocError, Location};
pub use format::format;
pub use model::{
    CURRENT_FORMAT, DocLink, DocNode, Document, Meta, Network, ParseTierError, ROOT_NETWORK,
    SourceLoc, Tier,
};
pub use package::{ROOM_FILE, canonicalize, room_file, split_prolog};
pub use parse::parse;
