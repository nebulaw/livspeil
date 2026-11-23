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
//!   top-level node. It holds an optional `charms` block (see below), the
//!   network and an optional `meta` block, in that order. `format` is required; `min-tier` defaults to `T0`.
//! - **Network**: exactly one in the room, `network "/"` (subnets nest
//!   theirs, see below). It holds
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
//!   NaN and infinities are rejected. Tables and curves are blocks (see
//!   below).
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
//! # Table and curve values
//!
//! `Table` and `Curve` values (M0.8) are not single KDL values; they are
//! the static value of an input written as a child block, before any
//! `anim` block:
//!
//! ```kdl
//! node "shelf" type="core.table.sort@1" id="01920000-0000-7000-8000-000000000003" {
//!   input "table" {
//!     table {
//!       column "name" type="String"
//!       column "pages" type="Int"
//!       row "Dune" 412
//!       row "Emma" 474
//!     }
//!   }
//! }
//! node "fade" type="core.curve.ramp@1" id="01920000-0000-7000-8000-000000000004" {
//!   input "curve" {
//!     curve post="loop" {
//!       key 0.0 0.0
//!       key 0.5 1.0 interp="bezier"
//!       key 1.0 0.0
//!     }
//!   }
//! }
//! ```
//!
//! - `table`: no arguments or properties. First every
//!   `column "<name>" type="<Type>"` (names unique and not empty; types
//!   `Float`, `Int`, `Bool`, `Vec2`, `Vec3`, `Vec4`, `Color`, `String`),
//!   then one `row <value>...` per row with exactly one value per column in
//!   the usual value encoding (an integer is accepted in a `Float` column).
//!   A table without columns is written `table` alone.
//! - `curve [pre="<x>"] [post="<x>"]`: the keys of an `anim` block
//!   (`key <position> <value>` with the same `interp=` and `tangent=`
//!   settings) holding numbers only; positions strictly increase; no
//!   `offset=`. Default settings are left out, as in `anim`.
//! - An input has at most one static value: a positional value or one
//!   block. Errors point at the offending `column`, `row` cell or `key`.
//!   Canonical form: block value, then `anim`.
//!
//! **Format version**: these additions are still format 1. They are
//! purely additive: every format 1 file written before them parses to the
//! same document and is written back identically, and the reserved
//! `expr=` was rejected before, so no older file can use it differently.
//! A build that predates them rejects such a file with a clear error
//! instead of losing data (the reader is strict), which is the same
//! outcome a format bump would give.
//!
//! # Subnets and Charms
//!
//! A subnet is a node of type `core.subnet@1` whose block holds, after its
//! `input`s, an optional `charm` block and its `network` (no path; bare
//! `network` when empty). Its interface is declared by boundary nodes
//! inside, one per port, named after the port and typed with
//! `port-type="<Type>"`: `core.subnet.input@1` for inputs, `core.output@1`
//! for outputs. A promoted parameter is an interface input linked to the
//! inner port; its `input "value"` holds the parameter's value (or
//! expression, or animation) for this instance.
//!
//! ```kdl
//! node "lamp" type="core.subnet@1" id="0193a6c0-1d2e-7b40-8000-000000000010" {
//!   charm "Daylight" version="1.0.0" icon="sun" min-tier="T1" {
//!     capability "time"
//!   }
//!   network {
//!     node "hour" type="core.subnet.input@1" id="0193a6c0-1d2e-7b40-8000-000000000011" port-type="Float" {
//!       input "value" 12.0
//!     }
//!     node "level" type="core.math.smoothstep@1" id="0193a6c0-1d2e-7b40-8000-000000000012"
//!     node "intensity" type="core.output@1" id="0193a6c0-1d2e-7b40-8000-000000000013" port-type="Float"
//!     link from="hour.value" to="level.x"
//!     link from="level.out" to="intensity.value"
//!   }
//! }
//! link from="clock.time" to="lamp.hour"
//! link from="lamp.intensity" to="out.value"
//! ```
//!
//! - Names are unique per network; ids are unique in the whole document.
//!   Names of subnets and of nodes inside them cannot contain `/` and
//!   cannot be `.` or `..`.
//! - A link lives in the network where both of its ends are visible. In
//!   the network around a subnet, `lamp.hour` names the subnet's interface
//!   port `hour` (the `value` input of its `core.subnet.input@1` node, or
//!   the `value` output of its `core.output@1` node).
//! - `ch()` paths are relative to the network of the node holding the
//!   expression: `"x/port"` sibling, `"../x/port"` parent network,
//!   `"/a/x/port"` absolute (see `livspeil_graph::Graph::set_expression`).
//! - `charm "<name>" [version=".."] [icon=".."] [min-tier="T<n>"]` with
//!   `capability "<name>"` children (in order, no duplicates); `min-tier`
//!   defaults to `T0` and is left out then.
//! - Meta positions of nodes inside subnets are keyed by their path
//!   without the leading `/` (`lamp/hour`).
//! - `charm`, `network` in a non-subnet node and unknown port types are
//!   errors with line and column.
//!
//! Still **format 1**: subnets are purely additive, since a nested
//! network, `charm` and `port-type=` were rejected before, so every older
//! file reads and writes back identically.
//!
//! # Shared Charm definitions (M0.16)
//!
//! A Charm can be defined once in the room's `charms` block (written
//! first, before `network "/"`; definitions sorted by id) and used by any
//! number of instances (`core.charm@1` nodes):
//!
//! ```kdl
//! charms {
//!   def "daylight" version="1.0.0" name="Daylight" icon="sun" min-tier="T1" {
//!     capability "time"
//!     network {
//!       node "hour" type="core.subnet.input@1" id="0193a6c0-1d2e-7b40-8000-000000000011" port-type="Float" {
//!         input "value" 12.0
//!       }
//!       node "level" type="core.math.smoothstep@1" id="0193a6c0-1d2e-7b40-8000-000000000012"
//!       node "intensity" type="core.output@1" id="0193a6c0-1d2e-7b40-8000-000000000013" port-type="Float"
//!       link from="hour.value" to="level.x"
//!       link from="level.out" to="intensity.value"
//!     }
//!   }
//! }
//! network "/" {
//!   node "desk" type="core.charm@1" id="0193a6c0-1d2e-7b40-8000-000000000020" def="daylight@1.0.0"
//!   node "porch" type="core.charm@1" id="0193a6c0-1d2e-7b40-8000-000000000021" def="daylight@1.0.0" {
//!     input "hour" 18.0
//!   }
//!   link from="desk.intensity" to="out.value"
//! }
//! ```
//!
//! - `def "<id>" version="<v>" [name=".."] [icon=".."] [min-tier="T<n>"]`
//!   with `capability` children, then its `network` (like a subnet's).
//!   `name` defaults to the id and is left out when equal to it. Node ids
//!   inside a definition are local to it; the graph derives each
//!   instance's node ids from the instance id and the local id
//!   (`livspeil_graph::instance_node_id`), so they are never written.
//! - A definition is self-contained: its links and `ch()`/`out()` paths
//!   stay inside it, and it cannot contain instances (yet).
//! - An instance is `node "<name>" type="core.charm@1" id=".."
//!   def="<id>@<version>"`; it has no `charm` or `network` block. Its
//!   `input "<port>"` lines name *interface inputs* of the definition and
//!   hold only the sources (value, expression, animation) that differ
//!   from the definition's defaults (the overrides); everything else
//!   follows the definition. Instances cannot change anything else
//!   inside: edit the definition, or detach the instance into a subnet.
//! - Links name an instance's ports like a subnet's (`desk.intensity`).
//! - Errors with line and column: an unknown definition, a version that
//!   is not the definition's, `def=` on another node type or missing on an
//!   instance, an input the definition's interface lacks, duplicate
//!   definitions, instances inside a definition, and every graph error
//!   inside a definition's network.
//!
//! Still **format 1**: purely additive (`charms` and `def=` were rejected
//! before), so every older file reads and writes back identically.
//!
//! **Names are references**: links, `ch()` paths and meta positions name
//! nodes by name. [`livspeil_graph::Graph::rename`] rewrites `ch()` paths
//! itself; a tool editing the document directly must rename all three.
//!
//! # Canonical form
//!
//! [`format()`] writes two-space indent, nodes sorted by id within each
//! network (UUIDv7, so
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
    CURRENT_FORMAT, DocAnim, DocCharmDef, DocDefRef, DocExpr, DocLink, DocNode, Document, Meta,
    Network, ParseTierError, ROOT_NETWORK, SourceLoc, Tier,
};
pub use package::{
    AddMode, AddedAsset, ROOM_FILE, add_asset, add_asset_with, canonicalize,
    canonicalize_assets_lock, check_assets, package_dir, read_assets_lock, room_file, scan_assets,
    sha256_file, split_prolog, write_assets_lock,
};
pub use parse::parse;
