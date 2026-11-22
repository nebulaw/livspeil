// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `out("node/port")` in expressions (M0.11, decision D48): reading
//! another node's output, as an edge for cycles, evaluation order,
//! invalidation, early cutoff, error containment, renames, moves,
//! deletes and subnets.

mod common;

use common::{float, graph};
use livspeil_graph::{
    CookError, CookStats, Cooker, ExprRef, Graph, GraphError, NodeId, RefKind, SUBNET_TYPE,
};
use livspeil_node_api::{CookContext, PortType, Value};
use livspeil_params::expr::ErrorKind;

const ADD: &str = "core.math.add@1";
const MUL: &str = "core.math.mul@1";
const CLOCK: &str = "core.time.clock@1";
const FRAGILE: &str = "test.fragile@1";
const SINK: &str = "test.sink@1";
const COLUMN: &str = "core.table.column@1";

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

fn expr_kind(err: GraphError) -> ErrorKind {
    match err {
        GraphError::Expression(e) => e.kind,
        other => panic!("expected an expression error, got {other:?}"),
    }
}

#[test]
fn out_reads_the_computed_output() {
    let mut g = graph();
    let src = g.add_node(ADD, "src").unwrap();
    let reader = g.add_node(ADD, "reader").unwrap();
    g.set_input(src, "a", float(2.0)).unwrap();
    g.set_input(src, "b", float(3.0)).unwrap();
    g.set_expression(reader, "a", r#"out("src/out") * 2"#)
        .unwrap();
    g.set_expression(reader, "b", r#"ch("src/a")"#).unwrap();
    let mut c = Cooker::new();
    // The reader pulls its upstream: src is cooked first.
    assert_eq!(out(&mut c, &g, reader), 12.0);
    assert_eq!(c.stats(), stats(2, 0));
    // `../` in the root network is the root, as with ch().
    g.set_expression(reader, "a", r#"out("../src/out")"#)
        .unwrap();
    assert_eq!(out(&mut c, &g, reader), 7.0);
}

#[test]
fn unknown_and_unusable_outputs_are_compile_errors() {
    let mut g = graph();
    let src = g.add_node(ADD, "src").unwrap();
    let reader = g.add_node(ADD, "reader").unwrap();
    let column = g.add_node(COLUMN, "column").unwrap();
    let sink = g.add_node(SINK, "sink").unwrap();
    let _ = (src, column, sink);
    assert_eq!(
        expr_kind(
            g.set_expression(reader, "a", r#"out("src/a")"#)
                .unwrap_err()
        ),
        ErrorKind::UnknownOutput("src/a".into())
    );
    assert_eq!(
        expr_kind(
            g.set_expression(reader, "a", r#"out("nope/out")"#)
                .unwrap_err()
        ),
        ErrorKind::UnknownOutput("nope/out".into())
    );
    assert_eq!(
        expr_kind(
            g.set_expression(reader, "a", r#"out("column/out")"#)
                .unwrap_err()
        ),
        ErrorKind::UnsupportedOutputType {
            path: "column/out".into(),
            ty: PortType::Table
        }
    );
    // A Bool output does not fit a Float input.
    assert_eq!(
        g.set_expression(reader, "a", r#"out("sink/flag")"#),
        Err(GraphError::TypeMismatch {
            expected: PortType::Float,
            found: PortType::Bool
        })
    );
    // But works where a Bool is expected.
    g.set_expression(reader, "a", r#"out("sink/flag") ? 1 : 0"#)
        .unwrap();
    assert_eq!(
        g.expression(reader, "a").unwrap(),
        Some(r#"out("sink/flag") ? 1 : 0"#)
    );
}

#[test]
fn cycles_through_links_ch_and_out_are_refused() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let b = g.add_node(ADD, "b").unwrap();
    let c = g.add_node(ADD, "c").unwrap();
    // Self-reference.
    assert_eq!(
        g.set_expression(a, "a", r#"out("a/out")"#),
        Err(GraphError::WouldCycle)
    );
    // a -> b by link, b -> c by out(); c -> a by any kind closes a cycle.
    g.link(a, "out", b, "a").unwrap();
    g.set_expression(c, "a", r#"out("b/out")"#).unwrap();
    for src in [r#"out("c/out")"#, r#"ch("c/a")"#] {
        assert_eq!(g.set_expression(a, "b", src), Err(GraphError::WouldCycle));
    }
    assert_eq!(g.link(c, "out", a, "b"), Err(GraphError::WouldCycle));
    // ch() then out(): b reads c's input; c reading b's output is a cycle.
    let mut g = graph();
    let b = g.add_node(ADD, "b").unwrap();
    let c = g.add_node(ADD, "c").unwrap();
    g.set_expression(b, "a", r#"ch("c/a")"#).unwrap();
    assert_eq!(
        g.set_expression(c, "b", r#"out("b/out")"#),
        Err(GraphError::WouldCycle)
    );
    // Replacing the expression removes the edge, so the reverse is fine.
    g.set_expression(b, "a", "1").unwrap();
    g.set_expression(c, "b", r#"out("b/out")"#).unwrap();
    // The same node read as ch() and out() in one expression.
    g.set_expression(c, "a", r#"out("b/out") + ch("b/a")"#)
        .unwrap();
    g.clear_expression(c, "a").unwrap();
    assert_eq!(g.referenced_by(b), [(c, "b".to_owned())]);
}

#[test]
fn invalidation_and_early_cutoff_through_out() {
    let mut g = graph();
    let src = g.add_node(ADD, "src").unwrap();
    let by_out = g.add_node(ADD, "by_out").unwrap();
    let by_ch = g.add_node(ADD, "by_ch").unwrap();
    let tail = g.add_node(ADD, "tail").unwrap();
    g.set_input(src, "a", float(1.0)).unwrap();
    g.set_input(src, "b", float(2.0)).unwrap();
    g.set_expression(by_out, "a", r#"out("src/out") * 10"#)
        .unwrap();
    g.set_expression(by_ch, "a", r#"ch("src/a")"#).unwrap();
    g.link(by_out, "out", tail, "a").unwrap();
    let mut c = Cooker::new();
    assert_eq!(out(&mut c, &g, tail), 30.0);
    assert_eq!(out(&mut c, &g, by_ch), 1.0);

    // Inputs change but the sum does not: src re-cooks, its out() readers
    // are cut off, its ch() readers re-cook.
    g.set_input(src, "a", float(2.0)).unwrap();
    g.set_input(src, "b", float(1.0)).unwrap();
    c.reset_stats();
    assert_eq!(out(&mut c, &g, tail), 30.0);
    assert_eq!(c.stats(), stats(1, 2));
    c.reset_stats();
    assert_eq!(out(&mut c, &g, by_ch), 2.0);
    assert_eq!(c.stats(), stats(1, 1));

    // The output changes: everything downstream re-cooks.
    g.set_input(src, "b", float(5.0)).unwrap();
    c.reset_stats();
    assert_eq!(out(&mut c, &g, tail), 70.0);
    assert_eq!(c.stats(), stats(3, 0));

    // Unrelated edits cost nothing.
    let other = g.add_node(ADD, "other").unwrap();
    g.set_input(other, "a", float(1.0)).unwrap();
    c.reset_stats();
    assert_eq!(out(&mut c, &g, tail), 70.0);
    assert_eq!(c.stats(), stats(0, 3));

    // A fresh cooker agrees.
    assert_eq!(out(&mut Cooker::new(), &g, tail), 70.0);
}

#[test]
fn time_dependence_flows_through_out() {
    let mut g = graph();
    let clock = g.add_node(CLOCK, "clock").unwrap();
    let reader = g.add_node(ADD, "reader").unwrap();
    let still = g.add_node(ADD, "still").unwrap();
    g.set_expression(reader, "a", r#"out("clock/time") + out("clock/frame")"#)
        .unwrap();
    let mut c = Cooker::new();
    let at = |c: &mut Cooker, frame: u64| {
        c.set_clock(CookContext {
            frame,
            time: frame as f64 * 0.5,
            dt: 0.5,
        });
    };
    at(&mut c, 2);
    assert_eq!(out(&mut c, &g, reader), 3.0);
    assert_eq!(out(&mut c, &g, still), 0.0);
    at(&mut c, 4);
    c.reset_stats();
    assert_eq!(out(&mut c, &g, reader), 6.0);
    assert_eq!(c.stats(), stats(2, 0));
    c.reset_stats();
    assert_eq!(out(&mut c, &g, still), 0.0);
    assert_eq!(c.stats(), stats(0, 1));
    let _ = clock;
}

#[test]
fn failing_targets_are_contained() {
    let mut g = graph();
    let fragile = g.add_node(FRAGILE, "fragile").unwrap();
    let reader = g.add_node(ADD, "reader").unwrap();
    g.set_input(fragile, "x", float(-1.0)).unwrap();
    g.set_expression(reader, "a", r#"out("fragile/out") + 1"#)
        .unwrap();
    let mut c = Cooker::new();
    // Never produced an output: the reader gets a typed error.
    assert_eq!(
        c.cook(&g, reader),
        Err(CookError::Upstream { node: fragile })
    );
    assert!(matches!(c.error(fragile), Some(CookError::Node(_))));
    // A good output.
    g.set_input(fragile, "x", float(2.0)).unwrap();
    assert_eq!(out(&mut c, &g, reader), 5.0);
    // Failing (or panicking) later keeps the last good output for readers.
    for x in [-3.0, 5000.0] {
        g.set_input(fragile, "x", float(x)).unwrap();
        c.reset_stats();
        assert_eq!(out(&mut c, &g, reader), 5.0);
        assert!(c.error(fragile).is_some());
        assert!(c.error(reader).is_none());
        assert_eq!(c.stats(), stats(1, 1));
    }
    // A fresh cooker has no last good output.
    assert_eq!(
        Cooker::new().cook(&g, reader),
        Err(CookError::Upstream { node: fragile })
    );
}

#[test]
fn renames_moves_and_deletes() {
    let mut g = graph();
    let src = g.add_node(ADD, "src").unwrap();
    let reader = g.add_node(ADD, "reader").unwrap();
    g.set_input(src, "a", float(4.0)).unwrap();
    g.set_expression(
        reader,
        "a",
        r#"out("src/out") + ch("src/a") + out("../src/out")"#,
    )
    .unwrap();
    let mut c = Cooker::new();
    assert_eq!(out(&mut c, &g, reader), 12.0);

    g.rename(src, "sun \"☀\"").unwrap();
    assert_eq!(
        g.expression(reader, "a").unwrap(),
        Some(r#"out("sun \"☀\"/out") + ch("sun \"☀\"/a") + out("../sun \"☀\"/out")"#)
    );
    g.rename(src, "src").unwrap();
    assert_eq!(
        g.expression(reader, "a").unwrap(),
        Some(r#"out("src/out") + ch("src/a") + out("../src/out")"#)
    );
    // Renames do not invalidate.
    c.reset_stats();
    assert_eq!(out(&mut c, &g, reader), 12.0);
    assert_eq!(c.stats().cooked, 0);

    // Deleting a node read through out() is refused.
    assert_eq!(
        g.remove_node(src),
        Err(GraphError::Referenced {
            node: src,
            by: reader,
            port: "a".to_owned()
        })
    );
    assert_eq!(g.referenced_by(src), [(reader, "a".to_owned())]);

    // Moving the reader into a subnet re-renders its paths.
    let s = g
        .add_node_in(None, NodeId::new(), SUBNET_TYPE, "s", None)
        .unwrap();
    let moved = g.move_node(reader, Some(s)).unwrap();
    assert_eq!(moved.len(), 1);
    assert_eq!(
        g.expression(reader, "a").unwrap(),
        Some(r#"out("../src/out") + ch("../src/a") + out("../src/out")"#)
    );
    c.reset_stats();
    assert_eq!(out(&mut c, &g, reader), 12.0);
    assert_eq!(c.stats().cooked, 0);
    // Moving the source next to it re-renders again.
    g.move_node(src, Some(s)).unwrap();
    assert_eq!(
        g.expression(reader, "a").unwrap(),
        Some(r#"out("src/out") + ch("src/a") + out("src/out")"#)
    );
    assert_eq!(out(&mut c, &g, reader), 12.0);

    // Once the expression is cleared the node can go.
    g.clear_expression(reader, "a").unwrap();
    assert!(g.referenced_by(src).is_empty());
    g.remove_node(src).unwrap();
    // Removing a subnet whose inside reads itself by out() is fine.
    let inner = g
        .add_node_in(Some(s), NodeId::new(), ADD, "inner", None)
        .unwrap();
    g.set_expression(reader, "b", r#"out("inner/out")"#)
        .unwrap();
    let _ = inner;
    g.remove_node(s).unwrap();
    assert_eq!(g.node_count(), 0);
}

#[test]
fn out_paths_through_subnets() {
    let mut g = graph();
    let s = g
        .add_node_in(None, NodeId::new(), SUBNET_TYPE, "s", None)
        .unwrap();
    let inner = g
        .add_node_in(Some(s), NodeId::new(), MUL, "inner", None)
        .unwrap();
    let root = g.add_node(ADD, "root").unwrap();
    let sibling = g
        .add_node_in(Some(s), NodeId::new(), ADD, "sibling", None)
        .unwrap();
    g.set_input(inner, "a", float(3.0)).unwrap();
    g.set_input(inner, "b", float(2.0)).unwrap();
    g.set_input(root, "b", float(1.0)).unwrap();
    g.set_expression(root, "a", r#"out("s/inner/out") + out("/s/inner/out")"#)
        .unwrap();
    g.set_expression(sibling, "a", r#"out("inner/out") * out("../root/out")"#)
        .unwrap();
    let mut c = Cooker::new();
    assert_eq!(out(&mut c, &g, root), 13.0);
    assert_eq!(out(&mut c, &g, sibling), 78.0);
    // A cycle across the boundary.
    assert_eq!(
        g.set_expression(inner, "a", r#"out("../root/out")"#),
        Err(GraphError::WouldCycle)
    );
    // Renaming the subnet rewrites paths through it.
    g.rename(s, "lamp").unwrap();
    assert_eq!(
        g.expression(root, "a").unwrap(),
        Some(r#"out("lamp/inner/out") + out("/lamp/inner/out")"#)
    );
    assert_eq!(
        g.expression(sibling, "a").unwrap(),
        Some(r#"out("inner/out") * out("../root/out")"#)
    );
    // Editing inside the subnet invalidates readers outside.
    g.set_input(inner, "b", float(1.0)).unwrap();
    c.reset_stats();
    assert_eq!(out(&mut c, &g, sibling), 21.0);
    assert_eq!(c.stats(), stats(3, 0));
    // Deleting the subnet is refused while root reads inside it.
    assert!(matches!(
        g.remove_node(s),
        Err(GraphError::Referenced { by, .. }) if by == root
    ));
}

#[test]
fn expression_refs_list_every_reference() {
    let mut g = graph();
    let a = g.add_node_with_id(id(1), ADD, "a").unwrap();
    let b = g.add_node_with_id(id(2), ADD, "b").unwrap();
    let r = g.add_node_with_id(id(3), ADD, "r").unwrap();
    let s = g.add_node_in(None, id(4), SUBNET_TYPE, "s", None).unwrap();
    let inner = g.add_node_in(Some(s), id(5), ADD, "inner", None).unwrap();
    assert!(g.expression_refs().is_empty());
    g.set_expression(r, "b", r#"ch("b/a") + out("a/out") + out("a/out")"#)
        .unwrap();
    g.set_expression(r, "a", r#"out("b/out") + ch("a/b")"#)
        .unwrap();
    g.set_expression(inner, "a", r#"out("../r/out")"#).unwrap();
    let refs = g.expression_refs();
    let row = |reader, reader_port: &str, kind, target, target_port: &str| ExprRef {
        reader,
        reader_port: reader_port.to_owned(),
        kind,
        target,
        target_port: target_port.to_owned(),
    };
    assert_eq!(
        refs,
        [
            row(r, "a", RefKind::Input, a, "b"),
            row(r, "a", RefKind::Output, b, "out"),
            row(r, "b", RefKind::Input, b, "a"),
            row(r, "b", RefKind::Output, a, "out"),
            row(inner, "a", RefKind::Output, r, "out"),
        ]
    );
    // referenced_by reports readers of both kinds, once per input.
    assert_eq!(
        g.referenced_by(a),
        [(r, "a".to_owned()), (r, "b".to_owned())]
    );
    assert_eq!(g.referenced_by(r), [(inner, "a".to_owned())]);
}

fn id(n: u128) -> NodeId {
    NodeId::from_uuid(uuid_from(n))
}

fn uuid_from(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(0x0192_0000_0000_7000_8000_0000_0000_0000 | n)
}

#[test]
fn values_are_typed() {
    // An Int output widens into a Float input.
    let mut g = graph();
    let clock = g.add_node(CLOCK, "clock").unwrap();
    let reader = g.add_node(ADD, "reader").unwrap();
    g.set_expression(reader, "a", r#"out("clock/frame")"#)
        .unwrap();
    let mut c = Cooker::new();
    c.set_clock(CookContext {
        frame: 7,
        time: 0.0,
        dt: 0.0,
    });
    assert_eq!(c.cook_output(&g, reader, "out"), Ok(Value::Float(7.0)));
    let _ = clock;
}
