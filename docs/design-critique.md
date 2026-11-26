<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Gallery design critique (M1.13)

A ranked list of what keeps the component gallery from a macOS-grade
result, for the owner to choose from. Nothing here is fixed by M1.13.

**Basis.** The screenshot goldens in `crates/livspeil-ui-shot/goldens/`
(CPU reference rasterizer, bundled Inter): every `gallery-*` page in dark
Simple, `gallery-theme-light-pro` and `gallery-macos-titlebar`. Items marked
"700×500" come from temporary renders at that size. Nothing was judged on a
real display, so contrast, hairlines and blur may read differently on a
panel. Icon placeholders are left out (they are being replaced).

**Effort.** S: up to half a day. M: one or two days. L: more.

## Ranked

### 1. The dark surface ladder is crushed into black (L to decide, S to change)

All dark screenshots, clearest in `gallery-text-fields-1x`,
`gallery-radios-1x`, `gallery-menus-1x`.

OKLCH L 0.10 / 0.15 / 0.19 / 0.23 renders as roughly `#030303` to `#1c1c1c`.
Window, card, field and track are hard to tell apart, the disabled segmented
control almost disappears, and the softer shadows of D60 cannot be seen at
all on black. macOS dark windows sit near `#1e1e1e` with grouped boxes and
controls a clear step above. A macOS-grade ladder would start around L 0.22
and step by about 0.04 to 0.05, keeping chroma 0 (D59). This changes UI spec
section 7, so it needs a decision.

### 2. The select's chevron is a missing glyph (S)

`gallery-select-1x` and `-2x`.

The `▾` in `Select` has no glyph in the bundled fonts: every select ends in
a small "not defined" box. A macOS pop-up button shows a crisp up-and-down
chevron, ideally from the icon atlas that is being added. This is a plain
bug and should not wait for the rest of the list.

### 3. Rounded frames have square contents (M)

`gallery-node-canvas-1x` (frame corners missing), `gallery-dock-1x`,
`gallery-menus-1x` (menu bar inside the rounded window box).

Clips are rectangular in the renderer, so square children paint over the
corner arcs and the border looks broken at all four corners. macOS clips
content to the container's continuous corners. Needs rounded clipping in the
paint and render crates.

### 4. The decorative backdrop looks like a rendering fault (S)

Every gallery page; `gallery-theme-light-pro`; 700×500.

The three colour blobs show hard-edged slivers left of the sidebar, stick out
below it in short windows and stop half way down tall ones. Behind Frost they
also tint the selected row differently on every page (blue, teal, mauve
grey), so selection has no stable colour. macOS lets the real desktop or
content show through a sidebar and draws selection consistently. Clip the
blobs to the sidebar, size them from the window, and soften them.

### 5. Everything is a card with a faint capital heading (M)

`gallery-buttons-1x`, `gallery-toggles-1x`, `gallery-text-fields-1x`,
`gallery-theme-1x`.

Every group is the same outlined box with an 11 px tertiary uppercase title,
so nothing leads the eye. System Settings uses sentence-case semibold
headings, inset groups with hairline separators between rows, controls
aligned to the trailing edge, and help text as a quiet footnote outside the
group.

### 6. Grid and alignment: stretched, uneven, mostly empty (M)

`gallery-toggles-1x`, `gallery-radios-1x`, `gallery-typography-1x`,
`gallery-theme-1x`; worse at 1274×1428.

Paired cards stretch to the taller one and leave large blank areas (Switches,
Segmented controls, Type scale), while the column beside them stacks two
cards that line up with nothing. Content has no maximum width, and most pages
use the top half only. macOS settings panes use one centred column of a
fixed readable width (about 600 to 700 px), each group as tall as its
content.

### 7. Light theme contrast and depth (M)

`gallery-theme-light-pro`.

Tertiary text (section headings, shortcut digits, swatch captions) is a pale
grey on `#eee`, below 4.5:1 by eye. Cards are grey on near white with a
muddy shadow, and the Frost sidebar barely separates from the window. macOS
light uses white groups on a light grey window, darker secondary text and
almost no shadow on grouped content. Check the tokens with a contrast test.

### 8. The toolbar competes with the page (M)

Every gallery page; `gallery-macos-titlebar`.

Three segmented controls and two long-labelled switches, "T0 T1 T2" among
them, are the heaviest row on screen and share no edge with the page title.
With the titlebar there are two stacked bars, and the title is centred on the
window rather than the content. macOS would use one unified toolbar with the
page title at its leading edge and appearance options behind one button.

### 9. Number fields read as broken selections (M)

`gallery-sliders-1x`, `gallery-dock-1x`, `gallery-node-canvas-1x`.

The range fill is a hard-edged block that cuts through the label ("Size"),
and label and value are centred as a pair, so X, Y and Size do not line up.
A macOS-grade scrub field keeps the label leading and the value trailing in
fixed columns, with a thin progress line or a faint fill behind both.

### 10. Control spacing and widths have no system (S)

`gallery-radios-1x`, `gallery-theme-light-pro`, `gallery-motion-1x`.

Radio rows are 40 px apart, switches 32, check boxes 24. Horizontal radios sit
so close that a circle reads as belonging to the label before it. Segmented
controls take segment widths of 36, 54, 64, 96 px and the intrinsic one on
neighbouring screens. In "Density sample" the caption touches the segmented
control. Use the 8/12/16 steps consistently: equal row pitch for form
controls, 20 px between horizontal radios, one segment sizing rule.

### 11. Field and track edges are nearly invisible (S)

`gallery-text-fields-1x`, `gallery-select-1x`, `gallery-radios-1x`.

A 6% white hairline on a field the same tone as its card leaves the field to
be found by its placeholder. macOS fields have a visibly different fill and a
slightly brighter top or bottom edge. Disabled should dim the text, not erase
the shape. Partly follows from item 1.

### 12. Right-to-left text is left aligned (S to M)

`gallery-text-fields-1x`, `-2x`.

The Arabic field's text starts at the left edge. macOS aligns a field's text
by its base direction, with the caret following.

### 13. Small windows are not designed for (L)

700×500, and any narrow Hyprland tile.

M1.13 stops content leaving its cards and lets pages scroll, but the 220 px
sidebar stays and takes a third of the width, the dock keeps its panel
minimums and cuts the inspector off, the node canvas minimap covers much of
the graph, and the virtual list truncates titles without an ellipsis. macOS
would collapse the sidebar behind a button, shrink or hide dock panels,
scale or hide the minimap and truncate with "…".

### 14. Ink has no legibility treatment (S)

`gallery-materials-1x`.

The Ink sample is bare text over saturated stripes. UI spec section 8 asks
for a subtle text shadow. Frost and Glass also look nearly alike in dark;
Glass should read lighter and thinner, with a clearer rim.

### 15. Smaller points (S each)

- Type scale (`gallery-typography-1x`): size labels are centred on each
  sample instead of sharing its baseline; the rows are cramped.
- Node canvas (`gallery-node-canvas-1x`): card text at the default zoom is
  about 9 px; the sample graph is not centred in the view.
- Menus page (`gallery-menus-1x`): two 280 px near-black boxes that are
  mostly empty.
- Sidebar (all pages): the shortcut digits are always shown and add noise;
  macOS sidebars lead with an icon and keep shortcuts in menus.
- Overlays page (`gallery-overlays-1x`): captions explain the implementation
  at length under each card.

## Suggested order

Items 2 and 4 are small and obvious. Item 1 needs a decision and changes how
several of the others look, so it should come before any tuning of spacing or
contrast (5, 7, 11).
