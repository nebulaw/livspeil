// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Push-based invalidation: what an edit, a removal, a rename or a clock
//! change marks dirty, several cookers on one graph, and a randomized
//! differential test (long-lived cookers against fresh ones).

mod common;

use common::{float, graph};
use livspeil_graph::{CookError, CookStats, Cooker, Graph, NodeId};
use livspeil_node_api::{CookContext, Value};
use livspeil_params::{AnimChannel, Keyframe};

const ADD: &str = "core.math.add@1";
const CLOCK: &str = "core.time.clock@1";
const LERP: &str = "core.math.lerp@1";

fn out(cooker: &mut Cooker, g: &Graph, id: NodeId) -> f64 {
    cooker
        .cook_output(g, id, "out")
        .expect("cook succeeds")
        .as_float()
        .expect("float output")
}

fn stats(cooked: u64, reused: u64) -> CookStats {
    CookStats { cooked, reused }
}

fn at(cooker: &mut Cooker, frame: u64) {
    cooker.set_clock(CookContext {
        frame,
        time: frame as f64 * 0.5,
        dt: 0.5,
    });
}

/// `src` is read through `ch()` by `reader`, which feeds `sink` by a link.
fn ch_chain() -> (Graph, NodeId, NodeId, NodeId) {
    let mut g = graph();
    let src = g.add_node(ADD, "src").unwrap();
    let reader = g.add_node(ADD, "reader").unwrap();
    let sink = g.add_node(ADD, "sink").unwrap();
    g.set_input(src, "a", float(2.0)).unwrap();
    g.set_expression(reader, "a", r#"ch("src/a") * 10"#)
        .unwrap();
    g.link(reader, "out", sink, "a").unwrap();
    (g, src, reader, sink)
}

#[test]
fn edits_invalidate_through_ch_edges() {
    let (mut g, src, _, sink) = ch_chain();
    let mut c = Cooker::new();
    assert_eq!(out(&mut c, &g, sink), 20.0);

    c.reset_stats();
    g.set_input(src, "a", float(3.0)).unwrap();
    assert_eq!(out(&mut c, &g, sink), 30.0);
    assert_eq!(c.stats(), stats(3, 0));

    // src.b is not read by the expression, but src's resolved inputs
    // changed (node-level granularity): the reader re-cooks, produces the
    // same output, and the sink is cut off.
    c.reset_stats();
    g.set_input(src, "b", float(1.0)).unwrap();
    assert_eq!(out(&mut c, &g, sink), 30.0);
    assert_eq!(c.stats(), stats(2, 1));

    // Pointing the expression elsewhere drops the edge: src edits no
    // longer reach the reader.
    g.set_expression(src, "b", "0").unwrap();
    let other = g.add_node(ADD, "other").unwrap();
    g.set_input(other, "a", float(4.0)).unwrap();
    let reader = g.find("reader").unwrap();
    g.set_expression(reader, "a", r#"ch("other/a")"#).unwrap();
    assert_eq!(out(&mut c, &g, sink), 4.0);
    c.reset_stats();
    g.set_input(src, "a", float(9.0)).unwrap();
    assert_eq!(out(&mut c, &g, sink), 4.0);
    assert_eq!(c.stats(), stats(0, 3));
    g.set_input(other, "a", float(5.0)).unwrap();
    assert_eq!(out(&mut c, &g, sink), 5.0);

    // Clearing the expression drops the edge too.
    g.clear_expression(reader, "a").unwrap();
    assert_eq!(out(&mut c, &g, sink), 0.0);
    c.reset_stats();
    g.set_input(other, "a", float(6.0)).unwrap();
    assert_eq!(out(&mut c, &g, sink), 0.0);
    assert_eq!(c.stats(), stats(0, 2));
}

#[test]
fn unrelated_edits_cost_nothing() {
    let (mut g, _, _, sink) = ch_chain();
    let lone = g.add_node(ADD, "lone").unwrap();
    let mut c = Cooker::new();
    out(&mut c, &g, sink);
    c.reset_stats();
    g.set_input(lone, "a", float(1.0)).unwrap();
    g.clear_input(lone, "a").unwrap();
    assert_eq!(out(&mut c, &g, sink), 20.0);
    assert_eq!(c.stats(), stats(0, 3));
}

#[test]
fn removal_invalidates_and_slots_are_not_confused() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let mid = g.add_node(ADD, "mid").unwrap();
    let sink = g.add_node(ADD, "sink").unwrap();
    g.set_input(a, "a", float(7.0)).unwrap();
    g.link(a, "out", mid, "a").unwrap();
    g.link(mid, "out", sink, "a").unwrap();
    g.set_input(sink, "b", float(1.0)).unwrap();
    let mut c = Cooker::new();
    assert_eq!(out(&mut c, &g, sink), 8.0);
    assert_eq!(out(&mut c, &g, mid), 7.0);

    // Removing the middle node: the sink falls back to its own value.
    g.remove_node(mid).unwrap();
    assert_eq!(out(&mut c, &g, sink), 1.0);
    assert_eq!(c.cook(&g, mid).unwrap_err(), CookError::UnknownNode(mid));
    assert_eq!(c.error(mid), None);

    // A new node takes the freed slot; it must not inherit mid's result.
    let fresh = g.add_node(ADD, "fresh").unwrap();
    assert_eq!(out(&mut c, &g, fresh), 0.0);
    g.link(fresh, "out", sink, "a").unwrap();
    assert_eq!(out(&mut c, &g, sink), 1.0);
    g.set_input(fresh, "b", float(2.0)).unwrap();
    assert_eq!(out(&mut c, &g, sink), 3.0);

    // Removing an upstream node that only the removed node read.
    g.remove_node(a).unwrap();
    assert_eq!(out(&mut c, &g, sink), 3.0);
}

#[test]
fn renames_do_not_invalidate() {
    let (mut g, src, reader, sink) = ch_chain();
    let mut c = Cooker::new();
    out(&mut c, &g, sink);
    c.reset_stats();
    g.rename(src, "source").unwrap();
    g.rename(reader, "reader2").unwrap();
    g.rename(sink, "sink2").unwrap();
    assert_eq!(out(&mut c, &g, sink), 20.0);
    assert_eq!(c.stats(), stats(0, 3));
    // The rewritten expression still tracks its source.
    g.set_input(src, "a", float(1.0)).unwrap();
    assert_eq!(out(&mut c, &g, sink), 10.0);
}

#[test]
fn clock_changes_recook_only_time_dependent_branches() {
    let mut g = graph();
    let clock = g.add_node(CLOCK, "clock").unwrap();
    let follower = g.add_node(ADD, "follower").unwrap();
    let after = g.add_node(ADD, "after").unwrap();
    let still = g.add_node(ADD, "still").unwrap();
    let expr = g.add_node(ADD, "expr").unwrap();
    let anim = g.add_node(ADD, "anim").unwrap();
    let reads_anim = g.add_node(ADD, "reads_anim").unwrap();
    g.link(clock, "frame", follower, "a").unwrap();
    g.link(follower, "out", after, "a").unwrap();
    g.link(still, "out", after, "b").unwrap();
    g.set_expression(expr, "a", "$F * 2").unwrap();
    g.set_animation(
        anim,
        "a",
        AnimChannel::new(vec![Keyframe::float(0.0, 0.0), Keyframe::float(10.0, 10.0)]).unwrap(),
    )
    .unwrap();
    g.set_expression(reads_anim, "a", r#"ch("anim/a") + 1"#)
        .unwrap();

    let mut c = Cooker::new();
    at(&mut c, 0);
    let nodes = [after, expr, reads_anim];
    for &n in &nodes {
        out(&mut c, &g, n);
    }

    // Same clock again: nothing.
    c.reset_stats();
    at(&mut c, 0);
    for &n in &nodes {
        out(&mut c, &g, n);
    }
    assert_eq!(c.stats(), stats(0, 4 + 1 + 2));

    // New frame: clock, follower, after, expr, anim, reads_anim re-cook;
    // `still` is reused.
    c.reset_stats();
    at(&mut c, 3);
    assert_eq!(out(&mut c, &g, after), 3.0);
    assert_eq!(out(&mut c, &g, expr), 6.0);
    assert_eq!(out(&mut c, &g, reads_anim), 2.5);
    assert_eq!(c.stats(), stats(6, 1));

    // Dropping the animation makes `anim` time-independent again.
    g.clear_animation(anim, "a").unwrap();
    assert_eq!(out(&mut c, &g, reads_anim), 1.0);
    c.reset_stats();
    at(&mut c, 4);
    assert_eq!(out(&mut c, &g, reads_anim), 1.0);
    assert_eq!(c.stats(), stats(0, 2));

    // Unlinking the clock: `after` no longer follows the frame.
    g.unlink(follower, "a").unwrap();
    assert_eq!(out(&mut c, &g, after), 0.0);
    c.reset_stats();
    at(&mut c, 5);
    assert_eq!(out(&mut c, &g, after), 0.0);
    assert_eq!(c.stats(), stats(0, 3));
}

#[test]
fn two_cookers_share_a_graph() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let b = g.add_node(ADD, "b").unwrap();
    let c_node = g.add_node(ADD, "c").unwrap();
    g.link(a, "out", b, "a").unwrap();
    g.link(b, "out", c_node, "a").unwrap();
    let mut one = Cooker::new();
    let mut two = Cooker::new();
    assert_eq!(out(&mut one, &g, c_node), 0.0);
    assert_eq!(out(&mut two, &g, c_node), 0.0);

    // An edit, cooked by `one` only.
    g.set_input(a, "a", float(1.0)).unwrap();
    assert_eq!(out(&mut one, &g, c_node), 1.0);
    // A second edit downstream: its walk may stop early, but `two` must
    // still see both edits.
    g.set_input(c_node, "b", float(10.0)).unwrap();
    assert_eq!(out(&mut two, &g, c_node), 11.0);
    assert_eq!(out(&mut one, &g, c_node), 11.0);

    // Repeated edits between cooks (a slider drag) and partial cooks.
    for i in 0..5 {
        g.set_input(a, "b", float(f64::from(i))).unwrap();
        g.set_input(b, "b", float(f64::from(i) * 2.0)).unwrap();
        if i % 2 == 0 {
            assert_eq!(out(&mut one, &g, b), 1.0 + 3.0 * f64::from(i));
        }
    }
    assert_eq!(out(&mut two, &g, c_node), 23.0);
    assert_eq!(out(&mut one, &g, c_node), 23.0);
    two.reset_stats();
    assert_eq!(out(&mut two, &g, c_node), 23.0);
    assert_eq!(two.stats(), stats(0, 3));

    // A cooker used on another graph resets, then comes back.
    let other = graph();
    let lone_graph_node = {
        let mut other = other;
        let n = other.add_node(ADD, "n").unwrap();
        other.set_input(n, "a", float(5.0)).unwrap();
        assert_eq!(out(&mut one, &other, n), 5.0);
        n
    };
    assert_eq!(
        one.cook(&g, lone_graph_node).unwrap_err(),
        CookError::UnknownNode(lone_graph_node)
    );
    one.reset_stats();
    assert_eq!(out(&mut one, &g, c_node), 23.0);
    assert_eq!(one.stats(), stats(3, 0));
}

#[test]
fn many_edits_between_cooks_keep_early_cutoff() {
    let mut g = graph();
    let head = g.add_node(ADD, "head").unwrap();
    let mut prev = head;
    for i in 0..50 {
        let n = g.add_node(ADD, &format!("n{i}")).unwrap();
        g.link(prev, "out", n, "a").unwrap();
        prev = n;
    }
    let mut c = Cooker::new();
    g.set_input(head, "a", float(1.0)).unwrap();
    assert_eq!(out(&mut c, &g, prev), 1.0);
    c.reset_stats();
    g.set_input(head, "a", float(0.0)).unwrap();
    g.set_input(head, "b", float(1.0)).unwrap();
    assert_eq!(out(&mut c, &g, prev), 1.0);
    assert_eq!(c.stats(), stats(1, 50));
}

/// Deterministic xorshift generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Property: two long-lived cookers that each cook random subsets of the
/// nodes after random edits (including node removal and re-adding, which
/// reuses slots, and clock changes) always agree with a fresh cooker.
#[test]
fn long_lived_cookers_match_fresh_cookers() {
    const PORTS: [&str; 2] = ["a", "b"];
    let exprs = [
        "$T * 0.5",
        "1 + $F",
        "ch(\"{n}/{p}\") * 0.5",
        "ch(\"../{n}/{p}\") + ch(\"{m}/{q}\")",
        "sin($T) + ch(\"{n}/{p}\")",
        "3",
    ];
    for seed in 1..=300u64 {
        let mut rng = Rng(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) | 1);
        let mut g = graph();
        let mut counter = 0;
        let mut ids: Vec<NodeId> = Vec::new();
        for _ in 0..7 {
            let ty = [ADD, ADD, LERP, CLOCK][rng.below(4)];
            ids.push(g.add_node(ty, &format!("x{counter}")).unwrap());
            counter += 1;
        }
        let mut cookers = [Cooker::new(), Cooker::new()];
        for step in 0..40 {
            let node = ids[rng.below(ids.len())];
            let port = PORTS[rng.below(2)];
            let name = |rng: &mut Rng, g: &Graph| {
                g.node_name(ids[rng.below(ids.len())])
                    .unwrap_or("none")
                    .to_owned()
            };
            let _ = match rng.below(12) {
                0 | 1 => g.set_input(node, port, float(rng.below(100) as f64 / 10.0)),
                2 => g.clear_input(node, port),
                3 | 4 => {
                    let template = exprs[rng.below(exprs.len())];
                    let source = template
                        .replace("{n}", &name(&mut rng, &g))
                        .replace("{p}", PORTS[rng.below(2)])
                        .replace("{m}", &name(&mut rng, &g))
                        .replace("{q}", PORTS[rng.below(2)]);
                    g.set_expression(node, port, &source)
                }
                5 => g.clear_expression(node, port),
                6 => g.set_animation(
                    node,
                    port,
                    AnimChannel::new(vec![
                        Keyframe::float(0.0, rng.below(10) as f64),
                        Keyframe::float(2.0, rng.below(10) as f64),
                    ])
                    .unwrap(),
                ),
                7 => g.clear_animation(node, port),
                8 => {
                    let from = ids[rng.below(ids.len())];
                    let out_port = if g.node_type(from) == Some(CLOCK) {
                        ["frame", "time", "dt"][rng.below(3)]
                    } else {
                        "out"
                    };
                    g.link(from, out_port, node, port)
                }
                9 => g.unlink(node, port).map(|_| ()),
                10 => g.rename(node, &format!("r{step}")).map(|_| ()),
                _ => {
                    // Remove a node (refused if read through ch()) and add
                    // a new one, which reuses the slot.
                    let idx = rng.below(ids.len());
                    if g.remove_node(ids[idx]).is_ok() {
                        let ty = [ADD, LERP, CLOCK][rng.below(3)];
                        ids[idx] = g.add_node(ty, &format!("x{counter}")).unwrap();
                        counter += 1;
                    }
                    Ok(())
                }
            };
            let which = rng.below(2);
            if rng.below(3) == 0 {
                let frame = rng.below(4) as u64;
                cookers[which].set_clock(CookContext {
                    frame,
                    time: frame as f64 * 0.4,
                    dt: 0.4,
                });
            }
            // Each cooker cooks a random subset; the last step cooks all.
            for (k, cooker) in cookers.iter_mut().enumerate() {
                for &id in &ids {
                    if step != 39 && rng.below(3) != 0 {
                        continue;
                    }
                    let cached = cooker.cook(&g, id);
                    let mut fresh = Cooker::new();
                    fresh.set_clock(cooker.clock());
                    let expected = fresh.cook(&g, id);
                    // A long-lived cooker may serve the last good output
                    // where a fresh one fails; otherwise they agree.
                    if expected.is_ok() {
                        assert_eq!(cached, expected, "seed {seed} step {step} cooker {k}");
                    }
                    // Cooking again changes nothing.
                    let before = cooker.stats();
                    let again = cooker.cook(&g, id);
                    assert_eq!(again, cached, "seed {seed} step {step} cooker {k}");
                    assert_eq!(cooker.stats().cooked, before.cooked);
                }
            }
        }
        // Outputs are typed values.
        for &id in &ids {
            if let Ok(outputs) = cookers[0].cook(&g, id) {
                assert!(outputs.iter().all(|v| !matches!(v, Value::Bool(_))));
            }
        }
    }
}
