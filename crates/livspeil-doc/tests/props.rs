// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Property: graph -> document -> text -> document -> graph is lossless,
//! including nested subnets, interface ports and Charm metadata.

mod common;

use common::{id, registry};
use livspeil_doc::{Document, canonicalize, format, parse};
use livspeil_graph::{CharmMeta, Graph, NodeId, SUBNET_TYPE};
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
    /// Up to two subnets: (nested in the first, name, id, Charm name).
    subnets: Vec<(bool, String, u128, Option<String>)>,
    /// (type, name, id, network: 0 root, 1 and 2 the subnets, port type)
    nodes: Vec<(usize, String, u128, usize, PortType)>,
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

const TYPES: [&str; 8] = [
    "test.all@1",
    "core.math.add@1",
    "core.time.timeofday@1",
    "core.color.mix@1",
    "core.logic.select@1",
    "core.math.lerp@1",
    "core.subnet.input@1",
    "core.output@1",
];

fn script() -> impl Strategy<Value = Script> {
    let name = prop_oneof![
        "[a-z]{1,6}",
        "[a-z]{1,3}\\.[a-z]{1,3}",
        any::<String>().prop_filter("non-empty", |s| !s.is_empty()),
    ];
    let port_type = prop::sample::select(
        common::ALL_TYPES
            .iter()
            .map(|(_, t)| *t)
            .collect::<Vec<_>>(),
    );
    let nodes = prop::collection::vec(
        (0..TYPES.len(), name, any::<u128>(), 0..3usize, port_type),
        0..10,
    );
    let subnets = prop::collection::vec(
        (
            any::<bool>(),
            "[a-z]{1,6}",
            any::<u128>(),
            prop::option::of("[A-Za-z ]{1,8}"),
        ),
        1..3,
    );
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
    (subnets, nodes, inputs, links, exprs, anims).prop_map(
        |(subnets, nodes, inputs, links, exprs, anims)| Script {
            subnets,
            nodes,
            inputs,
            links,
            exprs,
            anims,
        },
    )
}

/// Apply a script, ignoring edits the graph rejects (wrong types, cycles,
/// duplicate names): what remains is a valid random graph.
fn build(script: &Script) -> Graph {
    let reg = registry();
    let mut graph = Graph::new(reg);
    let mut ids = Vec::new();
    let mut nets: Vec<Option<NodeId>> = vec![None];
    for (nested, name, raw, charm) in &script.subnets {
        let parent = if *nested {
            nets.get(1).copied().flatten()
        } else {
            None
        };
        if let Ok(s) = graph.add_node_in(parent, id(*raw >> 8), SUBNET_TYPE, name, None) {
            if let Some(charm) = charm {
                let mut meta = CharmMeta::new(charm.clone());
                meta.version = Some("1.0".to_owned());
                meta.min_tier = (raw % 3) as u8;
                meta.capabilities = vec!["time".to_owned()];
                graph.set_charm(s, Some(meta)).unwrap();
            }
            nets.push(Some(s));
        }
    }
    for (t, name, raw, net, ty) in &script.nodes {
        let parent = nets.get(*net).copied().flatten();
        let port_type = (*t >= 6).then_some(*ty);
        if let Ok(n) = graph.add_node_in(parent, id(*raw >> 8), TYPES[*t], name, port_type) {
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
    // Wire the interface: try to feed every boundary node and read it on
    // both sides of its subnet (the first node whose port fits).
    let boundary: Vec<NodeId> = ids
        .iter()
        .copied()
        .filter(|&b| graph.port_type(b).is_some())
        .collect();
    for b in boundary {
        for &other in &ids {
            if other == b {
                continue;
            }
            let spec = graph.node_spec(other).unwrap().clone();
            for out in &spec.outputs {
                if graph.link(other, out.name, b, "value").is_ok() {
                    break;
                }
            }
            for inp in &spec.inputs {
                if graph.link(b, "value", other, inp.name).is_ok() {
                    break;
                }
            }
        }
    }
    for (n, i, template, m, j) in &script.exprs {
        let (node, read) = (ids[n % ids.len()], ids[m % ids.len()]);
        let spec = graph.node_spec(node).unwrap().clone();
        let port = spec.inputs[i % spec.inputs.len()].name;
        let read_spec = graph.node_spec(read).unwrap().clone();
        let read_port = read_spec.inputs[j % read_spec.inputs.len()].name;
        // Root readers of root nodes use the plain name (the pre-subnet
        // form); everything else the absolute path.
        let path = if graph.parent(node).is_none() && graph.parent(read).is_none() {
            format!("{}/{read_port}", graph.node_name(read).unwrap())
        } else {
            format!("{}/{read_port}", graph.path(read).unwrap())
        };
        let path = expr_quote(&path);
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
