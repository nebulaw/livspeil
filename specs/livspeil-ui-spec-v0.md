# Livspeil UI Spec v0

Livspeil UI (formerly "Livkit") is the UI toolkit and design system for Livspeil. Own renderer, own widgets, own motion. No Qt/QML, no Bevy UI. Dark-first, macOS-grade craft, but faster. Numeric values are starting points to be tuned in the gallery.

Naming: Rust crates `livspeil-ui-*`; npm scope `@livspeil/ui` reserved; config under `~/.config/livspeil/ui/`. License: **MPL-2.0** for every `livspeil-ui-*` crate (decision D33), so published Rooms, Charms and third-party apps can use it.

## 1. Principles
1. Response before beauty: visible feedback in the same frame as input, then settle.
2. Nothing blocks input. Every animation is interruptible and retargetable; springs keep velocity.
3. Few materials, used consistently. Restraint over decoration.
4. Everything is a token and configurable by plain files.
5. Accessible from the first widget (AccessKit), not retrofitted.
6. Cheap by default: render on demand, damage-tracked, blur cached, tiers degrade gracefully.
7. Agents can see it: every component renders headlessly to PNG.

## 2. Crates
| Crate | Purpose |
|---|---|
| `livspeil-ui-core` | entities/models, view tree, diffing, focus, input routing, animation driver, frame loop |
| `livspeil-ui-render` | wgpu renderer: display list, batching, effects (shadow, blur, grain) |
| `livspeil-ui-text` | shaping, layout, glyph atlas, editing primitives (on existing crates) |
| `livspeil-ui-layout` | Taffy integration plus custom layouts (dock, infinite canvas) |
| `livspeil-ui-theme` | tokens, materials, motion profiles, config loading and hot reload |
| `livspeil-ui-widgets` | the widget set |
| `livspeil-ui-a11y` | AccessKit tree building and actions |
| `livspeil-ui-host-winit` | desktop host (Linux and macOS) for the gallery and Dream Mode app |
| `livspeil-ui-host-bevy` | render-graph node and input bridge for the World |
| `livspeil-ui-host-wayland` | layer-shell host for the Shell; Linux only |
| `livspeil-ui-gallery` | component gallery app |
| `livspeil-ui-shot` | headless screenshot renderer and golden diff tool |

## 3. Rendering
- Display list of primitives produced from the view tree each dirty frame:
  - `Quad`: rect with continuous-corner (superellipse) radii, fill (solid or gradient), inner hairline border, shadow stack, clip, opacity, transform.
  - `Glyphs`: text runs from the glyph atlas.
  - `Image` / `Icon`: atlas-backed.
  - `Backdrop`: a region that samples and blurs what is behind it.
  - `Surface`: an external texture (3D viewport, world preview).
- Batching: sort by layer, batch by pipeline and texture; instanced quads evaluated as SDFs in the fragment shader; one shadow pass and one blur pipeline shared by all.
- Icons (Boxicons) are rasterized once per physical size into an alpha atlas (tiny-skia or lyon) and tinted at draw time. Vello is reserved for a future design canvas.
- Blur: dual-Kawase at half or quarter resolution, per region, cached while unchanged. T2 radius ~24 px at half res; T1 ~12 px at quarter res; T0 none (opaque tint).
- Grain/dither: tiled noise on large gradients and blur surfaces to prevent banding.
- Color: UI chrome composited in gamma-encoded sRGB; 3D scene linear; conversion at the `Surface` boundary.
- Frame loop: render on demand; no frame when nothing is dirty or animating; target latency 1 frame; damage rectangles where the host supports it.
- Determinism for tests: offscreen render at fixed DPI with a vendored Inter build; goldens rendered on a software Vulkan device (lavapipe) in CI; local macOS runs compare with tolerance.

## 4. Text
- Shaping/layout via an existing Rust text stack (Parley or cosmic-text class), variable fonts, Inter with `tnum` for parameter fields.
- Glyph atlas with grayscale AA (no LCD subpixel), gamma-correct blending, subpixel positioning on HiDPI.
- Text input: caret, selection, IME (winit IME; Wayland text-input v3 later), bidi, clipboard. Never hand-roll shaping or bidi.

## 5. State and views
- GPUI-style: state in `Model<T>` entities; views are cheap functions building an element tree; `notify` re-renders only views that observed the model; the element tree is diffed into a retained layout/render tree.
- Not immediate mode: inspectors, node canvas and timeline need persistent state, virtualization and accessibility identity.
- Stable `ElementId`s everywhere (AccessKit, animations, focus).

## 6. Animation and motion
- One `Animatable<T>` (floats, vectors, colors in OKLab, transforms). Drivers: Spring (`response` seconds, `damping` ratio) and Tween (`duration`, easing).
- Retargeting preserves velocity; gestures hand release velocity to the spring.
- Config: plain TOML, hot-reloaded, exact values:
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
- Budgets: at most two large animations at once; micro animations never allocate; Calm and Reduce Motion share one code path.

## 7. Tokens (starting values)
- Spacing: 2, 4, 8, 12, 16, 24, 32. Radii: xs 4, sm 6, md 10, lg 14, window 12 (continuous corners).
- Type (Inter): 11, 12, 13, 15, 17, 22, 28; weights 400, 500, 600; Simple density 13/15 body, Pro 12/13.
- Row heights: Simple 32, Pro 24. Icon sizes: 14, 16, 20, 24.
- Color roles (dark-first, OKLCH): `surface.0..3` neutral (chroma 0): dark L ≈ 0.10, 0.15, 0.19, 0.23, light #FAFAFA then L ≈ 0.95, 0.92, 0.89 (D59, D62); `text.primary/secondary/tertiary` (high, 62%, 42% alpha); `accent` (adaptive, clamped for contrast); `danger`, `warning`, `success`, `focus-ring`.
- Elevation (two-layer shadows): e1 `0 1 2`; e2 `0 4 12` + `0 1 2`; e3 `0 12 32` + `0 2 6`; e4 `0 24 64` + `0 4 12`.

## 8. Materials
| Material | Use | Recipe (T2) |
|---|---|---|
| Solid | dense tools, inspectors | surface.1/2, 1 px hairline (white 6%), e1 |
| Frost | menubar, docks, sidebars | backdrop blur 24, tint surface.1 at ~0.72, saturate ×1.3, inner highlight white 8%, e2 |
| Glass | tooltips, menus | backdrop blur 16, tint ~0.5, top rim highlight 12%, e3 |
| Ink | text over the scene | no surface; subtle text shadow |
T1 reduces blur and drops saturate; T0 and "Reduce transparency" use an opaque tint.

## 9. Widgets (v0)
Button, IconButton, Toggle, Checkbox, Radio, Slider, NumberField with drag-scrub, TextField, Select, SegmentedControl, Tabs, List/Tree (virtualized), Table, Panel/Dock/Splitter, ScrollView, Menu/MenuBar/ContextMenu, Tooltip, Popover, Dialog/Sheet, Toast, Titlebar, ColorPicker (OKLCH), CurveEditor, NodeCanvas, Viewport. Auto-generated inspector from kernel node specs.

## 10. Input and accessibility
Pointer, touch, keyboard; configurable keymaps (TOML actions); visible focus ring; full keyboard navigation; drag and drop; cursor shapes. AccessKit tree built incrementally with stable ids. Reduce motion, reduce transparency, high contrast, text scale.

## 11. Hosts
winit for gallery and Dream Mode, first-class on Linux (Wayland and X11) and macOS (D51); Bevy render-graph node; Wayland layer-shell for the Shell (Linux, uses ext-background-effect-v1).

## 12. Performance targets (hypotheses)
UI CPU per dirty frame < 1 ms on an M-series Mac; GPU < 1.5 ms at 1080p on Intel Xe with T1; idle = zero frames; 120 fps scroll and drag.

## 13. Screenshot harness
`cargo xtask shot <component> --state hover --theme dark --density pro --tier T1 -o out.png`; the gallery enumerates components and states; the same list drives golden tests; agents read the PNGs.

## 14. Milestones
M1.1 renderer (quad SDF, shadows, hairline, gradients, first Frost panel) · M1.2 text, layout, input, focus; Button and TextField · M1.3 animation driver, motion config, hot reload · M1.4 AccessKit · M1.5 screenshot harness and goldens · M1.6 Dock/Panel/Menu/Titlebar; NodeCanvas prototype.

## 15. Status as built (2026-11-25)
The original milestone plan above was re-cut into smaller PRs. Built and merged:

| Milestone | PR | What |
|---|---|---|
| M1.1 | #27 | renderer: SDF quads, shadows, gradients, dual-Kawase Frost/Glass blur, CPU reference; screenshot harness |
| M1.2 | #29 | text on Parley 0.13 (skrifa/zeno), editor with IME, glyph atlas |
| M1.3 | #28, #34 | core (models, views, input, focus, springs), Taffy 0.16 layout with bug workarounds, theme and motion config with hot reload |
| M1.4 | #32 | paint bridge: scene to display list, cached shaping, atlas uploads |
| M1.5 | #31 | winit 0.30 host: pacing, IME, cursors, clipboard |
| M1.6 | #33 | controls: Button, Toggle, Checkbox, Radio, SegmentedControl, Slider, NumberField scrub, TextField, Select, Tooltip |
| M1.7 | #30 | AccessKit 0.25 incremental trees and actions |
| M1.9 | #35 | gallery app; host fully wired (PaintRenderer, AccessKit adapter) |
| M1.10 | #36 | ScrollView, VirtualList, Panel, DockView, Menu/MenuBar/ContextMenu, Titlebar, Bézier wires, NodeCanvas |

Not built yet (from section 9 and 11): Tree, Table, ColorPicker, CurveEditor, Viewport, auto-generated inspector; Bevy and Wayland layer-shell hosts; damage tracking and blur caching (principle 6); high contrast and text scale. NodeCanvas misses the section 12 CPU target (4.7–9.5 ms per frame at 2000 nodes on a shared VM). Component details: `docs/widgets.md`, `docs/ui-host.md`, `docs/accessibility.md`, `docs/ui-config.md`.

