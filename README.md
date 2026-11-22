# Livspeil

An Arch-based operating system whose desktop is a scene you author: 2D or 3D, built from **Charms** (typed-port node-graph widgets), edited in place in **Dream Mode** and used in **Live Mode**. Think Houdini/Blender/Affinity-grade tools as the substrate, not a set of example scenes.

Status: **Phase 0 (kernel)**. This repository currently holds the headless, cross-platform kernel scaffold (milestone M0.1). Nothing here needs a GPU or a window, and everything builds on macOS and Linux.

## Layout

| Path | What |
|---|---|
| `crates/livspeil-node-api` | Public node/Charm API surface (MPL-2.0) |
| `crates/livspeil-graph` | Networks, type checking, cook scheduler (M0.2) |
| `crates/livspeil-params` | Parameters, animation, expressions (placeholder, M0.5) |
| `crates/livspeil-doc` | Document model and KDL (placeholder, M0.4) |
| `crates/livspeil-cmd` | Command bus, undo/redo, journal (M0.3) |
| `crates/livspeil-nodes-core` | First built-in nodes (placeholder, M0.6) |
| `crates/livspeil-testkit` | Golden-file harness |
| `crates/livspeil-cli` | `livspeil` headless command line |
| `xtask` | Project automation (`cargo xtask check`) |
| `specs/` | Design specs |
| `docs/` | Licensing and contribution documentation |

## Develop

```sh
cargo xtask check   # fmt, clippy -D warnings, tests, then cargo-deny and reuse lint if installed
```

Optional tools: `cargo install cargo-deny`, `pip install reuse`.

## License

Core: GPL-3.0-or-later. Node API, SDK and Livspeil UI: MPL-2.0. Docs and specs: CC BY 4.0. See [docs/licensing.md](docs/licensing.md) and [CONTRIBUTING.md](CONTRIBUTING.md).
