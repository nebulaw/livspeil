<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Known upstream issues

Bugs in dependencies that Livspeil works around, written so they can be
filed upstream as they are. Remove an entry (and its workaround) once a
fixed release is adopted.

## macOS (Metal): Frost and Glass blur did not render

**Status:** fix in this tree, **not verified on a Mac** (no Metal, no real
GPU in the environment that wrote it). Linux/Vulkan was never affected.

### Symptom

On a MacBook the gallery's Materials page showed no blurred backdrop
(Frost and Glass looked like flat opaque panels, or wrong). On Linux
(Vulkan) they blur correctly.

### Suspects, ranked

1. **The Metal surface cannot be sampled (evidence from source; very
   likely the cause).** `wgpu-hal` 30.0.2, `src/metal/adapter.rs`
   `surface_capabilities`, reports the usage `COLOR_TARGET | COPY_SRC |
   COPY_DST | STORAGE_READ_ONLY | STORAGE_WRITE_ONLY | STORAGE_READ_WRITE`
   and no `RESOURCE` (= `TEXTURE_BINDING`). Vulkan maps `SAMPLED` to it
   (`vulkan/conv.rs::map_vk_image_usage`) and swapchains offer it. The
   host configured `RENDER_ATTACHMENT | (caps.usages & TEXTURE_BINDING)`,
   which on Metal is `RENDER_ATTACHMENT` alone, and `PaintRenderer` then
   replaced every backdrop with an opaque quad of its tint, **silently**.
   That is exactly "blur shows nothing" on Metal and "works" on Vulkan.
   Asking for `TEXTURE_BINDING` anyway is rejected by wgpu validation
   (usage not in the capabilities), and Metal sets `framebufferOnly` when
   the usage is exactly `RENDER_ATTACHMENT`, so sampling a drawable is not
   possible by design.
   *Fix:* render-only targets draw into an internal `TEXTURE_BINDING`
   texture and blit it into the surface (`BlurPath::Offscreen`, see
   `docs/ui-host.md`). Tested on lavapipe: output equals the direct path
   bit for bit (`crates/livspeil-ui-render/tests/blur_paths.rs`).
2. **`Rgba16Float` blur levels not filterable/renderable (hypothesis,
   unlikely).** Apple GPUs filter and render `Rgba16Float`; wgpu's
   `downlevel_defaults` limits do not forbid it. If a device rejected it,
   pipeline or bind group creation fails validation. *Now detected:* the
   blur pipelines and a probe bind group are built in an error scope at
   `Renderer::new`; failure selects `Fallback` with the validation text
   (before: an uncaptured error, a panic by default).
3. **Surface format (hypothesis, unlikely).** Metal offers `Bgra8Unorm`
   (and sRGB variants); the host already picks the non-sRGB one, so the
   offscreen texture and the blit share it. A different format would
   select `Fallback` with the reason.
4. **Scissor or region rounding on Retina scales (hypothesis, low).**
   Blur passes scissor to the level textures; wgpu validates the
   rectangle, and the CPU-vs-GPU golden test covers scale 1 and 2 on
   lavapipe. Not seen as a Metal-only issue; unverified.
5. **Alpha mode (not blur-related).** Metal offers `Opaque` and
   `PostMultiplied`; the transparent-window hook asks for `PreMultiplied`
   and falls back to the first offered mode. Irrelevant to the gallery.

### What the owner can check

`cargo run -p livspeil-ui-gallery -- --gpu-info` and paste the output.
Expected on Metal after the fix: surface usages without `TEXTURE_BINDING`,
`blur device check: supported`, `blur path: offscreen texture + blit`. A
`FALLBACK` line carries the reason. `RUST_LOG=info cargo run -p
livspeil-ui-gallery -- materials` logs the same path on the first frame.

### Fallback (D63)

If blur cannot run, backdrops are drawn as plain opaque quads of their own
colour (no blur, no extra tint), one warning with the reason is logged and
the Materials page shows a note. The CPU reference renderer and the
screenshot goldens are unchanged.

## Taffy 0.16.0: stale hoisted out-of-flow lists panic in `round_layout`

**Status:** not fixed in taffy 0.16.0 (latest release) nor on `main` as of
2026-11. Worked around in `livspeil-ui-layout` (`LayoutTree` disables
Taffy's rounding and rounds itself); regression tests in
`crates/livspeil-ui-layout/tests/mutations.rs`.

### Summary

Removing an absolutely positioned child from an absolutely positioned
parent, so that the parent has no children left, makes the next
`compute_layout` panic with `invalid SlotMap key used` inside
`round_layout`.

### Reproduction

```rust
use taffy::prelude::*;

fn abs(x: f32) -> Style {
    Style {
        position: Position::Absolute,
        inset: Rect { left: length(x), right: auto(), top: length(0.0), bottom: auto() },
        size: Size { width: length(10.0), height: length(10.0) },
        ..Style::default()
    }
}

let mut t: TaffyTree<()> = TaffyTree::new();
let root = t.new_leaf(Style::default()).unwrap();
let parent = t.new_leaf(abs(5.0)).unwrap();
let child = t.new_leaf(abs(1.0)).unwrap();
t.set_children(root, &[parent]).unwrap();
t.set_children(parent, &[child]).unwrap();
t.compute_layout(root, Size::MAX_CONTENT).unwrap();
// hoisted_children(parent) == [child]
t.remove(child).unwrap();
t.compute_layout(root, Size::MAX_CONTENT).unwrap(); // panics: invalid SlotMap key
```

It does not panic if the parent keeps another child, or if the parent is
in flow (`position: relative`): in this case the root, not the parent,
ends up recording the child.

### Cause

`compute_oof_layout` records the out-of-flow boxes a containing block lays
out in its `hoisted_children` list, and `round_layout` walks that list.
The list is only rewritten when the node is laid out as a container
(`oof_positioning_area` is `Some`). `TaffyView::compute_child_layout`
dispatches a node with no children to the leaf measure function, whose
output has `oof_positioning_area: None`, so `compute_oof_layout` returns
early and the list of a node that just lost its last child keeps the
removed id. (`compute_empty_container_layout` in `flexbox.rs` sets the area
to `Some` precisely to clear stale lists, but a childless node never gets
there.) `compute_hidden_layout` has the same gap: a positioned box that is
`display: none` keeps its old list, so removing its child panics too, and a
reparented child is visited twice (once with the wrong origin).

Suggested fix: clear the hoisted list whenever a node is laid out without
running `compute_oof_layout` (leaf and hidden paths), or clear it in
`remove`/`remove_child`/`set_children` for every ancestor that lists the
child.

## Taffy 0.16.0: changes under a `display: none` box are not laid out

**Status:** same as above; worked around by `LayoutTree` clearing the cache
of every ancestor itself after each mutation.

`compute_hidden_layout` clears the cache of the hidden box's descendants
(through `LayoutInput::HIDDEN`, which bypasses `compute_cached_layout`) and
never refills it, while the hidden box itself is cached by its parent's
pass. `TaffyTree::mark_dirty` stops at the first node whose cache is
already empty, so a change below the hidden box (for example moving a
visible node into it with `set_children`) never dirties the hidden box or
the root, and the moved node keeps its old, visible layout. A from-scratch
layout of the same tree gives it a zero layout.

```rust
let mut t: TaffyTree<()> = TaffyTree::new();
let root = t.new_leaf(Style::default()).unwrap();
let hidden = t.new_leaf(Style { display: Display::None, ..Style::default() }).unwrap();
let inner = t.new_leaf(Style::default()).unwrap();
let leaf = t.new_leaf(Style { size: Size::from_lengths(30.0, 20.0), ..Style::default() }).unwrap();
t.set_children(root, &[hidden, leaf]).unwrap();
t.set_children(hidden, &[inner]).unwrap();
t.compute_layout(root, Size::MAX_CONTENT).unwrap();
t.set_children(inner, &[leaf]).unwrap();
t.compute_layout(root, Size::MAX_CONTENT).unwrap();
assert_eq!(t.layout(leaf).unwrap().size, Size::ZERO); // fails: still 30×20
```

## AccessKit's Linux adapter depends on the retired zbus 5 helper crates

**Status:** as of 2026-11-26. `accesskit_unix` 0.24.1 (the latest release)
depends on zbus 5 and `atspi`. The three advisories are ignored by id in
`deny.toml`.

RUSTSEC-2026-0370 (`zvariant`), -0371 (`zbus_names`) and -0372
(`zvariant_derive`) mark these crates as unmaintained: they were merged
into `zbus` 6.0. They are not vulnerability reports. No `cargo update` can
remove them, and dropping them would mean dropping screen-reader support on
Linux.

AccessKit ignores the same three ids upstream
(<https://github.com/AccessKit/accesskit/pull/834>): zbus 6 needs Rust 1.89,
above their minimum.

Remove the ignores when an `accesskit_unix` release builds on zbus 6.

