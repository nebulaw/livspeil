<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Livspeil: Vision, Decisions, Roadmap (hub)

Status: living doc. Rev 15 (2026-11-28). This repository copy is the source of truth. Details live in the specs:
- `livspeil-kernel-spec-v0.md` (kernel as built, v0.3)
- `livspeil-ui-spec-v0.md` (UI toolkit and design system; formerly `livkit-spec-v0.md`; section 12 option (b), scripted Charm views, is deferred by D38)
- `repo-conventions-and-licensing.md` (repo layout, CI, agent workflow)
- `licensing-and-contributions.md` (complete license and CLA summary, Rev 2; DCO dropped by D42; not in this repository, see `docs/licensing.md`)
- `claude-code-playbook.md` (how the project is built with Claude Code: roles, rounds, briefs, skills; mirrors `docs/claude-code-playbook.md` in the repo)
- In the repo: `docs/benchmarks.md`, `docs/nodes.md` (generated node reference), `docs/widgets.md`, `docs/ui-host.md`, `docs/ui-config.md`, `docs/accessibility.md`, `docs/known-issues.md` (upstream bugs and workarounds), `README.md` (CLI and gallery), `CONTRIBUTING.md`, `.claude/` (shared Claude Code setup).

## Thesis
Livspeil is an Arch-based OS with a creative platform built in. The desktop is whatever the user authors: from a plain, polished, macOS-like setup out of the box up to a procedural 2D/3D world. Two modes: **Live** (use) and **Dream** (edit, in place).
The product is the substrate (kernel), the editor (Dream Mode) and the UI system (Livspeil UI). Scenes are demos built with them, and scenes can be published together with their own Charms and widgets (D33).

## Decisions log
Round 1
- D1 Audience: everyone; simple UI and presets for newcomers, graphs/scripts/full control for power users (Houdini-grade, approachable).
- D2 2D and 3D scenes; defaults look like a normal polished desktop.
- D3 Rust + Smithay core.
- D4 App windows stay flat; they interact with the world and vice versa.
- D5 Scripting: Python first, then TypeScript/JS, Lua.
- D6 No hardware floor imposed.
- D7 Solo dev with parallel Claude Code agents: thin kernel, small isolated tasks, strong tests, written specs.
- D8 Dream Mode edits in place.
- D9 Vocabulary: Charms, Dream Mode, Live Mode.
- D10 First demo: "Black Room" (minimal black setup, Dream Mode, Affinity-like feel).
Round 2
- D11 World runtime base: Bevy (pinned, behind our interface).
- D12 Own UI toolkit (no Qt/QML, no Bevy UI).
- D13 Scripts out-of-process (Python/TS/Lua); WASM and native nodes in-process.
- D14 Dream Mode windows: dim, or collapse to scalable icon rectangles ("window tokens").
- D15 Visual first; audio deferred (third clock reserved).
- D16 ~~Dev on macOS~~ superseded by D51; Intel iGPU Linux laptops are the baseline; NVIDIA later.
- D17 Open source, GNU license.
Round 3
- D18 Dream Mode first as a cross-platform app; OS pieces after.
- D19 Broad hardware compatibility, graceful degradation by tiers.
- D20 Dark-first; macOS-style titlebar; app menu actions in the system menubar; macOS feel but faster; configurable speeds.
- D21 GPL-3.0-or-later (DCO part superseded by D42).
- D22 Inter font; Boxicons (free licensed set) icons.
- D23 UI toolkit + design system (originally named Livkit; renamed in D34).
- D24 Flagship showcase: the owner's own room ("The Room").
Round 4
- D25 ~~Charm SDK under LGPL~~ superseded by D33.
- D26 Contributions require a CLA (DCO part superseded by D42).
- D27 First-party apps in v1: Dream Mode, Settings, Launcher. Everything else reuses existing apps, themed.
- D28 Motion is configured with exact numeric config files plus profiles. A system-wide settings UI is a future project.
- D29 CSD (GTK/libadwaita) apps accepted as they are; Floor applies uniform rounded masks, shadows, blur.
- D30 The Room's contents are deferred; simple placeholder builds are fine for now.
Round 5
- D31 Document format: KDL v2 (adopted).
- D32 Repo is `nebulaw/livspeil` (public) and Claude may modify it.
Round 6
- D33 MPL-2.0 for the SDK (`livspeil-node-api`, Python module, interface files) and Livspeil UI; core GPL-3.0-or-later.
- D34 "Livkit" renamed **Livspeil UI** (crates `livspeil-ui-*`, npm scope `@livspeil/ui`). "Charms" stays.
- D35 Open source is the key value: Stallman tradition with modern improvements.
- D36 CLA with an open-source lock; GPLv3 section 7 Charm exception approved in principle.
Round 7
- D37 Official store accepts free licenses only.
- D38 Charm UI in v1 is node graphs only.
- D39 The owner archives `nebulaw/livtk` personally.
- D40 M0.2 graph model built next (done).
Round 8
- D41 Work is parallelized: agents in separate git worktrees per track; the orchestrating session reviews, test-merges all open branches together, and opens one PR per track.
- D42 DCO sign-off dropped; the CLA remains the contribution agreement once its text is final.
Round 9
- D43 `core.sky.sun_direction` rejects `day_of_year` outside 1..=366.
- D44 Room files stay `format=1` when syntax is purely additive.
- D45 `ch()` reads another node's resolved *inputs* (parameters), Houdini-style.
- D46 Sinks for `livspeil cook`: root `core.output` nodes if any, else nodes without outgoing links.
Round 10
- D47 Finish Phase 0 before starting Phase 1 (done, including the deferred items).
- D48 `out("node/port")` reads computed outputs in expressions (built, PR #19); references are hidden edges; the editor will draw them as dashed wires (`Graph::expression_refs()`).
Round 11
- D49 Events use stateful nodes: events change per-cooker runtime state, exposed as value outputs; deterministic tick order; per-tick cap with overflow reporting; state is never saved in room files.
- D50 Node specs: Rust `NodeSpec` is the source of truth, mirrored by `specs/<id>.node.toml` (generated header + hand-written description and `[[tests]]`), verified by a test, refreshed with `cargo xtask node-specs`.
Round 12
- D51 The owner now develops on Linux (moved back from macOS). macOS stays a supported, CI-tested platform; the desktop host must be first-class on Linux (Wayland and X11) as well as macOS.
- D52 MSRV is Rust 1.95; CI runs latest stable plus an exact-MSRV job. wasmtime stays on the 48.x LTS line.
- D53 BSL-1.0 is an allowed dependency license in `deny.toml` (needed by arboard's platform crates). Accepted by default; revisit if a policy reason appears.
- D54 Text stack: Parley 0.13 with skrifa/zeno rasterization; layout on Taffy 0.16 with our own workarounds for two upstream bugs (`docs/known-issues.md`); accessibility via AccessKit 0.25; desktop host on winit 0.30.
- D55 The repo ships a shared Claude Code setup (`.claude/`: permissions, `implementer` and `reviewer` agents, `/start-round`, `/land-branch`, `/fix-ci`) and `docs/claude-code-playbook.md`, so any Claude Code session reproduces the workflow.
D56–D64 were recorded outside this repository and still need to be copied
here. As cited by merged code and docs: D58 Floor window chrome and DMA-BUF,
and (with D64) the Floor spike; D59 neutral colours; D60 softer shadows and a
raised selection pill; D61 popups fade without scaling; D62 #FAFAFA light
surface; D63 blur fallback is never silent; D64 Floor spike.

Answers given in a local session (2026-11-26), numbers not yet assigned:
- Finish P1 polish before starting P2: the visual design is not there yet.
- The Floor spike is part of this round, as D64 records (confirmed
  2026-11-26). This replaces the earlier "not yet" answer from the same day.
- From 2026-11-26 the project is run from local Claude Code sessions and this
  document is the only decision log. New decisions start at D65.
- The hub doc, kernel spec and repository conventions are published in the
  repository (`docs/`, `specs/`).

Frame pipeline (2026-11-27)
- D65 Retained frame pipeline (`specs/livspeil-ui-frame-pipeline.md`): frame cost is proportional to what changed. Stages P0–P5, one PR each; damage tracking and blur caching (P5) wait for the Intel iGPU laptop.
Round 15 (2026-11-28)
- D66 Experience and features come first. Visual design polish is one of the last stages, after what is needed for the OS's features exists. The colours (D59) stay as they are. This replaces "finish P1 polish before starting P2" from 2026-11-26.

## Architecture overview
- **Floor**: Smithay compositor (windows, input, outputs, XWayland, layer-shell, titlebars, shadows, rounded masks, blur, window tokens).
- **Shell**: menubar, launcher, settings, Dream panels (Livspeil UI, layer-shell clients).
- **World**: Bevy scene runtime; back layer under windows, optional front layer over them with input holes.
- **Services**: data bus, script workers, package manager.
- Window <-> world channels: geometry/state, commands world->Floor, input holes, pixels via DMA-BUF (later).
- Fallbacks: World crash -> static backdrop + respawn -> Safe scene; Shell never depends on World.
- Floor renderer is an open P3 decision; decide by spike on the Intel laptop (now easier: development is on Linux, D51).
- Hardware tiers T0/T1/T2; scenes declare a minimum tier and may ship fallbacks.
- Published Rooms: `*.room` packages carry Charms, assets and `assets.lock`.

## Kernel as built (P0, complete)
Crates: `livspeil-node-api` (MPL-2.0), `livspeil-graph`, `livspeil-params`, `livspeil-cmd`, `livspeil-doc`, `livspeil-nodes-core` (nodes with `*.node.toml` specs), `livspeil-script` (out-of-process script workers + Python SDK), `livspeil-wasm-host` (sandboxed WASM node packages, wasmtime 48 LTS), `livspeil-cli` (`validate`, `fmt`, `cook`, `diff`, `assets`, `run`), `livspeil-testkit`.
- Values: Float, Int, Bool, Vec2/3/4, Color (OKLCH), String, Table, Curve (typed: float, colour, vector); polymorphic ports; Event ports with payloads.
- Inputs: link > expression (`ch()` inputs, `out()` outputs, `$F/$T/$DT`) > animation > static > default.
- Structure: subnets, Charm interfaces, promotion, metadata, `core.output`; shared Charm definitions with per-instance parameters (M0.16).
- Scheduler: push-based dirty marking with edit epochs, per-cooker validation, cached evaluation order, dense slots, early cutoff, contained failures; event ticks with per-cooker state.
- System access: read-only, rooted file-system snapshots, optional folder watching.
- Documents: KDL rooms; comment-preserving canonical `fmt`; `assets.lock`; semantic diff covering subnets, Charms, tables, curves.
- Performance (10k nodes, shared VM): mark dirty 0.7–112 µs (target < 0.2 ms), cold cook 1.7–2.6 ms (target < 5 ms); 1,000 timers per tick 0.34 ms.
- PRs #1–#26 merged (incl. MSRV raise, #26).

## Livspeil UI as built (P1, in progress)
All `crates/livspeil-ui-*` are MPL-2.0 and testable headlessly (CPU reference rasterizer, `TestHost`, golden screenshots via `cargo xtask shot`).
- M1.1 renderer (#27): wgpu SDF quads, analytic shadows, dual-Kawase backdrop blur (Frost/Glass), CPU reference renderer; screenshot harness.
- M1.2 text (#29): Parley shaping, editor with IME, glyph atlas; Inter, Noto Georgian/Arabic vendored.
- M1.3 core, layout, theme, motion (#28): models/views, input and focus, springs, Taffy layout (bug workarounds in #34), tokens, `motion.toml`/`theme.toml`/`keymap.toml` with hot reload.
- M1.4 paint bridge (#32), M1.5 winit host (#31), M1.6 widgets (#33), M1.7 AccessKit tree (#30).
- M1.9 gallery and fully wired host (#35): `cargo run -p livspeil-ui-gallery`; PaintRenderer, AccessKit adapter, IME, cursors, clipboard, caret blink.
- M1.10 containers (#36): ScrollView (momentum, rubber band), VirtualList (100k rows), Panel, DockView, Menu/MenuBar/ContextMenu, Titlebar with window requests, Bézier wire primitive, NodeCanvas over a `NodeGraphModel` trait with culling, LOD and minimap.
- M1.11 (#38): gallery pages for containers and the node canvas, host applies window requests, Linux (Wayland/X11) run docs.
- Round 14 (#39–#43, #45): optional Linux AT-SPI feature, neutral colours and softer shadows, popups without scaling, bounded trackpad overscroll, macOS blur path with fallback, IconButton styles, Tabs, Popover, Dialog/Sheet, Toast.
- Round 14 (#47, merged locally): frame HUD (F3) and frame log (F4), present-mode overrides, NodeCanvas CPU trims. The stutter cause is not yet identified and the 1 ms target is not met.
- Floor spike (#46, merged locally): `crates/livspeil-floor` (GPL, Linux only), nested compositor and untested TTY skeleton; renderer recommendation in `docs/decisions/floor-renderer.md`.
- M1.12: Boxicons free set (49 icons, CC BY 4.0) in `assets/icons/boxicons/`, `livspeil-ui-icons`, icons in `IconButton` and menus.
- M1.13: tab overflow affordance, wrapped text measured at its real height, narrow and short gallery windows; ranked critique in `docs/design-critique.md`.
- M1.14: text no longer trails the box it moves with (layout rounds each out-of-flow box in its own frame; text runs snap once).
- First real-window check by the owner (2026-11-26, Hyprland on Wayland, NVIDIA RTX 5070 Ti): everything works; the design needs more work.
- The stutter seen in that round came from the setup (a software adapter after a driver upgrade without a reboot; a debug build with Vulkan validation), and the real cost was rebuilding and re-uploading everything visible. Retained frame pipeline (D65):
  - P0 (#49): scripted scenarios, work budgets in CI, GPU timing, a warning for software rendering.
  - P1 (#50): compact instance records per primitive family; a wire is one curve primitive.
  - P2 (#51): property tree (layers) and layout boundaries; pan, zoom and scroll render no view and run no layout.
  - P4 (#53), before P3: the node canvas applies model changes per node; cards are views on layers; views can own what they create.
  - On the owner's machine (500 nodes, release, 144 Hz): pan went from 108 to 144 fps and dragging a node from 99 to 144 fps (`docs/benchmarks.md`).
  - Open: P3 (paint fragments, retained GPU buffers) and P5 (damage, blur caching).

## Roadmap
- P0 Foundations: done.
- P1 Livspeil UI + gallery (Linux and macOS), current focus: frame pipeline P3. At 2000 nodes a frame costs 0.6–0.9 ms of CPU except with the whole graph in view (1.5 ms). Then: damage tracking and blur caching (after the iGPU measurement), node delete/unlink/keyboard navigation, screen-reader verification on Linux (Orca) and macOS (VoiceOver), real-window check on macOS. Visual design polish comes late (D66).
- P2 Dream Mode app: Bevy viewport host for Livspeil UI, inspector, node editor on the real kernel graph (incl. dashed `ch()`/`out()` wires), timeline, glTF import, lighting; Black Room.
- P3 Linux OS pieces: Floor spike and build, layer-shell host for Livspeil UI, world protocol, tokens, blur, titlebars, menubar.
- P4 Integration, tiers, calm governor, Safe scene; Room publishing and license checks.
- P5 The Room in Dream Mode; ISO, installer, updates; Charm/Room store (free licenses only).
- Later: audio, AI Charm, scripted Charm views, video, simulation, NVIDIA polish, system-wide settings UI.

## Risks
1. Own UI toolkit (text/IME/scroll/a11y) is the largest solo risk — now mostly built and working in a real window on the owner's Linux machine; screen readers, macOS and weaker GPUs are untested.
2. Blur/effects cost on Intel iGPU.
3. ~~Floor needs Linux while dev is on macOS~~ resolved by D51; the new risk is keeping macOS green without daily use (CI covers it).
4. Bevy churn.
5. Floor renderer choice.
6. Third-party app menus/titlebars inconsistent.
7. DMA-BUF window textures in wgpu.
8. Scope and asset art cost.
9. License hygiene (CLA friction; MPL/GPL boundary for published Charms).
10. Golden-image nondeterminism across GPUs (CI uses lavapipe; GPU tests are serialized after a parallel-device crash).
11. Name checks pending for "Livspeil UI", `@livspeil/ui`, "Charms".
12. Third-party Charm trust: sandboxing and capability tokens before a store exists.
13. UI frame times are measured on the owner's desktop (RTX 5070 Ti) and headless only; the Intel iGPU laptop is not measured. Kernel benchmarks still come from a shared 2-vCPU VM.
14. Accept outside PRs only after the CLA is enabled.
15. Some nodes use platform libm; cook goldens rely on 6-decimal rounding with guard tests.
16. Hidden data flow through expressions; mitigated by drawing references in the editor.
17. Kernel complexity is substantial; keep the randomized differential tests as the safety net for every scheduler change.
18. Upstream Taffy 0.16 bugs worked around locally; re-check on each Taffy upgrade and report upstream.
19. Usage limits interrupt parallel agents; mitigated by small tracks, resuming agents, and committing early.

## Open questions, round 15
1. After the frame pipeline, which feature track comes first: the Dream Mode app on the real kernel graph (roadmap P2), or the Linux OS pieces, Floor and Shell (P3)? D66 puts features first but does not order them.
2. When is the Intel iGPU laptop measured? P5 and the iGPU budget wait for it.
3. When is the screen-reader pass (Orca, VoiceOver)? Accessibility is built and has never been heard.

Deferred by D66: the design critique (`docs/design-critique.md`), the dark surface ramp, and the missing cross, trash, plus and lock icons.
