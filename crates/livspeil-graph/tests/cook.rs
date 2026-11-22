// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Scheduler behavior: laziness, caching, early cutoff, time, failures.

mod common;

use common::{float, graph};
use livspeil_graph::{CookError, CookStats, Cooker, Graph, NodeId};
use livspeil_node_api::{CookContext, NodeError, Value};

const ADD: &str = "core.math.add@1";
const CLOCK: &str = "core.time.clock@1";
const FRAGILE: &str = "test.fragile@1";
const LIAR: &str = "test.liar@1";

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

#[test]
fn a_node_cooks_from_its_defaults() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let mut cooker = Cooker::new();
    assert_eq!(out(&mut cooker, &g, a), 0.0);
}

#[test]
fn values_flow_through_links() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let b = g.add_node(ADD, "b").unwrap();
    g.set_input(a, "a", float(1.0)).unwrap();
    g.set_input(a, "b", float(2.0)).unwrap();
    g.set_input(b, "b", float(10.0)).unwrap();
    g.link(a, "out", b, "a").unwrap();

    let mut cooker = Cooker::new();
    assert_eq!(out(&mut cooker, &g, b), 13.0);
}

#[test]
fn a_link_wins_over_a_set_value() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let b = g.add_node(ADD, "b").unwrap();
    g.set_input(a, "a", float(4.0)).unwrap();
    g.set_input(b, "a", float(100.0)).unwrap();
    g.link(a, "out", b, "a").unwrap();
    let mut cooker = Cooker::new();
    assert_eq!(out(&mut cooker, &g, b), 4.0);

    // Unlinking brings the explicit value back.
    g.unlink(b, "a").unwrap();
    assert_eq!(out(&mut cooker, &g, b), 100.0);
}

#[test]
fn only_what_is_requested_is_cooked() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let b = g.add_node(ADD, "b").unwrap();
    let unrelated = g.add_node(ADD, "unrelated").unwrap();
    g.link(a, "out", b, "a").unwrap();

    let mut cooker = Cooker::new();
    out(&mut cooker, &g, b);
    assert_eq!(cooker.stats(), stats(2, 0));
    out(&mut cooker, &g, unrelated);
    assert_eq!(cooker.stats(), stats(3, 0));
}

#[test]
fn unchanged_graphs_are_served_from_the_cache() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let b = g.add_node(ADD, "b").unwrap();
    g.link(a, "out", b, "a").unwrap();

    let mut cooker = Cooker::new();
    out(&mut cooker, &g, b);
    cooker.reset_stats();
    out(&mut cooker, &g, b);
    assert_eq!(cooker.stats(), stats(0, 2));
}

#[test]
fn an_edit_recooks_the_node_and_what_depends_on_it() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let b = g.add_node(ADD, "b").unwrap();
    let c = g.add_node(ADD, "c").unwrap();
    g.link(a, "out", b, "a").unwrap();
    g.link(b, "out", c, "a").unwrap();

    let mut cooker = Cooker::new();
    out(&mut cooker, &g, c);

    // Editing the middle node leaves the upstream node cached.
    cooker.reset_stats();
    g.set_input(b, "b", float(5.0)).unwrap();
    assert_eq!(out(&mut cooker, &g, c), 5.0);
    assert_eq!(cooker.stats(), stats(2, 1));

    // Editing the first node recooks the whole chain.
    cooker.reset_stats();
    g.set_input(a, "a", float(1.0)).unwrap();
    assert_eq!(out(&mut cooker, &g, c), 6.0);
    assert_eq!(cooker.stats(), stats(3, 0));
}

#[test]
fn early_cutoff_stops_propagation_when_outputs_do_not_change() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let b = g.add_node(ADD, "b").unwrap();
    let c = g.add_node(ADD, "c").unwrap();
    g.set_input(a, "a", float(1.0)).unwrap();
    g.set_input(a, "b", float(2.0)).unwrap();
    g.link(a, "out", b, "a").unwrap();
    g.link(b, "out", c, "a").unwrap();

    let mut cooker = Cooker::new();
    assert_eq!(out(&mut cooker, &g, c), 3.0);

    // 2 + 1 is still 3: only `a` is recooked, `b` and `c` are reused.
    cooker.reset_stats();
    g.set_input(a, "a", float(2.0)).unwrap();
    g.set_input(a, "b", float(1.0)).unwrap();
    assert_eq!(out(&mut cooker, &g, c), 3.0);
    assert_eq!(cooker.stats(), stats(1, 2));
}

#[test]
fn only_time_dependent_branches_recook_each_frame() {
    let mut g = graph();
    let clock = g.add_node(CLOCK, "clock").unwrap();
    let follower = g.add_node(ADD, "follower").unwrap();
    let unrelated = g.add_node(ADD, "unrelated").unwrap();
    // Int -> Float is an allowed implicit conversion.
    g.link(clock, "frame", follower, "a").unwrap();

    let mut cooker = Cooker::new();
    for frame in 0..3_u32 {
        cooker.set_clock(CookContext {
            frame: u64::from(frame),
            time: f64::from(frame) * 0.5,
            dt: 0.5,
        });
        cooker.reset_stats();
        assert_eq!(out(&mut cooker, &g, follower), f64::from(frame));
        assert_eq!(out(&mut cooker, &g, unrelated), 0.0);
        if frame == 0 {
            assert_eq!(cooker.stats(), stats(3, 0));
        } else {
            // clock and follower recook, `unrelated` is cached.
            assert_eq!(cooker.stats(), stats(2, 1));
        }
    }

    // Cooking the same frame again costs nothing.
    cooker.reset_stats();
    out(&mut cooker, &g, follower);
    assert_eq!(cooker.stats(), stats(0, 2));
}

#[test]
fn failures_keep_the_last_good_output_and_recover() {
    let mut g = graph();
    let fragile = g.add_node(FRAGILE, "fragile").unwrap();
    let after = g.add_node(ADD, "after").unwrap();
    g.link(fragile, "out", after, "a").unwrap();
    g.set_input(after, "b", float(1.0)).unwrap();

    let mut cooker = Cooker::new();
    g.set_input(fragile, "x", float(5.0)).unwrap();
    assert_eq!(out(&mut cooker, &g, after), 11.0);

    // A node error: the last good output stays, the error is recorded.
    g.set_input(fragile, "x", float(-1.0)).unwrap();
    assert_eq!(out(&mut cooker, &g, after), 11.0);
    assert!(matches!(
        cooker.error(fragile),
        Some(CookError::Node(NodeError::InvalidInput { port: "x", .. }))
    ));
    assert_eq!(cooker.error(after), None);

    // A panic is contained and reported the same way.
    g.set_input(fragile, "x", float(2000.0)).unwrap();
    assert_eq!(out(&mut cooker, &g, after), 11.0);
    assert!(matches!(
        cooker.error(fragile),
        Some(CookError::Panicked(msg)) if msg.contains("far too large")
    ));

    // Fixing the input clears the error and flows downstream again.
    g.set_input(fragile, "x", float(3.0)).unwrap();
    assert_eq!(out(&mut cooker, &g, after), 7.0);
    assert_eq!(cooker.error(fragile), None);
}

#[test]
fn a_node_that_never_succeeded_reports_its_error_downstream() {
    let mut g = graph();
    let fragile = g.add_node(FRAGILE, "fragile").unwrap();
    let after = g.add_node(ADD, "after").unwrap();
    g.link(fragile, "out", after, "a").unwrap();
    g.set_input(fragile, "x", float(-1.0)).unwrap();

    let mut cooker = Cooker::new();
    assert!(matches!(cooker.cook(&g, fragile), Err(CookError::Node(_))));
    assert_eq!(
        cooker.cook(&g, after).unwrap_err(),
        CookError::Upstream { node: fragile }
    );

    g.set_input(fragile, "x", float(1.0)).unwrap();
    assert_eq!(out(&mut cooker, &g, after), 2.0);
}

#[test]
fn outputs_that_violate_the_spec_are_rejected() {
    let mut g = graph();
    let liar = g.add_node(LIAR, "liar").unwrap();
    let mut cooker = Cooker::new();
    assert!(matches!(
        cooker.cook(&g, liar),
        Err(CookError::BadOutput(msg)) if msg.contains("should be Float")
    ));
}

#[test]
fn removing_a_node_reverts_its_consumers_to_their_own_values() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let b = g.add_node(ADD, "b").unwrap();
    g.set_input(a, "a", float(7.0)).unwrap();
    g.set_input(b, "a", float(1.0)).unwrap();
    g.set_input(b, "b", float(2.0)).unwrap();
    g.link(a, "out", b, "a").unwrap();

    let mut cooker = Cooker::new();
    assert_eq!(out(&mut cooker, &g, b), 9.0);
    g.remove_node(a).unwrap();
    assert_eq!(out(&mut cooker, &g, b), 3.0);
    assert_eq!(cooker.cook(&g, a).unwrap_err(), CookError::UnknownNode(a));
}

#[test]
fn very_deep_graphs_do_not_overflow_the_stack() {
    const DEPTH: usize = 20_000;
    let mut g = graph();
    let mut previous = g.add_node(ADD, "n0").unwrap();
    g.set_input(previous, "a", float(1.0)).unwrap();
    for i in 1..DEPTH {
        let next = g.add_node(ADD, &format!("n{i}")).unwrap();
        g.set_input(next, "b", float(1.0)).unwrap();
        g.link(previous, "out", next, "a").unwrap();
        previous = next;
    }

    let mut cooker = Cooker::new();
    assert_eq!(out(&mut cooker, &g, previous), DEPTH as f64);
    assert_eq!(cooker.stats().cooked, DEPTH as u64);

    // A change at the head recooks the chain exactly once.
    cooker.reset_stats();
    g.set_input(g.find("n0").unwrap(), "a", float(2.0)).unwrap();
    assert_eq!(out(&mut cooker, &g, previous), DEPTH as f64 + 1.0);
    assert_eq!(cooker.stats().cooked, DEPTH as u64);
}

#[test]
fn unknown_nodes_are_reported() {
    let g = graph();
    let missing = NodeId::new();
    let mut cooker = Cooker::new();
    assert_eq!(
        cooker.cook(&g, missing).unwrap_err(),
        CookError::UnknownNode(missing)
    );
}

#[test]
fn output_values_are_typed() {
    let mut g = graph();
    let clock = g.add_node(CLOCK, "clock").unwrap();
    let mut cooker = Cooker::new();
    cooker.set_clock(CookContext {
        frame: 3,
        time: 1.5,
        dt: 0.5,
    });
    assert_eq!(
        cooker.cook(&g, clock).unwrap(),
        vec![Value::Int(3), Value::Float(1.5), Value::Float(0.5)]
    );
    assert!(matches!(
        cooker.cook_output(&g, clock, "nope"),
        Err(CookError::BadOutput(_))
    ));
}
