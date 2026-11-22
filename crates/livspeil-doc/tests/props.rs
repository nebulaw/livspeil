// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Property: graph -> document -> text -> document -> graph is lossless.

mod common;

use common::{id, registry};
use livspeil_doc::{Document, canonicalize, format, parse};
use livspeil_graph::Graph;
use livspeil_node_api::{PortType, Value};
use livspeil_params::expr::quote as expr_quote;
use livspeil_params::{AnimChannel, Extrapolation, Interpolation, Keyframe, Tangent};
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
    /// (node index, input index, template, read node, read input index)
    exprs: Vec<(usize, usize, usize, usize, usize)>,
    /// (node index, channel); applied to the first input of the channel's
    /// type, if the node has one.
    anims: Vec<(usize, AnimChannel)>,
}

/// Expression templates; `{p}` is a quoted `ch()` path.
const EXPRS: [&str; 6] = [
    "ch({p})",
    "ch({p}) * 0.5",
    "sin($T) + 1.0",
    "ch({p}) + ch({p}) * $F",
    "\"tëxt \\\"q\\\" ✓\"",
    "2",
];

fn interpolation() -> impl Strategy<Value = Interpolation> {
    prop_oneof![
        Just(Interpolation::Step),
        Just(Interpolation::Linear),
        Just(Interpolation::Bezier),
        (1e-3..100.0f64, 0.0..3.0f64)
            .prop_map(|(frequency, damping)| Interpolation::Spring { frequency, damping }),
    ]
}

fn extrapolation() -> impl Strategy<Value = Extrapolation> {
    prop_oneof![
        Just(Extrapolation::Hold),
        Just(Extrapolation::Loop),
        Just(Extrapolation::PingPong),
    ]
}

fn tangent(ty: PortType) -> BoxedStrategy<Tangent> {
    prop_oneof![
        Just(Tangent::Auto),
        Just(Tangent::Flat),
        (value(ty), value(ty))
            .prop_map(|(incoming, outgoing)| Tangent::Explicit { incoming, outgoing }),
    ]
    .boxed()
}

fn channel() -> impl Strategy<Value = AnimChannel> {
    let ty = prop::sample::select(vec![
        PortType::Float,
        PortType::Vec2,
        PortType::Vec3,
        PortType::Vec4,
        PortType::Color,
    ]);
    ty.prop_flat_map(|ty| {
        let key = (0.001..10.0f64, value(ty), interpolation(), tangent(ty));
        (
            prop::collection::vec(key, 1..5),
            finite(),
            extrapolation(),
            extrapolation(),
            finite(),
        )
    })
    .prop_map(|(keys, start, pre, post, offset)| {
        let mut time = start;
        let keys = keys
            .into_iter()
            .map(|(step, value, interpolation, tangent)| {
                time += step;
                Keyframe {
                    time,
                    value,
                    interpolation,
                    tangent,
                }
            })
            .collect();
        AnimChannel::new(keys)
            .and_then(|c| c.with_extrapolation(pre, post).with_offset(offset))
            .unwrap_or_else(|_| AnimChannel::constant(0.0).unwrap())
    })
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
    let exprs = prop::collection::vec(
        (0..6usize, 0..8usize, 0..EXPRS.len(), 0..6usize, 0..8usize),
        0..8,
    );
    let anims = prop::collection::vec((0..6usize, channel()), 0..4);
    (nodes, inputs, links, exprs, anims).prop_map(|(nodes, inputs, links, exprs, anims)| Script {
        nodes,
        inputs,
        links,
        exprs,
        anims,
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
    for (n, i, template, m, j) in &script.exprs {
        let (node, read) = (ids[n % ids.len()], ids[m % ids.len()]);
        let spec = graph.node_spec(node).unwrap().clone();
        let port = spec.inputs[i % spec.inputs.len()].name;
        let read_spec = graph.node_spec(read).unwrap().clone();
        let read_port = read_spec.inputs[j % read_spec.inputs.len()].name;
        let path = expr_quote(&format!("{}/{read_port}", graph.node_name(read).unwrap()));
        let source = EXPRS[*template].replace("{p}", &path);
        let _ = graph.set_expression(node, port, &source);
    }
    for (n, channel) in &script.anims {
        let node = ids[n % ids.len()];
        let spec = graph.node_spec(node).unwrap().clone();
        if let Some(port) = spec.inputs.iter().find(|p| p.ty == channel.value_type()) {
            let _ = graph.set_animation(node, port.name, channel.clone());
        }
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
