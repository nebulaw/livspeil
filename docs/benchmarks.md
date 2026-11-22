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
