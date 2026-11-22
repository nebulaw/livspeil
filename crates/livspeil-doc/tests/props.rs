// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Property: graph -> document -> text -> document -> graph is lossless.

mod common;

use common::{id, registry};
use livspeil_doc::{Document, canonicalize, format, parse};
use livspeil_graph::Graph;
use livspeil_node_api::{PortType, Value};
use proptest::prelude::*;

fn finite() -> impl Strategy<Value = f64> {
    prop_oneof![
        any::<f64>().prop_filter("finite", |v| v.is_finite()),
        -1e3..1e3f64,
        Just(0.0),
        Just(-0.0),
    ]
}

fn value(ty: PortType) -> BoxedStrategy<Value> {
    match ty {
        PortType::Float => finite().prop_map(Value::Float).boxed(),
        PortType::Int => any::<i64>().prop_map(Value::Int).boxed(),
        PortType::Bool => any::<bool>().prop_map(Value::Bool).boxed(),
        PortType::Vec2 => [finite(), finite()].prop_map(Value::Vec2).boxed(),
        PortType::Vec3 => [finite(), finite(), finite()].prop_map(Value::Vec3).boxed(),
        PortType::Vec4 => [finite(), finite(), finite(), finite()]
            .prop_map(Value::Vec4)
            .boxed(),
        PortType::Color => [finite(), finite(), finite(), finite()]
            .prop_map(Value::Color)
            .boxed(),
        _ => any::<String>().prop_map(Value::String).boxed(),
    }
}

/// A random edit script: node types, names, input settings, link attempts.
#[derive(Debug, Clone)]
struct Script {
    nodes: Vec<(usize, String, u128)>,
    /// (node index, input index, value seed per type)
    inputs: Vec<(usize, usize, Value)>,
    /// (from node, output index, to node, input index)
    links: Vec<(usize, usize, usize, usize)>,
}

const TYPES: [&str; 6] = [
    "test.all@1",
    "core.math.add@1",
    "core.time.timeofday@1",
    "core.color.mix@1",
    "core.logic.select@1",
    "core.math.lerp@1",
];

fn script() -> impl Strategy<Value = Script> {
    let name = prop_oneof![
        "[a-z]{1,6}",
        "[a-z]{1,3}\\.[a-z]{1,3}",
        any::<String>().prop_filter("non-empty", |s| !s.is_empty()),
    ];
    let nodes = prop::collection::vec((0..TYPES.len(), name, any::<u128>()), 0..6);
    let inputs = prop::collection::vec(
        (
            0..6usize,
            0..8usize,
            prop::sample::select(common::ALL_TYPES.to_vec()),
        )
            .prop_flat_map(|(n, i, (_, ty))| (Just(n), Just(i), value(ty))),
        0..12,
    );
    let links = prop::collection::vec((0..6usize, 0..8usize, 0..6usize, 0..8usize), 0..10);
    (nodes, inputs, links).prop_map(|(nodes, inputs, links)| Script {
        nodes,
        inputs,
        links,
    })
}

/// Apply a script, ignoring edits the graph rejects (wrong types, cycles,
/// duplicate names): what remains is a valid random graph.
fn build(script: &Script) -> Graph {
    let reg = registry();
    let mut graph = Graph::new(reg);
    let mut ids = Vec::new();
    for (t, name, raw) in &script.nodes {
        if let Ok(n) = graph.add_node_with_id(id(*raw >> 8), TYPES[*t], name) {
            ids.push(n);
        }
    }
    if ids.is_empty() {
        return graph;
    }
    for (n, i, value) in &script.inputs {
        let node = ids[n % ids.len()];
        let spec = graph.node_spec(node).unwrap().clone();
        let port = spec.inputs[i % spec.inputs.len()].name;
        let _ = graph.set_input(node, port, value.clone());
    }
    for (a, o, b, i) in &script.links {
        let (from, to) = (ids[a % ids.len()], ids[b % ids.len()]);
        let from_spec = graph.node_spec(from).unwrap().clone();
        let to_spec = graph.node_spec(to).unwrap().clone();
        let out = from_spec.outputs[o % from_spec.outputs.len()].name;
        let inp = to_spec.inputs[i % to_spec.inputs.len()].name;
        let _ = graph.link(from, out, to, inp);
    }
    graph
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn graph_doc_text_round_trip(script in script(), name in any::<String>()) {
        let graph = build(&script);
        let doc = Document::from_graph(&graph, name).unwrap();
        let text = format(&doc).unwrap();
        let parsed = parse(&text).unwrap();
        prop_assert_eq!(&parsed, &doc);
        prop_assert_eq!(format(&parsed).unwrap(), text.clone());
        prop_assert_eq!(canonicalize(&text).unwrap(), text);
        let rebuilt = parsed.to_graph(registry()).unwrap();
        prop_assert_eq!(rebuilt.snapshot(), graph.snapshot());
    }
}
