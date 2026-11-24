<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Livspeil UI widgets

`crates/livspeil-ui-widgets` (MPL-2.0) is the first real widget set of
Livspeil UI (UI spec §9). Every widget is headless and tested with the core's
`TestHost`: behavior tests, scene snapshots (text dumps of paint commands) in
`tests/golden/`, and a property test for text editing.

## Common behavior

- **Density.** Sizes follow the theme density: Simple uses 32 px rows and
  13 px body text, Pro uses 24 px rows and 12 px text, with smaller
  indicators (16 → 14 px), switches (36×20 → 28×16) and paddings.
- **Scheme.** Colors come from the theme's color roles (`surface.0..3`,
  `text.*`, `accent`, `danger`, `focus-ring`), so dark and light both work;
  the accent is the contrast-clamped adaptive accent.
- **States.** Hover, press and keyboard focus change visuals through element
  refinements animated with the motion profile's springs (micro for
  controls, standard for the segmented pill and the select popup). Disabled
  widgets render at 40% opacity, are not focusable, have no handlers and are
  reported disabled.
- **Focus.** Tab / Shift+Tab move through focusable widgets; the core draws
  the focus ring when focus came from the keyboard. Text and number fields
  also show an accent border and caret whenever they are focused.
- **Semantics.** Each widget has `semantics()` returning a
  `livspeil_ui_widgets::Semantics` (role, label, value, numeric range,
  toggled/selected/expanded state, placeholder, text selection, disabled,
  focusable, supported actions, and parts as children). The same description
  is declared on the widget's element with the core accessibility builders
  (`Semantics::apply`), so `livspeil-ui-a11y` exposes it. Roles the core
  does not have yet map to the closest core role (see the table).
- **Keymap.** Widget keys are keymap actions in key contexts, installed by
  `Env::new` (`livspeil_ui_widgets::WIDGET_KEYMAP`) and rebindable in
  `keymap.toml`.

## Setup

```rust
let mut app = App::new();
let env = Env::new(&mut app, FontSystem::new()); // or Env::headless(&mut app)
let content = app.new_view(MyView { env: env.clone(), /* … */ });
let root = env.root(&mut app, content); // overlay layer + focus tracking
app.set_root(root);
```

`Env` holds the font system (text layout for fields and tooltips), the
clipboard (`Clipboard` trait; hosts install the platform clipboard,
`MemoryClipboard` is the default) and focus tracking. `WidgetRoot` is a 1×1
grid: the content and the overlay layer share the cell, so popups and
tooltips paint and hit-test above everything.

## Widgets

| Widget | Kind | Semantics (core role) | Keyboard |
|---|---|---|---|
| `Button` (primary, secondary, ghost, destructive) | builder | Button (Button) | Space/Enter activate |
| `IconButton` | builder | Button, named by its label | Space/Enter activate |
| `Toggle` | builder | Switch, toggled (Switch) | Space/Enter flip |
| `Checkbox` | builder | CheckBox, toggled incl. mixed (CheckBox) | Space/Enter: mixed/unchecked → checked → unchecked |
| `Radio` | builder | RadioButton, selected (CheckBox, checked = selected) | Space/Enter select |
| `RadioGroup` | builder | RadioGroup with RadioButton parts (Group) | one tab stop; arrows next/previous enabled option, wrapping; Home/End; Space/Enter select the first if none |
| `SegmentedControl` | builder | SegmentedControl with Segment parts (TabList / Tab) | one tab stop; Left/Right (no wrap); Home/End |
| `Slider` | view, `Model<f32>` | Slider, numeric (Slider + SetValue) | arrows ±step; Shift or Page Up/Down ×10; Home/End |
| `NumberField` | view, `Model<f64>` | SpinButton, numeric (Slider + SetValue) | Up/Right +step, Down/Left −step, Shift ×10, Alt ×0.1, Page Up/Down ×10, Home/End; Enter/Space or a digit start typing |
| `TextField` | view | TextInput with value, placeholder, selection (TextInput + SetValue) | see below |
| `Select` | view, `Model<Option<usize>>` | ComboBox, value, expanded; ListBoxOption parts (Button + expanded; ListItem) | see below |
| `Tooltip` | attached to an element | Tooltip (bubble: Label; anchor gets the text as its description) | — |
| `Label` | builder | Label | — |

Builders render with `render(&theme)` and report changes through callbacks
(`on_click`, `on_change`, `on_select`); the caller owns the state. Views own
editing or popup state and write to their model.

### Button and IconButton

Rest, hover (+lightness), pressed (scale 0.97) and keyboard focus (focus-ring
border) animate with the micro spring. Primary uses the accent, destructive
the danger color, secondary a raised surface with a hairline, ghost has no
surface until hovered. The icon slot of `IconButton` is a sized placeholder
box named `icon:<name>` until the icon atlas exists.

### Toggle, Checkbox, Radio, RadioGroup

The switch thumb and the check mark / radio dot spring between states and
keep their velocity when retargeted mid-flight. The whole row (indicator and
label) is the click and focus target.

### SegmentedControl

Equal-width segments on a track; the selected segment sits on a pill that
slides with the standard spring (no visible overshoot in the snappy profile,
settles in well under a second, keeps velocity when retargeted).

### Slider

Press jumps to the pointer; dragging follows the pointer 1:1 (no spring);
release snaps to the step and the thumb springs there with the release
velocity. The thumb grows on hover (×1.15) and press (×1.25). Optional value
label (`show_value`).

### NumberField

- **Scrub:** press and drag horizontally. After the 3 px drag threshold the
  raw value changes by `dx × sensitivity × precision`, where `sensitivity`
  defaults to `step` (one pixel = one step) and precision is Shift ×10,
  Alt ×0.1 (both or neither ×1). Modifiers are read on every move. The raw
  value is clamped to `min..=max` (so reversing responds at once at a
  bound) and shown rounded to multiples of `step` (of `step × 0.1` while
  Alt is held), clamped, without float noise. Vertical movement is ignored.
- **Type:** a click without dragging, Enter/Space, or typing a digit, sign
  or decimal point starts editing (the text is selected; a typed character
  replaces it). Enter commits, Escape reverts, Tab/Shift+Tab commit and move
  focus, losing focus commits. Typed values are clamped but not snapped;
  `,` is accepted as the decimal point; invalid text reverts.
- **Display:** the step's decimals (or `decimals(n)`), plus up to three more
  when the value is finer (Alt scrubbing). The editor lays out digits with
  tabular figures (`tnum`). A fill bar shows the position in a finite range.

### TextField

Single line, backed by `livspeil_ui_text::Editor` and laid out with
`livspeil-ui-text` for caret and hit testing.

| Keys (context `TextField`) | Action |
|---|---|
| Left / Right | caret by grapheme (visual, bidi-aware); Shift extends |
| Ctrl or Alt + Left / Right | by word; Shift extends |
| Home / End, Cmd+Left / Right, Up / Down | to the start / end; Shift extends |
| Backspace / Delete | delete the selection or a grapheme; Ctrl or Alt: a word |
| Ctrl/Cmd+A | select all |
| Ctrl/Cmd+C, X, V | copy, cut, paste (`edit.*` actions, through `Env`'s clipboard; pasted line breaks become spaces) |
| Ctrl/Cmd+Z, Ctrl/Cmd+Shift+Z, Ctrl+Y | undo, redo (typing is grouped, broken at spaces and moves) |
| Enter | submit (`on_submit`) |
| Escape | cancel an IME composition |

Mouse: click places the caret, Shift+click and drag extend the selection,
double-click selects a word, triple-click selects all. The field scrolls
horizontally to keep the caret visible. **IME:** the preedit is shown at the
caret with a 1 px underline, and the input method's target range with a
2 px underline; caret moves are ignored while composing; commit inserts the
text as one undo step. A placeholder is shown while empty.

### Select

Click, Space, Enter, Alt+Down, Up or Down open the popup with the chosen
option highlighted. While open: Up/Down move over enabled options (no wrap),
Home/End go to the ends, Enter/Space choose, Escape closes, Tab closes and
moves focus; hovering highlights, clicking an option chooses it, clicking
outside closes. Typing a letter highlights (open) or chooses (closed) the
next option starting with it. While closed, Home/End choose the first/last
option. The popup is a Glass list in the overlay layer, below the trigger
(above when there is no room), fading and scaling in with the standard
spring; focus returns to the trigger after choosing.

### Tooltip

`Tooltip::new(text).attach(&env, element)`. After the pointer rests on the
element for the delay (500 ms by default), a Glass bubble appears centered
below it (above when there is no room, clamped to the window) and fades in.
It hides when the pointer leaves the element or a button is pressed. While a
tooltip is pending the overlay renders once per frame; once shown the app is
idle again.

## Known gaps

- Builders capture state when rendered; several key presses between two
  frames act on the same state (hosts draw a frame between events).
- Focus changes made directly with `App::focus` are noticed by text and
  number fields only at the next `Env::sync_focus` (pointer press, Tab,
  key event). Use `Env::focus` in code.
- No caret blink (idle means zero frames), no cursor shapes, no IME cursor
  area reporting and no system clipboard yet: these need host hooks.
- Popups and tooltips are removed without an exit animation.
- The core proof widgets (`livspeil_ui_core::widgets`) remain as test
  fixtures for the core and `livspeil-ui-a11y`.
