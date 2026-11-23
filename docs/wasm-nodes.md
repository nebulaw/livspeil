# WebAssembly nodes

Third-party nodes run inside the Livspeil process only as sandboxed WebAssembly (decision D13). A **node package** is one wasm module that describes one or more node types and cooks them. Milestone M0.15 adds the host (`crates/livspeil-wasm-host`), the interface (`wit/livspeil-node.wit`), an example package (`examples/wasm/hello-node/`) and `livspeil cook --wasm`.

## Using a package

```sh
livspeil cook examples/wasm/hello-node/hello.room \
    --wasm examples/wasm/hello-node/hello-node.wasm --frames 240 --every 30
```

`--wasm FILE` (repeatable) loads a package (binary `.wasm`, or `.wat` text) and registers its node types next to the core nodes before the room is loaded. A package that cannot be loaded, or whose node types clash with registered ones, stops the command with exit code 1. In Rust:

```rust
let host = livspeil_wasm_host::WasmHost::new()?;         // one per process
let package = host.load_file(Path::new("hello-node.wasm"))?;
package.register(&mut registry)?;                        // Arc<dyn NodeImpl> per node type
```

`--wasm` is the CLI's cargo feature `wasm`, on by default. The host pins wasmtime to its 48.x LTS line (security fixes for two years; needs Rust 1.95, the workspace MSRV). `cargo build -p livspeil-cli --no-default-features` builds a smaller CLI without wasmtime (`--wasm` then fails with exit code 1). CI checks that build.

`validate` and `diff` do not take `--wasm` yet: a room that uses package nodes fails `validate` with "unknown node type".

## Authoring a package

A package is a **core wasm module** (32-bit memory) that:

- **imports nothing**: no WASI, no clock, no random source, no file system, no network. A module with any import is refused at load time.
- exports `memory` and three functions:

| Export | Signature | Meaning |
|---|---|---|
| `livspeil_describe` | `() -> i64` | The `describe` buffer (below), as `(ptr << 32) \| len` |
| `livspeil_alloc` | `(len: i32) -> i32` | Return a pointer to `len` writable bytes (the host writes the cook input there) |
| `livspeil_cook` | `(node: i32, ptr: i32, len: i32) -> i64` | Cook node type `node` (index into the described list) on the input buffer at `ptr`; return the result buffer as `(ptr << 32) \| len` |

The host calls `livspeil_alloc` once per cook, writes the input, then calls `livspeil_cook`. Every call runs in a fresh instance (see Limits), so a simple bump allocator that never frees is enough, and globals never carry state from one cook to the next.

From Rust, build a `cdylib` for `wasm32-unknown-unknown` with `panic = "abort"`, export the three functions with `#[unsafe(no_mangle)] pub extern "C"`, and avoid crates that pull in `wasm-bindgen` or WASI (they add imports). The example in this repository is hand-written WAT, because the wasm targets could not be installed where M0.15 was built; `cargo xtask wasm-example` assembles it into the committed `hello-node.wasm`.

### Rules checked at load time

- At least one node type, at most 256; each `(id, version)` once.
- Ids are namespaced: two or more dot-separated `[a-z][a-z0-9_]*` segments (`acme.wave.sine`). The `core` namespace is reserved for built-in nodes. Use a namespace you own (your project or handle).
- Versions are at least 1; summaries are non-empty (at most 1024 bytes); ids and port names at most 128 bytes.
- Port names match `[a-z][a-z0-9_]*` and are unique among a node's inputs (and among its outputs); at most 64 inputs and 64 outputs; at least one output.
- Port types: Float, Int, Bool, Vec2, Vec3, Vec4, Color (OKLCH + alpha), String.
- Input defaults have exactly the port's type and finite floats; outputs have no default. An input without default starts at the type's zero value.

### Rules checked at cook time

- The result holds exactly one value per output, of the output's type (an Int is accepted for a Float output).
- An `invalid-input` error must name an input port; otherwise it is reported as a plain failure.
- Non-time-dependent nodes receive a zero context (`frame = 0, time = 0, dt = 0`): they are cached across frames and must not depend on the clock.

## ABI reference (version 1)

The logical types are those of `wit/livspeil-node.wit` (world `livspeil:node/node-package@0.1.0`). Across linear memory they use this byte encoding, all little-endian:

| Type | Encoding |
|---|---|
| `bool` | `u8`: 0 or 1 |
| `u32`, `u64`, `s64`, `f64` | 4 or 8 bytes |
| `string` | `u32` byte length, then UTF-8 bytes |
| `list<T>` | `u32` count, then the items |
| `option<T>` | `u8` 0 (none), or 1 then `T` |
| `port-type` | `u8`: 0 Float, 1 Int, 2 Bool, 3 Vec2, 4 Vec3, 5 Vec4, 6 Color, 7 String |
| `value` | the `port-type` tag, then the payload: `f64` / `s64` / `bool` / 2, 3 or 4 × `f64` / `string` |

**describe buffer**: `"LSND"`, `u32` ABI version (1), then `list<node-spec>`, where `node-spec` is `id: string, version: u32, summary: string, time_dependent: bool, inputs: list<port-spec>, outputs: list<port-spec>` and `port-spec` is `name: string, type: port-type, default: option<value>`.

**cook input** (written by the host): `frame: u64, time: f64, dt: f64, inputs: list<value>` (one value per input port, already of the port's type). Offsets: frame +0, time +8, dt +16, count +24, first value +28.

**cook result** (returned by the guest): `u8` 0 then `list<value>` (ok), or `u8` 1 then a `node-error`: `u8` 0 `port: string, reason: string` (invalid input) or `u8` 1 `reason: string` (failed). Trailing bytes are an error.

Example: the result `ok([Float 2.0])` is the 14 bytes `00 | 01 00 00 00 | 00 | 00 00 00 00 00 00 00 40`.

Deferred to a later ABI version: Table, Curve and Event values, Enum ports, event handlers (`on_event`) and runtime state, seeded randomness, system bindings, and loading the same world as a **component** (the WIT types will stay the same; the host will then accept both forms).

## Limits and sandbox

| Limit | Default | On violation |
|---|---|---|
| Fuel per call (instantiation included) | 10 000 000 (about one unit per wasm instruction) | "ran out of fuel" |
| Linear memory | 64 MiB | "exceeded the memory limit" (at load time if the initial memory is larger) |
| Wasm stack | 512 KiB | "overflowed the wasm stack" |
| Tables | 4 tables, 100 000 elements | growth fails |

Fuel is deterministic: the same inputs stop at the same instruction on every machine, unlike a wall-clock timeout. Determinism also requires: NaN canonicalization (one NaN bit pattern everywhere), relaxed SIMD off (SIMD128 is on), and threads, shared memory, memory64, multi-memory, GC, exceptions and wide arithmetic unavailable. Every `describe` and every cook runs in a fresh instance of the compiled module, with its own store, fuel and memory: a node cannot keep state between cooks.

Traps (unreachable, division by zero, out-of-bounds access), fuel exhaustion, the memory limit and ABI violations (buffers outside memory, malformed results) become `NodeError::Failed` naming the node; the host never panics, and the next cook starts clean. The limits are host-wide (`WasmLimits`, `WasmHost::with_limits`).

Modules are compiled once per host, cached by the SHA-256 of their bytes (`WasmHost::cached`); loading the same bytes again reuses the compiled module and its node specs.

## Licensing

Packages may use any license, including proprietary for private use. They interact with Livspeil only through the interface in `wit/livspeil-node.wit` and this ABI, which are MPL-2.0 like the rest of the Charm SDK; the host (`livspeil-wasm-host`) is GPL-3.0-or-later and is never linked into a package. The planned GPLv3 section 7 "Charm exception" will state that such packages are not derivative works of Livspeil (see [licensing.md](licensing.md)). The official store accepts free licenses only. The example package is CC0-1.0.

## Proposal: packages in documents

Not implemented in M0.15; documents do not reference packages yet. Proposed design:

1. A Room package stores a node package as an asset, `assets/<sha256>.wasm`, listed in `assets.lock` like any asset (hash, size, source, SPDX license, author).
2. `room.kdl` declares it at the top of the room: `package "assets/<sha256>.wasm"` (repeatable). The document format version stays 1 if older readers may ignore unknown top-level entries; otherwise it is bumped.
3. Loading a room verifies each declared package against `assets.lock` (hash and size), loads it with `WasmHost` and registers its node types before the networks are built. A missing or mismatched package is a load error that names the package.
4. Node types from packages are referenced as usual (`type="acme.wave.sine@1"`); id clashes between packages, or with core nodes, are load errors.
5. `validate`, `cook` and `diff` load declared packages automatically; `--wasm` stays for packages under development.
6. Dream Mode shows a package's nodes with its license and author from `assets.lock`.

Open questions: whether packages may be shared between rooms (a user-wide content-addressed store), per-package limits, and signing for the official store.
