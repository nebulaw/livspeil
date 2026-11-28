<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Livspeil Dream Mode app, first slice (v0)

Status: **accepted** (2026-11-28, D69). The kernel's position commands
(section 3) are built; the app is not. Roadmap P2; D18
(Dream Mode first as an app), D66 (experience before polish).

## 1. What it is for

The first program that is Livspeil rather than a component gallery: open a
room, see its graph, change it and watch the values move. It connects the
kernel (graph, commands, documents, cooking) to Livspeil UI, which have never
met.

```sh
cargo run -p livspeil-dream -- examples/sun-and-lamp.room
```

## 2. Scope

Three slices, one pull request each.

### DM1: open and watch

- Open the room named on the command line.
- The node canvas shows the root network: real node names, ports, port types
  and links, at the positions of the document's `meta`; nodes without one are
  laid out automatically.
- A subnet or Charm instance is one node (not entered yet).
- An inspector beside the canvas shows the selected node: each input with its
  value and where it comes from (link, expression, animation, set value or
  default), each output with its current value (colors with a swatch).
- Transport: play, pause, one frame forward, back to start; the frame number
  and time. Cooking follows the frame clock of kernel spec section 5.
- A node that fails to cook is marked, with the error in the inspector.

Done when the lamp color and intensity of `sun-and-lamp.room` visibly change
through a simulated day.

### DM2: edit

- Move nodes. Connect and disconnect (dragging off an input); only links the
  kernel would accept are offered.
- Edit unconnected inputs in the inspector: numbers, booleans, strings,
  vectors and colors as number fields; expressions as text, with the kernel's
  error shown. Animation channels are shown, not edited.
- Rename a node (its position entry and `ch()` paths follow).
- Undo and redo. A drag or a scrub is one step.
- Save, keeping comments as `livspeil fmt` does. An unsaved marker in the
  title; closing with unsaved changes asks.

Done when a room edited and saved by the app passes `livspeil validate` and
`livspeil diff` reports exactly the edits made.

### DM3: build

- Add a node from a searchable list of the registered node types.
- Delete the selection.
- New empty room; save as.

Done when `sun-and-lamp.room` can be rebuilt from an empty room in the app.

### Not in these slices

Entering subnets and making Charms; the 3D viewport (Bevy); timeline and
animation editing; assets; scripts and WASM packages; dashed `ch()`/`out()`
wires; a file chooser (rooms are named on the command line, "save as" takes a
typed path); visual polish (D66).

## 3. Design

New crate `crates/livspeil-dream` (GPL-3.0-or-later, library and binary). It
depends on kernel and UI crates; neither side learns about the other.

- **Session**: the one owner of the document. Holds the `Graph` (which holds
  the node positions), the history, the cooker and the clock. All changes go
  through it as commands, so the canvas, inspector and menus cannot disagree.
  It is plain Rust with no UI dependency, tested headlessly.
- **Canvas adapter**: implements `NodeGraphModel` over the session. Maps
  kernel node ids to canvas ids, and turns the `DocChange` events of each
  command into `changes_since`, so an edit costs what changed (frame pipeline
  P4).
- **Inspector**: a view of the session's selected node, built from the node's
  spec.
- **Cooking**: on the UI thread, once per frame while playing, only for what
  is shown (the selected node and the sinks); the kernel's caching does the
  rest. Moving it off the thread waits for a graph that needs it.
- **Errors** are shown where they happen. No `unwrap`; a bad file is a
  message, not a crash.

Needed in existing crates:

| Crate | Addition |
|---|---|
| `livspeil-graph`, `livspeil-cmd`, `livspeil-doc` | node positions as graph data with a `SetPosition` command (built) |
| `livspeil-graph` | ask whether a link is allowed without making it |
| `livspeil-doc` | write a changed document keeping the comments of the old text |
| `livspeil-ui-widgets` | NodeCanvas: disconnect, delete requests, marking a failed node |

## 4. Testing

- Session: unit tests for every edit with undo and redo, and a differential
  test that random edits then save and reload give the same graph.
- App: headless `TestHost` tests of the flows above, and golden screenshots
  of the window with each example room.
- The `DemoGraph` canvas tests stay; the adapter gets the same
  retained-against-fresh test over random commands.

## 5. Settled (2026-11-28)

1. **Node moves are commands.** Positions are graph data with a
   `SetPosition` command, so a move is undone and journaled like any edit.
   They take no part in evaluation (kernel spec, section 3).
2. **Live values** are shown in the inspector only in these slices.
3. The crate and binary are `livspeil-dream`.
