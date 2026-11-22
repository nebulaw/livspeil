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

The scheduler has **no separate "mark dirty" step**. `Graph::set_input` only
stores the value and bumps the node's revision (`O(log n)`). Invalidation is
lazy: the next `Cooker::cook(sink)` walks everything the sink depends on
(an iterative DFS for the evaluation order), and for each node compares a
signature (own revision, upstream output versions, frame) with the cached
one. A node whose signature matches is reused; otherwise it is re-cooked, and
if its outputs come out equal its version is kept, so its dependents'
signatures still match (early cutoff).

So "mark the affected subgraph dirty" corresponds to two costs:

1. `set_input`: the edit itself. This is what a UI pays synchronously.
2. The validation pass of the next cook, which is what actually discovers
   the affected subgraph. Its cost is the **warm** case (nothing changed: the
   pass visits and validates every dependency of the sink and cooks
   nothing). It is paid on every cook regardless of how small the change is.

"Cook 10,000 trivial nodes" corresponds to **cold** (empty cache) and to
**edit_head** on chain and dag (a change that forces all 10,000 nodes to
re-cook).

## Results: cook scheduler

Times per operation. `edit_head` is `set_input` + re-cook of the sink;
`edit_head_cook_only` times only the re-cook; `cutoff` is two `set_input`s
that swap the head's inputs (same sum) + re-cook; `build` is `add_node` for
every node plus every `link`.

| Case | chain 1k | chain 10k | tree 1k | tree 10k | dag 1k | dag 10k |
|---|---:|---:|---:|---:|---:|---:|
| cold | 564 µs | 6.83 ms | 1.05 ms | 8.39 ms | 600 µs | 7.15 ms |
| warm (nothing changed) | 302 µs | 4.58 ms | 520 µs | 5.21 ms | 427 µs | 6.15 ms |
| set_input | 46 ns | 55 ns | 203 ns | 187 ns | 45 ns | 52 ns |
| edit_head | 486 µs | 6.51 ms | 506 µs | 4.85 ms | 602 µs | 8.72 ms |
| edit_head_cook_only | 514 µs | 6.39 ms | 355 µs | 4.74 ms | 550 µs | 8.37 ms |
| cutoff | 344 µs | 4.88 ms | 363 µs | 4.86 ms | 475 µs | 5.29 ms |
| nodes cooked by edit_head / cutoff | 1000 / 1 | 10000 / 1 | 10 / 1 | 14 / 1 | 1000 / 1 | 10000 / 1 |
| build | 1.29 ms | 15.4 ms | 1.07 ms | 10.8 ms | 1.65 ms | 24.1 ms |

## Comparison with the targets (10,000 nodes)

| Target | Measured | Result |
|---|---|---|
| Change one parameter: the edit (`set_input`) < 0.2 ms | 52 to 187 ns | **pass** (by 1000x) |
| Change one parameter and find the affected subgraph < 0.2 ms (validation pass = warm cook) | 4.6 to 6.2 ms | **fail** (about 25 to 30x over) |
| Cook 10,000 trivial nodes < 5 ms single-threaded (cold) | 6.8 to 8.4 ms | **fail** (1.4 to 1.7x over) |
| Re-cook 10,000 trivial nodes after an edit < 5 ms (chain, dag `edit_head`) | 6.5 to 8.7 ms | **fail** (1.3 to 1.7x over) |
| Early cutoff limits the work to the changed node | 1 node cooked | **works**, but the cook still costs as much as a warm cook (4.9 to 5.3 ms) |

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

## Observations

**Where the time goes (cook).** The per-node overhead of the scheduler is
about 450 to 600 ns (warm) and the work of actually cooking an `add` node
adds only about 150 to 200 ns on top (compare `edit_head` with `cutoff` on
the chain). The cost is dominated by bookkeeping, not by node work. Reading
`cook.rs` and `graph.rs` (not profiled), per visited node the scheduler:

- inserts the node into a `HashSet<NodeId>` (SipHash of a 16-byte UUID) and
  walks its links with a `BTreeMap` range query in `eval_order`;
- in `cook_one`, looks the node up in a `BTreeMap<NodeId, Node>`, walks the
  links again with a second `BTreeMap` range query, looks up every upstream
  node in the `HashMap<NodeId, Entry>` cache, and allocates two `Vec`s
  (`linked` and the signature's `upstream`) even when the node is reused;
- when it re-cooks, additionally allocates the input `Vec`, clones upstream
  `Value`s, allocates the output `Vec`, compares it with the cached outputs
  and re-inserts the cache entry.

The tree is slower per node than the chain on cold and warm cooks because
its DFS stack and the UUID-keyed maps are accessed in a less cache-friendly
order.

**Why the "mark dirty" target fails.** The edit itself is cheap, but the
lazy design makes every cook pay for a full validation of the sink's
dependencies, so the cost of finding the affected subgraph scales with the
graph, not with the change: an edit in the tree that re-cooks 14 nodes still
costs 4.9 ms, and early cutoff (1 node re-cooked) costs as much as a warm
cook. Early cutoff works as designed; it saves node work but not traversal.

**Optimization opportunities** (not implemented here; listed roughly by
expected impact):

1. *Push-based invalidation.* On an edit, walk the `outgoing` index
   downstream from the edited node and mark dependents dirty (cost
   proportional to the affected subgraph, the spec's model). A cook then
   visits only dirty nodes and a cook with nothing dirty is `O(1)`. This is
   the only change that can meet the 0.2 ms target on a 10k graph; the others
   shave constant factors.
2. *Cache the evaluation order* per sink, keyed by a structural revision of
   the graph that changes only on add/remove/link/unlink. Value edits (the
   common case: slider drags, animation) would then skip the DFS and its
   `HashSet` entirely.
3. *Dense node indices.* Map `NodeId` (UUID) to a `u32` slot once and keep
   nodes, links and the cook cache in `Vec`s indexed by slot (or at least
   use a cheaper hasher than SipHash for UUID keys). This removes most
   `BTreeMap`/`HashMap` lookups from the hot loop.
4. *No allocations for reused nodes.* Compare the signature in place
   (or with a fixed-size buffer / small-vector) instead of building
   `linked` and `upstream` `Vec`s per visit, and gather links once instead
   of twice.
5. *Construction* (10 to 24 ms for 10k nodes, 1 to 2.4 µs per node and
   link) allocates the node name twice (node and name index), generates a
   UUID v7 per node and inserts into three ordered maps. Fine for document
   loading at this size; dense indices would help here too.

Items 1 to 4 together should bring cold cooks of 10k trivial nodes well
under 5 ms on this machine (the node work itself is about 1.5 to 2 ms) and make
small edits cost microseconds. Parallel evaluation is not needed to reach
the targets.

**Command bus.** `History::apply` costs about twice `Command::apply`: it
clones the command for the journal entry and stores both the command and its
inverse. Undo and redo wrap each step in a `Transaction` and clone its
commands. Journal replay is dominated by `serde_json` parsing (353 of
646 ns per entry). None of this needs attention now.

**Expressions.** Numeric evaluation is in the tens of nanoseconds and does
not allocate; vector programs are about twice as expensive, and string
programs allocate (`concat`, `str`, `upper`) and cost a few hundred
nanoseconds. Compilation is in the low microseconds.
