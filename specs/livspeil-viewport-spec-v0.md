<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Livspeil 3D viewport (v0)

Status: **proposed** (2026-11-29). Nothing here is built. Section 9 lists
what has to be decided. Roadmap P2.

## 1. What it is for

Dream Mode edits a graph; it does not yet show the room the graph describes.
The viewport shows it: a 3D picture beside the node canvas that changes as
nodes are edited and as the clock runs.

Done when a room built in the app shows objects lit by a sun that moves
through the day and a lamp whose colour and strength its nodes drive.

## 2. What exists and what is missing

- The kernel reserves the port types `Geometry`, `Material` and `Scene`
  (kernel spec, section 4), but no value carries them. Nothing can say what
  is in a room.
- The kernel spec reserves "a Bevy adapter mapping cooked `Scene` outputs to
  ECS" (section 11). D11: Bevy, pinned, behind our interface.
- The UI spec names a `Surface` primitive and a `Viewport` widget; neither
  is built. Its Bevy host (the UI drawn as a node of Bevy's render graph) is
  meant for the World of the OS and is not built.
- `core.sky.sun_direction` already gives a unit vector with +y up, +x east
  and +z south, which is a right-handed, y-up frame, as Bevy's is.
- Bevy 0.20.1 (2026-10-28) uses wgpu 30, as Livspeil UI does. It needs Rust
  1.97.1; the workspace's minimum is 1.95, and CI checks the whole workspace
  with it.

## 3. How Bevy and the UI meet

**The app keeps its window and loop. Bevy renders off screen into a
texture on the UI's GPU device, and the UI draws that texture.**

- Bevy is given the host's instance, adapter, device and queue
  (`RenderCreation::manual`), so nothing is copied between devices.
- It runs without its window, input, UI and audio plugins and without
  pipelined rendering. The app calls it only when the picture must change.
- Livspeil UI gets a `Surface`: an element showing an external texture,
  clipped and composited like any other. It knows nothing about Bevy.

Why not the UI spec's way (the UI inside Bevy's frame): the editor would
then run on Bevy's loop and schedule, every UI frame would cost a Bevy
frame, and an app with no 3D view would still need Bevy. The UI's retained
frame pipeline (D65) stays as it is this way. The World, where a scene fills
the screen behind windows, is a different program and can still use the
other way.

**A frame is rendered** when the cooked scene changed, the view camera
moved, the viewport changed size, or the clock is playing. Otherwise the
last texture is shown and nothing is drawn.

## 4. How a room says what is in it

**Scenes are values, made by nodes.** A `Scene` value is an immutable,
cheap-to-clone description in plain data: no Bevy types, so the kernel stays
headless and a scene can be checked in a test or printed by the CLI.

A scene is a list of items:

| Item | Fields |
|---|---|
| object | shape (box, sphere, plane, cylinder) with its size; transform; material (base colour, roughness, metallic, emissive colour and strength) |
| sun | direction towards the sun, colour, illuminance in lux |
| point light | position, colour, power in lumens, range |
| camera | position, target, vertical field of view |
| environment | background colour, ambient colour and strength |

New nodes (each with its spec file and golden cases, as any node):

| Node | In | Out |
|---|---|---|
| `core.scene.shape` | kind, size, position, rotation, scale, colour, roughness, metallic, emissive, emissive strength | scene |
| `core.scene.sun` | direction, colour, illuminance | scene |
| `core.scene.point_light` | position, colour, power, range | scene |
| `core.scene.camera` | position, target, fov | scene |
| `core.scene.environment` | background, ambient colour, ambient strength | scene |
| `core.scene.merge` | four scenes, each optional | scene |

- Units: metres, degrees, lux, lumens. Colours are OKLCH like every Livspeil
  colour; the adapter converts them.
- A root `core.output` of type `Scene` is what the viewport shows. With
  several, they are merged in node order. With none, the viewport says so.
- `merge` takes a fixed number of inputs because a node's ports are fixed
  and an input takes one link. Larger scenes chain merges. A variable number
  of inputs is a kernel change of its own and is not part of this.
- `Scene` values are only computed. They are never static values in a
  document, so the room format does not change. Scripts and WASM packages
  cannot read or make them yet.
- A scene is bounded (a fixed maximum number of items) so that a graph
  cannot ask for unlimited memory; past it, the node reports an error.

Rejected: nodes that name an engine object and set its properties (nodes
must be pure), and a fixed scene whose lamp is driven by an output of a
certain name (it would show only what the code already knows).

## 5. The adapter

New crate `crates/livspeil-world` (GPL-3.0-or-later): cooked scene in,
picture out. It depends on Bevy and the scene data; the kernel and the UI
crates depend on neither it nor Bevy.

- It keeps one engine entity per scene item and updates what differs, so a
  moving sun changes one transform and does not rebuild the scene. Items are
  matched by their place in the list and their kind.
- Meshes and materials are shared between equal items.
- Its interface is ours (D11): set the scene, set the view camera, set the
  size, render into this texture. Bevy types do not leave the crate.

## 6. In Dream Mode

- The viewport is beside the node canvas, with a splitter between them; it
  can be hidden.
- Orbit with a drag, pan with Shift+drag, zoom by scrolling; **Frame all**.
  This view camera is the app's, not the room's, and is not saved. **Room
  camera** looks through the room's camera node instead.
- Selecting and moving objects in the viewport is not part of this.
- If the GPU cannot run Bevy, the app says so where the viewport would be
  and everything else works.

## 7. Slices

One pull request each.

- **V0: spike.** Throwaway code, kept on its branch: a lit cube rendered by
  Bevy on the host's device and shown in the Dream window. Measured on the
  owner's machine: it works at all; frame time at 1280×720 and 2560×1440;
  start-up time; release binary size; clean build time and disk. The device
  features and limits Bevy needs are written down: the host now asks for
  WebGL-level limits, which will not be enough. This answers whether
  section 3 holds before anything depends on it.
- **V1: Surface** in `livspeil-ui-*`, with a host option for the device
  features and limits an app needs. The CPU reference renderer draws a
  placeholder, so goldens stay deterministic.
- **V2: scene values and nodes** in the kernel, headless. `livspeil cook`
  prints a scene. A new example room.
- **V3: the viewport** in Dream Mode with `livspeil-world`.

V2 does not depend on V0 or V1.

## 8. Testing

- Scene nodes: golden cases; invalid inputs; the item limit.
- Adapter: tests of the engine's entities after setting and changing scenes,
  without a GPU, including that an unchanged item is not touched.
- Rendering: a test on a real GPU, run locally, checking coarse properties
  (the lamp's side is brighter, its colour dominates). CI has no GPU, so
  pixels are not goldens.
- UI: `Surface` goldens with the placeholder; headless app tests that the
  viewport asks for a frame only when it should.

## 9. To decide

1. **How Bevy and the UI meet** (section 3). Proposed: Bevy off screen on
   the UI's device, the app owns the loop.
2. **How a room describes a scene** (section 4). Proposed: `Scene` values.
3. **Rust version.** Proposed: raise the workspace minimum to 1.97.1.
   Alternative: keep 1.95 for the kernel and UI and leave the Bevy crates
   out of that CI job, which keeps two versions to look after for crates
   that no outside user depends on yet.
4. **First vocabulary** (the tables in section 4): is anything needed from
   the start that is missing, such as importing glTF models? Proposed: not
   yet; shapes first, models after.
5. **Order.** Proposed: V0 first, since it is the part that could fail.

## 10. Risks

- Sharing a device is a path fewer Bevy users take; Bevy sets its own
  device callbacks, which may conflict with the host's handling of errors
  and device loss. V0 must show it.
- Bevy's releases change its API often; one pinned version, one crate that
  touches it.
- Build time, binary size and disk grow a lot; unmeasured until V0.
- Older GPUs (D19) may not run it, or only in reduced form. The Intel iGPU
  laptop is still unmeasured.
- Rendering on the UI thread can delay the editor in a heavy scene. Moving
  it off the thread waits for a scene that needs it.
