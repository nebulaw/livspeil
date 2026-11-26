<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Livspeil UI widgets

`crates/livspeil-ui-widgets` (MPL-2.0) is the widget set of
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
  (`Semantics::apply`), so `livspeil-ui-a11y` exposes it. Each widget role
  has the core (and AccessKit) role of the same name, except the
  segmented control, exposed as a tab list of tabs (see the table).
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
`MemoryClipboard` is the default) and focus tracking. `WidgetRoot` puts the
content first and the overlay layer, an absolutely positioned box the size
of the viewport, after it, so popups and tooltips paint and hit-test above
everything.

## Text layout

A text element is one box of its natural (unwrapped) width. When layout gives
it less width than that (a stretched child of a narrow column, a flex item
with `min_size.width = 0`), it is as tall as its wrapped lines. Text never
makes a flex item shrink by itself. The height comes from
`TextMeasurer::wrapped_height`, whose default breaks greedily after spaces
and hyphens.

`Element::gradient_x(from, to, start, end)` fills an element with a
left-to-right gradient (used by the tab fades).

Popups (`Popover`, `Select`, `Tooltip`) anchor at `App::visual_bounds`, the
element's position after its ancestors' transforms, so they open in the right
place inside a scrolled view.

## Widgets

| Widget | Kind | Semantics (core role) | Keyboard |
|---|---|---|---|
| `Button` (primary, secondary, ghost, destructive) | builder | Button (Button) | Space/Enter activate |
| `IconButton` | builder | Button, named by its label; toggle style reports toggled | Space/Enter activate |
| `Tabs` | view, `Model<usize>` | TabList with Tab parts (TabList / Tab) | one tab stop; Left/Right (no wrap), Home/End; Delete/Backspace close |
| `Popover` | view | Popover group named by its label (Group); trigger reports expanded | Escape closes; Tab stays inside |
| `Dialog` (also sheet) | view | Dialog named by the title, described by the message (Dialog) | Tab trapped; Enter default, Escape cancel |
| `Toasts` | view, `Toast` values | each toast a live region: Status (polite) or Alert (danger) (Status / Alert) | Escape in a toast dismisses |
| `Toggle` | builder | Switch, toggled (Switch) | Space/Enter flip |
| `Checkbox` | builder | CheckBox, toggled incl. mixed (CheckBox) | Space/Enter: mixed/unchecked → checked → unchecked |
| `Radio` | builder | RadioButton, selected (RadioButton, checked = selected) | Space/Enter select |
| `RadioGroup` | builder | RadioGroup with RadioButton parts (RadioGroup) | one tab stop; arrows next/previous enabled option, wrapping; Home/End; Space/Enter select the first if none |
| `SegmentedControl` | builder | SegmentedControl with Segment parts (TabList / Tab) | one tab stop; Left/Right (no wrap); Home/End |
| `Slider` | view, `Model<f32>` | Slider, numeric (Slider + SetValue) | arrows ±step; Shift or Page Up/Down ×10; Home/End |
| `NumberField` | view, `Model<f64>` | SpinButton, numeric (SpinButton + SetValue) | Up/Right +step, Down/Left −step, Shift ×10, Alt ×0.1, Page Up/Down ×10, Home/End; Enter/Space or a digit start typing |
| `TextField` | view | TextInput with value, placeholder, selection (TextInput + SetValue) | see below |
| `Select` | view, `Model<Option<usize>>` | ComboBox, value, expanded; ListBoxOption parts in a ListBox popup (ComboBox; ListBox; ListBoxOption) | see below |
| `Tooltip` | attached to an element | Tooltip (bubble: Tooltip; anchor gets the text as its description) | — |
| `Label` | builder | Label | — |
| `ScrollView`, `VirtualList` | view | ScrollView (ScrollView); virtual rows in a List | context `ScrollView`: arrows a line, Page Up/Down a page, Home/End |
| `Panel` | builder | Panel (Group, named by the title) | — |
| `DockView` | view | splitters: Splitter (Separator, numeric size) | — |
| `Menu` / `ContextMenu` | data + `MenuPopup` view | Menu with MenuItem / MenuItemCheckBox / Separator parts | context `Menu`, see below; Shift+F10 opens a context menu |
| `MenuBar` | view | MenuBar with MenuItem titles (MenuBar; MenuItem, expanded) | F10 focuses, Left/Right, Down/Enter open; Alt+letter |
| `Titlebar` | builder | Titlebar (Toolbar) | — |
| `NodeCanvas` | view over a `NodeGraphModel` | NodeCanvas (Group); cards Node (Group, selected) | context `NodeCanvas`: Ctrl/Cmd+A, Escape, Space (pan) |

Builders render with `render(&theme)` and report changes through callbacks
(`on_click`, `on_change`, `on_select`); the caller owns the state. Views own
editing or popup state and write to their model.

### Button and IconButton

Rest, hover (+lightness), pressed (scale 0.97) and keyboard focus (focus-ring
border) animate with the micro spring. Primary uses the accent, destructive
the danger color, secondary a raised surface with a hairline, ghost has no
surface until hovered.

`IconButton` icons are Boxicons names (`bx-copy`) from the embedded set (see
[Icons](#icons)), drawn in the button's foreground color; the icon element is
named `icon:<name>`. Sizes are `IconSize`
14/16/20/24 (the button is the icon plus 5 px each side; without `size` the
icon and button follow the density). Styles (`IconButtonStyle`): plain,
`filled()` (accent, or the `ButtonVariant` color) and `toggle(on)`, which
sits on the same raised pill as the selected segment while on and is
exposed as a toggle button. `tooltip(&env)` shows the label as a tooltip.

### Icons

`livspeil_ui_core::icon(name)` is a 16x16 element (set another size with
`size`, a color with `text_color`; default: primary text color). The set is
the 49 [Boxicons](../assets/icons/boxicons/README.md) files in
`assets/icons/boxicons/` (CC BY 4.0); `livspeil_ui_icons::IconId::all` lists
the names, and the `icons-*` screenshot scenes show them. It is a small set:
there is no trash, plus, lock, pin or cross icon, for example.

A name outside the set is not an error: it draws an outlined, rounded box of
the icon's size in the icon's color, so the mistake is visible. Icons are
decorative and absent from the accessibility tree; whatever shows one needs
its own label (`IconButton` requires one). Toast close buttons use the "×"
character, as tab close buttons do, because the set has no cross.

### Toggle, Checkbox, Radio, RadioGroup

The switch thumb and the check mark / radio dot spring between states and
keep their velocity when retargeted mid-flight. The whole row (indicator and
label) is the click and focus target.

### SegmentedControl

Equal-width segments on a track; the selected segment sits on a pill that
slides with the standard spring (no visible overshoot in the snappy profile,
settles in well under a second, keeps velocity when retargeted).

### Tabs

`Tabs::new(&env, selected, ["Scene", ...])` (`TabItem` for per-tab
`closable`/`disabled`). The selected tab sits on a raised pill (D60): dark,
a lighter pill on a darker track; light, a bright pill on a slightly darker
track, with only the faint `e1` contact shadow. The pill's position and
width spring with the Standard class of the `tabs` motion scope (velocity
kept when retargeted; instant under Reduce Motion). Tabs are as wide as
their title (56 to 220 px) and the bar is never wider than its container;
when tabs do not fit it scrolls horizontally (wheel, trackpad; a vertical
wheel also works; the content moves with the delta, as in `ScrollView`) and
choosing a tab by click or keys, the selected one included, scrolls it fully
into view. An end that hides tabs fades into the track color (also on Frost,
as the track is opaque) and shows a chevron (`tabs-more-left`,
`tabs-more-right`) that scrolls 60% of the width without changing the
selection; with no hidden tabs there is neither, and a bar that fits leaves
wheel events to its ancestors. Scrolling is a simple spring, without the
momentum of `ScrollView`. Close buttons
(`close:<title>`) remove the tab, choose the tab that took its place and
call `on_close(index)`; `Tabs::push` and `Tabs::close` do the same from
code. Keys (context `Tabs`): Left/Right, Home/End, Delete/Backspace
(`tabs.close`).

### Popover

`Popover::new(&env, |app| content).trigger(|theme| element).placement(..)
.size(w, h)`; or `Popover::open_at(view, app, anchor, restore)`. An
arrow-less Glass panel placed top, bottom, left or right of the anchor by
`place_popover`: the preferred side if it fits, else the opposite side if
that fits, else the roomier one, always clamped to an 8 px margin. It fades
in and slides 4 px away from the anchor (no scale, D61) and fades out when
closed; under Reduce Motion the slide is instant. Escape and a click
outside (consumed by a scrim) close it; focus moves into the panel (Tab
stays in it) and returns to the trigger. Overlay layers (`OverlayKind::Layer`)
know when they are leaving, so closing animates and removal happens after
0.2 s (at once when motion is instant).

### Dialog and sheet

`Dialog::new(&env, title).message(..).button(DialogButton::new("Delete")
.destructive().default_button().on_press(..))`, `.sheet()` for a sheet
attached to the window's top edge. A scrim (black at 45% dark, 25% light)
takes every press (`dismiss_on_scrim` makes it cancel). Focus goes to the
default button and Tab cycles inside the scrim and panel (a focus scope);
Enter presses the default button unless a button has focus (then that
one), Escape presses the cancel button (or just closes). Any button closes
and then runs its callback; focus returns to the opener. Dialogs fade in and
slide 8 px; sheets slide down from above the window with the Large class;
neither scales; both fade out. Height is measured from the wrapped title and
message plus `content(height, ..)`.

### Toasts

`Toasts::new(&env, app)` once per app, then `Toasts::show(view, app,
Toast::new("Saved").title(..).tone(..).action("Undo", f).duration(..)
.sticky())`. Toasts stack at the bottom right (newest lowest, at most
four); older ones move up with the Standard spring. They slide in from the
right while fading (the slide is instant under Reduce Motion) and fade out
after `TOAST_DURATION` (4 s) counted with the frame clock; the countdown
pauses while the pointer is over a toast or focus is on its buttons. The
action button runs and dismisses; the close button and Escape dismiss.
Each toast is a live region: `Status` (AccessKit Status, polite) or `Alert`
(assertive) for the danger tone. A stack with a timed toast renders every
frame; with none it is idle.

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
- **Name:** `label(...)` shows an inline label and names the field;
  `accessible_label(...)` only names it (for fields labelled by a separate
  text, as in node cards).

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

### ScrollView and VirtualList

`ScrollView::new(&env, |app, info| content)` is a view (vertical by
default; `axis(Horizontal | Both)`); the content builder gets the offset and
viewport size and is translated by the offset (no relayout while
scrolling). Physics live in `Scroller` (pure, testable):

- Trackpad pixel deltas move the content 1:1. Past an end they are resisted
  like UIKit's rubber band (`x → (1 − 1/(x·0.55/d + 1))·d`, `d` the viewport
  extent). Without a delta for 60 ms the gesture counts as released: the
  content glides with the tracked release velocity, decaying as
  `v·e^(−t/0.325 s)` until below 20 px/s, bounces at the ends with a
  critically damped spring, or springs back from overscroll. (Hosts that
  already deliver OS momentum events can turn `momentum` off.)
- Wheel lines (40 px each), keys and scroll-into-view spring to a target
  (Standard spring of the `scroll` motion scope, never overshooting).
- Reduce Motion: no overscroll and no bounce; targets jump. Speed 0 also
  turns momentum off. `ScrollPhysics::from_motion` derives this every frame;
  `physics(...)` fixes it.
- Overlay scrollbars (6 px thumbs) fade in while scrolling or hovered and
  out 0.9 s later; thumbs drag. Overscroll shrinks the thumb.
- Ctrl/Cmd+scroll is not consumed (zoom gestures of ancestors).
- `ui.scroll_into_view` (assistive technology's ScrollIntoView, and every
  Tab focus move through `WidgetRoot`) scrolls the least distance that
  shows the element with an 8 px margin.

`VirtualList::build(&env, count, row_height, |app, i| row)` is a
`ScrollView` over a spacer of the full height that mounts only the rows in
view plus 4 overscan rows each side (`visible_range`), keyed `row-<i>`: with
100 000 rows a frame holds under 30 rows.

### Panel, DockView

`Panel::new(title)` (Solid, or `frost()`) has a header (title, trailing
actions), a hairline and a clipping body. `DockView::new(center)
.panel(edge, size, min, content)` lays panels out like
`livspeil_ui_layout::Dock` (left/right full height, top/bottom between)
with flexbox, and uses `Dock` for the splitter math: a 1 px hairline on the
inner edge with a 4 px grab area each side, ew/ns-resize cursors, sizes
following the pointer's total travel clamped to the panel minimum and to
what leaves the center its minimum; a double press resets the initial size.
`DockView::toggle` / `set_collapsed` animate the panel width to zero with
the Large spring (scope `panels`; instant under Reduce Motion) while the
content keeps its size and slides out.

### Menus

`Menu::new().item(MenuItem::new("Save").action("app.save"))
.separator().submenu("Arrange", sub)`. Items have a label, an optional icon
(`icon("bx-copy")`, 14 px in the check-mark column; check-mark items show only
the check mark, and submenu entries have no icon), shortcut text
(`shortcut(...)`, or the action's last global binding in the keymap,
formatted `Ctrl+Shift+Z`), a check mark (`checked`), `disabled`, and
`on_select`; choosing closes the menu, returns focus, runs `on_select` and
dispatches the action from the focused element.

One menu system is open at a time (`Env` owns the `MenuPopup` view): Glass
levels in the overlay layer that take focus (key context `Menu`): Up/Down
over enabled items, wrapping; Home/End; Right opens a submenu with its
first item highlighted (or the next menu bar menu); Left closes it (or the
previous menu bar menu); Enter/Space choose; Escape closes one level; Tab
closes all; a typed letter highlights the next item starting with it.
Hovering an item with a submenu opens it after 200 ms (`SUBMENU_DELAY`);
hovering another item closes it after the same delay, so the pointer can
cross items on the way into a submenu. Submenus open to the right of their
item (left at the window edge); pressing outside closes.

`MenuBar` (Frost) opens its menus on press below their titles; while open,
hovering another title switches. Register it with `Env::set_menubar` for
F10 (focus the bar) and Alt+letter (open the menu starting with the
letter); `Env::focus_root` makes these work before anything has focus.
A registered bar that is not in the tree (another page is shown) ignores
both. `MenuBar::replace_menu` swaps a menu, for example to update check
marks after a choice.
`ContextMenu::new(menu).attach(&env, el)` opens on secondary press at the
pointer (Shift+F10 at the element's center), flipped left/up and clamped so
it stays on screen (`place_menu`).

### Titlebar

`Titlebar::new(title)` for windows with a hidden or full-size-content
native titlebar: 28 px tall (the host's macOS inset; 38 px with
`leading`/`trailing` toolbar items), 78 px kept free for the traffic
lights, the title centered in the window, optional Frost surface. The bar
is a window drag region (`Element::window_drag_region` in the core): a
press on empty space or the title requests `WindowRequest::Drag`, a double
press `WindowRequest::Zoom`; buttons keep their clicks. Hosts read the
requests with `App::take_window_requests`; `livspeil-ui-host-winit` applies
them (`docs/ui-host.md`, "Window chrome and window requests").

### NodeCanvas

`NodeCanvas::new(&env, model)` shows any `NodeGraphModel` (no kernel
dependency): node ids in paint order, titles, positions, typed input and
output ports, parameter rows, links, `can_connect` (the type check; default
equal type names on different nodes), `type_color`, and edit requests
(`set_position`, `connect`, `disconnect`, `set_param`). Reads are cached per
`revision()`. `DemoGraph` is an in-memory model (`sample()`, and
`grid(nodes, links)` for benchmarks).

- Pan: Space+drag, middle drag, two-finger scroll. Zoom about the pointer:
  Ctrl/Cmd+scroll (pinch arrives as Ctrl+pixel scroll) and the wheel, on
  the Standard spring of scope `canvas` (instant under Reduce Motion); the
  canvas point under the pointer stays fixed during the animation.
- Selection: click selects only that node, Shift+click toggles, a click on
  empty space clears, dragging on empty space draws a rubber band (Shift
  adds), Ctrl/Cmd+A, Escape. Dragging a node moves the whole selection.
- Wires: links are cubic Béziers with horizontal tangents (`Wire::link`).
  Dragging from a port draws a pending wire that snaps to the nearest
  compatible port within 24 view px; incompatible ports dim; release on a
  snapped port connects (from inputs too, backward).
- Cards: header, port rows with colored dots, `NumberField` and `Toggle`
  parameter rows (named by the parameter for assistive technology). The
  selection ring is drawn above the header and rows, below the port dots. Level of detail: full from zoom 0.6, header and dots from
  0.3, plain blocks below.
- The canvas learns its size from layout (`ViewContext::layout_bounds`
  in the core: a view that reads it renders again when its bounds
  change), so culling and the minimap are right before any pointer event.
- Virtualization: only nodes and wires intersecting the view plus 96 px
  overscan are built, from a uniform-grid `CanvasIndex`; the Glass minimap
  (bottom right, click or drag to move the view) is a child view rebuilt
  only when the model revision or selection changes.

Benchmark (`cargo bench -p livspeil-ui-widgets --bench node_canvas`, 2000
nodes, 3000 links, 1600×1000, one frame after a pan: render, diff, layout,
core paint list; 2-core Linux VM, release): zoom 1 (64 nodes, 108 wires
built) 4.7 ms; zoom 0.4 (324 compact nodes, 533 wires) 9.1 ms; fit all
(2000 blocks, 3000 wires) 9.5 ms; building the geometry cache 1.2 ms.

### Wires (core paint primitive)

`Element::wires(vec![Wire::link(a, b, width, color)])` paints cubic Béziers
in the element's local coordinates (above its background, below its
children) as `PaintCmd::Wire`. The paint bridge flattens each to segments
of about 10 screen px and draws each as a rotated pill-shaped quad, so the
CPU and GPU renderers need nothing new; strokes thinner than a physical
pixel fade instead of disappearing.

## Known gaps

- `Menu`, `MenuBar` and `ContextMenu` still anchor at layout bounds, so a
  menu bar inside a scrolled view opens its menus at the unscrolled position;
  text field IME placement has the same limit.
- Clips are rectangular: square children paint over rounded corners.
- `ScrollView` clamps its offset again only at the next scroll after the
  window grows.
- `DockView` does not shrink panels below their minimums in a narrow
  container.
- Builders capture state when rendered; several key presses between two
  frames act on the same state (hosts draw a frame between events).
- Focus changes made directly with `App::focus` are noticed by text and
  number fields only at the next `Env::sync_focus` (pointer press, Tab,
  key event). Use `Env::focus` in code.
- Caret blink, cursor shapes, IME placement, the system clipboard and
  window requests come from `livspeil-ui-host-winit` (`docs/ui-host.md`);
  headless hosts have none of them.
- Select popups, menus and tooltips are removed without an exit animation (popovers, dialogs and toasts fade out).
- Dialogs do not yet mark the content behind them inert for assistive technology (the scrim only blocks pointer input and Tab), and AccessKit's modal flag is not set.
- Scrim colors are black with an alpha, not a theme token.
- Scroll release is inferred from a 60 ms pause in pixel deltas (no touch
  phases from hosts yet); Alt alone does not focus the menu bar (no
  modifier-only key events), use F10.
- NodeCanvas: no link removal by dragging off an input, no deleting nodes,
  no keyboard navigation between nodes; a pan frame rebuilds the visible
  cards (the benchmark above is the budget to beat).
- Overlapping wire segments double up under partial opacity (wires are
  drawn opaque).
