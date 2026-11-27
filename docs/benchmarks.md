# Kernel benchmarks

Measurements of the Phase 0 headless kernel against the performance
hypotheses in `specs/livspeil-kernel-spec-v0.md`, section 5:

> change one parameter in a 10,000-node graph and mark the affected subgraph
> dirty in < 0.2 ms; cook 10,000 trivial nodes in < 5 ms single-threaded.

The benchmarks use [criterion](https://crates.io/crates/criterion) 0.7
(dev-dependency only, default features off) and live in:

- `crates/livspeil-graph/benches/cook.rs`: the cook scheduler.
- `crates/livspeil-params/benches/expr.rs`: the expression compiler and VM.
- `crates/livspeil-cmd/benches/history.rs`: the command bus, undo/redo and the
  journal.

Run them all with `cargo xtask bench` (extra arguments go to criterion, for
example `cargo xtask bench -- --warm-up-time 1 --measurement-time 3`), or one
crate with `cargo bench -p livspeil-graph`. CI compiles the benchmarks
(`cargo clippy --all-targets`) but does not run them.

## Machine

| | |
|---|---|
| CPU | Intel(R) Xeon(R) Processor @ 2.10GHz (cloud VM), `nproc` = 2 |
| Compiler | rustc 1.97.0 (2d8144b78 2026-07-07), `cargo bench` release profile |
| Settings | `--warm-up-time 1 --measurement-time 3`, 100 samples (20 for the command bus) |
| Date | 2026-11-22 |

The VM was shared with other build jobs while these ran, so expect noise of
roughly 10 to 40 %; one obviously disturbed group (chain, 10k) was re-run and
the re-run is reported. Numbers are criterion's point estimate (the middle of
the confidence interval). Treat them as orders of magnitude, not as a
baseline for regression checks.

The cook scheduler was measured twice: first for the original lazy
scheduler (M0.2, the expression and command bus results below are from the
same session), then again for M0.2b (push-based invalidation) with the old
scheduler re-measured back to back on the same VM, same commands and
settings, so the before/after tables are comparable. During the second
session other agents were compiling on the VM; the re-measured old
scheduler came out 20 to 60 % slower than in the first session, which
gives an idea of the noise.

## Graph shapes

All graphs consist of `core.math.add@1` nodes (two `Float` inputs, one
output), the cheapest real node.

- **chain**: node `i` feeds input `a` of node `i + 1`. Head: the first node;
  sink: the last. An edit at the head affects every node.
- **tree**: binary fan-in tree in heap layout (node `i` is fed by `2i + 1` and
  `2i + 2`). Sink: the root; head: the deepest leaf. An edit at the head
  affects only the `floor(log2 n) + 1` nodes on its path (14 for 10k).
- **dag**: node `i` is fed by `i - 1` on `a` (a backbone, so the sink depends
  on everything) and, with probability 3/4, by a uniformly random earlier
  node on `b` (SplitMix64, fixed seed). About 1.75 links per node.

Before timing, each configuration asserts through `Cooker::stats` that the
cases do what they claim: a cold cook cooks `n` nodes, a warm cook cooks 0
and reuses `n`, an edit at the head cooks exactly the affected nodes, and the
cutoff case cooks exactly 1 node.

## How the spec's targets map onto the scheduler

Since M0.2b invalidation is **push-based** (design in the module docs of
`crates/livspeil-graph/src/cook.rs` and on `Graph` in `graph.rs`):

1. An edit such as `Graph::set_input` gets a new epoch and marks the node
   and everything downstream of it (links and `ch()` readers) dirty with
   that epoch. This *is* the spec's "mark the affected subgraph dirty". The
   walk stops at nodes that are already dirty for every cooker, so a second
   edit before the next cook (a slider drag between frames) is `O(1)`.
2. A cook of a target that is not dirty (and not downstream of a
   time-dependent node after a clock change) is `O(1)`. Otherwise the
   cooker walks the target's cached evaluation order and visits only the
   dirty nodes; a dirty node re-cooks only if something it reads really
   changed, and a re-cooked node with equal outputs stops propagation
   (early cutoff).

So the costs that map onto the spec are:

- **mark_dirty**: `set_input` right after a cook, which walks the whole
  affected subgraph (the cook in between is not timed). The worst case is an
  edit at the head of the chain or dag, which marks all 10,000 nodes.
- **warm**: a cook with nothing dirty (the validation the old scheduler
  paid on every cook).
- **cold** (empty cache) and **edit_head** on chain and dag (an edit that
  forces all 10,000 nodes to re-cook): "cook 10,000 trivial nodes".

Before M0.2b there was no mark step: `set_input` only bumped a revision and
every cook re-validated everything upstream of the sink (the warm case),
so finding the affected subgraph cost a full validation pass.

## Results: cook scheduler

Times per operation. `edit_head` is `set_input` + re-cook of the sink;
`edit_head_cook_only` times only the re-cook; `mark_dirty` times only a
`set_input` that follows a cook; `cutoff` is two `set_input`s that swap the
head's inputs (same sum) + re-cook; `set_input` is the edit repeated without
cooks in between; `build` is `add_node` for every node plus every `link`.

Before (lazy scheduler, `main` at `022a31d`, re-measured back to back with
the "after" table):

| Case | chain 1k | chain 10k | tree 1k | tree 10k | dag 1k | dag 10k |
|---|---:|---:|---:|---:|---:|---:|
| cold | 826 µs | 12.1 ms | 872 µs | 12.1 ms | 922 µs | 12.3 ms |
| warm (nothing changed) | 522 µs | 7.62 ms | 500 µs | 9.99 ms | 570 µs | 7.86 ms |
| set_input | 60 ns | 65 ns | 144 ns | 208 ns | 55 ns | 69 ns |
| edit_head | 726 µs | 8.69 ms | 590 µs | 8.23 ms | 793 µs | 11.3 ms |
| edit_head_cook_only | 743 µs | 8.36 ms | 553 µs | 9.79 ms | 735 µs | 10.9 ms |
| cutoff | 500 µs | 6.72 ms | 523 µs | 9.47 ms | 625 µs | 8.10 ms |
| build | 1.39 ms | 15.6 ms | 1.19 ms | 13.4 ms | 1.92 ms | 21.0 ms |

After (M0.2b: push-based invalidation, cached evaluation order, dense
slots, no allocation for reused nodes):

| Case | chain 1k | chain 10k | tree 1k | tree 10k | dag 1k | dag 10k |
|---|---:|---:|---:|---:|---:|---:|
| cold | 99 µs | 1.66 ms | 112 µs | 2.10 ms | 118 µs | 2.47 ms |
| warm (nothing changed) | 57 ns | 86 ns | 38 ns | 40 ns | 60 ns | 74 ns |
| set_input | 53 ns | 57 ns | 135 ns | 227 ns | 47 ns | 53 ns |
| mark_dirty | 5.8 µs | 59.8 µs | 354 ns | 660 ns | 5.7 µs | 97.0 µs |
| edit_head | 77.6 µs | 893 µs | 3.15 µs | 43.6 µs | 73.8 µs | 1.48 ms |
| edit_head_cook_only | 63.2 µs | 860 µs | 2.64 µs | 33.2 µs | 73.2 µs | 1.25 ms |
| cutoff | 12.6 µs | 368 µs | 2.25 µs | 26.2 µs | 15.9 µs | 502 µs |
| build | 1.52 ms | 18.0 ms | 1.46 ms | 12.1 ms | 2.11 ms | 26.8 ms |
| nodes cooked by edit_head / cutoff | 1000 / 1 | 10000 / 1 | 10 / 1 | 14 / 1 | 1000 / 1 | 10000 / 1 |

The `warm` time is mostly cloning the sink's output `Vec` that `cook`
returns. `CookStats` are unchanged by M0.2b (a warm cook still reports
`cooked = 0, reused = n`; nodes that are not dirty count as reused without
being visited), and the benchmark's sanity checks assert the same counts as
before.

## Comparison with the targets (10,000 nodes)

| Target | Before | After | Result |
|---|---|---|---|
| Change one parameter: the edit (`set_input`) < 0.2 ms | 55 to 208 ns | 53 to 227 ns | **pass** |
| Change one parameter and mark the affected subgraph dirty < 0.2 ms (`mark_dirty`; before M0.2b: the validation pass, `warm`) | 7.6 to 10.0 ms | 0.7 to 97 µs (all 10,000 nodes affected on chain and dag) | **pass** (was **fail**) |
| A cook with nothing dirty (`warm`) | 7.6 to 10.0 ms | 40 to 86 ns | `O(1)` |
| Cook 10,000 trivial nodes < 5 ms single-threaded (`cold`) | 12.1 to 12.3 ms | 1.7 to 2.5 ms | **pass** (was **fail**) |
| Re-cook 10,000 trivial nodes after an edit < 5 ms (chain, dag `edit_head`) | 8.7 to 11.3 ms | 0.9 to 1.5 ms | **pass** (was **fail**) |
| Early cutoff limits the work to the changed node | 1 node cooked, 6.7 to 9.5 ms | 1 node cooked, 26 µs (tree) to 0.5 ms (dag) | **works**; the remaining cost is marking and checking the 10,000 conservatively dirty nodes |

## Results: expression language

`compile` is source to bytecode (lex, parse, type check, codegen);
`eval` runs the compiled program once. Context: `$F = 1234`, `$T = 20.566`,
four `ch()` slots.

| Expression | Source | compile | eval |
|---|---|---:|---:|
| constant | `1.0 + 2.0 * 3.0` | 548 ns | 35 ns |
| numeric | `sin($T * tau * 0.25) * 0.5 + 0.5` | 1.07 µs | 47 ns |
| numeric_long | `clamp(smoothstep(…), -1.0, 1.0) + lerp(…) * ($F % 60 < 30 ? 1.0 : -1.0)` | 4.33 µs | 96 ns |
| vector | `normalize(vec3(sin($T), cos($T), 1.0) * 2.0 + vec3(…)).xz` | 1.96 µs | 121 ns |
| channels | `ch("sun/elevation") * 0.5 + ch("sky/haze") * ch("sky/gain")` | 1.41 µs | 39 ns |
| vector_channels | `length(ch("cam/pos") - vec3(0.0, 1.0, 0.0)) / ch("sky/gain")` | 1.68 µs | 56 ns |
| string | `concat("frame ", $F, ": ", upper("clock"), " ", str(round($T)))` | 1.79 µs | 323 ns |

There is no spec target for expressions. For scale: 10,000 numeric
expressions per frame cost about 0.5 ms; re-compiling an expression on every
keystroke in an editor (1 to 5 µs) is free.

## Results: command bus

10,000 `SetInput` commands spread over 100 `core.math.add` nodes, each its
own undo step (no coalescing group).

| Operation (10,000 commands or entries) | Total | Per command |
|---|---:|---:|
| `History::apply` | 3.07 ms | 307 ns |
| `History::undo` (all steps) | 3.20 ms | 320 ns |
| `History::redo` (all steps) | 3.19 ms | 319 ns |
| `Command::apply` alone (no history) | 1.65 ms | 165 ns |
| Journal write (`JournalWriter::append` to a `Vec<u8>`) | 2.66 ms | 266 ns |
| Journal read (`read_journal`, parse only) | 3.53 ms | 353 ns |
| Journal replay (`replay`: parse + apply) | 6.46 ms | 646 ns |

There is no spec target for the command bus. At about 0.3 µs per edit and
0.65 µs per replayed entry, a 60 Hz slider drag or a journal of a whole
editing session is far from being a bottleneck.

## Results: events (M0.10)

Measured 2026-11-23 on the same kind of VM (shared with other build jobs),
`--warm-up-time 1 --measurement-time 2`. Group `events` in
`crates/livspeil-graph/benches/cook.rs`:

| Case (10,000 nodes) | Time |
|---|---|
| `timers_tick`: 1,000 timers poll, ~100 fire, 100 counters change state and invalidate their part of an 8,000-add chain | 0.34 ms |
| `timers_tick_cook`: the same plus the cook of the chain's sink | 1.43 ms |
| `loop`: a gate feeding itself, cut at the 10,000-item limit | 0.86 ms |

The events phase costs about 0.1 to 0.3 us per queue item (each handler call checks
that the handler's inputs are cooked, which is `O(1)` when they are
fresh). A tick with no polling nodes and no injected events returns
immediately, so value-only graphs pay nothing for it.

**Value cooking with events in place.** The cooker now looks up a node's
state version before re-cooking it and calls `cook_with_state`. The 10k
value cases (`cold`, `warm`, `edit_head_cook_only`, `cutoff` for chain,
tree and dag) were run twice back to back against `main` built from the
same sources. The differences changed sign between rounds and stayed
within the VM's noise (about 10 %): for example, chain `cold` 1.90 / 1.96
ms on `main` against 2.24 / 1.94 ms, dag `cold` 3.15 / 2.67 against 2.89 /
2.76 ms, and `warm` 41 to 109 ns on both. There is no measurable
regression.

## Observations

**Where the time goes (cook, before M0.2b).** The old scheduler's per-node
overhead was about 450 to 600 ns (warm), against about 150 to 200 ns for
actually cooking an `add` node: per visited node it hashed UUIDs into a
`HashSet` and a `HashMap`, did two `BTreeMap` range queries over the links
and allocated two `Vec`s even when the node was reused. And because
invalidation was lazy, every cook paid a full validation of the sink's
dependencies, so the cost of finding the affected subgraph scaled with the
graph, not with the change.

**What M0.2b changed.** In order of impact:

1. *Push-based invalidation* (edits mark the downstream dirty, cooks visit
   only dirty nodes): warm cooks went from milliseconds to tens of
   nanoseconds, and small edits (tree) re-cook in microseconds.
2. *Cached evaluation order* per target (the last 8 targets), plus each
   target's upstream size and the time-dependent closure, until the node
   set, links, `ch()` edges or time dependence change.
3. *Dense slots*: nodes live in a slab indexed by `usize` slots; link
   sources, `ch()` targets, dependents and the cooker's cache are indexed
   by slot, and the dirty walk reads a compact (CSR) copy of the
   dependents lists. The hot paths hash no UUIDs.
4. *No allocation for reused nodes*: per-node signatures became a single
   stamp compared with upstream versions, and gathered inputs reuse a
   buffer. A re-cooked node still allocates its output `Vec` (the node API
   returns one).

**What is left.** A re-cooked `add` node costs about 85 to 125 ns in total
(`edit_head_cook_only` / 10,000). Cutoff on the chain and dag still marks
all 10,000 nodes dirty and checks each one (about 35 to 50 ns per node)
because invalidation is conservative: it cannot know before cooking that
the head's output will not change. `build` got 15 to 27 % slower on chain and dag
(tree 10 % faster; partly noise): each link now also updates the dense
mirrors. Construction still allocates the node name twice,
generates a UUID v7 per node and inserts into ordered maps, which is fine
for document loading at this size.

Parallel evaluation is not needed to reach the targets.

**Command bus.** `History::apply` costs about twice `Command::apply`: it
clones the command for the journal entry and stores both the command and its
inverse. Undo and redo wrap each step in a `Transaction` and clone its
commands. Journal replay is dominated by `serde_json` parsing (353 of
646 ns per entry). None of this needs attention now.

**Expressions.** Numeric evaluation is in the tens of nanoseconds and does
not allocate; vector programs are about twice as expensive, and string
programs allocate (`concat`, `str`, `upper`) and cost a few hundred
nanoseconds. Compilation is in the low microseconds.

## NodeCanvas frames and the Linux stutter (round 14)

Report: in the gallery's NodeCanvas "Stress" mode the window stutters on
the Linux desktop (Wayland, NVIDIA RTX 5070 Ti) with no input lag, and is
smoother on a MacBook. This section records what was measured and read,
what changed, and what still has to be measured on that machine with the
frame HUD (`docs/ui-host.md`, "Frame HUD and frame log"). Nothing here was
run on a GPU, a display, Wayland or macOS: the numbers are CPU-only, from
the shared cloud VM above.

### The CPU frame, split

The owner's `cargo bench -p livspeil-ui-widgets --bench node_canvas` (CPU
frame build only: render, diff, layout, core paint list; 2000 nodes, 3000
links) gave 1.21 ms at zoom 1 (64 nodes, Full), 2.35 ms at zoom 0.4 (324,
Compact) and 3.25 ms fit-all (2000, Block). That bench stops before the
two steps that run on every real frame, so
`cargo bench -p livspeil-ui-gallery --bench frame_phases` adds them: the
paint bridge (scene to display list, wires flattened into rotated pill
quads) and the encoder (display list to 320-byte GPU instances). Medians
of 200 frames on the VM, microseconds:

| | build | paint | encode | total |
|---|---|---|---|---|
| zoom 1, before | 2274 | 340 | 1220 | 3834 |
| zoom 1, after | 2532 | 361 | 576 | 3470 |
| zoom 0.4, before | 7082 | 959 | 3947 | 11989 |
| zoom 0.4, after | 6675 | 862 | 1688 | 9224 |
| fit-all, before | 8196 | 1086 | 4682 | 13964 |
| fit-all, after | 9735 | 843 | 2041 | 12619 |

Display list sizes: 9,031 items (zoom 1), 26,985 (0.4), 29,569 (fit-all),
that is 2.9 to 9.5 MB of instance data uploaded per frame.

Wall times on this VM drift by 25 % or more between runs: `build` is code
this round barely touched and moved from 2.3 to 3.0 ms and back, and
`graph_cache_build_2k` (untouched) went from 1.50 to 1.19 ms between two
runs of the criterion bench. So the instruction counts from
`valgrind --tool=callgrind` on the same bench (5 frames per
configuration after warm-up, three configurations; `LIVSPEIL_BENCH_ITERS=5`)
are the number to trust:

| | before | after |
|---|---|---|
| whole run | 6.31 G | 5.45 G (-13.6 %) |
| `encode` | 1.90 G | 1.28 G (-32 %) |
| `NodeCanvas::render` | 1.03 G | 0.93 G (-10 %) |
| `App::draw_frame` (build) | 3.75 G | 3.63 G (-3 %) |
| `PaintBridge::paint_scene` | 0.53 G | 0.41 G (-22 %, the bench now reuses its display list) |

The encoder's wall time fell more than its instruction count (-53 to
-57 % in the table) because a frame's instance vector is megabytes: a
fresh allocation each frame is an `mmap`, page faults and an `munmap`.

Where the rest goes (callgrind, whole run): taffy layout 20 % self
(`LayoutTree::compute` 24 % inclusive), `reconcile` and the element
builders (320-plus-byte `Element`s are moved by value through every
builder call: `memcpy` is 15 %), `paint_node` 25 % inclusive, 15 %
hashing (`SipHash` on `NodeId`, `EntityId`, `Rc<str>` keys), allocation
and `format!` for keys and names. About 4 us per element in Block LOD
(one element per card), 20 to 35 us per card in Compact and Full (header,
text, port dots with hover transitions and three closures each). The
1 ms target at 2000 nodes is not reachable by trimming `NodeCanvas`
alone: every frame of a pan re-renders the view, re-diffs and re-lays-out
every visible card, although a pan only changes the canvas transform. The
next step with real payoff is in the core: a paint-only transform update
for `canvas()` elements (and keeping per-card elements across frames),
which would make a pan cost the paint list and nothing else.

### What changed in this round

- `render::encode_into` reuses the previous frame's vectors, skips the
  sort when layers are already ordered, memoizes OKLab to sRGB (a wire's
  segments, and most quads, share a few colors) and writes instances in
  place; the renderer reuses the instance upload buffer. `PaintRenderer`
  keeps its display list and frame, and no longer clones the whole scene
  to widen text runs (`TextContext::width_slack`).
- `NodeCanvas` stopped cloning the visible `NodeGeom`s (each with its
  port and parameter vectors) and stopped building one minimap rect per
  node (2,000 of them) on every frame; the wire list is built in one pass.
- Goldens are unchanged (`cargo xtask shot` and the shot tests pass).

Not done, with the reason: caching flattened wires per zoom bucket.
Wires are laid out relative to the visible rect, whose origin moves with
every pan frame, so a cache keyed on the wire misses on every pan frame; a
cache in canvas coordinates needs the wire layer to span the whole graph,
which changes the float arithmetic (and so the pixels) of every wire.
Paint is 0.3 to 1.1 ms of the frame above, so the gain is small anyway.
Lowering the minimum of 4 segments per wire at Block LOD would remove
about 4,000 quads at fit-all but moves pixels; left alone ("do not change
visuals").

### Candidate causes of the stutter

Read from the code (no GPU here), ordered by how likely they are to matter
on NVIDIA and Wayland and not on Metal. "HUD" says where to look.

1. **A new multi-megabyte staging buffer every frame.** The renderer
   uploads all instances with `Queue::write_buffer` each frame (2.9 MB at
   zoom 1, 8.6 to 9.5 MB at zoom 0.4 and fit-all). wgpu-core creates a
   fresh staging buffer for each `write_buffer` call and frees it after the
   submission; on a discrete GPU under Vulkan that is a large
   `vkAllocateMemory` plus a PCIe copy per frame, which is exactly the
   kind of cost that is cheap on Apple's unified memory and shows up as
   occasional long frames elsewhere. Not changed (it needs a GPU to
   verify): the options are a smaller instance (320 bytes now; much of it
   is the inverse transform, clip and gradient fields most quads do not
   use), a persistent `StagingBelt`, or skipping instances outside the
   viewport earlier. HUD: `submit` (p95 and max, in ms) and `MB` on the
   last line, at zoom 1 versus fit-all. If `submit` max is several
   milliseconds and grows with the upload size, this is it.
2. **Swapchain reconfiguration every frame.** After presenting, a
   `Suboptimal` surface is reconfigured (recreating the swapchain); so are
   `Outdated` and `Lost`. Some Wayland and NVIDIA combinations report
   suboptimal constantly (fractional scaling, surface transform). HUD:
   "swapchain reconfigures" must stay at a handful after the window
   settles; if it grows by one per frame, that is the stutter.
3. **Present mode and frame latency.** Default is FIFO with a maximum
   frame latency of 1 (wgpu asks Vulkan for two images, or the driver's
   minimum). With FIFO the CPU blocks in `acquire` until vsync; that time
   is shown separately, so a frame whose `build + paint + encode + submit`
   is under the refresh interval but whose interval is two refreshes
   means the swapchain ran dry (try `LIVSPEIL_FRAME_LATENCY=2`).
   `LIVSPEIL_PRESENT_MODE=mailbox` (never blocks, newest frame wins) and
   `immediate` (tears) separate "the compositor paces badly" from "our
   frames are slow". The HUD shows requested and actual mode and the
   modes the surface supports.
4. **Frame pacing in the host** (read, nothing wrong found). A redraw is
   requested once per event batch while the app needs a frame, and again
   right after each presented frame while an animation runs, never when
   idle; `pre_present_notify` is called before every present, so Wayland
   frame callbacks are requested. `FrameScheduler` derives `dt` from the
   wall clock (clamped to 50 ms), so a slow frame shows as a bigger step,
   not as a slowdown. Redraws that only re-render the last scene
   (expose) are counted ("re-render only") and redraws that drew nothing
   are counted ("empty redraws") to catch redraw requests while idle.
5. **CPU frame cost** (measured above): at 1.2 to 3.3 ms of CPU build on
   the owner's machine (plus paint, encode and submit, about 0.5 to 2 ms
   by the table scaled by the same factor) a frame fits a 60 Hz or 144 Hz
   interval, so uneven *CPU* time is not the first suspect.
6. **Per-frame GPU object creation** (read): the instance buffer, atlas
   textures, blur level textures and pipelines persist between frames
   (the buffer grows to the next power of two); a bind group and a
   command encoder are created per frame (cheap), and blur level textures
   only when the surface size changes. Nothing to fix.
7. **Allocations per frame**: the scene clone for text widening, the
   display list, the encoded frame and the upload bytes were all fresh
   allocations per frame; fixed above. Remaining per-frame allocation is
   the element tree itself and one `Vec` per wire (flattening) and per
   shadowed quad.

### What to send back

Run the gallery with `--hud` on the Linux desktop (checklist in the PR
description and `docs/ui-host.md`), open the Node canvas page, switch to
Stress, zoom and pan for 20 s, press F4 and send
`~/.local/state/livspeil/gallery-frames.log`. Repeat with
`LIVSPEIL_PRESENT_MODE=mailbox` and `LIVSPEIL_FRAME_LATENCY=2`. The log
holds the graphics report, the run summary and a CSV line per frame
(`interval_us`, every phase, items, instances, upload bytes).

## Compact instances and curves (frame pipeline P1)

Stage P1 of `specs/livspeil-ui-frame-pipeline.md`: one record format per
primitive family (section 4.5) instead of one 320-byte instance, and a wire
as one curve instead of many rotated quads. Measured 2026-11-27, before
(commit `4793319`) and after back to back, release profile.

Machine: 12 threads, RTX 5070 Ti, driver 620.73.04, Linux, Vulkan. Nothing
here used a window: no swapchain, no present, no vsync.

### Gallery scenarios, offscreen

The scenarios of `frame_budget.rs` at its 1274x1428 size and scale 1,
rendered into an offscreen texture. Bytes and instances are the most of any
frame (the budgets); times are medians of 88 frames. "Submit" is
`Renderer::render` (buffer writes, recording, submission), GPU is the
timestamp query. A temporary harness took these times; it is not in the
repository, and they are not the on-screen numbers of the pipeline spec's
section 2.

| Scenario | instances | uploaded | draw calls | encode ms | submit ms | GPU ms |
|---|---:|---:|---:|---:|---:|---:|
| `pan-sample` | 579 -> 545 | 185 KB -> 33 KB | 5 -> 7 | 0.025 -> 0.023 | 0.24 -> 0.20 | 0.13 -> 0.11 |
| `pan-stress` | 17,894 -> 4,500 | 5.73 MB -> 345 KB | 5 -> 7 | 1.02 -> 0.27 | 1.86 -> 0.29 | 0.17 -> 0.17 |
| `pan-stress-fit` | 15,477 -> 2,169 | 4.95 MB -> 175 KB | 5 -> 7 | 0.88 -> 0.18 | 1.65 -> 0.25 | 0.15 -> 0.15 |
| `zoom-stress` | 18,137 -> 4,500 | 5.80 MB -> 345 KB | 5 -> 7 | 0.98 -> 0.21 | 1.64 -> 0.28 | 0.18 -> 0.18 |
| `drag-stress` | 18,089 -> 4,572 | 5.79 MB -> 351 KB | 5 -> 7 | 1.11 -> 0.29 | 2.21 -> 0.31 | 0.18 -> 0.18 |
| `scroll-list` | 1,777 -> 1,777 | 569 KB -> 105 KB | 3 -> 3 | 0.076 -> 0.070 | 0.38 -> 0.14 | 0.15 -> 0.12 |

Upload fell 16.6 times on `pan-stress` and 5.4 to 28 times across the
scenarios. The two extra draw calls on the node canvas are the run of curves
and the draw that resumes after it. Views rendered, elements reconciled and
layouts are unchanged (P2).

### `frame_phases` (2000 nodes, 3000 links, 1600x1000)

`cargo bench -p livspeil-ui-gallery --bench frame_phases`, medians of 200
frames, middle of three runs, microseconds:

| | build | paint | encode | instances | uploaded | draw calls |
|---|---:|---:|---:|---:|---:|---:|
| zoom 1, before | 2169 | 301 | 500 | 5,552 | 1.78 MB | 3 |
| zoom 1, after | 2090 | 150 | 250 | 3,337 | 295 KB | 5 |
| zoom 0.4, before | 5163 | 934 | 1577 | 22,110 | 7.08 MB | 3 |
| zoom 0.4, after | 4446 | 268 | 432 | 5,765 | 483 KB | 5 |
| fit-all, before | 6767 | 908 | 1816 | 29,571 | 9.46 MB | 3 |
| fit-all, after | 5676 | 165 | 690 | 7,006 | 605 KB | 5 |

`build` is code this stage did not touch.

### Quads only (`cargo bench -p livspeil-ui-render --bench quads`)

10,000 quads, nearly all of different colors, 1,000 with a shadow, encode
only: 1.31 ms before and 1.40 ms after into a fresh frame (7 % slower), 1.31
and 1.35 ms into a reused frame (3 %), which is how hosts encode. This case
has none of what the stage speeds up (no wires, no glyph runs); the time is
mostly color conversion, and the cause of the difference was not profiled.

### The curve's cost on the GPU

Without a bound on the search, every stroke pixel measured its distance to
all (up to 64) segments, and `pan-stress` took 0.24 ms of GPU time against
0.17 before. The encoder now stores how many neighbouring segments can be
nearer than a box's own (`curve::search_window`; a few for forward links,
all of them for curves that may double back), which brought it back to 0.17
with identical pixels. Not measured: an integrated GPU, where fragment work
costs far more.

## Frame pipeline P1 and P2 on screen

2026-11-28, release build, RTX 5070 Ti, driver 620.73.04, Hyprland, 144 Hz,
Vulkan, FIFO, 1600x1000 at scale 1, gallery scenarios, 7 s each, medians
without the first 30 frames. One run per scenario. "Before" is the P0 table of
`specs/livspeil-ui-frame-pipeline.md` section 2, taken a day earlier; "after"
is P1 and P2 together. P1 alone was not measured on screen. CPU = build +
paint + encode.

| Scenario | fps | CPU ms | submit ms | GPU ms | instances | uploaded |
|---|---:|---:|---:|---:|---:|---:|
| `pan-sample` | 144 -> 144 | 1.29 -> 0.35 | 1.23 -> 1.13 | 0.12 -> 0.11 | 579 -> 545 | 0.19 MB -> 33 KB |
| `pan-stress` | 108 -> 144 | 5.66 -> 1.30 | 1.95 -> 0.53 | 0.17 -> 0.17 | 16,871 -> 4,330 | 5.40 MB -> 331 KB |
| `pan-stress-fit` | 144 -> 143 | 3.18 -> 1.31 | 2.19 -> 0.90 | 0.14 -> 0.16 | 19,799 -> 2,134 | 6.34 MB -> 172 KB |
| `zoom-stress` | 124 -> 144 | 4.79 -> 1.43 | 1.70 -> 0.53 | 0.17 -> 0.17 | 13,608 -> 3,060 | 4.35 MB -> 235 KB |
| `drag-stress` | 99 -> 118 | 6.66 -> 6.34 | 2.07 -> 0.48 | 0.17 -> 0.17 | 18,449 -> 4,573 | 5.90 MB -> 349 KB |
| `scroll-list` | - -> 144 | - -> 0.94 | - -> 0.88 | - -> 0.11 | - -> 1,312 | - -> 77 KB |
| `hover-buttons` | - -> 144 | - -> 0.31 | - -> 0.83 | - -> 0.07 | - -> 458 | - -> 26 KB |

`scroll-list` has no "before": the P0 scenario never scrolled. `hover-buttons`
did not exist.

- **Dragging is not fixed.** `build` is 5.8 of its 6.3 ms, 86 of 648 frames
  were late (p95 interval 13.8 ms) and the mean rate was 101 fps. P4.
- Pan and zoom hold the refresh rate; at most 8 of about 940 frames were late.
- The 1 ms CPU target of the pipeline spec's section 3 (2000 nodes) is not
  met: the 500-node scenarios already take 1.3 to 1.4 ms. Paint, encode and
  upload still cover the whole scene (P3).

