<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Livspeil Kernel Spec v0.3 (as built)

Status: updated 2026-11-28 (node positions, D69). Phase 0 is complete.
The kernel is headless and cross-platform (macOS and Linux) and never depends
on Bevy, wgpu, Wayland or UI crates. Code comments cite this spec by section
number, so keep the numbers stable.

Module documentation in each crate is the detailed design; this spec states
the contract.

## 1. Goals and non-goals

Goals: one typed graph model for scenes, Charms, materials, animation and
logic; lazy, cached, deterministic evaluation; every mutation a serializable
command; a text-first, diffable document format; fully testable without a
GPU; third-party code only out of process or sandboxed.

Non-goals in v0: audio clock, rendering, networking, marketplace,
collaboration, parallel evaluation, native third-party nodes.

## 2. Crates

| Crate | Purpose | License |
|---|---|---|
| `livspeil-node-api` | `Value`, `PortType`, `Table`, `Curve`, events, system bindings, type variables, `NodeSpec`, `NodeImpl`, `NodeRegistry`; optional `serde` | MPL-2.0 (D33) |
| `livspeil-graph` | graph, subnets, Charms, type and cycle checks, input sources, scheduler (`Cooker`) | GPL-3.0-or-later |
| `livspeil-params` | expression language (`expr`), animation channels (`anim`), curve sampling | GPL |
| `livspeil-cmd` | commands with inverses, `History`, JSON Lines journal | GPL |
| `livspeil-doc` | KDL room documents, migrations, diff, assets | GPL |
| `livspeil-nodes-core` | built-in nodes and their spec files | GPL |
| `livspeil-script` | out-of-process script workers | GPL |
| `livspeil-wasm-host` | sandboxed WebAssembly node packages | GPL |
| `livspeil-cli` | `livspeil validate / fmt / cook / diff / assets / run` | GPL |
| `livspeil-testkit` | golden-file harness | GPL |

Outside `crates/`: the Python SDK (`sdk/python`, MPL-2.0) and the WASM node
interface (`wit/livspeil-node.wit`).

## 3. Data model

- Node ids are UUIDv7 (`NodeId`); names are unique per network.
- A node has a type (`core.math.add@1`), named typed input and output ports,
  and per input up to four sources.
- Links connect an output to an input; one link per input; value links form
  no cycles.
- Storage is flat: every node of every network lives in one `Graph` and
  records its parent subnet.
- **Subnets** (`core.subnet@1`) own a child network. Their interface is one
  boundary node per port (`core.subnet.input@1`, `core.output@1`). A link
  may cross one subnet boundary through the interface. Root `core.output`
  nodes mark sinks.
- **Promotion** exposes an inner input as an input of its subnet.
- **Layout**: a node may have an editor position. It is edited and saved like
  the rest of the graph but takes no part in evaluation. Coordinates are
  within ±10,000,000. Nodes inside a Charm
  instance have none, and definitions carry none.
- A **Charm** is a subnet with metadata (`CharmMeta`: name, version, icon,
  required capabilities, minimum tier). Metadata does not affect cooking.
- **Shared Charm definitions**: a graph holds a library of `CharmDef`s. An
  instance (`core.charm@1`) materializes the definition's nodes with ids
  derived from the instance id. Only interface input sources are
  per-instance; editing a definition changes every instance; an instance can
  be detached into a subnet. Definitions cannot contain instances yet.

## 4. Values and types

- `Value`: Float, Int, Bool, Vec2/3/4, Color (OKLCH + alpha), String, Table
  (typed columns; also used for lists), Curve (float, vector or color keys),
  Event (a placeholder in value cooking).
- `PortType` also reserves Enum, Text, Path, Image, Geometry, Material,
  Scene and Window; no value carries them yet.
- Only `Int -> Float` converts implicitly.
- **Polymorphic ports**: a node type may declare type variables with a
  constraint class (`Numeric`, `Vector`, `Interpolable`, `Any`). Each
  instance resolves them from its links, values, expressions and animation,
  or from a type pinned at creation.

## 5. Evaluation

- Input source priority: link > expression > animation > static value > spec
  default.
- Values are pulled lazily and cached. Events are pushed, once per tick, in
  the phase order **input -> events -> cook -> publish**.
- Clock: frame, time, dt (`CookContext`). Time-dependent nodes: spec flag,
  expressions using `$F/$T/$DT`, or animated inputs.
- Scheduler: every edit gets an epoch and marks the node and its downstream
  (links, `ch()` and `out()` readers) dirty; each cooker keeps its own
  validation epochs, so several cookers per graph are correct; cached
  evaluation order; dense slots; early cutoff when outputs do not change.
- Node errors and panics are contained; the last good output is kept.
- **Events** (D49): event links are not value dependencies and may form
  loops. Event nodes keep runtime state per cooker, and a state change
  invalidates their values downstream. State is never saved. Delivery is
  FIFO in a stable order, cut at a per-tick limit (default 10,000) and
  reported.
- **System bindings**: a cooker may hold a read-only `SystemHost`; nodes
  that read it are re-cooked only when its revision changes.
- Targets on 10k nodes: mark dirty < 0.2 ms, cold cook < 5 ms
  single-threaded. Both met on a shared VM (0.7–112 µs and 1.7–2.6 ms); see
  `docs/benchmarks.md`.

## 6. Expressions and animation

- Language: Pratt parser + bytecode VM; Int/Float/Bool/vectors/colors/
  strings; `?:` and `if()`; swizzles; `$F $T $DT`; about 40 functions; Int
  `/` and `%` floored; IEEE floats; deterministic via `libm`; nesting capped
  at 256, source at 64 KiB.
- `ch("node/port")` reads another node's resolved **input** (D45);
  `out("node/port")` reads its computed **output** (D48). Both are edges for
  cycle checks and invalidation. Renaming rewrites the path text; deleting a
  referenced node is refused. `Graph::expression_refs()` lists them.
- Paths are relative to the network of the node holding the expression.
- Table and Curve values are not expression types.
- Animation channels: step, linear, bezier (auto, flat or explicit
  tangents), spring; hold/loop/ping-pong; offset; Float, Vec and Color.

## 7. Commands

- Primitive edits: `CreateNode`, `DeleteNode`, `SetInput`, `ClearInput`,
  `Link`, `Unlink`, `Rename`, `SetExpression`, `ClearExpression`,
  `SetAnimation`, `ClearAnimation`, `Transaction`.
- Layout: `SetPosition` (a missing position unplaces). Commands that delete or
  rebuild nodes restore positions in their inverses.
- Networks and Charms: `CreateNodeIn`, `CreateSubnet`, `AddInterfacePort`,
  `RemoveInterfacePort`, `MoveNode`, `MoveNodeRewired`, `Promote`,
  `Unpromote`, `SetCharm`.
- Definitions: `AddDefinition`, `RemoveDefinition`, `UpdateDefinition`,
  `Instantiate`, `Detach`, `Attach`, `MakeDefinition`.
- `apply` returns the exact inverse and `DocChange` events, or fails and
  leaves the graph unchanged. `History` coalesces by group. The journal
  records do/undo/redo with an origin and replays to the same state. A step
  that only places nodes keeps one entry per node, however long the drag.
- Edits inside a Charm instance are refused.

## 8. Core nodes

55 spec files; `docs/nodes.md` is the generated reference.

| Group | Nodes |
|---|---|
| math | add, sub, mul, div, lerp, clamp, remap, min, max, smoothstep, abs, pow, sin, cos, wrap, round |
| logic | select, compare, and, or, not |
| vector | compose3, decompose3, length3, normalize3, dot3 |
| time, sky | clock, timeofday, sun_direction |
| color, text | mix, compose, gradient; format |
| table, curve | count, column, get, get_value, filter_text, sort; ramp |
| fs | list_dir (the only node that reads the system) |
| event | every, on_change, counter, toggle, latch, gate, select |
| structure | core.subnet, core.subnet.input, core.output, core.charm |

`core.logic.select`, `core.math.lerp` and `core.curve.ramp` have polymorphic
`@2` versions; `@1` stays registered.

Rules: nodes are deterministic pure functions of inputs, params, time (if
flagged), seeded RNG and, for event nodes, runtime state. Errors are typed
`NodeError`s. `core.sky.sun_direction` rejects `day_of_year` outside 1..=366
(D43).

## 9. Node specs

The Rust `NodeSpec` is the source of truth (D50). It is mirrored by
`crates/livspeil-nodes-core/specs/<id>.node.toml`
(`<id>@<version>.node.toml` beyond version 1): a generated header plus a
hand-written `description` and `[[tests]]`. A test fails when they differ;
`cargo xtask node-specs` refreshes them. Incompatible changes to ports or
behaviour bump the node's version.

## 10. Documents

`Name.room/room.kdl` (KDL v2, D31), `format=1`:

```kdl
room "Sun and Lamp" format=1 min-tier="T1" {
  network "/" {
    node "lamp" type="core.math.lerp@1" id="0192...-uuid" {
      input "a" 0.2
      input "b" 0.5 expr="ch(\"sun/offset\") + 0.1" {
        anim post="loop" { key 0.0 0.2 interp="bezier"; key 2.0 0.8 }
      }
    }
    link from="sun.phase" to="lamp.t"
  }
  meta { node "lamp" x=360.0 y=40.0 }
}
```

- Also stored: tables and curves (blocks), subnets (nested `network`,
  `charm` metadata, `port-type`), shared definitions (a `charms` block and
  `def=` on instances).
- Canonical writer (nodes by id, inputs by port, links sorted), strict
  reader with line:column errors, comment-preserving `fmt`.
- Migrations with a frozen format-1 fixture; purely additive syntax stays
  format 1 (D44).
- `assets/` + `assets.lock`: content-addressed files with sha256, size, SPDX
  license, source and author.
- `meta` is the layout: loading places the graph's nodes, saving writes their
  positions under their paths. An entry naming no node is an error. A root
  name containing `/` wins over the nested path it spells; saving a nested
  position hidden that way is an error.
- `livspeil diff` matches nodes by id.
- `livspeil cook` sinks (D46): root `core.output` nodes, otherwise nodes
  without outgoing links.

## 11. Third-party code

**Scripts** (D5, D13; `docs/script-protocol.md`): out of process, one JSON
object per line on stdin/stdout, protocol version 1. Scripts read through
queries and write only through commands (journaled with origin `script`).
Capabilities: `graph.read`, `graph.write`, `graph.cook`, `fs.read:<dir>`;
none by default. Timeouts, run time and message size are limited.
Capabilities scope the kernel API; they are **not** an OS sandbox yet.
Python is implemented; TypeScript and Lua are not.

**WASM node packages** (D13, D52; `docs/wasm-nodes.md`): core wasm modules
with no imports, on wasmtime 48 LTS. Every call runs in a fresh instance
with fuel, memory and stack limits and canonical NaNs. Ports: Float, Int,
Bool, Vec2/3/4, Color, String. Ids are namespaced outside `core.*`.

**Reserved**: a Bevy adapter mapping cooked `Scene` outputs to ECS (P2).

## 12. Testing

Unit tests per node, golden files (cook, expressions, animation, documents,
CLI), property tests (commands, documents, expressions), randomized
differential tests for scheduler caching (long-lived vs fresh cookers), spec
file tests, criterion benchmarks. CI on macOS and Linux with latest stable,
plus a Rust 1.95 MSRV job (D52).

## 13. Tiers and Charm metadata

Rooms and Charms declare a minimum hardware tier (`T0`, `T1`, `T2`) and
Charms declare the capabilities they need. The kernel stores, serializes and
diffs both; nothing enforces them yet.

## 14. Milestones

Done: workspace, graph and scheduler (M0.1–M0.2b), commands (M0.3), KDL
documents (M0.4), expressions and animation (M0.5), nodes and `cook`
(M0.6), subnets and Charms (M0.7), tables, curves and fs (M0.8), diff and
assets (M0.9), events (M0.10), `out()` (M0.11), node spec files (M0.13),
script workers (M0.14), WASM host (M0.15), shared definitions (M0.16),
polymorphic ports (M0.17).

Open: instances inside definitions; enforcement of tiers and capabilities;
OS sandboxing of scripts; `validate` and `diff` with WASM packages;
re-measuring benchmarks on real hardware.
