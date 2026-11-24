<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Accessibility in Livspeil UI

Livspeil UI is built to be accessible from the first widget, not
retrofitted (UI spec, principle 5 and section 10). Every element can say
what it is, what it is called, what state it is in and what can be done
with it. Screen readers such as VoiceOver (macOS) and Orca (Linux, through
AT-SPI) read this information through [AccessKit](https://accesskit.dev).

Two crates are involved:

- `livspeil-ui-core` has the semantics model (`semantics.rs`). It does not
  depend on AccessKit.
- `livspeil-ui-a11y` turns the core's retained tree into AccessKit tree
  updates and turns AccessKit action requests back into core actions.

## The semantics model

An element can declare:

| Property | Builder | Meaning |
|---|---|---|
| role | `.role(Role::Button)` | What the element is: `Window`, `Dialog`, `Group`, `Label`, `Heading`, `Button`, `Link`, `CheckBox`, `Switch`, `Slider`, `ProgressIndicator`, `TextInput`, `Image`, `List`, `ListItem`, `ScrollView`, `Menu`, `MenuItem`, `TabList`, `Tab`, `Toolbar`, `Generic` |
| label | `s.label("Volume")` | The accessible name |
| description | `s.description("…")` | Extra help text |
| value | `s.value(SemanticValue::text("Ada"))` or `SemanticValue::number(v, min, max, step)` | Text or numeric value |
| checked | `s.checked(true)` or `Toggled::Mixed` | Check box or switch state |
| expanded | `s.expanded(false)` | Expanded or collapsed |
| selected | `s.selected(true)` | Selection state (list items, tabs) |
| disabled | `s.disabled(true)` | Shown, but cannot be used |
| hidden | `s.hidden(true)` | Not shown to assistive technology, with its subtree |
| actions | `s.action(SemanticAction::ScrollIntoView)` | Extra supported actions |

All properties except the role are set with `.semantics(|s| …)`. You can
call it more than once, and each call edits the semantics already there.
For example:

```rust
div()
    .role(Role::Slider)
    .semantics(|s| s.label("Volume").value(SemanticValue::number(0.5, 0.0, 1.0, 0.1)))
    .on_set_value(|req, cx| { /* ValueRequest::Number(v) */ })
```

### Inferred actions

Most actions come from the element's handlers. You do not need to declare
them:

| Action | Supported when the element has |
|---|---|
| Activate (click) | `.on_click`, or an `on_action("ui.activate")` handler |
| Focus | a tab index (`.focusable()`, `.tab_index(n)`) |
| Increment / Decrement | `on_action("ui.increment")` / `("ui.decrement")` handlers |
| Set value | `.on_set_value(…)` |
| Scroll into view | only when declared |

A disabled element supports no actions. Every request to a disabled
element, or to an element inside a hidden subtree, is refused with a typed
`SemanticActionError`.

### Which elements are exposed

Assistive technology does not see every `div`. An element is exposed when
it:

- is the window root (role `Window`),
- declares a role,
- is a text element (role `Label`, and its text is the name), or
- is focusable or has a label (role `Generic`).

The other elements are transparent: their exposed descendants are attached
to the nearest exposed ancestor. Buttons, links, check boxes, switches,
sliders, progress indicators, text inputs, images, labels, headings, tabs
and menu items have presentational children, as in ARIA. The screen reader
sees only the control itself. The text inside a button is not read twice.

### How widgets declare semantics

The widgets in `livspeil-ui-widgets` describe themselves with
`livspeil_ui_widgets::Semantics` (`semantics()` on each widget) and apply
it to their element with the core's builders (`.role(..)`,
`.semantics(..)`, `.on_set_value(..)`). The widget table in
`docs/widgets.md` lists each widget's role. Callers can add to a widget's
semantics on the rendered element, for example
`Toggle::new(on).render(&theme).semantics(|s| s.label("Wi-Fi"))`. A new
widget declares its role and state in `render`, in the same way.

## From the core to AccessKit

`livspeil_ui_a11y::A11yTree` is the adapter-facing API:

```rust
let mut a11y = A11yTree::new();
// after each frame, focus change or action:
if let Some(update) = a11y.update(&app, scale_factor) {
    adapter.update_if_active(|| update);
}
// from the adapter's ActionHandler:
handle_action(&mut app, request)?;
```

- **Node ids.** `NodeId(element_id.0)`. Element ids are hashes of the
  parent id and the key (or the index among unkeyed siblings). A node keeps
  its id across renders, and keyed children keep theirs when they are
  reordered.
- **Bounds.** Absolute layout bounds (before animated transforms) are
  multiplied by the scale factor, so they are in physical pixels relative
  to the window.
- **Focus.** The focused element, or its nearest exposed ancestor. When
  nothing is focused, the focus is the root, as AccessKit requires.
- **Values.** A `Label`'s text is its AccessKit `value`, following the
  AccessKit convention. Other roles use `label`. Numeric values set
  `numeric_value`, `min_numeric_value`, `max_numeric_value` and
  `numeric_value_step`.
- **Actions.** AccessKit to core: `Click` → activate (the click handler, or
  else `ui.activate` from the target upward), `Focus` / `Blur`,
  `Increment` / `Decrement` → `ui.increment` / `ui.decrement` from the
  target upward, `SetValue` (text or numeric data) → `on_set_value`,
  `ScrollIntoView` → `ui.scroll_into_view` from the target upward. Scroll
  containers handle that action once they exist. Other actions return
  `A11yError::UnsupportedAction`.

### Incremental updates

`A11yTree` keeps the last AccessKit node it sent for every id. An update
works like this:

1. Walk the retained tree and build the AccessKit node of every exposed
   element. This is cheap: there is no layout and no allocation beyond the
   nodes.
2. Send only the nodes that differ from the cached ones, plus the focus.
   The first update (or the first after `reset` / `full_update`) sends the
   full tree with `TreeInfo`.
3. Removed nodes are not sent. AccessKit drops them when their parent's
   new child list no longer has them, so a parent whose children changed
   is always among the sent nodes.
4. When nothing changed, including the focus, `update` returns `None` and
   the host sends nothing.

A text change sends one node. A keyed reorder sends the parent (new child
order) and the children whose bounds moved, with the same ids. A focus
change sends no nodes. The walk is O(exposed nodes) per update. If
profiling shows a need, the core can later add a semantics generation
counter so that clean subtrees are skipped.

## Testing approach

The tests are in `crates/livspeil-ui-a11y/tests/tree.rs` and run headless
with `TestHost`:

- **Snapshot.** `A11yTree::dump()` prints one line per node with its role,
  name, value, range, states, actions and bounds, and `*` marks the focus.
  The sample view (`livspeil-ui-widgets` Button, Toggle and Slider, a
  list, a hidden subtree, a disabled button and a text input inside a
  transparent wrapper) is compared with
  `tests/golden/a11y-tree.golden`. Refresh it with `UPDATE_GOLDEN=1`.
- **Incremental.** Changing one label sends exactly that node. A keyed
  reorder keeps ids and removes nothing. Adding and removing items, hiding
  and showing, and disabling and enabling send the expected nodes.
- **Focus.** Tab traversal and AccessKit `Focus` / `Blur` are reported.
- **Round trips.** An AccessKit `Click` fires the button and toggle
  handlers. `Increment`, `Decrement` and `SetValue` change the slider model.
  Disabled and hidden targets, unknown trees and unsupported actions give
  typed errors.
- **Property test.** Random sequences of mutations (insert, remove and
  swap list items, rename, hide, disable, click, increment, focus) are
  applied. After each step the test applies the update to a mirror that
  behaves like a platform adapter. It then checks that every child id
  exists, every sent node is reachable from the root, no node has two
  parents, no id appears twice in an update, and the focus is in the tree.
  Finally, it checks that the incrementally maintained mirror equals a
  fresh full tree.

## Plan for VoiceOver and AT-SPI

The platform side lives in the host, not in these crates:

1. **winit host (macOS first).** Use `accesskit_winit::Adapter`, created
   before the window is shown. In `ActivationHandler::request_initial_tree`,
   return `A11yTree::full_update(&app, window.scale_factor())`. After every
   drawn frame, and after input that changes focus, call
   `adapter.update_if_active(|| a11y.update(..))` when `update` returns
   `Some`. Forward `ActionHandler` requests to the UI thread and call
   `handle_action`, then request a frame. On `DeactivationHandler`, call
   `A11yTree::reset`.
2. **VoiceOver (macOS).** `accesskit_winit` uses `accesskit_macos`.
   Manual checks: VO navigation reads every widget with role, name
   and value, VO-Space activates, VO-Shift-Up/Down adjusts the slider, and
   the focus ring follows VoiceOver focus.
3. **AT-SPI (Linux).** `accesskit_winit` uses `accesskit_unix` (D-Bus,
   AT-SPI). Run the same checks with Orca. The Wayland layer-shell Shell
   host uses `accesskit_unix` directly with the same `A11yTree`.
4. **Later.** Window title as the root name, text selection and caret for
   text inputs (`SetTextSelection`, text runs), scroll containers handling
   `ui.scroll_into_view` and the scroll actions, live regions for
   notifications, and `transform` on nodes for animated or canvas-zoomed
   content. Reduce motion, reduce transparency, high contrast and text
   scale are already in the theme and motion configuration
   ([ui-config.md](ui-config.md)).
