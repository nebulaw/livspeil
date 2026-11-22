// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Golden files for the canonical output of hand-built graphs.

mod common;

use std::path::{Path, PathBuf};

use common::{id, registry};
use livspeil_doc::{Document, Tier, format, parse};
use livspeil_graph::Graph;
use livspeil_node_api::Value;
use livspeil_testkit::assert_golden;

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn sun_and_lamp() -> Graph {
    let mut g = Graph::new(registry());
    let sun = g
        .add_node_with_id(id(1), "core.time.timeofday@1", "sun")
        .unwrap();
    let lamp = g
        .add_node_with_id(id(2), "core.color.mix@1", "lamp")
        .unwrap();
    g.set_input(sun, "offset", Value::Float(0.0)).unwrap();
    g.set_input(lamp, "a", Value::Color([0.8, 0.12, 60.0, 1.0]))
        .unwrap();
    g.set_input(lamp, "b", Value::Color([0.35, 0.08, 260.0, 1.0]))
        .unwrap();
    g.link(sun, "phase", lamp, "t").unwrap();
    g
}

fn check(name: &str, graph: &Graph, doc: &Document) {
    let text = format(doc).unwrap();
    assert_golden(&golden_dir(), name, &text);
    let back = parse(&text).unwrap();
    assert_eq!(&back, doc);
    assert_eq!(
        back.to_graph(registry()).unwrap().snapshot(),
        graph.snapshot()
    );
}

#[test]
fn sun_and_lamp_document() {
    let graph = sun_and_lamp();
    let mut doc = Document::from_graph(&graph, "Sun and Lamp").unwrap();
    doc.min_tier = Tier(1);
    doc.meta.positions.insert("sun".into(), [120.0, 40.0]);
    doc.meta.positions.insert("lamp".into(), [360.0, 40.0]);
    check("sun_and_lamp", &graph, &doc);
}

#[test]
fn every_value_type() {
    let mut graph = Graph::new(registry());
    let n = graph.add_node_with_id(id(7), "test.all@1", "all").unwrap();
    for (port, value) in [
        ("f", Value::Float(-1.5e-7)),
        ("i", Value::Int(-42)),
        ("b", Value::Bool(true)),
        ("v2", Value::Vec2([1.0, 2.0])),
        ("v3", Value::Vec3([0.1, 0.2, 0.3])),
        ("v4", Value::Vec4([0.0, -0.0, 1e21, 4.0])),
        ("c", Value::Color([0.5, 0.0, 0.0, 0.25])),
        ("s", Value::String("tab\there \"quoted\" \\ é\n".into())),
    ] {
        graph.set_input(n, port, value).unwrap();
    }
    let doc = Document::from_graph(&graph, "All values").unwrap();
    check("every_value_type", &graph, &doc);
}

#[test]
fn empty_document() {
    let graph = Graph::new(registry());
    let doc = Document::from_graph(&graph, "").unwrap();
    check("empty", &graph, &doc);
}
