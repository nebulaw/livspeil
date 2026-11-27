<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Livspeil UI frame pipeline (retained rendering)

Status: **accepted** (D65, 2026-11-27); P0, P1, P2 and P4 built. Extends
`livspeil-ui-spec-v0.md` sections 3, 5, 12 and 13. Nothing else is built
unless marked so.

## 1. Rule

**The cost of a frame is proportional to what changed, not to what is on
screen.** Panning, zooming, scrolling and dragging change a few numbers; they
must not rebuild, re-lay-out, repaint or re-upload what did not change.

## 2. Evidence (baseline, stage P0)

Release build, RTX 5070 Ti, driver 620.73, Wayland (Hyprland), 144 Hz
(6.94 ms per frame), gallery scenarios in a fixed 1600x1000 window, 7 s
each, medians (2026-11-27). CPU = build + paint + encode; submit includes
the instance upload; GPU is the draw and blur passes.

| Scenario | fps | CPU ms | submit ms | GPU ms | instances | uploaded |
|---|---:|---:|---:|---:|---:|---:|
| `pan-sample` (6 nodes) | 144 | 1.29 | 1.23 | 0.12 | 579 | 0.19 MB |
| `pan-stress` (500 nodes) | 108 | 5.66 | 1.95 | 0.17 | 16,871 | 5.40 MB |
| `pan-stress-fit` | 144 | 3.18 | 2.19 | 0.14 | 19,799 | 6.34 MB |
| `zoom-stress` | 124 | 4.79 | 1.70 | 0.17 | 13,608 | 4.35 MB |
| `drag-stress` (one node) | 99 | 6.66 | 2.07 | 0.17 | 18,449 | 5.90 MB |
| `scroll-list` (100,000 rows) | 144 | 1.63 | 0.90 | 0.13 | 1,289 | 0.41 MB |

The GPU is nearly idle. The cost is CPU work and moving data to the card.
At 2560x1440 an uncontrolled run of the stress pan reached only 59 fps.

Where the time goes, for one pan frame:

1. **Everything visible is rebuilt.** A pan re-renders the canvas view,
   reconciles about 1,800 elements and is most of `build`.
2. **Layout runs on every canvas frame** although no box changed size
   (0.4-0.5 ms). Scrolling already avoids it.
3. **The whole scene is repainted, re-converted and re-encoded.**
4. **All instances are uploaded again.** Every instance is 80 floats
   (320 bytes), whether it is a card, one glyph or one piece of a wire.
5. **Wires are many quads**: each is cut into pieces about 10 px long.
6. **Dragging one node is the worst case**: the model revision changes, so
   the geometry cache of all nodes is rebuilt (2,328 elements reconciled,
   two views rendered).

Not the cause: present mode (FIFO, mailbox, immediate) and frame latency
(1, 2) made no meaningful difference. Earlier observations of a stalling
gallery came from the setup: a debug build with Vulkan validation, and a
software adapter after a driver upgrade without a reboot.

Not baselined yet: hovering or pressing a control (no scenario injects
pointer input; one is added with P2, which changes that path), and GPU upload
time on its own.

The same scenarios run headlessly in
`crates/livspeil-ui-gallery/tests/frame_budget.rs`, which holds their work
counts as budgets.

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

**As built (P2).**

- `Layer` is a `Copy` handle from `cx.layer("key")`, owned by the view and
  dropped with it; `Element::layer(layer)` takes it by value. A dropped
  layer is inert.
- Properties are transform (a scroll offset is a translation), opacity, and
  a frame that replaces the box of a childless element (scrollbar thumbs,
  the minimap viewport frame: they resize every frame, and scaling them
  would distort borders and radii). Clip stays an element attribute
  (`clip()`): nothing animates it yet.
- The values live beside the element tree (`App`), so views can use layers
  while the tree is reconciled; painting, hit testing, culling, bounds
  queries and accessibility read them through one transform function.
- `App::animate` runs per-frame tasks before views render; springs and
  momentum step there. There is no layer-owned spring: the owners' motion
  (anchored zoom, scroll physics) is not an interpolation of the transform.
- `Element::cull_offscreen` replaces `Element::canvas`: paint and hit
  testing skip children outside the visible area. This is what keeps a
  built region larger than the view from costing paint time or instances.
- Translations are not yet snapped to physical pixels per layer (4.6); the
  paint bridge snaps text and icons as before.
- A layer element is the repaint boundary P3 will cut at; the scene format
  is unchanged.

### 4.2 Layout boundaries

An absolutely positioned box with a definite size is a layout boundary: a
change inside it lays out only its subtree, and moving it changes its layer
transform without running layout. Node cards, popups, tooltips and toasts
are such boxes.

**As built (P2).** In `livspeil-ui-layout` the boundary's contents are a
Taffy tree of their own, rooted at a copy of the box's style with the size
its parent gave it. "Definite" is a length or percentage in both directions
(and no percentage padding). Moving is not a layer transform: for a box
placed by `left` and `top` alone, Taffy is given zero insets and the real
ones are added when rounding, so a style change moves it with no layout
(`FrameStats::layout` stays false). Boxes with an `auto` size are not
boundaries. A 600-seed differential test compares the incrementally edited
tree with plain Taffy.

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

### 4.5 Instance formats (built, P1)

One format per primitive family instead of one 320-byte union:

| Family | Contents | Size goal | Built |
|---|---|---:|---:|
| Sprite (glyph, icon) | rect, atlas rect, colour, property index | <= 48 B | 48 B |
| Quad | rect, radii, fill, border, property index | <= 96 B | 96 B |
| Effect (shadow, gradient, material) | indexed from a side table | as needed | shadow 96 B, gradient +96 B, backdrop 112 B |
| Curve (wire) | four control points, width, colour | <= 64 B | 64 B |
| Property | transform, inverse, clip, opacity | | 80 B |

A frame is three arrays (`livspeil-ui-render`, `record.rs`): a **record
arena** of 16-byte slots, where every record is a whole number of slots and
effect data is a record referred to by slot; the **property table**, one
entry per distinct transform, clip and opacity; and the **draw order**, one
`u32` per instance (family and slot) read as a per-instance vertex
attribute. So an instance costs its record plus 4 bytes.

One pipeline reads every family. Painter order is the draw order and a
change of family costs nothing; separate per-family pipelines were rejected
because cards interleave quads and text and would switch pipelines several
times per node. A single arena (instead of one typed buffer per family)
uses two storage buffers of the four the downlevel limits allow, and lets
P3 give each fragment one contiguous range.

A **curve is one instance**: the vertex shader expands it to a box per
polyline segment along the Bezier, and each pixel is drawn by the box of its
nearest segment, so one curve never blends twice. Runs of curves are
separate draws (more vertices per instance). The CPU reference renderer
implements the same primitive, so goldens stay the contract.

For P3: records are position independent apart from slot and property
indices. Some values do depend on the on-screen scale and are fixed at
encode time: hairline border width, the minimum shadow sigma, and a curve's
minimum width, fade, segment count and search window. A retained fragment
must be re-encoded when its scale changes, or those must move to the shader.

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
  when the view leaves it, or the level of detail changes. (Built in P2,
  with a margin of a quarter of the view per side; it also rebuilds when the
  view size changes or zooming in leaves the region over twice as wide as
  needed. Lists and `ScrollView` content do the same through
  `ScrollInfo::region`. The first bullet above is built; cards as layers and
  change sets are P4.)
- `NodeGraphModel` reports **what** changed (`changes_since`: a node moved
  or was edited) in addition to a revision, so the geometry cache and spatial
  index update incrementally. A structural change, or a model that cannot
  tell, rebuilds, which stays the correct fallback.
- The minimap is its own fragment, updated by the same change set.

Built in P4:

- A card is a view on its own layer; the wires are one view. The canvas
  reads the model without subscribing to it, watches it, and applies the
  changes at the start of the next frame (`ViewContext::watch`,
  `App::untracked`, `App::animate`), so a burst of edits costs one update and
  the cost is inside the frame.
- A moved node: its card's transform, the wire view, one minimap rect. No
  canvas or card render, no layout.
- Wires of all built links are one element, rebuilt when any of them changes;
  fragments (P3) are what would make that per link.
- The minimap remaps exactly when the graph's bounds change, so that a
  long-lived canvas draws what a new one draws.

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

Built with P2, comparing scenes (not pixels): random property changes and
edits against a new app (`livspeil-ui-core/tests/layers.rs`), random pans,
zooms, moves and selections against a new `NodeCanvas`
(`livspeil-ui-widgets/tests/node_canvas.rs`), random scrolls and renders
against a fresh accessibility tree (`livspeil-ui-a11y/tests/positions.rs`),
and random layout edits against plain Taffy
(`livspeil-ui-layout/src/tree_tests.rs`).

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
| P0 (built) | Scenarios (pan, zoom, drag, scroll), work counters, GPU timing, section 6 | baseline of every target recorded |
| P1 (built) | Instance families and the curve primitive | upload down at least 5x on the stress graph (16.6x on `pan-stress`, `docs/benchmarks.md`); goldens reviewed |
| P2 (built) | Property tree and layout boundaries | pan, zoom and scroll render no view and run no layout |
| P3 | Paint fragments and retained buffers | upload targets of section 3 met |
| P4 (built, see below) | NodeCanvas change sets and virtualization margin | CPU targets of section 3 met at 2000 nodes |
| P5 | Damage and blur caching | idle GPU work gone; measured on the iGPU |

P1 is independent of P2. P3 needs P2.

### P2 results

Work per measured frame in the headless budgets
(`tests/frame_budget.rs`, 88 frames at 1274x1428; "rendered" and "layout"
count frames). Instance counts are in the budgets; P1 changed them:

| Scenario | rendered | layout | most reconciled |
|---|---|---|---|
| `pan-sample` | 88 → 0 | 88 → 0 | 80 → 0 |
| `pan-stress` | 88 → 0 | 88 → 0 | 1,827 → 0 |
| `pan-stress-fit` | 88 → 0 | 88 → 0 | 507 → 0 |
| `zoom-stress` | 88 → 0 | 88 → 0 | 1,827 → 0 |
| `drag-stress` | 88 → 88 | 88 → 0 | 2,328 → 2,607 |
| `scroll-list` | 88 → 1 | 0 → 1 | 388 → 466 |
| `hover-buttons` (new) | 0 | 0 | 0 |

`drag-stress` reconciles more because the canvas now builds the margin
around the view; P4 removes that render. The P0 `scroll-list` never
scrolled (an unmeasured list clamped every offset to zero), so its P0
timings in section 2 are of a list re-rendering in place.

CPU time of P2 without P1, headless, release, i5-12500T, same steps (median
of 900 frames; build is the scenario step plus `App::draw_frame`, CPU adds
scene conversion and encoding):

| Scenario | build ms | CPU ms |
|---|---|---|
| `pan-sample` | 0.27 → 0.02 | 0.35 → 0.10 |
| `pan-stress` | 7.18 → 0.72 | 9.93 → 3.25 |
| `pan-stress-fit` | 3.05 → 0.41 | 5.12 → 2.41 |
| `zoom-stress` | 6.51 → 0.30 | 9.90 → 1.97 |
| `drag-stress` | 8.88 → 5.49 | 11.85 → 7.35 |
| `scroll-list` | 0.78 → 0.10 | 1.17 → 0.41 |

Times on screen, for P1 and P2 together, are in `docs/benchmarks.md`
("Frame pipeline P1 and P2 on screen").

### P4 results

Dragging one node of 2000 (`frame_phases`, headless, release, i5-12500T,
CPU in ms: build, scene conversion and encoding):

| Zoom | before | after | pan before | pan after |
|---|---:|---:|---:|---:|
| 1 (64 nodes built) | 3.87 | 0.59 | 0.52 | 0.60 |
| 0.4 (440 built) | 10.33 | 0.86 | 0.86 | 0.93 |
| fit all (2000 built) | 7.29 | 1.53 | 1.06 | 1.51 |

The `drag-stress` budget renders one view and reconciles 2 elements (was 2
views and 2,607).

- Section 3's 1 ms is met for dragging and panning except with the whole
  graph in view, where encoding alone takes 0.8 ms. That needs P3.
- Panning got slower, most with everything in view: each card now has its own
  transform, so painting walks 4,000 more commands and the property table has
  one entry per card (+160 KB uploaded fit to all). With retained fragments
  (P3) an unchanged card is not painted or encoded again.
- Not measured on screen yet.

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
   use? Decided: the handle.
2. Resolved: P5 waits until the Intel iGPU laptop is measured.
