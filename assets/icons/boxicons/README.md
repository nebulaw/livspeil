<!-- SPDX-FileCopyrightText: 2026 Livspeil contributors -->
<!-- SPDX-License-Identifier: CC-BY-4.0 -->

# Boxicons (free set)

49 icons from the Boxicons free set, v3.0.8, filled style, regular weight 400
(decision D22).

- Copyright 2025 Boxicons, <https://boxicons.com>
- Icons licensed under Creative Commons Attribution 4.0 International
  (CC BY 4.0): <https://creativecommons.org/licenses/by/4.0/>
- Boxicons' own license text: `LICENSE.txt` here
  (<https://docs.boxicons.com/free>). It also covers Boxicons fonts and code,
  which are not included in this repository.

## Modifications

The SVG files and `LICENSE.txt` are byte-for-byte as published. Only the
directory layout changed (`svgs/filled/regular/400/` was flattened into this
directory). At run time Livspeil UI does not alter the shapes; it rasterizes
them to coverage masks and draws them in a theme colour.

## Using them

The files are embedded by `crates/livspeil-ui-icons`, which looks icons up by
file name without `.svg` (for example `bx-copy`). Its parser accepts only the
SVG subset these files use and its tests fail on anything else. To add an
icon, put the file here and add its name to the list in that crate.
