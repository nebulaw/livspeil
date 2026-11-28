# Livspeil

An Arch-based operating system whose desktop is a scene you author: 2D or 3D, built from **Charms** (typed-port node-graph widgets), edited in place in **Dream Mode** and used in **Live Mode**. Think Houdini/Blender/Affinity-grade tools as the substrate, not a set of example scenes.

Status: **Phase 0 (kernel) and Phase 1 (UI toolkit)**. This repository holds the headless, cross-platform kernel and the Livspeil UI crates (state and views, layout, theme, rendering, text, widgets, accessibility and a desktop host). The kernel needs no GPU or window; the UI crates are tested headlessly too (CPU reference rasterizer, `TestHost`). Everything builds on macOS and Linux.

## Layout

| Path | What |
|---|---|
| `crates/livspeil-node-api` | Public node/Charm API surface (MPL-2.0) |
| `crates/livspeil-graph` | Networks, type checking, cook scheduler (M0.2) |
| `crates/livspeil-params` | Parameters, animation, expressions (expression language done; parameters next, M0.5) |
| `crates/livspeil-doc` | Document model, KDL v2 `room.kdl` format, canonical form, migrations (M0.4) |
| `crates/livspeil-cmd` | Command bus, undo/redo, journal (M0.3) |
| `crates/livspeil-nodes-core` | Core nodes: math, logic, vector, time, sky, color, text (M0.6), subnet and output nodes (M0.7), tables, ramps and fs (M0.8), events (M0.10) |
| `crates/livspeil-testkit` | Golden-file harness |
| `crates/livspeil-script` | Script worker host: out-of-process scripts, capabilities, limits (M0.14) |
| `crates/livspeil-wasm-host` | Sandboxed WebAssembly node packages (third-party Charms in process) |
| `crates/livspeil-cli` | `livspeil` headless command line (`validate`, `fmt`, `cook`, `diff`, `assets`, `run`) |
| `crates/livspeil-floor` | The Floor, Livspeil's Wayland compositor (Smithay), Linux only: spike with a nested build and a bare-metal skeleton; see `docs/decisions/floor-renderer.md` (GPL-3.0-or-later) |
| `crates/livspeil-ui-core` | UI core: models, views, element diffing, input and focus, animation driver, frame loop, paint list (MPL-2.0, like every UI crate) |
| `crates/livspeil-ui-layout` | Geometry, Taffy flex/grid/block layout over a retained tree, dock and infinite-canvas layouts |
| `crates/livspeil-ui-theme` | Design tokens, materials, motion profiles, theme/motion config loading and hot reload |
| `crates/livspeil-ui-render` | Display list, wgpu renderer (quads, materials with backdrop blur, glyphs) and CPU reference rasterizer |
| `crates/livspeil-ui-text` | Text shaping, layout, editing model and glyph atlas (on Parley) |
| `crates/livspeil-ui-icons` | The embedded Boxicons set and its coverage rasterizer (MPL-2.0 code; the icons are CC BY 4.0) |
| `crates/livspeil-ui-paint` | Paint bridge: core scenes to display lists, cached text shaping and glyph atlas uploads |
| `crates/livspeil-ui-widgets` | Widgets: buttons, toggles, check boxes, radios, segmented control, slider, number and text fields, select, tooltip |
| `crates/livspeil-ui-a11y` | Accessibility: incremental AccessKit trees from the UI core and action routing |
| `crates/livspeil-ui-shot` | Headless screenshots of UI scenes and the golden-image diff harness |
| `crates/livspeil-ui-host-winit` | Desktop host: winit window, wgpu surface and `PaintRenderer`, frame pacing, input, IME, cursors, clipboard and AccessKit (macOS first) |
| `crates/livspeil-dream` | Dream Mode app, first slice: build or open a room, edit its graph and watch it run |
| `crates/livspeil-ui-gallery` | Component gallery: every widget, material, the type scale and motion settings, live (app and headless pages) |
| `sdk/python` | Python SDK for scripts (MPL-2.0, standard library only) |
| `xtask` | Project automation (`cargo xtask check`) |
| `specs/` | Design specs |
| `docs/` | Licensing, contribution and component documentation (nodes, scripting, UI host, widgets, accessibility, known upstream issues) |
| `.claude/` | Claude Code setup: shared permissions, agents and skills; see [docs/claude-code-playbook.md](docs/claude-code-playbook.md) |

## Command line

`livspeil` works on room packages (`Name.room/` or its `room.kdl`):

```sh
cargo run -p livspeil-cli -- validate examples/sun-and-lamp.room
cargo run -p livspeil-cli -- fmt --check examples/sun-and-lamp.room
cargo run -p livspeil-cli -- cook examples/sun-and-lamp.room --frames 600 --every 50
```

`fmt` rewrites `room.kdl` in canonical form (sorted nodes, inputs and
links, two-space indent). It keeps the leading comment block and the `//`
comments attached to items: lines directly above a node, input, link,
network, `charm` or `meta` entry, and a comment at the end of its line;
they move with their item. Other comments (block comments, comments inside
`anim`/`table`/`curve` blocks or behind slashdashed items, comments not
directly above an item) are dropped; see `crates/livspeil-doc/src/comments.rs`.

`cook` drives the graph with a deterministic frame clock (frame `i` at
`start + i/fps` seconds, `--fps 60` by default) and prints the outputs of
the sinks: the root network's `core.output` nodes, or when it has none its
nodes with no outgoing links (or the nodes named with `--node`). [`examples/sun-and-lamp.room`](examples/sun-and-lamp.room/room.kdl)
is the Phase 0 acceptance scene: 600 frames cover one day starting at 06:00;
the hour drives the sun (the window light) and a daylight factor that picks
the lamp's color (night blue, warm, daylight) and intensity:

```text
frame=0 time=0.000000 lamp_color.out=oklch(0.628704 0.065007 51.483430 1.000000)
frame=0 time=0.000000 lamp_intensity.out=0.792593
frame=0 time=0.000000 sun.direction=vec3(0.917409 0.264725 -0.297121) sun.elevation=15.350625 sun.azimuth=72.054432
...
frame=300 time=5.000000 lamp_color.out=oklch(0.876543 0.075088 67.477719 1.000000)
frame=300 time=5.000000 lamp_intensity.out=0.407407
frame=300 time=5.000000 sun.direction=vec3(-0.917409 0.264725 -0.297121) sun.elevation=15.350625 sun.azimuth=287.945568
```

[`examples/charm-demo.room`](examples/charm-demo.room/room.kdl) shows
Charms (M0.7, M0.16): a shared "Daylight" Charm definition (hour in, lamp
color and intensity out, two promoted parameters) used by two instances
with different parameters, printed through four `core.output` nodes.

File-system nodes (`core.fs.list_dir`) see the room package (or
`--fs-root DIR`) and nothing outside it. A run lists each folder once (a
snapshot); with `--watch-fs` the listed folders are checked every frame and
re-listed when files were created, deleted or renamed, re-cooking only what
reads them.

Floats are printed with six decimals so the output (and its golden files in
`crates/livspeil-cli/tests/golden/`) is identical on macOS and Linux;
`--precision N` picks another number of decimals and `--precision full`
the exact shortest round-trip form (both text and JSONL).
`--format jsonl` prints one JSON object per frame, `--stats` adds the
scheduler's cooked/reused counts, and `livspeil --help` documents every
option and exit code (0 ok, 1 load error, 2 usage error, 3 node error).

### Events

Values are pulled and cached; events are pushed, once per tick, in the
phase order **input -> events -> cook -> publish** (M0.10). `Event` ports
carry triggers with an optional payload; event links are written like any
other link and may form loops (each tick is cut at an event limit, 10,000
queue items by default, and the overflow is reported). Event nodes
(`core.event.every`, `on_change`, `counter`, `toggle`, `latch`, `gate`,
`select`) keep runtime state per cooker, never saved in `room.kdl`; a state
change invalidates the node's value outputs and their downstream, so a
counter's `count` or a latch's `out` can drive any parameter. Design:
`crates/livspeil-graph/src/cook/events.rs`.

[`examples/blink.room`](examples/blink.room/room.kdl) blinks a lamp every
0.5 s and counts the blinks. `--event NODE.PORT@FRAME[=VALUE]` injects an
event (into an `Event` input, or out of an `Event` output) for testing,
with an optional payload in the `room.kdl` value syntax (`=1.0`, `=3`,
`=#true`, `='"text"'`, `='(vec3)"1 2 3"'`), and `Event` outputs print what
fired in the frame:

```sh
cargo run -p livspeil-cli -- cook examples/blink.room --frames 40 --fps 20 --every 10 \
    --event blinks.reset@30
```

```text
frame=10 time=0.500000 changed.value=event(0.200000)
frame=10 time=0.500000 count.value=2
frame=10 time=0.500000 light.value=0.200000
frame=10 time=0.500000 ticks.value=event(1)
```

Event overflows and handler errors are reported for every frame: printed
frames list them, and frames skipped by `--every` are summarized before the
next printed frame (`frames 3..49: 2 event errors (last: timer: ...)`). A
handler error in any frame makes the exit code 3.

### `diff`

`livspeil diff <a> <b> [--format text|jsonl] [--meta]` compares two
documents semantically: nodes are matched by id across the whole tree (the
root network and every subnet), so a rename is one change and does not show
its links as changed, and a node moved into or out of a subnet is one
`moved` change. It reports header changes (name, min-tier), added and
removed nodes, renames, type changes, moves between networks, `port-type`
changes, Charm metadata (name, version, icon, min-tier, capabilities added
or removed), input values, expressions, animations and `Curve` values
(settings and keys, matched by time), `Table` values (a size summary with
the number of changed cells, plus changed columns and rows when at most 20
cells changed), and links (with their network); editor positions (`meta`)
only with `--meta`. Nodes are named by path (`lamp`, `desk_lamp/sunrise`).
Values are shown in their canonical `room.kdl` text. The order is
deterministic: header, nodes by id (each node's ports by name), links by
network, meta.

```text
~ room min-tier: "T0" -> "T1"
~ node "lamp" renamed: "light" -> "lamp"
~ node "lamp" input "a": 0.2 -> 0.3
~ node "lamp" input "t" anim key 1.0: 0.8 interp="bezier" tangent="flat" -> 0.9 interp="step"
+ node "glow" type="core.math.add@1" id="01920000-0000-7000-8000-000000000005"
- link "light.out" -> "old_add.b"
~ node "lamp/bias" moved: "/" -> "/lamp"
~ node "lamp" charm version: "1.0.0" -> "1.1.0"
+ node "lamp" charm capability: "net"
~ node "lamp/hour" port-type: "Float" -> "Int"
~ node "palette" input "table" table: 2 cols x 3 rows -> 2 cols x 4 rows (3 cells changed)
~ node "palette" input "table" table row 1: "b" 2.0 -> "b" 2.5
+ node "ramp" input "curve" curve key 0.75: 0.5
+ link "dawn.out" -> "bias.a" in "/lamp"
```

`--format jsonl` prints one object per change, keys in a fixed order and
left out when they do not apply:

```json
{"op":"changed","kind":"input","node":"lamp","id":"01920000-0000-7000-8000-000000000002","port":"a","old":"0.2","new":"0.3"}
{"op":"changed","kind":"node-move","node":"bias","id":"01930000-0000-7000-8000-000000000002","path":"lamp/bias","old":"\"/\"","new":"\"/lamp\""}
```

- `op`: `added`, `removed`, `changed`.
- `kind`: `room-name`, `format`, `min-tier`, `node`, `node-name`,
  `node-type`, `node-move`, `port-type`, `charm` (a whole block),
  `charm-field`, `charm-capability`, `input`, `expr`, `anim` (a whole
  channel), `anim-setting`, `anim-key`, `table`, `table-column`,
  `table-row`, `curve-setting`, `curve-key`, `link`, `position`.
- `node`, `id`: the node (its new name if renamed); `path`: its path, only
  for nodes inside a subnet; `port`: the input; `setting`: `pre`, `post` or
  `offset`; `time`: the key time (curve key position); `field`: the Charm
  field (`name`, `version`, `icon`, `min-tier`); `cells` (number): changed
  table cells; `column`: table column name; `row` (number): table row
  index; `from`, `to`: link endpoints as `node.port`; `network`: the
  link's network path, only inside a subnet.
- `old`, `new`: canonical KDL text of the value (`"0.2"`, `"\"text\""`,
  `"(color)\"oklch(...)\""`), left out for additions and removals
  respectively; for `node` the quoted type id, for `anim` a summary such as
  `"3 keys 0.0..2.0 pre=\"loop\""`, for `charm` a summary, for `table`
  the size (`"2 cols x 3 rows"`), for `table-row` the row's cells, for
  Table/Curve `input` values `"table 2 cols x 3 rows"` or
  `"curve 2 keys 0.0..1.0"`, for `node-move` the quoted network path, for
  `position` `"x y"`.

New kinds and keys may be added; existing ones keep their meaning.

Exit codes follow `git diff --exit-code`: 0 no differences, 1 differences,
2 usage error, 3 a document cannot be read or parsed. The diff is the
library function `livspeil_doc::diff`, so Dream Mode shows the same changes.

### `assets`

A room package may ship files in `assets/`, named by content
(`assets/<sha256>.<ext>`), each recorded in `assets.lock`:

```kdl
assets-lock format=1 {
  asset "assets/3239c1cf...ac00.png" sha256="3239c1cf...ac00" size=78 license="CC0-1.0" source="original" author="Livspeil contributors"
}
```

`sha256`, `size`, `license` (an SPDX expression) and `source` (`"original"`
or a URL) are required, `author` is optional. The file is strict (unknown
keys are errors, with line and column) and written canonically (entries
sorted by path; leading `//` comment lines are kept).

```sh
livspeil assets add examples/sun-and-lamp.room picture.png --license CC-BY-4.0 \
    --source https://example.org/picture.png --author "Jane Doe"
livspeil assets check examples/sun-and-lamp.room [--strict]
```

`assets add` copies the file to `assets/<sha256>.<ext>` and updates
`assets.lock`. If the path is already listed with other data (license,
source, author, or a stale hash or size) it refuses, unless `--replace`,
which rewrites that entry canonically. `livspeil fmt` also rewrites the
package's `assets.lock` canonically, and `fmt --check` fails if it is not. `assets check` (and `validate`, whenever the package has an
`assets.lock` or `assets/`) reports, as errors, hash or size mismatches,
missing files, files in `assets/` that are not listed, and malformed license
expressions (or `NONE`/`NOASSERTION`). Licenses outside the allow-list
(CC0-1.0, CC-BY-4.0, CC-BY-SA-4.0, OFL-1.1, MIT, Apache-2.0, MPL-2.0,
GPL-3.0-or-later, LGPL-3.0-or-later) and a missing `author` for licenses
that require attribution are warnings; `--strict` fails on warnings too.

## Scripting

Scripts run **out of process** and talk to the kernel over the script
worker protocol ([`docs/script-protocol.md`](docs/script-protocol.md)): one
JSON object per line on the script's stdin and stdout. A script reads the
graph with queries, cooks nodes, and changes the room only by sending
commands to the command bus, so its edits are validated, undoable and
journaled with origin `script`. It can do only what its **capabilities**
allow: `graph.read`, `graph.write`, `graph.cook`, `fs.read:<dir>` (none by
default). Python comes first, with the SDK in [`sdk/python`](sdk/python)
(Python 3.10+, no dependencies):

```python
import livspeil
from livspeil import SetInput

session = livspeil.connect()
with session.graph.transaction():          # one undo step
    for node in session.graph.nodes(type="core.math.remap"):
        session.graph.apply(SetInput(node, "out_max", node.input("out_max") * 0.8))
```

```sh
livspeil run examples/sun-and-lamp.room examples/scripts/dim_lamps.py \
    --grant graph.read,graph.write [--save] [--journal edits.jsonl]
livspeil run examples/sun-and-lamp.room examples/scripts/sun_elevation.py --grant graph.cook
livspeil run examples/blink.room examples/scripts/blink_watcher.py \
    --grant graph.read,graph.write --ticks 40 --fps 20
```

```text
run dim_lamps.py on "Sun and Lamp" (granted: graph.read, graph.write)
log info: dimming 2 lamp nodes by 20%
applied 1 command (1 undo step):
  transaction (5 steps)
    set_input lamp_color.c0 = oklch(0.28 0.08 260 1)
    ...
    set_input lamp_intensity.out_max = 0.16
script dim_lamps.py exited with status 0
```

Scripts that call `session.listen()` react to the scene through
`@session.on_tick` and `@session.on_event` hooks. The host enforces a
handshake timeout, per-handler timeouts (cancel, then kill), a total run
time and a message size limit. Capabilities scope the kernel API; they are
not an OS sandbox yet. Exit code 4 means the script failed; the room is
then not saved.

## Try Dream Mode

```sh
cargo run --release -p livspeil-dream -- examples/sun-and-lamp.room
cargo run --release -p livspeil-dream          # a new, empty room
```

Rooms are kept in `~/Livspeil` (set `LIVSPEIL_ROOMS` to move it): **Open**
(Ctrl+O) lists them and also takes a typed path, and "save as" puts a name
given alone there.

Press **Play**, select a node (`lamp_color` shows its color change through a
simulated day), pan with middle drag or two fingers and zoom with Ctrl+scroll.
`--select=NAME` and `--play` start that way.

**Add** (or Shift+A on the canvas) opens a searchable list of node types;
Delete removes the selected nodes. Edit by dragging nodes and wires (drag off
a linked input to remove its link), and in the inspector: type a value,
`=expression`, or nothing for the default, then Enter. Ctrl+Z and
Ctrl+Shift+Z undo and redo. Ctrl+S saves into the room file, Ctrl+Shift+S
saves as a new room folder, Ctrl+N starts a new room
(`specs/livspeil-dream-spec-v0.md`). Work on a copy of an example, or save it
under another name first (Ctrl+Shift+S).

## Try the gallery

```sh
cargo run -p livspeil-ui-gallery            # opens on Buttons
cargo run -p livspeil-ui-gallery -- dock    # or any page by its slug (below)
```

`cargo run -p livspeil-ui-gallery -- --gpu-info` prints the GPU backend,
adapter, driver, surface formats and usages, and which backdrop-blur path
is chosen (see `docs/ui-host.md`, "Backdrop blur paths"); paste it into
bug reports.

A window with a Frost sidebar of pages and a toolbar opens (Metal on
macOS, Vulkan or OpenGL ES on Linux). The sidebar is grouped and scrolls
when the window is short:

| Group | Pages (slug) |
|---|---|
| Controls | `buttons`, `toggles`, `radios`, `sliders`, `text-fields`, `select` |
| Containers | `scroll`, `dock`, `menus`, `node-canvas` |
| System | `materials`, `typography`, `motion`, `theme` |

Things to look at:

- **Toolbar**: Dark/Light, Simple/Pro density, material tier T0/T1/T2,
  Reduce motion and Reduce transparency. Everything changes live.
- **Keyboard**: Tab and Shift+Tab reach every control (focus ring);
  Space/Enter press; arrows step sliders, radios and segments;
  Cmd+1..9 and Cmd+0 (Ctrl on Linux) switch to the first ten pages
  (Buttons to Node canvas).
- **Pointer**: a pointing hand over buttons and toggles, an I-beam over
  text fields, a resize cursor over sliders, number fields and dock
  splitters. Drag a number field sideways to scrub (Shift ×10, Alt ×0.1)
  and watch the preview box move.
- **Text fields**: the caret blinks only while a field is focused (and
  stops after 15 s without input); IME candidates appear at the caret;
  copy and paste use the system clipboard. Georgian and Arabic samples.
- **Scrolling**: a scroll view (trackpads move 1:1 with rubber banding and
  momentum, wheels and keys glide, overlay scrollbars) next to a virtual
  list of 100,000 rows that only builds the rows in view. Focus the list
  and press End.
- **Panels & dock**: a mini editor (layers, viewport, inspector,
  timeline). Drag the splitters, double-click one to reset it, and use the
  switches to collapse a side.
- **Menus**: a menu bar (shortcuts, check marks, a submenu, disabled
  items; F10 and Alt+F/E/V from the keyboard) and an area with a context
  menu (right-click, or focus it and press Shift+F10).
- **Node canvas**: the sample graph (pan with Space-drag or two fingers,
  Ctrl/Cmd+scroll to zoom, drag from a port to connect); switch the toolbar
  to "Stress · 500" for 500 nodes and 750 links, then Fit.
- **Materials**: Solid, Frost, Glass and Ink over stripes; switch tiers
  and Reduce transparency to compare.
- **Motion**: drag Response and Damping, press Animate; change the
  profile, the global speed and Reduce motion.
- **Screen readers**: VoiceOver on macOS (Cmd+F5); Orca on Linux (below).
  The window is named "Livspeil Gallery"; pages are tabs, every control
  has a name, value and state, and actions (press, increment, set value,
  focus) work.
- **Hot reload**: edit `~/.config/livspeil/ui/{theme,motion,keymap}.toml`
  (see `docs/ui-config.md`) while it runs; the toolbar follows.
- **Idle**: when nothing moves the process draws no frames.
- **Frame HUD**: F3 shows or hides a frame-time overlay (bottom right),
  F4 writes a summary and the per-frame CSV of the last 10 s to the frame
  log (on macOS hold fn for F3 and F4). Start with it visible using
  `--hud` or `LIVSPEIL_HUD=1`. It shows FPS, interval avg/p95/max, late
  frames (interval over 1.5x the median), the CPU phases (build, paint,
  encode, submit, acquire, present), wgpu backend, adapter, driver,
  present mode, surface format, frame latency, scale factor and monitor
  refresh rate. The log is `$XDG_STATE_HOME/livspeil/gallery-frames.log`
  (`~/.local/state/livspeil/` by default; `~/Library/Logs/livspeil/` on
  macOS); the path is printed at startup. Send that file when reporting
  stutter. `LIVSPEIL_PRESENT_MODE=fifo|mailbox|immediate|auto` (or
  `--present-mode=`) and `LIVSPEIL_FRAME_LATENCY=1..3` change how frames
  are presented; the HUD shows what the surface actually uses. Details:
  `docs/ui-host.md`, "Frame HUD and frame log".
- **Window**: on macOS the gallery draws its own titlebar under the
  transparent native one (drag it to move the window, double-click to
  zoom). On Linux the window keeps the system's decorations.

### On Linux

The window works on Wayland and X11. Packages (plus a Rust toolchain and a
C toolchain with `pkg-config` for the build):

- **Arch**: `vulkan-icd-loader`, the Vulkan driver for your GPU
  (`vulkan-radeon` for AMD, `vulkan-intel` for Intel, `nvidia-utils` for
  NVIDIA), and `fontconfig`. `mesa` provides the OpenGL ES fallback.
- **Debian/Ubuntu**: `mesa-vulkan-drivers libfontconfig1-dev` (plus your
  vendor's driver on NVIDIA).

Building the whole workspace on Linux also compiles the Floor
(`livspeil-floor`), which needs the xkbcommon development files
(`libxkbcommon` on Arch, `libxkbcommon-dev` on Debian/Ubuntu). Its `tty`
feature additionally needs libseat, libinput, udev, GBM and DRM
(`seatd libinput systemd-libs libdrm mesa` on Arch;
`libseat-dev libinput-dev libudev-dev libgbm-dev libdrm-dev` on Debian).
macOS never builds the Floor.

winit 0.30 picks the backend itself: Wayland when `WAYLAND_DISPLAY` (or
`WAYLAND_SOCKET`) is set and not empty, otherwise X11 through `DISPLAY`.
There is no `WINIT_UNIX_BACKEND` any more. To use X11 (XWayland) from a
Wayland session, clear the Wayland variables for the run:

```sh
WAYLAND_DISPLAY= WAYLAND_SOCKET= cargo run -p livspeil-ui-gallery
```

In an X11-only session, or an `env -u WAYLAND_DISPLAY` shell, X11 is used
anyway. On compositors without server-side decorations (GNOME) winit draws
its own title bar (without a title text: no fonts are bundled for it).
`WINIT_X11_SCALE_FACTOR` overrides the scale factor on X11.

Accessibility uses AccessKit's AT-SPI support: start Orca (Super+Alt+S on
GNOME, or run `orca`; Arch package `orca`, which pulls in `at-spi2-core`)
before or after opening the gallery. The tree is sent once a screen reader
enables the accessibility bus.

Screenshots of every page (no window needed): `cargo xtask shot
gallery-buttons-1x` (or `--list`), written to `target/shots/`.

## Develop

```sh
cargo xtask check   # fmt, clippy -D warnings, tests, then cargo-deny and reuse lint if installed
```

Optional tools: `cargo install cargo-deny`, `pip install reuse`.

Debug builds optimize third-party dependencies (see `Cargo.toml`); judge
smoothness and measure only with `--release`.

## License

Core: GPL-3.0-or-later. Node API, SDK and Livspeil UI: MPL-2.0. Docs and specs: CC BY 4.0. See [docs/licensing.md](docs/licensing.md) and [CONTRIBUTING.md](CONTRIBUTING.md).
