// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Document model, KDL serialization and migrations (milestone M0.4; see
//! `specs/livspeil-kernel-spec-v0.md`, section 10).
//!
//! A document is a directory package `Name.room/` holding `room.kdl`,
//! optionally `assets/` with `assets.lock` (see the [`assets`] module:
//! content-addressed files with recorded hash, size, source and license;
//! [`check_assets`] verifies them) and later `thumbnails/`. [`diff()`]
//! compares two documents semantically (nodes matched by id; see the
//! [`diff`](mod@diff) module). `room.kdl` is KDL v2:
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
//!   unset inputs use the type's default; expressions and animation are
//!   described below), then
//!   `link from="<node>.<output>" to="<node>.<input>"`. The port is the text
//!   after the last dot, so node names may contain dots.
//! - **Meta**: optional, last, editor-only data: `node "<name>" x=.. y=..`.
//! - **Values**: see the table in the `value` module docs: `1.0` is a
//!   Float and `1` an Int, `#true`/`#false`, quoted strings, and annotated
//!   strings for `(vec2)`, `(vec3)`, `(vec4)` and `(color)"oklch(L C H A)"`.
//!   NaN and infinities are rejected.
//!
//! # Input sources: values, expressions, animation
//!
//! An input can have up to three sources besides a link: a static value,
//! an expression and an animation channel. The graph keeps all of them and
//! uses the highest-priority one (link, expression, animation, static
//! value, spec default), so a document stores every source that is set, all
//! in the one `input` node of the port:
//!
//! ```kdl
//! node "lamp" type="core.math.lerp@1" id="01920000-0000-7000-8000-000000000002" {
//!   input "a" 0.2
//!   input "b" 0.5 expr="ch(\"sun/offset\") * 0.5" {
//!     anim pre="loop" post="ping-pong" offset=1.5 {
//!       key 0.0 0.2
//!       key 1.0 0.8 interp="bezier" tangent="explicit" in=0.0 out=1.0
//!       key 2.0 0.4 interp="spring" frequency=2.0 damping=0.5 tangent="flat"
//!       key 3.0 0.5 interp="step"
//!     }
//!   }
//!   input "t" expr="ch(\"sun/offset\")"
//! }
//! ```
//!
//! - `input "<port>" [<value>] [expr="<source>"] [{ anim ... }]`: at least
//!   one of the three. Canonical order is the order of the line: static
//!   value (positional), then `expr=`, then the `anim` child block.
//! - **Expressions** are stored verbatim as a KDL string (so quotes inside
//!   are escaped: `expr="ch(\"sun/offset\")"`; `ch("node/port")` reads an
//!   *input* of another node). [`parse`] does not compile
//!   them; [`Document::to_graph`] does, after every node exists, and reports
//!   unknown `ch()` paths, type errors and cycles at the `expr=` property.
//! - **Animation**: `anim [pre="<x>"] [post="<x>"] [offset=<seconds>]` with
//!   extrapolations `"hold"` (default), `"loop"` and `"ping-pong"`, and
//!   offset `0.0` by default. Its children are keys, in strictly increasing
//!   time: `key <time> <value>` plus optional properties
//!   - `interp=`: `"linear"` (default), `"step"`, `"bezier"`, or `"spring"`
//!     with the required `frequency=<Hz>` and `damping=<ratio>`;
//!   - `tangent=`: `"auto"` (default), `"flat"`, or `"explicit"` with the
//!     required `in=<slope>` and `out=<slope>` (values of the key's type).
//!
//!   Key values and slopes use the value encoding above and must be
//!   Float, Vec2, Vec3, Vec4 or Color, all of one type; integers are read
//!   as floats. Channel rules (see `livspeil_params::anim`) are checked by
//!   [`parse`] and reported at the offending `key`. The writer leaves out
//!   settings that are at their default and writes key properties in the
//!   order `interp`, `frequency`, `damping`, `tangent`, `in`, `out`.
//!
//! **Format version**: these additions are still format 1. They are
//! purely additive: every format 1 file written before them parses to the
//! same document and is written back identically, and the reserved
//! `expr=` was rejected before, so no older file can use it differently.
//! A build that predates them rejects such a file with a clear error
//! instead of losing data (the reader is strict), which is the same
//! outcome a format bump would give.
//!
//! **Names are references**: links, `ch()` paths and meta positions name
//! nodes by name. [`livspeil_graph::Graph::rename`] rewrites `ch()` paths
//! itself; a tool editing the document directly must rename all three.
//!
//! # Canonical form
//!
//! [`format()`] writes two-space indent, nodes sorted by id (UUIDv7, so
//! creation order; renames do not move nodes), inputs sorted by port name
//! (sources ordered as above), links sorted by `(from, to)` text, meta
//! sorted by node name,
//! floats in Rust's shortest round-trip form (`1.0`, `0.1`, `1e-7`),
//! strings always quoted with KDL escapes. `parse(format(doc)) == doc`
//! and formatting is idempotent. [`canonicalize`] additionally keeps a
//! leading block of `//` comments (license headers); other comments are
//! dropped.
//!
//! [`parse`] is syntactic and strict (unknown nodes and properties are
//! errors); [`Document::to_graph`] checks the document against a node
//! registry (node types, ports, value types, links, expressions).

mod anim;
pub mod assets;
mod convert;
pub mod diff;
mod error;
mod format;
pub mod migrate;
mod model;
mod package;
mod parse;
mod value;

pub use assets::{AssetEntry, AssetIssue, AssetsError, AssetsLock, IssueKind, Severity};
pub use diff::{Change, DocDiff, Op, Subject, diff};
pub use error::{DocError, Location};
pub use format::format;
pub use model::{
    CURRENT_FORMAT, DocAnim, DocExpr, DocLink, DocNode, Document, Meta, Network, ParseTierError,
    ROOT_NETWORK, SourceLoc, Tier,
};
pub use package::{
    AddedAsset, ROOM_FILE, add_asset, canonicalize, check_assets, package_dir, read_assets_lock,
    room_file, scan_assets, sha256_file, split_prolog, write_assets_lock,
};
pub use parse::parse;
