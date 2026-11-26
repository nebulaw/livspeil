<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Decision: Floor renderer (spike, round 14, decisions D58 and D64)

Status: **proposed**. The owner decides after running
`docs/floor-spike-checklist.md` on the RTX 5070 Ti machine. Scope:
`crates/livspeil-floor` (GPL-3.0-or-later, Linux only; the Floor is
core, not UI).

**Recommendation: option A, Smithay's own GLES renderer with custom
shaders. Confidence: about 70% that A is the right first renderer, and
only about 40% on the Intel and NVIDIA performance questions, because
nothing here ran on real GPUs.**

![Nested Floor with the real foot terminal: server-side titlebar,
rounded clip, soft shadow, blurred Frost panel over the terminal
text](floor-nested-foot.png)

(`floor-nested --spawn foot --screenshot`, Xvfb and llvmpipe in the build
sandbox. Neutral greys, no coloured chrome (D59), two-layer soft shadow,
no scale animation (D61).)

## The options

- **A. Smithay GLES.** `smithay::backend::renderer::gles` draws
  everything. Rounded masks, shadows and blur are custom GLSL ES shaders.
- **B. wgpu.** A renderer built on wgpu that shares code with
  `livspeil-ui-render`. Client buffers come in through DMA-BUF import.
- **C. Hybrid.** Smithay renders client surfaces to textures; Livspeil
  UI/wgpu composites decorations and effects on top.

## What the spike built (option A)

Everything below runs in this sandbox except where noted.

- `floor-nested`: the Floor in a winit window (Smithay winit backend, EGL).
  xdg-shell, xdg-decoration (always server side), wl_shm, linux-dmabuf
  global (100 formats advertised under Mesa llvmpipe), data device,
  pointer and keyboard. Real clients tried: `foot` and the
  smithay-client-toolkit fixture `floor-test-client`.
- Server-side titlebar and buttons drawn by the Floor, a rounded clip of
  the client (custom texture shader through
  `GlesFrame::override_default_tex_program`, SDF with 1 px anti-aliasing),
  two-layer soft shadow (elevation e3, analytic Gaussian falloff in a
  pixel shader), and a Frost-like panel with dual-Kawase backdrop blur
  (scene to an offscreen texture, blur on a cropped region at half and
  quarter resolution, saturate, tint, rim highlight).
- `floor-tty` (`--features tty`): libseat session, udev primary GPU, DRM
  device through the session, GBM, one connector, `GbmBufferedSurface`,
  libinput, VT switching. **Compiles only; never run.**
- Tests: 15 unit tests (frame geometry, SDF, shadow falloff against the
  normal CDF, frame statistics) that also run on macOS; 4 protocol tests
  that serve the real compositor state over a socket pair to a real
  Wayland client (map, server-side decoration negotiation, titlebar drag,
  close button, disconnect). No display or GPU needed.
- Zero `unsafe` in the crate. The nested path uses Smithay's safe winit
  backend; the TTY path uses `GbmGlesBackend`/`GpuManager`, which create
  the EGL display and context safely (calling `GlesRenderer::new` and
  `EGLDisplay::new` directly would be `unsafe` and would clash with the
  workspace `forbid`).

## Measured (sandbox only, not representative)

`floor-nested --bench 400` (release build), 1280x800, two windows, the top
one animated every frame, Xvfb with **llvmpipe (a software rasteriser)**
on a shared 2-core VM with a load average around 5. CPU time to record and
swap one frame; the "done" column adds a 1x1 read-back that cannot return
before the GPU finished the frame.

| Effects | cpu mean / p50 / p95 (ms) | done mean / p50 (ms) |
|---|---|---|
| everything (shadows, round, blur panel) | 30.7 / 28.3 / 48.3 | 29.9 / 27.4 |
| no blur (plain tint panel) | 16.3 / 16.1 / 22.4 | 15.2 / 15.3 |
| no blur, no shadows | 10.1 / 9.1 / 15.7 | 9.4 / 8.3 |
| nothing (background and clients only) | 10.7 / 9.7 / 17.9 | 10.1 / 8.4 |

What this does and does not say. A software rasteriser costs in
proportion to pixels touched, so the numbers rank the effects but say
nothing about an iGPU or a discrete GPU. Shadows cost about 6 ms here
because every window draws a bounds-sized shader quad (about 400 x 400
extra pixels each). The blur path costs about 14 ms more, and I infer
(not measured separately) that most of it is the two extra full-screen
passes it forces (scene to texture, texture to window), not the blur
itself, which works on a 408 x 120 crop. That points at the obvious fixes:
damage tracking and copying only what the panel needs. No numbers exist
for Intel iGPU, NVIDIA, or any real display.

There is no GPU timer in the harness: GL timer queries need `unsafe` GL
calls in this crate, which the workspace forbids. The 1x1 read-back is
the safe substitute.

## Comparison

"Verified" means I read the crate source in `~/.cargo/registry` (Smithay
0.7.0, wgpu and wgpu-hal 30.0.2, wayland-protocols 0.32.13) or ran it.
Everything else is marked as unsure.

| | A. Smithay GLES | B. wgpu | C. Hybrid |
|---|---|---|---|
| **Effort** | Lowest. Spike: about 1.3k lines for renderer, shaders and the nested compositor, working in a day. Smithay's helpers (render elements, `OutputDamageTracker`, `DrmCompositor` with plane assignment, multi-GPU, dmabuf, explicit-sync handlers) are built for it and not used yet. | Highest. Smithay 0.7.0 has no wgpu `Renderer`, so its element, damage and `DrmCompositor` machinery do not apply: we own surface import, damage, buffer release and scanout. A third-party `smithay-renderer-vulkan` crate (0.1.2) showed up on crates.io; not evaluated. | High. A and B's work plus the bridge. |
| **Intel iGPU (baseline)** | Unsure, untested. The shaders are GLES 2 (`#version 100`), the lowest common denominator. Blur runs on a crop at quarter resolution. Likely fine on Mesa iris/anv, but this is an expectation, not a measurement. | Unsure. Needs Vulkan; Mesa anv works for the UI crates, but the Floor path (import, present) is untested. | Worst: two APIs on one small GPU, plus a copy or an interop sync per frame. |
| **NVIDIA** | Unsure. Smithay's EGL goes through GBM; I found no EGLStreams code in `backend/egl`. Smithay's EGL code carries a workaround for NVIDIA proprietary 520 (modifier query returns `EGL_BAD_PARAMETER`), so NVIDIA is a known case. Needs `nvidia_drm.modeset=1`. The checklist tests this. | Unsure. Vulkan on NVIDIA is the best-supported API, but wgpu's DMA-BUF import is single-plane only (below), and explicit sync matters on NVIDIA. | Unsure, plus interop between two APIs on the same NVIDIA GPU. |
| **DMA-BUF client buffers** | Verified: `ImportDma` for `GlesRenderer` (EGL_EXT_image_dma_buf_import and modifiers), `DmabufState` global, `DmabufHandler::dmabuf_imported`. Wired up in the spike; formats advertised, but no real dmabuf client was tried (needs hardware). | Verified, with limits: wgpu-hal Vulkan has `texture_from_dmabuf_fd`: `unsafe`, "currently only supports single-plane DMA-bufs", needs the `VULKAN_EXTERNAL_MEMORY_DMA_BUF` feature (VK_EXT_external_memory_dma_buf and VK_EXT_image_drm_format_modifier). `unsafe` conflicts with the workspace lint, so the Floor would need a carve-out. No sync handling in that call. | As B for the import into wgpu, plus exporting a GLES texture as a dmabuf (Smithay side, not checked). |
| **Blur and effects quality and cost** | Smithay has **no blur** and **no ext-background-effect handler** (grep of 0.7.0 finds neither); custom shaders work (verified here, including the mask and the dual-Kawase chain). Quality is whatever we write; dual Kawase at quarter resolution is as good as the UI renderer's. No caching or damage yet, so blur is redone each frame. | Best fit for sharing the finished UI effects (SDF quads, shadow stack, Frost/Glass blur with a CPU reference and screenshot goldens). Same cost model as A once the pipeline exists. | Best visuals on paper (UI effects), worst cost: every frame crosses APIs. |
| **ext-background-effect-v1** | wayland-protocols 0.32.13 ships the bindings (`ext::background_effect`, `staging` feature). Version 1 has one capability (`blur`) and a per-surface `set_blur_region`. We write the `GlobalDispatch`/`Dispatch` ourselves (two small objects) and feed the regions to the blur pass; the spike's panel is the renderer half of this. | Same protocol work; the blur pass is the UI's. | Same. |
| **Sharing code with Livspeil UI** | Little at runtime: shaders are GLSL ES vs the UI's WGSL. Shareable: portable math (`geometry.rs` is dependency-free and matches the shader), design tokens (the UI theme crate is MPL, a GPL crate may use it), and the CPU glyph atlas of `livspeil-ui-text` for titlebar text (not wired; the spike draws no title text). | Most: renderer, effects, text, theme tokens, goldens. | Same as B for decorations. |
| **Risk** | Low. Single well-trodden path (the same stack as anvil and most Smithay compositors). Main risk: GLES 2 limits (no timer queries without unsafe, no compute) and our manual frame path needs porting onto damage tracking. | High. Unsafe hal calls, single-plane import, no explicit sync, no Smithay damage or scanout machinery, KMS presentation via VK_KHR_display (wgpu-hal `drm` feature, `create_surface_from_drm_plane`) is untested here and its interplay with a libseat-owned DRM fd is unknown. | Highest. |

## Why A

1. **It works end to end now.** A real terminal (foot) runs in the nested
   Floor with titlebar, rounded clip, shadow and a blurred panel over its
   text. B and C have no equivalent path today.
2. **The hard parts of B are open and mostly unsafe.** Single-plane dmabuf
   import through an `unsafe` hal call, no wgpu-side explicit sync, no
   Smithay `Renderer`. Each is solvable, none is cheap.
3. **Effects are the small part.** The Floor needs window chrome (SDF
   shapes, one shadow, one blur). Shell and app UI are Wayland clients
   that already use the wgpu renderer; a Frost panel in the Shell asks the
   Floor for blur with ext-background-effect-v1 and the Floor blurs behind
   it. That removes the main reason to share a renderer.
4. **Reversible.** The Floor's state, protocol handling, input and window
   model (`state.rs`, `common.rs`) are renderer independent and tested
   headless. Only `render.rs` and `shaders.rs` (about 1k lines) are
   GLES-specific.

Assumption to confirm: the Floor itself does not have to render the
user-authored scene (World). If it does, sharing the wgpu renderer
becomes much more valuable and B or a scene-as-client design (the World
as a fullscreen client below the windows) needs another look.

## Known gaps in the spike

- No damage tracking: every frame redraws everything. Next to fix: render
  through `OutputDamageTracker` with custom elements (`PixelShaderElement`
  exists), blur cache while the backdrop is unchanged.
- No title text, no button glyphs, no resize, maximise, minimise or
  fullscreen, no popup grabs, no layer-shell, no XWayland, no
  fractional scale, no cursor-shape, no primary selection, no
  xdg-activation (foot warns about each of these at start-up).
- Panel is a fixed rectangle, not driven by any client; blur region
  protocol not implemented.
- HiDPI: output pixels are logical pixels (scale 1).
- TTY build: compiles only. Hotplug, multi-output, hardware cursor plane,
  direct scanout, VRR and a session launcher are not done (see
  `docs/floor-tty-plan.md`).
- Window frames of clients that draw their own decorations get only a
  shadow, never a titlebar (decided by whether the client bound the
  decoration protocol).

## What I would test next

1. The checklist on the RTX 5070 Ti: nested under Hyprland, then the
   TTY build. Send back the numbers and logs it lists.
2. The same bench on the Intel iGPU baseline machine, with
   `intel_gpu_top` running, at 1080p and 4K. Decide from that whether
   blur needs caching before anything else.
3. Real dmabuf clients (mpv, a Vulkan app, Firefox) in the nested and TTY
   builds, and `wp_linux_drm_syncobj` explicit sync on NVIDIA.
4. Port the draw loop onto `OutputDamageTracker` and `DrmCompositor`
   (direct scanout, overlay planes) and re-measure.
5. Implement ext-background-effect-v1 and drive the panel from a client.
6. Only if A hits a wall: a one-day wgpu-hal prototype of dmabuf import
   plus VK_KHR_display presentation, to price B with facts.

## Licensing

Smithay is MIT (`cargo info smithay`); the new dependency tree (Smithay,
winit already in use, drm, gbm, input, libseat bindings, wayland-rs,
smithay-client-toolkit) passes `cargo deny check` with the existing allow
list. `deny.toml` is unchanged; nothing was silently allowed.
