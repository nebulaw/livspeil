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
| `crates/livspeil-nodes-core` | Core nodes: math, logic, vector, time, sky, color, text (M0.6; events, fs, tables, ramps and subnets deferred) |
| `crates/livspeil-testkit` | Golden-file harness |
| `crates/livspeil-cli` | `livspeil` headless command line (`validate`, `fmt`, `cook`; `diff` reserved) |
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
the sinks, the nodes with no outgoing links (or the nodes named with
`--node`). [`examples/sun-and-lamp.room`](examples/sun-and-lamp.room/room.kdl)
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

Floats are printed with six decimals so the output (and its golden files in
`crates/livspeil-cli/tests/golden/`) is identical on macOS and Linux.
`--format jsonl` prints one JSON object per frame, `--stats` adds the
scheduler's cooked/reused counts, and `livspeil --help` documents every
option and exit code (0 ok, 1 load error, 2 usage error, 3 node error).

## Develop

```sh
cargo xtask check   # fmt, clippy -D warnings, tests, then cargo-deny and reuse lint if installed
```

Optional tools: `cargo install cargo-deny`, `pip install reuse`.

## License

Core: GPL-3.0-or-later. Node API, SDK and Livspeil UI: MPL-2.0. Docs and specs: CC BY 4.0. See [docs/licensing.md](docs/licensing.md) and [CONTRIBUTING.md](CONTRIBUTING.md).
