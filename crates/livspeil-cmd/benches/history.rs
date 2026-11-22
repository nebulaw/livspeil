// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Command bus benchmarks: 10,000 `SetInput` commands through the undo
//! history (apply, undo all, redo all), and the JSON Lines journal (write,
//! parse, replay) with 10,000 entries.

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use livspeil_cmd::{
    Command, EntryKind, History, JournalEntry, JournalWriter, Origin, read_journal, replay,
};
use livspeil_graph::{Graph, NodeId};
use livspeil_node_api::{NodeRegistry, Value};

const COMMANDS: usize = 10_000;
const NODES: usize = 100;

fn graph() -> (Graph, Vec<NodeId>) {
    let mut registry = NodeRegistry::new();
    livspeil_nodes_core::register_all(&mut registry).expect("core nodes register");
    let mut graph = Graph::new(Arc::new(registry));
    let ids = (0..NODES)
        .map(|i| {
            graph
                .add_node("core.math.add@1", &format!("n{i}"))
                .expect("add node")
        })
        .collect();
    (graph, ids)
}

fn commands(ids: &[NodeId]) -> Vec<Command> {
    (0..COMMANDS)
        .map(|i| Command::SetInput {
            node: ids[i % ids.len()],
            port: if i % 2 == 0 { "a" } else { "b" }.to_owned(),
            // Exactly representable, distinct values.
            value: Value::Float(f64::from(u32::try_from(i).expect("small")) * 0.5),
        })
        .collect()
}

/// Which phase of apply / undo / redo a benchmark times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Apply,
    Undo,
    Redo,
}

/// Runs apply-all, undo-all and redo-all on a fresh history, timing one
/// phase. Every phase leaves the graph as the next one expects.
fn run_phase(graph: &mut Graph, cmds: &[Command], phase: Phase) -> Duration {
    let mut history = History::new();
    let start = Instant::now();
    for cmd in cmds {
        black_box(
            history
                .apply(graph, cmd.clone(), Origin::Ui, None)
                .expect("apply"),
        );
    }
    let apply = start.elapsed();
    let start = Instant::now();
    for _ in 0..cmds.len() {
        black_box(history.undo(graph).expect("undo"));
    }
    let undo = start.elapsed();
    let start = Instant::now();
    for _ in 0..cmds.len() {
        black_box(history.redo(graph).expect("redo"));
    }
    let redo = start.elapsed();
    match phase {
        Phase::Apply => apply,
        Phase::Undo => undo,
        Phase::Redo => redo,
    }
}

fn bench_history(c: &mut Criterion) {
    let (mut graph, ids) = graph();
    let cmds = commands(&ids);

    let mut group = c.benchmark_group("history/10k_set_input");
    group.sample_size(20);
    for (name, phase) in [
        ("apply", Phase::Apply),
        ("undo", Phase::Undo),
        ("redo", Phase::Redo),
    ] {
        group.bench_function(name, |b| {
            b.iter_custom(|iters| {
                (0..iters)
                    .map(|_| run_phase(&mut graph, &cmds, phase))
                    .sum()
            });
        });
    }
    // Raw Command::apply without the history, as a baseline.
    group.bench_function("command_apply_only", |b| {
        b.iter(|| {
            for cmd in &cmds {
                black_box(cmd.apply(&mut graph).expect("apply"));
            }
        });
    });
    group.finish();
}

fn bench_journal(c: &mut Criterion) {
    let (_, ids) = graph();
    let entries: Vec<JournalEntry> = commands(&ids)
        .into_iter()
        .map(|command| JournalEntry {
            kind: EntryKind::Do,
            origin: Origin::Ui,
            group: None,
            command,
        })
        .collect();

    let mut writer = JournalWriter::new(Vec::new());
    for entry in &entries {
        writer.append(entry).expect("append");
    }
    let bytes = writer.into_inner();

    let mut group = c.benchmark_group("journal/10k_set_input");
    group.sample_size(20);
    group.bench_function("write", |b| {
        b.iter(|| {
            let mut writer = JournalWriter::new(Vec::with_capacity(bytes.len()));
            for entry in &entries {
                writer.append(entry).expect("append");
            }
            black_box(writer.into_inner())
        });
    });
    group.bench_function("read", |b| {
        b.iter(|| black_box(read_journal(&bytes[..]).expect("read")));
    });
    // Replay needs the nodes the journal addresses: each iteration gets a
    // fresh graph with the same ids, built outside the timed region.
    group.bench_function("replay", |b| {
        b.iter_batched_ref(
            || replay_target(&ids),
            |graph| black_box(replay(&bytes[..], graph).expect("replay")),
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

/// A graph whose nodes have the ids the journal refers to.
fn replay_target(ids: &[NodeId]) -> Graph {
    let mut registry = NodeRegistry::new();
    livspeil_nodes_core::register_all(&mut registry).expect("core nodes register");
    let mut graph = Graph::new(Arc::new(registry));
    for (i, &id) in ids.iter().enumerate() {
        graph
            .add_node_with_id(id, "core.math.add@1", &format!("n{i}"))
            .expect("add node");
    }
    graph
}

criterion_group!(benches, bench_history, bench_journal);
criterion_main!(benches);
