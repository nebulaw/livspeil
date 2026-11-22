// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Cook scheduler benchmarks against the kernel spec's targets (section 5):
//! "change one parameter in a 10,000-node graph and mark the affected
//! subgraph dirty in < 0.2 ms; cook 10,000 trivial nodes in < 5 ms
//! single-threaded".
//!
//! Every graph is made of `core.math.add` nodes in one of three shapes:
//!
//! - `chain`: node `i` feeds input `a` of node `i + 1`. Head: node 0; sink:
//!   the last node. Every node depends on the head.
//! - `tree`: a binary fan-in tree in heap layout (node `i` is fed by nodes
//!   `2i + 1` and `2i + 2`). Sink: the root; head: the last leaf, so a change
//!   at the head re-cooks only the ~log2(n) nodes on its path to the root.
//! - `dag`: node `i` is fed by node `i - 1` on `a` (a backbone, so the sink
//!   depends on everything) and, with probability 3/4, by a random earlier
//!   node on `b` (fixed seed). Head: node 0; sink: the last node.
//!
//! Cases per shape and size: `cold` (first cook with an empty cache),
//! `warm` (re-cook with nothing changed), `edit_head` (set one input at the
//! head, then re-cook the sink; reported both as the total and as the cook
//! alone), `mark_dirty` (`set_input` right after a cook, which walks the
//! whole affected subgraph; the cook is not timed), `cutoff` (swap the
//! head's two inputs so its output stays the same: the head re-cooks and
//! early cutoff stops propagation), `set_input` (the edit alone, repeated
//! without cooks in between, so after the first one the walk stops at the
//! head) and `build` (`add_node` + `link` for the whole graph).
//!
//! Invalidation is push-based: `set_input` marks the edited node and its
//! downstream dirty (the spec's "mark the affected subgraph dirty"), and a
//! cook visits only dirty nodes, so `warm` measures the `O(1)` fast path.
//! See `docs/benchmarks.md` for how the spec's targets map onto these cases.
//!
//! Events (M0.10), group `events`: `timers` is a 10,000-node graph of
//! 1,000 `core.event.every` timers (period 0.1 s, staggered offsets, so
//! about 100 fire per 10 ms tick), each triggering a `core.event.counter`,
//! and a chain of 8,000 adds whose `b` inputs read the counters (add `i`
//! reads counter `i / 8`). `tick` is the events phase alone (polls,
//! deliveries, state changes and their invalidation); `tick_cook` adds the
//! cook of the chain's sink. `loop` is a gate feeding itself, cut at the
//! default limit of 10,000 queue items per tick.

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use livspeil_graph::{Cooker, ExternalEvent, Graph, NodeId};
use livspeil_node_api::{CookContext, Event, NodeRegistry, Value};

const ADD: &str = "core.math.add@1";
const SIZES: [usize; 2] = [1_000, 10_000];

/// SplitMix64: a tiny deterministic PRNG, enough to shape a random DAG.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform-ish in `0..bound` (`bound > 0`); the modulo bias is irrelevant here.
    fn below(&mut self, bound: usize) -> usize {
        // Truncation is fine: `bound` fits in usize, so the remainder does too.
        #[allow(clippy::cast_possible_truncation)]
        let r = (self.next() % bound as u64) as usize;
        r
    }
}

#[derive(Debug, Clone, Copy)]
enum Shape {
    Chain,
    Tree,
    Dag,
}

impl Shape {
    const ALL: [Shape; 3] = [Shape::Chain, Shape::Tree, Shape::Dag];

    fn name(self) -> &'static str {
        match self {
            Shape::Chain => "chain",
            Shape::Tree => "tree",
            Shape::Dag => "dag",
        }
    }
}

/// The links of a shape as `(from, to, to_port)` node indices, in an order
/// in which every link's target has no outgoing links yet (keeps the cycle
/// check cheap, as it is when a user wires a graph up front to back).
fn plan(shape: Shape, n: usize) -> Vec<(usize, usize, &'static str)> {
    let mut links = Vec::with_capacity(2 * n);
    match shape {
        Shape::Chain => {
            for i in 1..n {
                links.push((i - 1, i, "a"));
            }
        }
        Shape::Tree => {
            for child in (1..n).rev() {
                let port = if child % 2 == 1 { "a" } else { "b" };
                links.push((child, (child - 1) / 2, port));
            }
        }
        Shape::Dag => {
            let mut rng = SplitMix64(0x5EED_1234_ABCD_0001);
            for i in 1..n {
                links.push((i - 1, i, "a"));
                if rng.below(4) != 0 {
                    links.push((rng.below(i), i, "b"));
                }
            }
        }
    }
    links
}

/// Head and sink node indices of a shape.
fn ends(shape: Shape, n: usize) -> (usize, usize) {
    match shape {
        Shape::Chain | Shape::Dag => (0, n - 1),
        Shape::Tree => (n - 1, 0),
    }
}

fn registry() -> Arc<NodeRegistry> {
    let mut registry = NodeRegistry::new();
    livspeil_nodes_core::register_all(&mut registry).expect("core nodes register");
    Arc::new(registry)
}

struct Built {
    graph: Graph,
    head: NodeId,
    sink: NodeId,
}

fn build(
    registry: &Arc<NodeRegistry>,
    names: &[String],
    links: &[(usize, usize, &'static str)],
) -> Graph {
    let mut graph = Graph::new(Arc::clone(registry));
    let ids: Vec<NodeId> = names
        .iter()
        .map(|name| graph.add_node(ADD, name).expect("add node"))
        .collect();
    for &(from, to, port) in links {
        graph
            .link(ids[from], "out", ids[to], port)
            .expect("link is valid");
    }
    graph
}

fn names(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("n{i}")).collect()
}

fn built(registry: &Arc<NodeRegistry>, shape: Shape, n: usize) -> Built {
    let names = names(n);
    let graph = build(registry, &names, &plan(shape, n));
    let (head, sink) = ends(shape, n);
    let head = graph.find(&names[head]).expect("head exists");
    let sink = graph.find(&names[sink]).expect("sink exists");
    Built { graph, head, sink }
}

/// Number of nodes the head reaches, itself included (what an edit at the
/// head has to re-cook without early cutoff).
fn affected(shape: Shape, n: usize) -> u64 {
    match shape {
        Shape::Chain | Shape::Dag => n as u64,
        // Depth of the last leaf in heap layout, plus one.
        Shape::Tree => u64::from(usize::BITS - n.leading_zeros()),
    }
}

/// Checks that the benchmarked cases do the work they claim to do, so the
/// numbers mean what the report says.
fn sanity(registry: &Arc<NodeRegistry>, shape: Shape, n: usize) {
    let Built {
        mut graph,
        head,
        sink,
    } = built(registry, shape, n);
    let mut cooker = Cooker::new();
    cooker.cook(&graph, sink).expect("cold cook");
    assert_eq!(
        cooker.stats().cooked,
        n as u64,
        "cold cook cooks every node"
    );

    cooker.reset_stats();
    cooker.cook(&graph, sink).expect("warm cook");
    assert_eq!(cooker.stats().cooked, 0, "warm cook cooks nothing");
    assert_eq!(cooker.stats().reused, n as u64);

    cooker.reset_stats();
    graph.set_input(head, "b", Value::Float(1.0)).expect("set");
    cooker.cook(&graph, sink).expect("edit cook");
    assert_eq!(
        cooker.stats().cooked,
        affected(shape, n),
        "an edit at the head re-cooks what it reaches"
    );

    graph.set_input(head, "a", Value::Float(0.0)).expect("set");
    graph.set_input(head, "b", Value::Float(1.0)).expect("set");
    cooker.cook(&graph, sink).expect("cook");
    cooker.reset_stats();
    graph.set_input(head, "a", Value::Float(1.0)).expect("set");
    graph.set_input(head, "b", Value::Float(0.0)).expect("set");
    cooker.cook(&graph, sink).expect("cutoff cook");
    assert_eq!(cooker.stats().cooked, 1, "early cutoff stops at the head");
}

fn bench_cook(c: &mut Criterion) {
    let registry = registry();
    for shape in Shape::ALL {
        let mut group = c.benchmark_group(format!("cook/{}", shape.name()));
        for n in SIZES {
            sanity(&registry, shape, n);
            let Built {
                mut graph,
                head,
                sink,
            } = built(&registry, shape, n);

            // (a) First cook from an empty cache.
            group.bench_function(BenchmarkId::new("cold", n), |b| {
                b.iter_batched(
                    Cooker::new,
                    |mut cooker| {
                        black_box(cooker.cook(&graph, sink).expect("cook"));
                        cooker
                    },
                    BatchSize::LargeInput,
                );
            });

            let mut cooker = Cooker::new();
            cooker.cook(&graph, sink).expect("cook");

            // (b) Nothing changed: pure cache validation.
            group.bench_function(BenchmarkId::new("warm", n), |b| {
                b.iter(|| black_box(cooker.cook(&graph, sink).expect("cook")));
            });

            // The edit alone (the spec's "change one parameter").
            let mut flip = false;
            group.bench_function(BenchmarkId::new("set_input", n), |b| {
                b.iter(|| {
                    flip = !flip;
                    let v = if flip { 1.0 } else { 0.0 };
                    graph.set_input(head, "b", Value::Float(v)).expect("set");
                });
            });
            // Leave the graph in a state the cooker has seen.
            cooker.cook(&graph, sink).expect("cook");

            // The edit right after a cook: marks the whole affected subgraph
            // dirty (the cook between edits is not timed).
            group.bench_function(BenchmarkId::new("mark_dirty", n), |b| {
                b.iter_custom(|iters| {
                    let mut total = Duration::ZERO;
                    for _ in 0..iters {
                        flip = !flip;
                        let v = if flip { 1.0 } else { 0.0 };
                        let start = Instant::now();
                        graph.set_input(head, "b", Value::Float(v)).expect("set");
                        total += start.elapsed();
                        black_box(cooker.cook(&graph, sink).expect("cook"));
                    }
                    total
                });
            });

            // (c) Change one input at the head and re-cook the sink.
            group.bench_function(BenchmarkId::new("edit_head", n), |b| {
                b.iter(|| {
                    flip = !flip;
                    let v = if flip { 1.0 } else { 0.0 };
                    graph.set_input(head, "b", Value::Float(v)).expect("set");
                    black_box(cooker.cook(&graph, sink).expect("cook"));
                });
            });
            // (c') The same, timing only the re-cook.
            group.bench_function(BenchmarkId::new("edit_head_cook_only", n), |b| {
                b.iter_custom(|iters| {
                    let mut total = Duration::ZERO;
                    for _ in 0..iters {
                        flip = !flip;
                        let v = if flip { 1.0 } else { 0.0 };
                        graph.set_input(head, "b", Value::Float(v)).expect("set");
                        let start = Instant::now();
                        black_box(cooker.cook(&graph, sink).expect("cook"));
                        total += start.elapsed();
                    }
                    total
                });
            });

            // (d) Swap the head's inputs: new input values, same output, so
            // early cutoff stops after re-cooking the head.
            graph.set_input(head, "a", Value::Float(0.0)).expect("set");
            graph.set_input(head, "b", Value::Float(1.0)).expect("set");
            cooker.cook(&graph, sink).expect("cook");
            let mut swapped = false;
            group.bench_function(BenchmarkId::new("cutoff", n), |b| {
                b.iter(|| {
                    swapped = !swapped;
                    let (x, y) = if swapped { (1.0, 0.0) } else { (0.0, 1.0) };
                    graph.set_input(head, "a", Value::Float(x)).expect("set");
                    graph.set_input(head, "b", Value::Float(y)).expect("set");
                    black_box(cooker.cook(&graph, sink).expect("cook"));
                });
            });

            // (e) Graph construction: add_node + link.
            let node_names = names(n);
            let links = plan(shape, n);
            group.bench_function(BenchmarkId::new("build", n), |b| {
                b.iter_batched(
                    || (),
                    |()| build(&registry, &node_names, &links),
                    BatchSize::LargeInput,
                );
            });
        }
        group.finish();
    }
}

/// The `timers` graph: returns it and the chain's sink.
fn timers(registry: &Arc<NodeRegistry>) -> (Graph, NodeId) {
    const TIMERS: usize = 1_000;
    const ADDS: usize = 8_000;
    let mut graph = Graph::new(Arc::clone(registry));
    let mut counters = Vec::with_capacity(TIMERS);
    for i in 0..TIMERS {
        let every = graph
            .add_node("core.event.every@1", &format!("t{i}"))
            .expect("add");
        graph
            .set_input(every, "period", Value::Float(0.1))
            .expect("set");
        #[allow(clippy::cast_precision_loss)]
        let offset = (i % 10) as f64 * 0.01 + 0.005;
        graph
            .set_input(every, "offset", Value::Float(offset))
            .expect("set");
        let counter = graph
            .add_node("core.event.counter@1", &format!("c{i}"))
            .expect("add");
        graph
            .link(every, "fired", counter, "trigger")
            .expect("link");
        counters.push(counter);
    }
    let mut prev = None;
    for i in 0..ADDS {
        let add = graph.add_node(ADD, &format!("a{i}")).expect("add");
        if let Some(p) = prev {
            graph.link(p, "out", add, "a").expect("link");
        }
        graph
            .link(counters[i / 8], "count", add, "b")
            .expect("link");
        prev = Some(add);
    }
    (graph, prev.expect("chain is not empty"))
}

fn clock(frame: u64) -> CookContext {
    #[allow(clippy::cast_precision_loss)]
    let time = frame as f64 * 0.01;
    CookContext {
        frame,
        time,
        dt: 0.01,
    }
}

fn bench_events(c: &mut Criterion) {
    let registry = registry();
    let mut group = c.benchmark_group("events");
    let (graph, sink) = timers(&registry);
    assert_eq!(graph.node_count(), 10_000);

    // Sanity: about a tenth of the timers fire per tick.
    let mut cooker = Cooker::new();
    cooker.cook(&graph, sink).expect("cook");
    let report = cooker.tick(&graph, clock(1), &[]);
    assert_eq!(report.fired.len(), 100, "100 timers fire per tick");
    assert!(report.errors.is_empty() && !report.overflowed());

    let mut frame = 1;
    group.bench_function(BenchmarkId::new("timers_tick", 10_000), |b| {
        b.iter(|| {
            frame += 1;
            black_box(cooker.tick(&graph, clock(frame), &[]));
        });
    });
    group.bench_function(BenchmarkId::new("timers_tick_cook", 10_000), |b| {
        b.iter(|| {
            frame += 1;
            black_box(cooker.tick(&graph, clock(frame), &[]));
            black_box(cooker.cook(&graph, sink).expect("cook"));
        });
    });

    let mut graph = Graph::new(Arc::clone(&registry));
    let gate = graph.add_node("core.event.gate@1", "gate").expect("add");
    graph.link(gate, "out", gate, "trigger").expect("link");
    let start = [ExternalEvent::new(&graph, gate, "trigger", Event::bang()).expect("event")];
    let mut cooker = Cooker::new();
    assert!(cooker.tick(&graph, clock(0), &start).overflowed());
    group.bench_function(BenchmarkId::new("loop", 10_000), |b| {
        b.iter(|| {
            frame += 1;
            black_box(cooker.tick(&graph, clock(frame), &start));
        });
    });
    group.finish();
}

criterion_group!(benches, bench_cook, bench_events);
criterion_main!(benches);
