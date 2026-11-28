// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `Graph::check_link` says exactly what linking would, for every pair of
//! ports of graphs built at random (concrete, polymorphic, subnet and event
//! ports, free and occupied inputs).

use std::sync::Arc;

use livspeil_graph::{Graph, GraphError, NodeId};
use livspeil_node_api::NodeRegistry;
use livspeil_nodes_core::register_all;
use uuid::Uuid;

const TYPES: [&str; 12] = [
    "core.math.add@1",
    "core.math.mul@1",
    "core.logic.compare@1",
    "core.logic.not@1",
    "core.logic.select@2",
    "core.math.lerp@2",
    "core.curve.ramp@2",
    "core.color.mix@1",
    "core.text.format@1",
    "core.event.every@1",
    "core.event.counter@1",
    "core.subnet@1",
];

fn registry() -> Arc<NodeRegistry> {
    let mut r = NodeRegistry::new();
    register_all(&mut r).unwrap();
    Arc::new(r)
}

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        usize::try_from((z ^ (z >> 31)) % n as u64).unwrap()
    }
}

fn id(n: usize) -> NodeId {
    NodeId::from_uuid(Uuid::from_u128(n as u128 + 1))
}

type PortId = (NodeId, String);

fn ports(g: &Graph) -> (Vec<PortId>, Vec<PortId>) {
    let (mut inputs, mut outputs) = (Vec::new(), Vec::new());
    for node in g.node_ids() {
        let spec = g.node_spec(node).unwrap();
        inputs.extend(spec.inputs.iter().map(|p| (node, p.name.to_owned())));
        outputs.extend(spec.outputs.iter().map(|p| (node, p.name.to_owned())));
    }
    (inputs, outputs)
}

/// The graph of `seed`: the same every time it is called.
fn build(registry: &Arc<NodeRegistry>, seed: u64) -> Graph {
    let mut rng = Rng(seed);
    let mut g = Graph::new(Arc::clone(registry));
    for n in 0..7 {
        let kind = TYPES[rng.below(TYPES.len())];
        let parent = (n > 0 && rng.below(5) == 0)
            .then(|| g.node_ids().find(|&p| g.is_subnet(p)))
            .flatten();
        g.add_node_in(parent, id(n), kind, &format!("n{n}"), None)
            .unwrap();
    }
    let (inputs, outputs) = ports(&g);
    if inputs.is_empty() || outputs.is_empty() {
        return g;
    }
    for _ in 0..12 {
        let (to, to_port) = &inputs[rng.below(inputs.len())];
        let (from, from_port) = &outputs[rng.below(outputs.len())];
        let _ = g.link(*from, from_port, *to, to_port);
    }
    g
}

#[test]
fn check_link_agrees_with_linking() {
    let registry = registry();
    let (mut allowed, mut refused, mut replaced) = (0, 0, 0);
    for seed in 0..300 {
        let g = build(&registry, seed);
        let before = g.snapshot();
        let (inputs, outputs) = ports(&g);
        for (to, to_port) in &inputs {
            for (from, from_port) in &outputs {
                let checked = g.check_link(*from, from_port, *to, to_port);
                let mut real = build(&registry, seed);
                let occupied = real.unlink(*to, to_port).unwrap_or(false);
                let done = real.link(*from, from_port, *to, to_port);
                assert_eq!(
                    checked.is_ok(),
                    done.is_ok(),
                    "seed {seed}: {from}.{from_port} -> {to}.{to_port} \
                     (occupied {occupied}): {checked:?} vs {done:?}"
                );
                assert!(!matches!(checked, Err(GraphError::InputOccupied { .. })));
                allowed += usize::from(done.is_ok());
                refused += usize::from(done.is_err());
                replaced += usize::from(occupied && done.is_ok());
            }
        }
        assert_eq!(g.snapshot(), before, "checking changes nothing");
    }
    assert!(
        allowed > 500 && refused > 500 && replaced > 100,
        "{allowed} {refused} {replaced}"
    );
}
