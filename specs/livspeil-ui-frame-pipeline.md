<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Livspeil UI frame pipeline (retained rendering)

Status: **proposed** (2026-11-27). Extends `livspeil-ui-spec-v0.md`
sections 3, 5, 12 and 13. Nothing here is built unless marked so.

## 1. Rule

**The cost of a frame is proportional to what changed, not to what is on
screen.** Panning, zooming, scrolling and dragging change a few numbers; they
must not rebuild, re-lay-out, repaint or re-upload what did not change.

## 2. Evidence

Release build, RTX 5070 Ti, driver 620.73, Wayland (Hyprland), 144 Hz
(6.94 ms per frame), the gallery's Node canvas panned by a script,
1274x1428 window unless noted. CPU = build + paint + encode, medians.

| Scenario | fps | CPU ms | submit ms | instances | uploaded per frame |
|---|---:|---:|---:|---:|---:|
| Sample, 6 nodes | 144 | 1.2 | 1.2 | 579 | 0.19 MB |
| Stress 500, zoom 1 | 111-122 | 5.2 | 2.0 | 16,200 | 5.2 MB |
| Stress 500, fit all | 144 | 2.9 | 1.8 | 15,000 | 4.8 MB |
| Stress 500, zoom 1, 2560x1440 | 59 | 11.2 | 3.8 | 36,100 | 11.6 MB |

Present mode (FIFO, mailbox, immediate) and frame latency (1, 2) made no
meaningful difference. The last row is one run, at the size the compositor
gave that window.

Where the time goes, for one pan frame:

1. **Everything visible is rebuilt.** A pan re-renders the canvas view,
   builds 2,000-2,600 elements and diffs them: about 2.7-3.0 ms, most of
   `build`.
2. **Layout runs although no box changed**: 0.4-0.5 ms.
3. **The whole scene is repainted, re-converted and re-encoded**: paint and
   encode, 1.1-3.5 ms.
4. **All instances are uploaded again.** Every instance is 80 floats
   (320 bytes), whether it is a card, one glyph or one piece of a wire.
5. **Wires are many quads**: each is cut into pieces about 10 px long.

Not the cause: the present mode and the swapchain. Two earlier observations
were artifacts of the setup: a debug build with Vulkan validation, and a
software adapter after a driver upgrade without a reboot.

Dragging a node is not measured yet. From the code, every move bumps the
model revision, which rebuilds the geometry cache of **all** nodes.

## 3. Targets

On the machine above, release build; each is a scripted scenario.

| Scenario | CPU per frame | uploaded per frame |
|---|---:|---:|
| Pan or zoom, any graph size | <= 1 ms at 2000 nodes | <= 4 KB, plus newly revealed content |
| Drag one node, 2000 nodes | <= 1 ms | that node and its wires |
| Scroll a list | <= 0.5 ms | <= 4 KB, plus newly revealed rows |
| Hover or press a control | <= 0.5 ms | that control |
| Idle | no frames | nothing |

Sustained 144 fps follows. The budget on an Intel Xe iGPU (UI spec 13) is
measured on that hardware before it is claimed.

## 4. Design

Four retained stages, each keyed so it can be reused:

```
views -> element tree -> layout tree -> paint fragments -> GPU buffers
                                        + property tree
```

### 4.1 Property tree (compositor properties)

Transform, scroll offset, opacity and clip are **properties**, held in a tree
beside the paint content, not baked into it. A view owns a handle:

```rust
let layer = cx.layer();               // stable across renders
div().layer(&layer).children(..)      // content, in local coordinates
layer.set_transform(app, transform);  // property change only
```

Setting a property marks the frame dirty but no view: no render, no diff, no
layout, no repaint. Animations of these properties use the same path. Hit
testing, accessibility bounds, tooltips, popups and IME placement read the
same tree, so they cannot disagree with what is drawn.

### 4.2 Layout boundaries

An absolutely positioned box with a definite size is a layout boundary: a
change inside it lays out only its subtree, and moving it changes its layer
transform without running layout. Node cards, popups, tooltips and toasts
are such boxes.

### 4.3 Paint fragments

A layer is also a repaint boundary. Painting yields one fragment per
boundary: commands in local coordinates, plus a generation. The scene is the
ordered list of fragments with their property nodes. A fragment is repainted
only when something inside it changed. The paint bridge caches each
fragment's display items (shaped text, glyph and icon placements) by
`(fragment, generation, raster scale)`.

### 4.4 Retained GPU buffers

- Each fragment owns a range in a persistent instance buffer, managed by a
  free-list allocator with compaction.
- Instances refer to a **property table** (a small storage buffer) by index
  for transform, clip and opacity. Moving a canvas writes one entry.
- Only changed ranges are written, through a staging ring.
- Draw order is a list of ranges, rebuilt only when fragments are added,
  removed or reordered.

### 4.5 Instance formats

One format per primitive family instead of one 320-byte union:

| Family | Contents | Size goal |
|---|---|---:|
| Sprite (glyph, icon) | rect, atlas rect, colour, property index | <= 48 B |
| Quad | rect, radii, fill, border, property index | <= 96 B |
| Effect (shadow, gradient, material) | indexed from a side table | as needed |
| Curve (wire) | four control points, width, colours | <= 64 B |

A **curve is one instance**: the vertex shader expands it to a strip along
the Bezier and the fragment shader antialiases across it. The CPU reference
renderer implements the same primitive, so goldens stay the contract.

### 4.6 Zoom and text

While a zoom is moving, glyphs and icons are drawn from the existing atlas
entries through the layer transform. When it settles they are rasterized at
the final scale and swapped in. Translations snap to physical pixels per
layer, so content never moves relative to its box.

### 4.7 NodeCanvas on this pipeline

- The canvas is one layer; pan and zoom set its transform.
- Each card is a layer; dragging sets that card's transform and rebuilds
  only the wires attached to it.
- Virtualization builds a region larger than the viewport and rebuilds only
  when the view leaves it, or the level of detail changes.
- `NodeGraphModel` reports **what** changed (moved, edited, topology) in
  addition to a revision, so the geometry cache and spatial index update
  incrementally. A full rebuild stays the correct fallback.
- The minimap is its own fragment, updated by the same change set.

### 4.8 Damage and blur

Fragments give exact damage. A backdrop blur is recomputed only when damage
intersects what it samples. Presenting partial damage is used where the
surface supports it.

## 5. Invariants and how they are tested

- **Retained equals fresh**: for random sequences of edits and property
  changes, the incrementally maintained scene renders the same pixels as one
  built from scratch (differential test, as the kernel scheduler has).
- **Work is counted, not timed, in CI**: views rendered, elements
  reconciled, layouts run, fragments repainted and bytes uploaded per
  scenario have asserted upper bounds. These are deterministic.
- **Time is measured on real hardware** by gallery scenarios
  (`--scenario=...`), recorded in `docs/benchmarks.md`.
- CPU reference and GPU renderers keep matching.
- `unsafe` stays forbidden.

## 6. Developer setup (part of the work)

- Optimize dependencies in the dev profile, so debug builds stay usable.
- Vulkan validation off by default; opt in with an environment variable.
- A visible warning when the adapter is a software renderer, and the build
  profile and adapter in the HUD.
- GPU time through timestamp queries.

## 7. Stages

Each stage is one pull request, leaves `main` working, and states its
before and after numbers.

| Stage | Content | Done when |
|---|---|---|
| P0 | Scenarios (pan, zoom, drag, scroll), work counters, GPU timing, section 6 | baseline of every target recorded |
| P1 | Instance families and the curve primitive | upload down at least 5x on the stress graph; goldens reviewed |
| P2 | Property tree and layout boundaries | pan, zoom and scroll render no view and run no layout |
| P3 | Paint fragments and retained buffers | upload targets of section 3 met |
| P4 | NodeCanvas change sets and virtualization margin | CPU targets of section 3 met at 2000 nodes |
| P5 | Damage and blur caching | idle GPU work gone; measured on the iGPU |

P1 is independent of P2. P3 needs P2.

## 8. Alternatives not taken

- **Make the current rebuild faster** (arenas, fewer allocations, memoized
  rows): a constant factor; cost would still grow with what is visible.
- **Replace the renderer** (for example with a compute rasterizer): does not
  touch element rebuilding, layout or repainting, which are most of the time.
- **Immediate mode**: contradicts the UI spec (section 5): persistent state,
  virtualization and accessibility identity.
- **A render thread now**: hides cost instead of removing it and adds
  latency and synchronization. Revisit with P3 numbers.

The shape (property trees, repaint boundaries, retained draw data) is the
established one in browser compositors and in Flutter's layer tree.

## 9. Open questions

1. `Layer` handle (4.1) or properties bound to models: which API do views
   use? Proposed: the handle.
2. Does P5 wait until the Intel iGPU laptop is measured?
