// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `out()` expressions (M0.11) in `room.kdl`: they are plain expression
//! source text, so documents need nothing new; these tests pin that down.

mod common;

use std::path::Path;

use common::{id, registry};
use livspeil_doc::{DocError, Document, canonicalize, format, parse};
use livspeil_graph::{Graph, GraphError, NodeId, RefKind, SUBNET_TYPE};
use livspeil_node_api::Value;
use livspeil_params::expr::quote;

#[test]
fn out_expressions_round_trip_through_text_and_graphs() {
    let mut graph = Graph::new(registry());
    let sun = graph
        .add_node_with_id(id(1), "core.time.timeofday@1", "sun \"☀\"")
        .unwrap();
    let lamp = graph
        .add_node_with_id(id(2), "core.math.lerp@1", "lamp")
        .unwrap();
    let s = graph
        .add_node_in(None, id(3), SUBNET_TYPE, "rig", None)
        .unwrap();
    let inner = graph
        .add_node_in(Some(s), id(4), "core.math.add@1", "inner", None)
        .unwrap();
    graph.set_input(sun, "offset", Value::Float(6.0)).unwrap();
    let hour = quote("sun \"☀\"/hour");
    graph
        .set_expression(lamp, "t", &format!("smoothstep(5, 8, out({hour}))"))
        .unwrap();
    graph
        .set_expression(lamp, "a", r#"out("rig/inner/out") + ch("rig/inner/b")"#)
        .unwrap();
    graph
        .set_expression(
            inner,
            "a",
            r#"out("../lamp/out") * 0 + out("/sun \"☀\"/phase")"#,
        )
        .unwrap_err(); // lamp already reads inner: a cycle.
    graph
        .set_expression(inner, "a", r#"max(0, out("/sun \"☀\"/phase") / 90)"#)
        .unwrap();

    let doc = Document::from_graph(&graph, "out").unwrap();
    let text = format(&doc).unwrap();
    assert!(
        text.contains(r#"expr="smoothstep(5, 8, out(\"sun \\\"☀\\\"/hour\"))""#),
        "{text}"
    );
    let back = parse(&text).unwrap();
    assert_eq!(back, doc);
    assert_eq!(format(&back).unwrap(), text);
    assert_eq!(canonicalize(&text).unwrap(), text);
    let rebuilt = back.to_graph(registry()).unwrap();
    assert_eq!(rebuilt.snapshot(), graph.snapshot());
    assert_eq!(rebuilt.expression_refs(), graph.expression_refs());
    assert!(
        rebuilt
            .expression_refs()
            .iter()
            .any(|r| r.kind == RefKind::Output && r.target == inner && r.reader == lamp)
    );
}

const HEADER: &str = "room \"r\" format=1 {\n  network \"/\" {\n";

/// Two `core.math.add@1` nodes `a` and `b` whose `a` inputs carry `ra`
/// and `rb`.
fn adds(ra: &str, rb: &str) -> String {
    format!(
        "{HEADER}    node \"a\" type=\"core.math.add@1\" id=\"01920000-0000-7000-8000-000000000001\" {{\n      input \"a\" {ra}\n    }}\n    node \"b\" type=\"core.math.add@1\" id=\"01920000-0000-7000-8000-000000000002\" {{\n      input \"a\" {rb}\n    }}\n  }}\n}}\n"
    )
}

#[test]
fn out_forward_references_and_cycles_in_documents() {
    // `a` reads `b`, which comes later.
    let text = adds(r#"expr="out(\"b/out\") + 1""#, "2.0");
    let graph = parse(&text).unwrap().to_graph(registry()).unwrap();
    let a = graph.find("a").unwrap();
    assert_eq!(
        graph.expression(a, "a").unwrap(),
        Some(r#"out("b/out") + 1"#)
    );

    // A cycle closed by out() is reported at the second expression.
    let text = adds(r#"expr="out(\"b/out\")""#, r#"expr="ch(\"a/b\")""#);
    let e = parse(&text)
        .unwrap()
        .to_graph(registry())
        .expect_err("a cycle");
    assert!(
        matches!(
            &e,
            DocError::Graph {
                source: GraphError::WouldCycle,
                ..
            }
        ),
        "{e}"
    );
    assert!(e.to_string().contains("node 'b' input 'a' expr"), "{e}");

    // Unknown output ports are expression errors.
    let text = adds(r#"expr="out(\"b/nope\")""#, "2.0");
    let e = parse(&text)
        .unwrap()
        .to_graph(registry())
        .expect_err("unknown output");
    assert!(
        e.to_string()
            .contains("expression error: unknown output path \"b/nope\""),
        "{e}"
    );
}

#[test]
fn the_out_demo_example_round_trips() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/out-demo.room/room.kdl");
    let text = std::fs::read_to_string(path).unwrap();
    assert_eq!(canonicalize(&text).unwrap(), text);
    let doc = parse(&text).unwrap();
    let graph = doc.to_graph(registry()).unwrap();
    let back = Document::from_graph(&graph, doc.name.clone()).unwrap();
    assert_eq!(
        back.to_graph(registry()).unwrap().snapshot(),
        graph.snapshot()
    );
    let refs = graph.expression_refs();
    let expr_level = graph.find("expr_level").unwrap();
    let time_of_day: NodeId = graph.find("time_of_day").unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].reader, expr_level);
    assert_eq!(refs[0].target, time_of_day);
    assert_eq!(refs[0].kind, RefKind::Output);
    assert_eq!(refs[0].target_port, "hour");
}
