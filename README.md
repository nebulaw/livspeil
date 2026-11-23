# Livspeil

An Arch-based operating system whose desktop is a scene you author: 2D or 3D, built from **Charms** (typed-port node-graph widgets), edited in place in **Dream Mode** and used in **Live Mode**. Think Houdini/Blender/Affinity-grade tools as the substrate, not a set of example scenes.

Status: **Phase 0 (kernel)**. This repository currently holds the headless, cross-platform kernel scaffold (milestone M0.1). Nothing here needs a GPU or a window, and everything builds on macOS and Linux.

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
| `crates/livspeil-cli` | `livspeil` headless command line (`validate`, `fmt`, `cook`, `diff`, `assets`) |
| `xtask` | Project automation (`cargo xtask check`) |
| `specs/` | Design specs |
| `docs/` | Licensing and contribution documentation |

## Command line

`livspeil` works on room packages (`Name.room/` or its `room.kdl`):

```sh
cargo run -p livspeil-cli -- validate examples/sun-and-lamp.room
cargo run -p livspeil-cli -- fmt --check examples/sun-and-lamp.room
cargo run -p livspeil-cli -- cook examples/sun-and-lamp.room --frames 600 --every 50
```

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
subnets and Charms (M0.7): a "Daylight" Charm (hour in, lamp color and
intensity out, two promoted parameters) instantiated twice with different
parameters, printed through four `core.output` nodes.

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

## Develop

```sh
cargo xtask check   # fmt, clippy -D warnings, tests, then cargo-deny and reuse lint if installed
```

Optional tools: `cargo install cargo-deny`, `pip install reuse`.

## License

Core: GPL-3.0-or-later. Node API, SDK and Livspeil UI: MPL-2.0. Docs and specs: CC BY 4.0. See [docs/licensing.md](docs/licensing.md) and [CONTRIBUTING.md](CONTRIBUTING.md).
