// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Input sources: expressions, animation channels, `ch()` dependencies,
//! renames, deletes and caching.

mod common;

use common::{float, graph};
use livspeil_graph::{CookError, CookStats, Cooker, Graph, GraphError, NodeId};
use livspeil_node_api::{CookContext, PortType, Value};
use livspeil_params::expr::ErrorKind;
use livspeil_params::{AnimChannel, Keyframe};

const ADD: &str = "core.math.add@1";
const SINK: &str = "test.sink@1";

fn out(cooker: &mut Cooker, g: &Graph, id: NodeId) -> f64 {
    cooker
        .cook_output(g, id, "out")
        .expect("cook succeeds")
        .as_float()
        .expect("float output")
}

fn ramp() -> AnimChannel {
    AnimChannel::new(vec![Keyframe::float(0.0, 0.0), Keyframe::float(1.0, 10.0)]).unwrap()
}

fn at(cooker: &mut Cooker, frame: u64) {
    cooker.set_clock(CookContext {
        frame,
        time: frame as f64 * 0.5,
        dt: 0.5,
    });
}

#[test]
fn priority_is_link_expression_animation_value_default() {
    let mut g = graph();
    let src = g.add_node(ADD, "src").unwrap();
    let n = g.add_node(ADD, "n").unwrap();
    g.set_input(src, "a", float(100.0)).unwrap();
    let mut c = Cooker::new();
    at(&mut c, 1); // time 0.5

    assert_eq!(out(&mut c, &g, n), 0.0); // default
    g.set_input(n, "a", float(1.0)).unwrap();
    assert_eq!(out(&mut c, &g, n), 1.0); // value
    g.set_animation(n, "a", ramp()).unwrap();
    assert_eq!(out(&mut c, &g, n), 5.0); // animation
    g.set_expression(n, "a", "2 + 1").unwrap();
    assert_eq!(out(&mut c, &g, n), 3.0); // expression (Int widened)
    g.link(src, "out", n, "a").unwrap();
    assert_eq!(out(&mut c, &g, n), 100.0); // link

    g.unlink(n, "a").unwrap();
    assert_eq!(out(&mut c, &g, n), 3.0);
    g.clear_expression(n, "a").unwrap();
    assert_eq!(out(&mut c, &g, n), 5.0);
    g.clear_animation(n, "a").unwrap();
    assert_eq!(out(&mut c, &g, n), 1.0);
}

#[test]
fn getters_and_snapshot() {
    let mut g = graph();
    let n = g.add_node(ADD, "n").unwrap();
    assert_eq!(g.expression(n, "a").unwrap(), None);
    g.set_expression(n, "b", "$T * 2").unwrap();
    g.set_animation(n, "a", ramp()).unwrap();
    assert_eq!(g.expression(n, "b").unwrap(), Some("$T * 2"));
    assert_eq!(g.animation(n, "a").unwrap(), Some(&ramp()));
    assert_eq!(g.expressions(n).collect::<Vec<_>>(), [("b", "$T * 2")]);
    assert_eq!(g.animations(n).count(), 1);
    let snap = g.snapshot();
    assert_eq!(
        snap.nodes[0].expressions,
        [("b".to_owned(), "$T * 2".to_owned())]
    );
    assert_eq!(snap.nodes[0].animations, [("a".to_owned(), ramp())]);
    assert!(matches!(
        g.expression(n, "nope"),
        Err(GraphError::UnknownPort { .. })
    ));
}

#[test]
fn expressions_are_checked_when_set() {
    let mut g = graph();
    let n = g.add_node(ADD, "n").unwrap();
    let s = g.add_node(SINK, "s").unwrap();
    assert_eq!(
        g.set_expression(n, "a", "true"),
        Err(GraphError::TypeMismatch {
            expected: PortType::Float,
            found: PortType::Bool
        })
    );
    assert_eq!(
        g.set_expression(s, "count", "1.5"),
        Err(GraphError::TypeMismatch {
            expected: PortType::Int,
            found: PortType::Float
        })
    );
    let Err(GraphError::Expression(err)) = g.set_expression(n, "a", r#"ch("ghost/a")"#) else {
        panic!("unknown path must fail");
    };
    assert_eq!(err.kind, ErrorKind::UnknownChannel("ghost/a".into()));
    assert!(matches!(
        g.set_expression(n, "a", "1 +"),
        Err(GraphError::Expression(_))
    ));
    assert!(matches!(
        g.set_expression(n, "a", r#"ch("n")"#),
        Err(GraphError::Expression(_))
    ));
    assert_eq!(
        g.set_animation(s, "count", ramp()),
        Err(GraphError::TypeMismatch {
            expected: PortType::Int,
            found: PortType::Float
        })
    );
    // Nothing was stored.
    assert_eq!(
        g.snapshot()
            .nodes
            .iter()
            .map(|n| n.expressions.len())
            .sum::<usize>(),
        0
    );
    // ch() of a Bool input into an Int input via a conditional.
    g.set_expression(s, "count", r#"ch("s/flag") ? 1 : 2"#)
        .expect_err("self reference is a cycle");
    let s2 = g.add_node(SINK, "s2").unwrap();
    g.set_expression(s2, "count", r#"ch("../s/flag") ? 1 : 2"#)
        .unwrap();
}

#[test]
fn ch_references_count_as_edges_for_cycles() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let b = g.add_node(ADD, "b").unwrap();
    let c = g.add_node(ADD, "c").unwrap();
    assert_eq!(
        g.set_expression(a, "a", r#"ch("a/b")"#),
        Err(GraphError::WouldCycle)
    );
    g.link(a, "out", b, "a").unwrap();
    // b depends on a, so a may not read b.
    assert_eq!(
        g.set_expression(a, "a", r#"ch("b/b")"#),
        Err(GraphError::WouldCycle)
    );
    // c reads b; then linking c into a closes a -> b -> c -> a.
    g.set_expression(c, "a", r#"ch("b/a") + 1"#).unwrap();
    assert_eq!(g.link(c, "out", a, "b"), Err(GraphError::WouldCycle));
    // And a reading c is a cycle through the ch() edge as well.
    assert_eq!(
        g.set_expression(a, "b", r#"ch("c/b")"#),
        Err(GraphError::WouldCycle)
    );
    // Replacing c's expression with one that reads a is fine.
    g.set_expression(c, "a", r#"ch("a/a")"#).unwrap();
    g.clear_expression(c, "a").unwrap();
    g.link(c, "out", a, "b").unwrap();
}

#[test]
fn ch_reads_the_resolved_input_and_invalidates() {
    let mut g = graph();
    let src = g.add_node(ADD, "src").unwrap();
    let mid = g.add_node(ADD, "mid").unwrap();
    let lamp = g.add_node(ADD, "lamp").unwrap();
    let other = g.add_node(ADD, "other").unwrap();
    g.set_input(src, "a", float(3.0)).unwrap();
    g.link(src, "out", mid, "a").unwrap();
    // mid.a is linked: ch() sees the link's value.
    g.set_expression(lamp, "a", r#"ch("mid/a") * 2"#).unwrap();
    let mut c = Cooker::new();
    assert_eq!(out(&mut c, &g, lamp), 6.0);

    // Changing an unrelated node: nothing re-cooks.
    g.set_input(other, "a", float(1.0)).unwrap();
    c.reset_stats();
    assert_eq!(out(&mut c, &g, lamp), 6.0);
    assert_eq!(
        c.stats(),
        CookStats {
            cooked: 0,
            reused: 3
        }
    );

    // Changing mid.b changes mid's inputs but not mid.a: lamp re-cooks
    // (node-level granularity) and gives the same answer.
    g.set_input(src, "a", float(4.0)).unwrap();
    assert_eq!(out(&mut c, &g, lamp), 8.0);

    // Time-independent expressions are reused across frames.
    at(&mut c, 7);
    c.reset_stats();
    assert_eq!(out(&mut c, &g, lamp), 8.0);
    assert_eq!(c.stats().cooked, 0);
}

#[test]
fn time_dependence_comes_from_expressions_and_animations() {
    let mut g = graph();
    let e = g.add_node(ADD, "e").unwrap();
    let anim = g.add_node(ADD, "anim").unwrap();
    let reader = g.add_node(ADD, "reader").unwrap();
    g.set_expression(e, "a", "$F").unwrap();
    g.set_animation(anim, "a", ramp()).unwrap();
    g.set_expression(reader, "a", r#"ch("anim/a") + ch("e/a")"#)
        .unwrap();
    let mut c = Cooker::new();
    for frame in 0..4 {
        at(&mut c, frame);
        let t = frame as f64 * 0.5;
        assert_eq!(out(&mut c, &g, e), frame as f64);
        assert_eq!(out(&mut c, &g, anim), (t * 10.0).min(10.0));
        assert_eq!(out(&mut c, &g, reader), (t * 10.0).min(10.0) + frame as f64);
    }
}

#[test]
fn runtime_errors_keep_the_last_good_output() {
    let mut g = graph();
    let src = g.add_node(SINK, "src").unwrap();
    let n = g.add_node(ADD, "n").unwrap();
    g.set_input(src, "count", Value::Int(2)).unwrap();
    g.set_expression(n, "a", r#"10 / ch("src/count")"#).unwrap();
    let mut c = Cooker::new();
    assert_eq!(out(&mut c, &g, n), 5.0);
    assert_eq!(c.error(n), None);

    g.set_input(src, "count", Value::Int(0)).unwrap();
    assert_eq!(out(&mut c, &g, n), 5.0);
    let Some(CookError::Expression { port, error }) = c.error(n) else {
        panic!("expected an expression error, got {:?}", c.error(n));
    };
    assert_eq!(*port, "a");
    assert_eq!(error.kind, ErrorKind::DivisionByZero);

    // A fresh cooker has no last good output.
    assert!(matches!(
        Cooker::new().cook(&g, n),
        Err(CookError::Expression { .. })
    ));

    g.set_input(src, "count", Value::Int(5)).unwrap();
    assert_eq!(out(&mut c, &g, n), 2.0);
    assert_eq!(c.error(n), None);
}

#[test]
fn renames_rewrite_references_and_deletes_are_refused() {
    let mut g = graph();
    let sun = g.add_node(ADD, "sun").unwrap();
    let lamp = g.add_node(ADD, "lamp").unwrap();
    let src = r#"ch("sun/a") + ch("../sun/b") * ch("lamp2/a")"#;
    let lamp2 = g.add_node(ADD, "lamp2").unwrap();
    g.set_expression(lamp, "a", src).unwrap();
    g.set_input(sun, "a", float(1.0)).unwrap();
    g.set_input(sun, "b", float(2.0)).unwrap();
    let mut c = Cooker::new();
    assert_eq!(out(&mut c, &g, lamp), 1.0);

    assert_eq!(g.rename(sun, "star").unwrap(), "sun");
    assert_eq!(
        g.expression(lamp, "a").unwrap(),
        Some(r#"ch("star/a") + ch("../star/b") * ch("lamp2/a")"#)
    );
    // Renaming does not invalidate.
    c.reset_stats();
    assert_eq!(out(&mut c, &g, lamp), 1.0);
    assert_eq!(c.stats().cooked, 0);
    g.rename(sun, "sun").unwrap();
    assert_eq!(g.expression(lamp, "a").unwrap(), Some(src));
    assert_eq!(g.referenced_by(sun), [(lamp, "a".to_owned())]);

    assert_eq!(
        g.remove_node(sun),
        Err(GraphError::Referenced {
            node: sun,
            by: lamp,
            port: "a".to_owned()
        })
    );
    assert!(g.contains(sun));
    // The reader itself can go, which releases the reference.
    g.remove_node(lamp).unwrap();
    assert!(g.referenced_by(sun).is_empty());
    assert!(g.referenced_by(lamp2).is_empty());
    g.remove_node(sun).unwrap();
}

/// Deterministic xorshift generator for the randomized test.
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

/// Property: after any sequence of edits and clock changes, a long-lived
/// cooker returns exactly what a fresh cooker returns, for every node.
#[test]
fn cached_cooks_equal_fresh_cooks() {
    const NAMES: [&str; 6] = ["n0", "n1", "n2", "n3", "n4", "n5"];
    const PORTS: [&str; 2] = ["a", "b"];
    let exprs = [
        "$T * 0.5",
        "1 + $F",
        "ch(\"{n}/{p}\") * 0.5",
        "lerp(0.2, 1.0, ch(\"{n}/{p}\")) + 1",
        "ch(\"../{n}/{p}\") + ch(\"{m}/{q}\")",
        "sin($T) + ch(\"{n}/{p}\")",
        "3",
    ];
    for seed in 1..=200u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut g = graph();
        let ids: Vec<NodeId> = NAMES
            .iter()
            .map(|n| {
                let ty = if rng.below(3) == 0 {
                    "core.math.lerp@1"
                } else {
                    ADD
                };
                g.add_node(ty, n).unwrap()
            })
            .collect();
        let mut cooker = Cooker::new();
        for step in 0..40 {
            let node = ids[rng.below(ids.len())];
            let port = PORTS[rng.below(2)];
            let _ = match rng.below(9) {
                0 => g.set_input(node, port, float(rng.below(100) as f64 / 10.0)),
                1 | 2 => {
                    let template = exprs[rng.below(exprs.len())];
                    let source = template
                        .replace("{n}", NAMES[rng.below(NAMES.len())])
                        .replace("{p}", PORTS[rng.below(2)])
                        .replace("{m}", NAMES[rng.below(NAMES.len())])
                        .replace("{q}", PORTS[rng.below(2)]);
                    g.set_expression(node, port, &source)
                }
                3 => g.clear_expression(node, port),
                4 => g.set_animation(
                    node,
                    port,
                    AnimChannel::new(vec![
                        Keyframe::float(0.0, rng.below(10) as f64),
                        Keyframe::float(2.0, rng.below(10) as f64),
                    ])
                    .unwrap(),
                ),
                5 => g.clear_animation(node, port),
                6 => g.link(ids[rng.below(ids.len())], "out", node, port),
                7 => g.unlink(node, port).map(|_| ()),
                _ => g.rename(node, &format!("{}_{step}", NAMES[0])).map(|_| ()),
            };
            if rng.below(2) == 0 {
                let frame = rng.below(5) as u64;
                cooker.set_clock(CookContext {
                    frame,
                    time: frame as f64 * 0.4,
                    dt: 0.4,
                });
            }
            let clock = cooker.clock();
            for &id in &ids {
                let cached = cooker.cook(&g, id);
                let mut fresh = Cooker::new();
                fresh.set_clock(clock);
                let expected = fresh.cook(&g, id);
                assert_eq!(cached, expected, "seed {seed} step {step} node {id}");
            }
        }
    }
}
