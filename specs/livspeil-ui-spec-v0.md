<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Livspeil UI Spec v0

Livspeil UI (formerly "Livkit", renamed in D34) is the UI toolkit and design system for Livspeil (decision D23). Own renderer, own widgets, own motion. No Qt/QML, no Bevy UI. Dark-first, macOS-grade craft, but faster. Numeric values are starting points to be tuned in the gallery. Rev 3 (2026-11-26): section 15 records what is built; section 16 the resolved decisions.

Naming: Rust crates `livspeil-ui-*`; npm scope `@livspeil/ui` reserved for the TypeScript bindings and Charm UI helpers; config under `~/.config/livspeil/ui/`. Published Rooms and Charms can use the toolkit, which is why it is MPL-2.0 (D33).

## 1. Principles
1. Response before beauty: visible feedback in the same frame as input, then settle.
2. Nothing blocks input. Every animation is interruptible and retargetable; springs keep velocity.
3. Few materials, used consistently. Restraint over decoration.
4. Everything is a token and everything is configurable by plain files (system-wide settings UI is a later project, not in this plan).
5. Accessible from the first widget (AccessKit), not retrofitted.
6. Cheap by default: render on demand, damage-tracked, blur cached, tiers degrade gracefully.
7. Agents can see it: every component renders headlessly to PNG.

## 2. Crates
| Crate | Purpose |
|---|---|
| `livspeil-ui-core` | entities/models, view tree, diffing, focus, input routing, animation driver, frame loop |
| `livspeil-ui-render` | wgpu renderer: display list, batching, effects (shadow, blur, grain); CPU reference renderer |
| `livspeil-ui-text` | shaping, layout, glyph atlas, editing primitives (on Parley) |
| `livspeil-ui-icons` | the embedded Boxicons set and its coverage rasterizer |
| `livspeil-ui-layout` | Taffy integration plus custom layouts (dock, infinite canvas) |
| `livspeil-ui-theme` | tokens, materials, motion profiles, config loading and hot reload |
| `livspeil-ui-paint` | paint bridge: view scene to display list, cached shaping, atlas uploads |
| `livspeil-ui-widgets` | the widget set (section 9) |
| `livspeil-ui-a11y` | AccessKit tree building and actions |
| `livspeil-ui-host-winit` | desktop host, first-class on Linux (Wayland, X11) and macOS (D51), used by the gallery and Dream Mode app |
| `livspeil-ui-host-bevy` | render-graph node and input bridge for embedding in the World (not built) |
| `livspeil-ui-host-wayland` | layer-shell host (smithay-client-toolkit) for the Shell; Linux only (not built) |
| `livspeil-ui-gallery` | component gallery app |
| `livspeil-ui-shot` | headless screenshot renderer and golden diff tool |

License: **MPL-2.0** (D33, "choose the loose option, relicense later if needed"). File-level copyleft keeps improvements to the toolkit open while letting published scenes, Charms and third-party apps use it under any license. The CLA's open-source lock keeps later relicensing possible within free licenses.

## 3. Rendering
- **Display list** of primitives produced from the view tree each dirty frame:
  - `Quad`: rect with continuous-corner (superellipse) radii, fill (solid or gradient), inner hairline border, shadow stack, clip, opacity, transform.
  - `Glyphs`: text runs from the glyph atlas.
  - `Image` / `Icon`: atlas-backed.
  - `Backdrop`: a region that samples and blurs what is behind it.
  - `Surface`: an external texture (the 3D viewport, a world preview).
  - Wires (Bézier strokes for the node canvas) are flattened by the paint bridge into rotated pill quads; the renderer has no separate path primitive.
- **Batching**: sort by layer, batch by pipeline and texture; instanced quads evaluated as SDFs in the fragment shader; one shadow pass and one blur pipeline shared by all.
- **Icons are not drawn with Vello.** Boxicons are simple filled paths, so they are rasterized once per needed physical size into an alpha atlas (zeno, the glyph rasterizer; the atlas is the glyph atlas), cached, and tinted at draw time. Vello is reserved for the Design Studio canvas (vector content), not for UI chrome.
- **Blur**: dual-Kawase at half or quarter resolution, per-region, cached while the backdrop region and the content under it are unchanged (caching not built yet). Quality by tier: T2 radius ~24 px at half res; T1 ~12 px at quarter res; T0 none (opaque tint).
- **Grain/dither**: tiled noise overlay on large gradients and blur surfaces to prevent banding.
- **Color**: UI chrome composited in gamma-encoded sRGB; 3D scene in linear; conversion at the `Surface` boundary. HDR/wide gamut later.
- **Frame loop**: render on demand. Nothing dirty and nothing animating means no frame. The animation driver requests the next frame only while something is moving. Present mode and frame latency configurable (target latency = 1 frame). Damage rectangles limit redraw where the host supports it (not built yet).
- **Determinism for tests**: golden screenshots are rendered by the CPU reference renderer, so they are identical on every machine; the wgpu renderer is compared against it within tolerance (lavapipe in Linux CI, GPU tests serialized).

## 4. Text
- Shaping/layout on Parley 0.13 with skrifa/zeno rasterization; variable fonts, Inter with `tnum` for parameter fields; Noto Sans Georgian and Arabic vendored for coverage and bidi tests.
- Glyph atlas with grayscale antialiasing (no LCD subpixel), gamma-correct blending, subpixel positioning on HiDPI.
- Text input: caret, selection, IME (winit IME events; Wayland text-input v3 in the layer-shell host later), bidi, clipboard.
- Never hand-roll shaping or bidi.

## 5. State and views
- GPUI-style: state lives in `Model<T>` entities; views are cheap functions that build an element tree; `notify` re-renders only views that observed the model; the element tree is diffed into a retained layout/render tree so layout and GPU buffers persist.
- Why not immediate mode: complex panels (inspector, node canvas, timeline) need persistent state, virtualization, accessibility identity.
- Stable `ElementId`s everywhere (needed for AccessKit, animations, focus).
- Xilem-style diffing was considered; models + views is built and kept.
- Positions that change every frame (pan, zoom, scroll) are layers: compositor properties that change without rendering a view or running layout (`livspeil-ui-frame-pipeline.md`, 4.1).

## 6. Animation and motion
- One `Animatable<T>` abstraction (floats, vectors, colors in OKLab, transforms). Drivers: **Spring** (`response` seconds, `damping` ratio) and **Tween** (`duration`, easing).
- Retargeting preserves velocity. Gestures hand their release velocity to the spring.
- Layout animations (shared-element, FLIP-style) provided by the framework, not hand-coded per widget (not built yet).
- **Config**: plain TOML, hot-reloaded, exact values, because the owner wants exact configuration:
```toml
# ~/.config/livspeil/ui/motion.toml
profile = "snappy"          # snappy | fluid | calm | off
speed = 1.0                 # global multiplier; 0 = instant
reduce_motion = false       # replaces movement with fades

[profiles.snappy]
micro    = { response = 0.10, damping = 0.90 }   # hover, press, toggle
standard = { response = 0.16, damping = 0.86 }   # menus, popovers, selection
large    = { response = 0.26, damping = 0.82 }   # panels, window tokens, camera moves

[overrides.windows]
speed = 1.2
```
- Budgets: at most two large animations at once; micro animations never allocate; Calm profile and Reduce Motion share one code path.

## 7. Tokens (starting values)
- **Spacing**: 2, 4, 8, 12, 16, 24, 32 (4 px base).
- **Radii**: xs 4, sm 6, md 10, lg 14, window 12 (continuous corners).
- **Type** (Inter): 11, 12, 13, 15, 17, 22, 28; weights 400, 500, 600; Simple density uses 13/15 body, Pro density 12/13 with tighter rows.
- **Row heights**: Simple 32, Pro 24.
- **Icon sizes**: 14, 16, 20, 24.
- **Color roles (dark-first, OKLCH)**: `surface.0..3` neutral (chroma 0): dark L ~0.10, 0.15, 0.19, 0.23; light #FAFAFA then L ~0.95, 0.92, 0.89 (D59, D62). `text.primary/secondary/tertiary` (high, 62%, 42% alpha), `accent` (adaptive from the scene/wallpaper, clamped for contrast), `danger`, `warning`, `success`, `focus-ring`.
- **Elevation** (shadow stacks): e1 `0 1 2` + low alpha; e2 `0 4 12` + `0 1 2`; e3 `0 12 32` + `0 2 6`; e4 `0 24 64` + `0 4 12`. Always two layers: ambient plus contact.

## 8. Materials
| Material | Use | Recipe (T2) |
|---|---|---|
| Solid | dense tools, inspectors | surface.1/2, 1 px hairline (white 6% alpha), e1 |
| Frost | menubar, docks, sidebars | backdrop blur 24, tint surface.1 at ~0.72, saturate x1.3, inner highlight white 8%, e2 |
| Glass | transient overlays, tooltips, menus | backdrop blur 16, tint ~0.5, top rim highlight 12%, e3 |
| Ink | text/controls directly over the scene | no surface; subtle text shadow for legibility |
T1 reduces blur radius/resolution and drops saturate; T0 and "Reduce transparency" replace blur with an opaque tint.

## 9. Widget set v0
Button, IconButton, Toggle, Checkbox, Radio, Slider, **NumberField with drag-scrub** (Houdini-style), TextField, Select, SegmentedControl, Tabs, List and Tree (virtualized), Table, Panel/Dock/Splitter, ScrollView (momentum, rubber-band, configurable), Menu/MenuBar/ContextMenu, Tooltip, Popover, Dialog/Sheet, Toast, **Titlebar**, ColorPicker (OKLCH), CurveEditor, **NodeCanvas** (infinite canvas, virtualization, minimap), Timeline (later), Viewport (embeds a `Surface`).
Auto-generated inspector: kernel `ParamSpec` to widgets (slider, scrub field, color, curve, file, enum).
Built so far: see section 15.

## 10. Input and accessibility
- Pointer, touch, keyboard, gamepad later; configurable keymaps (`Action` system, TOML), visible focus ring, full keyboard navigation, drag and drop, cursor shapes.
- AccessKit tree built incrementally from the view tree with stable ids. Test with Orca on Linux (AT-SPI) and VoiceOver on macOS.
- Settings that map to the same config system: reduce motion, reduce transparency, high contrast, text scale.

## 11. Hosts
- **winit**: gallery and the Dream Mode app window, on Linux (Wayland and X11) and macOS. On Linux it keeps system decorations by default; on macOS it draws the Livspeil Titlebar in a transparent full-size titlebar.
- **Bevy**: Livspeil UI renders as a render-graph node into the World's frame (shares the wgpu device); input forwarded from Bevy; bevy_ui disabled. (bevy_egui demonstrates the pattern.)
- **Wayland layer-shell**: separate process for the Shell; Linux only; uses ext-background-effect-v1 so the Floor blurs behind panels.
- **Floor-embedded**: open decision (see hub doc, Floor renderer options A/B/C).

## 12. Charms that ship UI (from D33)
Published scenes can carry their own widgets. A Charm UI is either (a) a node graph that outputs `Window`/`Scene` content using core widgets, or (b) an out-of-process script using the TypeScript or Python binding (`@livspeil/ui`) to describe views, rendered by the host. Option (b) is deferred (D38: v1 is node graphs only). Charms never receive raw GPU access; they get tokens, widgets and the `Surface` handle through the SDK. This keeps third-party UI themed, accessible and sandboxable.

## 13. Performance targets (hypotheses)
- Typical Dream Mode layout: UI CPU time per dirty frame < 1 ms on a modern desktop CPU (originally stated for an M-series Mac); GPU time of the whole UI < 1.5 ms at 1080p on an Intel Xe-class iGPU with T1 settings.
- Idle: zero frames, zero GPU wakeups.
- 120 fps scroll and drag on capable displays.
- First measurement: NodeCanvas with 2000 nodes / 3000 links takes 4.7–9.5 ms per frame on a shared 2-core VM (bench `crates/livspeil-ui-widgets/benches/node_canvas.rs`); above target, to be optimized and re-measured on real hardware.

## 14. Screenshot harness (`livspeil-ui-shot`)
- `cargo xtask shot` renders every registered scene (widgets, gallery pages, containers) at 1x and 2x and diffs against goldens in `crates/livspeil-ui-shot/goldens/`.
- Agents read the PNGs to iterate on visuals; golden changes are reviewed by hand.

## 15. Status as built (2026-11-26)
The original M1.1–M1.6 plan was re-cut into smaller PRs:

| Milestone | PR | What |
|---|---|---|
| M1.1 | #27 | renderer: SDF quads, shadows, gradients, dual-Kawase Frost/Glass blur, CPU reference; screenshot harness |
| M1.2 | #29 | text on Parley, editor with IME, glyph atlas |
| M1.3 | #28, #34 | core, Taffy layout (with upstream bug workarounds), theme and motion config with hot reload |
| M1.4 | #32 | paint bridge |
| M1.5 | #31 | winit host: pacing, IME, cursors, clipboard |
| M1.6 | #33 | controls: Button, Toggle, Checkbox, Radio, SegmentedControl, Slider, NumberField scrub, TextField, Select, Tooltip |
| M1.7 | #30 | AccessKit incremental trees and actions |
| M1.9 | #35 | gallery app (`cargo run -p livspeil-ui-gallery`); host fully wired |
| M1.10 | #36 | ScrollView, VirtualList, Panel, DockView, Menu/MenuBar/ContextMenu, Titlebar with window requests, Bézier wires, NodeCanvas over `NodeGraphModel` |
| M1.11 | #38 | gallery pages for containers and NodeCanvas; host applies window requests; Linux run docs |
| Round 14 | #39–#43, #45 | optional Linux AT-SPI feature; neutral colours and softer shadows (D59, D60, D62); popups fade without scaling (D61); bounded trackpad overscroll; macOS blur path and fallback (D63); IconButton styles, Tabs, Popover, Dialog/Sheet, Toast |
| M1.12–M1.14 | round 14 polish | Boxicons icon atlas (`livspeil-ui-icons`); tab overflow affordance, wrapped-text height, narrow gallery layouts; text moves with its box |

Not built yet: Tree, Table, ColorPicker, CurveEditor, Viewport, auto-generated inspector; `Image` drawing (icons do not use it); the `Surface` primitive; layout animations; adaptive accent from the scene (the accent is only contrast-clamped); drag and drop between elements; Bevy and layer-shell hosts; damage tracking and blur caching; high contrast and text scale; NodeCanvas node delete, unlink-by-drag and keyboard navigation.

Icons (M1.12): the 49 Boxicons free icons of `assets/icons/boxicons/` are embedded in `livspeil-ui-icons`, parsed by a strict SVG-subset parser and rasterized with zeno. `PaintCmd::Icon` becomes one sprite in the glyph atlas, so the CPU and wgpu renderers draw it with the glyph pipeline (including its coverage gamma), snapped to physical pixels under translation and uniform scale (sizes in quarter-pixel steps). `IconButton` and `MenuItem` use them.

Checked in a real window by the owner on 2026-11-26 (Hyprland, Wayland, NVIDIA RTX 5070 Ti): the gallery works. The visual design is not yet at the target; P1 polish continues.

Acceptance still open: measured 120 fps, Intel iGPU budget under T1, screen-reader pass (Orca and VoiceOver), macOS real-window check.

## 16. Decisions (resolved)
1. Models + views (GPUI-style) kept; built.
2. Text stack: Parley (D54; `docs/decisions/ui-text-stack.md`).
3. Goldens: CPU reference renderer, identical everywhere; GPU compared within tolerance.
4. Charm UI binding: graph-only for v1 (D38).
(License: MPL-2.0, D33. Name: Livspeil UI, D34.)
