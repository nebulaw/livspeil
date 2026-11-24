<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Known upstream issues

Bugs in dependencies that Livspeil works around, written so they can be
filed upstream as they are. Remove an entry (and its workaround) once a
fixed release is adopted.

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
