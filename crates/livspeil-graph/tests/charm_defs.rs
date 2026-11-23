// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared Charm definitions and instances (M0.16), rewired moves, and a
//! randomized differential test (long-lived against fresh cookers) with
//! definition edits.

mod common;

use common::{float, graph};
use livspeil_graph::{
    CHARM_TYPE, CharmDef, CharmMeta, Cooker, Graph, GraphError, INPUT_TYPE, INTERFACE_PORT, NodeId,
    OUTPUT_TYPE, SUBNET_TYPE, instance_node_id,
};
use livspeil_node_api::{CookContext, PortType, Value};
use uuid::Uuid;

const ADD: &str = "core.math.add@1";
const MUL: &str = "core.math.mul@1";
const CLOCK: &str = "core.time.clock@1";
const COUNTER: &str = "core.event.counter@1";
const EVERY: &str = "core.event.every@1";

fn id(n: u128) -> NodeId {
    NodeId::from_uuid(Uuid::from_u128(n))
}

fn out(c: &mut Cooker, g: &Graph, node: NodeId, port: &str) -> f64 {
    c.cook_output(g, node, port).unwrap().as_float().unwrap()
}

/// Local ids of the `scale` definition.
const IN: u128 = 0x101;
const GAIN: u128 = 0x102;
const MULT: u128 = 0x103;
const OUT: u128 = 0x104;
const EXTRA: u128 = 0x105;

/// `scale`: `out = in * gain`, with interface inputs `in` (default 1.0)
/// and `gain` (default `gain`), output `out`. `v2` adds 10 through an extra
/// node and renames `mult` to `times`.
fn scale(gain: f64, v2: bool) -> CharmDef {
    let mut g = graph();
    let s = g
        .add_node_in(None, id(0x100), SUBNET_TYPE, "scratch", None)
        .unwrap();
    let input = g
        .add_node_in(Some(s), id(IN), INPUT_TYPE, "in", Some(PortType::Float))
        .unwrap();
    g.set_input(input, INTERFACE_PORT, float(1.0)).unwrap();
    let gain_in = g
        .add_node_in(Some(s), id(GAIN), INPUT_TYPE, "gain", Some(PortType::Float))
        .unwrap();
    g.set_input(gain_in, INTERFACE_PORT, float(gain)).unwrap();
    let name = if v2 { "times" } else { "mult" };
    let mult = g.add_node_in(Some(s), id(MULT), MUL, name, None).unwrap();
    let output = g
        .add_node_in(Some(s), id(OUT), OUTPUT_TYPE, "out", Some(PortType::Float))
        .unwrap();
    g.link(input, INTERFACE_PORT, mult, "a").unwrap();
    g.link(gain_in, INTERFACE_PORT, mult, "b").unwrap();
    if v2 {
        let extra = g
            .add_node_in(Some(s), id(EXTRA), ADD, "extra", None)
            .unwrap();
        g.set_expression(extra, "b", "ch(\"times/b\") * 0 + 10")
            .unwrap();
        g.link(mult, "out", extra, "a").unwrap();
        g.link(extra, "out", output, INTERFACE_PORT).unwrap();
    } else {
        g.link(mult, "out", output, INTERFACE_PORT).unwrap();
    }
    g.set_charm(s, Some(CharmMeta::new("Scale"))).unwrap();
    let version = if v2 { "2.0.0" } else { "1.0.0" };
    g.definition_from_subnet(s, "scale", version).unwrap()
}

fn iface(instance: NodeId, local: u128) -> NodeId {
    instance_node_id(instance, id(local))
}

#[test]
fn derived_ids_are_deterministic_and_distinct() {
    let a = instance_node_id(id(1), id(2));
    assert_eq!(a, instance_node_id(id(1), id(2)));
    assert_eq!(a.uuid().get_version_num(), 8);
    let mut seen = std::collections::BTreeSet::new();
    for i in 0..64 {
        for l in 0..64 {
            assert!(seen.insert(instance_node_id(id(i), id(l))));
        }
    }
    // Pinned: journals and documents rely on these ids never changing.
    assert_eq!(
        instance_node_id(id(1), id(2)).to_string(),
        instance_node_id(id(1), id(2))
            .uuid()
            .hyphenated()
            .to_string()
    );
}

#[test]
fn instances_share_a_definition() {
    let mut g = graph();
    g.add_definition(scale(2.0, false)).unwrap();
    let x = g.add_node_with_id(id(1), ADD, "x").unwrap();
    g.set_input(x, "a", float(3.0)).unwrap();
    let a = g.instantiate(None, id(10), "a", "scale").unwrap();
    let b = g.instantiate(None, id(20), "b", "scale").unwrap();
    assert!(g.is_instance(a) && g.is_subnet(a));
    assert_eq!(g.node_type(a), Some(CHARM_TYPE));
    assert_eq!(g.instance_def(b), Some("scale"));
    assert_eq!(g.instances_of("scale"), vec![a, b]);
    assert_eq!(g.charm(a).unwrap().version.as_deref(), Some("1.0.0"));
    assert_eq!(g.charm(a).unwrap().name, "Scale");
    // Contents with derived ids, addressable by path.
    assert_eq!(g.find_path("/a/mult"), Some(iface(a, MULT)));
    assert_eq!(g.find_path("/b/mult"), Some(iface(b, MULT)));
    assert_eq!(g.node_count(), 1 + 2 * 5);

    g.link(x, "out", iface(a, IN), INTERFACE_PORT).unwrap();
    // Per-instance parameter.
    g.set_input(iface(b, GAIN), INTERFACE_PORT, float(5.0))
        .unwrap();
    let mut c = Cooker::new();
    assert_eq!(out(&mut c, &g, iface(a, OUT), INTERFACE_PORT), 6.0);
    assert_eq!(out(&mut c, &g, iface(b, OUT), INTERFACE_PORT), 5.0);
    // Clearing an instance input restores the definition's default.
    g.clear_input(iface(b, GAIN), INTERFACE_PORT).unwrap();
    assert_eq!(
        g.input_value(iface(b, GAIN), INTERFACE_PORT).unwrap(),
        Some(&float(2.0))
    );
    assert_eq!(out(&mut c, &g, iface(b, OUT), INTERFACE_PORT), 2.0);

    // Snapshots list definitions and instances.
    let snap = g.snapshot();
    assert_eq!(snap.definitions.len(), 1);
    let node = snap.nodes.iter().find(|n| n.id == a).unwrap();
    assert_eq!(node.def.as_deref(), Some("scale"));
}

#[test]
fn instance_contents_are_read_only() {
    let mut g = graph();
    g.add_definition(scale(2.0, false)).unwrap();
    let x = g.add_node(ADD, "x").unwrap();
    let a = g.instantiate(None, id(10), "a", "scale").unwrap();
    let mult = iface(a, MULT);
    let locked = |r: Result<(), GraphError>| {
        assert!(matches!(r, Err(GraphError::InInstance(_))), "{r:?}");
    };
    locked(g.set_input(mult, "a", float(1.0)));
    locked(g.clear_input(mult, "a"));
    locked(g.set_expression(mult, "a", "1"));
    locked(g.clear_expression(mult, "a"));
    locked(g.clear_animation(mult, "a"));
    locked(g.unlink(mult, "a").map(|_| ()));
    locked(g.rename(mult, "m").map(|_| ()));
    locked(g.remove_node(mult));
    locked(g.move_node(mult, None).map(|_| ()));
    locked(g.move_node(x, Some(a)).map(|_| ()));
    locked(g.promote(mult, "a", NodeId::new(), "p").map(|_| ()));
    locked(g.unpromote(mult, "a").map(|_| ()));
    locked(g.set_charm(a, None).map(|_| ()));
    locked(
        g.add_node_in(Some(a), NodeId::new(), ADD, "y", None)
            .map(|_| ()),
    );
    locked(g.link(x, "out", mult, "a"));
    locked(g.rename(iface(a, IN), "inp").map(|_| ()));
    assert!(matches!(
        g.add_node_in(None, NodeId::new(), CHARM_TYPE, "c", None),
        Err(GraphError::NotAnInstance(_))
    ));
    // The instance itself and its interface inputs are editable.
    g.rename(a, "renamed").unwrap();
    g.set_expression(iface(a, IN), INTERFACE_PORT, "ch(\"../x/a\") + 1")
        .unwrap();
    g.link(x, "out", iface(a, GAIN), INTERFACE_PORT).unwrap();
    // Reading inside is fine.
    let r = g.add_node(ADD, "r").unwrap();
    g.set_expression(r, "b", "ch(\"renamed/mult/a\")").unwrap();
    assert!(matches!(
        g.remove_node(a),
        Err(GraphError::Referenced { .. })
    ));
    g.clear_expression(r, "b").unwrap();
    g.remove_node(a).unwrap();
    assert_eq!(g.node_count(), 2);
    assert!(g.instances_of("scale").is_empty());
    g.remove_definition("scale").unwrap();
}

#[test]
fn updating_a_definition_updates_every_instance() {
    let mut g = graph();
    g.add_definition(scale(2.0, false)).unwrap();
    let x = g.add_node_with_id(id(1), ADD, "x").unwrap();
    g.set_input(x, "a", float(3.0)).unwrap();
    let a = g.instantiate(None, id(10), "a", "scale").unwrap();
    let b = g.instantiate(None, id(20), "b", "scale").unwrap();
    g.link(x, "out", iface(a, IN), INTERFACE_PORT).unwrap();
    g.link(x, "out", iface(b, IN), INTERFACE_PORT).unwrap();
    g.set_input(iface(b, GAIN), INTERFACE_PORT, float(5.0))
        .unwrap();
    // An outside reader of an inner node.
    let r = g.add_node_with_id(id(2), ADD, "r").unwrap();
    g.set_expression(r, "a", "ch(\"a/mult/b\")").unwrap();
    let sink = g.add_node_with_id(id(3), ADD, "sink").unwrap();
    g.link(iface(a, OUT), INTERFACE_PORT, sink, "a").unwrap();

    let mut c = Cooker::new();
    assert_eq!(out(&mut c, &g, sink, "out"), 6.0);
    assert_eq!(out(&mut c, &g, iface(b, OUT), INTERFACE_PORT), 15.0);
    assert_eq!(out(&mut c, &g, r, "out"), 2.0);

    let report = g.update_definition(scale(4.0, true)).unwrap();
    assert_eq!(report.instances, vec![a, b]);
    assert_eq!(
        report.rewritten,
        vec![(r, "a".to_owned(), "ch(\"a/mult/b\")".to_owned())]
    );
    assert!(report.unlinked.is_empty());
    assert_eq!(g.expression(r, "a").unwrap(), Some("ch(\"a/times/b\")"));
    assert_eq!(g.charm(a).unwrap().version.as_deref(), Some("2.0.0"));
    // a follows the new default gain 4; b keeps its override 5; both add 10.
    assert_eq!(out(&mut c, &g, sink, "out"), 22.0);
    assert_eq!(out(&mut c, &g, iface(b, OUT), INTERFACE_PORT), 25.0);
    assert_eq!(out(&mut c, &g, r, "out"), 4.0);
    assert_eq!(g.find_path("/a/times"), Some(iface(a, MULT)));
    // Links into the interface are kept.
    assert_eq!(
        g.link_source(iface(a, IN), INTERFACE_PORT).unwrap(),
        Some((x, "out"))
    );
    let mut fresh = Cooker::new();
    assert_eq!(out(&mut fresh, &g, sink, "out"), 22.0);

    // And back: the instances are exactly as an instance of v1 would be.
    g.update_definition(scale(2.0, false)).unwrap();
    assert_eq!(out(&mut c, &g, sink, "out"), 6.0);
    assert_eq!(out(&mut c, &g, iface(b, OUT), INTERFACE_PORT), 15.0);
    assert_eq!(g.expression(r, "a").unwrap(), Some("ch(\"a/mult/b\")"));
    assert_eq!(g.node_count(), 3 + 2 * 5);
}

#[test]
fn failed_updates_change_nothing() {
    let mut g = graph();
    g.add_definition(scale(2.0, false)).unwrap();
    let a = g.instantiate(None, id(10), "a", "scale").unwrap();
    let r = g.add_node_with_id(id(2), ADD, "r").unwrap();
    // Reads a node v3 removes.
    g.set_expression(r, "a", "ch(\"a/mult/b\")").unwrap();
    let before = g.snapshot();
    let mut v3 = scale(2.0, false);
    v3.nodes.retain(|n| n.id != id(MULT));
    v3.links.retain(|l| l.from != id(MULT) && l.to != id(MULT));
    v3.version = "3.0.0".to_owned();
    assert!(matches!(
        g.update_definition(v3),
        Err(GraphError::Referenced { node, .. }) if node == iface(a, MULT)
    ));
    assert_eq!(g.snapshot(), before);

    // A cycle with the links around an instance whose input does not
    // reach its output yet.
    g.clear_expression(r, "a").unwrap();
    let mut split = scale(2.0, false);
    split.id = "split".to_owned();
    split.links.retain(|l| l.from != id(IN));
    g.add_definition(split.clone()).unwrap();
    let b = g.instantiate(None, id(20), "b", "split").unwrap();
    g.link(iface(b, OUT), INTERFACE_PORT, r, "a").unwrap();
    g.link(r, "out", iface(b, IN), INTERFACE_PORT).unwrap();
    let before = g.snapshot();
    let mut short = split;
    short.links.retain(|l| l.to != id(OUT));
    short.links.push(livspeil_graph::NamedLink {
        from: id(IN),
        from_port: INTERFACE_PORT.to_owned(),
        to: id(OUT),
        to_port: INTERFACE_PORT.to_owned(),
    });
    assert!(matches!(
        g.update_definition(short),
        Err(GraphError::WouldCycle)
    ));
    assert_eq!(g.snapshot(), before);
    assert!(matches!(
        g.update_definition(CharmDef::new("nope", "1", CharmMeta::new("N"))),
        Err(GraphError::UnknownDefinition(_))
    ));
}

#[test]
fn removing_an_interface_port_drops_its_links() {
    let mut g = graph();
    g.add_definition(scale(2.0, false)).unwrap();
    let a = g.instantiate(None, id(10), "a", "scale").unwrap();
    let x = g.add_node_with_id(id(1), ADD, "x").unwrap();
    g.link(x, "out", iface(a, GAIN), INTERFACE_PORT).unwrap();
    let mut v = scale(2.0, false);
    v.nodes.retain(|n| n.id != id(GAIN));
    v.links.retain(|l| l.from != id(GAIN));
    let report = g.update_definition(v).unwrap();
    assert_eq!(report.unlinked.len(), 1);
    assert_eq!(report.unlinked[0].from, x);
    assert!(!g.contains(iface(a, GAIN)));
    assert!(g.outgoing_links(x).is_empty());
}

#[test]
fn definitions_are_validated() {
    let mut g = graph();
    let mut d = scale(2.0, false);
    d.id = "bad id".to_owned();
    assert!(matches!(
        g.add_definition(d),
        Err(GraphError::InvalidDefinition { .. })
    ));
    let mut d = scale(2.0, false);
    d.version = "1@2".to_owned();
    assert!(g.add_definition(d).is_err());
    let mut d = scale(2.0, false);
    let mult = d.nodes.iter_mut().find(|n| n.id == id(MULT)).unwrap();
    mult.expressions
        .push(("b".to_owned(), "ch(\"../x/a\")".to_owned()));
    assert!(matches!(
        g.add_definition(d),
        Err(GraphError::InvalidDefinition { reason, .. }) if reason.contains("stay inside")
    ));
    let mut d = scale(2.0, false);
    let mult = d.nodes.iter_mut().find(|n| n.id == id(MULT)).unwrap();
    mult.expressions
        .push(("b".to_owned(), "ch(\"/a/in/value\")".to_owned()));
    assert!(g.add_definition(d).is_err());
    let mut d = scale(2.0, false);
    let mult = d.nodes.iter_mut().find(|n| n.id == id(MULT)).unwrap();
    mult.type_id = "core.nope@1".to_owned();
    assert!(matches!(
        g.add_definition(d),
        Err(GraphError::InvalidDefinition { reason, .. }) if reason.contains("unknown node type")
    ));
    g.add_definition(scale(2.0, false)).unwrap();
    assert!(matches!(
        g.add_definition(scale(2.0, false)),
        Err(GraphError::DuplicateDefinition(_))
    ));
    g.instantiate(None, id(10), "a", "scale").unwrap();
    assert!(matches!(
        g.remove_definition("scale"),
        Err(GraphError::DefinitionInUse { .. })
    ));
    assert!(matches!(
        g.instantiate(None, id(10), "b", "scale"),
        Err(GraphError::DuplicateId(_))
    ));
    assert!(matches!(
        g.instantiate(None, id(11), "a", "scale"),
        Err(GraphError::DuplicateName(_))
    ));
    assert!(matches!(
        g.instantiate(None, id(11), "b", "nope"),
        Err(GraphError::UnknownDefinition(_))
    ));
    // Instances cannot be captured as part of a definition yet.
    let s = g.add_node_in(None, id(30), SUBNET_TYPE, "s", None).unwrap();
    g.move_node(id(10), Some(s)).unwrap();
    assert!(matches!(
        g.definition_from_subnet(s, "outer", "1"),
        Err(GraphError::InvalidDefinition { .. })
    ));
}

#[test]
fn detach_and_attach() {
    let mut g = graph();
    g.add_definition(scale(2.0, false)).unwrap();
    let a = g.instantiate(None, id(10), "a", "scale").unwrap();
    g.set_input(iface(a, GAIN), INTERFACE_PORT, float(7.0))
        .unwrap();
    let before = g.snapshot();
    assert_eq!(g.detach(a).unwrap(), "scale");
    assert_eq!(g.node_type(a), Some(SUBNET_TYPE));
    assert!(!g.is_instance(a));
    assert_eq!(g.charm(a).unwrap().version.as_deref(), Some("1.0.0"));
    g.attach(a, "scale").unwrap();
    assert_eq!(g.snapshot(), before);
    g.detach(a).unwrap();
    g.set_input(iface(a, MULT), "a", float(1.0)).unwrap();
    assert!(matches!(
        g.attach(a, "scale"),
        Err(GraphError::InvalidDefinition { .. })
    ));
    assert!(matches!(g.detach(a), Err(GraphError::NotAnInstance(_))));
}

#[test]
fn make_definition_keeps_the_data_flow() {
    let mut g = graph();
    let x = g.add_node(ADD, "x").unwrap();
    g.set_input(x, "a", float(3.0)).unwrap();
    let s = g.add_node_in(None, id(10), SUBNET_TYPE, "s", None).unwrap();
    let input = g
        .add_node_in(Some(s), id(IN), INPUT_TYPE, "in", Some(PortType::Float))
        .unwrap();
    let k = g
        .add_node_in(Some(s), id(GAIN), INPUT_TYPE, "k", Some(PortType::Float))
        .unwrap();
    g.set_expression(k, INTERFACE_PORT, "ch(\"../x/a\") * 2")
        .unwrap();
    let mult = g.add_node_in(Some(s), id(MULT), MUL, "mult", None).unwrap();
    let output = g
        .add_node_in(Some(s), id(OUT), OUTPUT_TYPE, "out", Some(PortType::Float))
        .unwrap();
    g.link(x, "out", input, INTERFACE_PORT).unwrap();
    g.link(input, INTERFACE_PORT, mult, "a").unwrap();
    g.link(k, INTERFACE_PORT, mult, "b").unwrap();
    g.link(mult, "out", output, INTERFACE_PORT).unwrap();
    let y = g.add_node(ADD, "y").unwrap();
    g.link(output, INTERFACE_PORT, y, "a").unwrap();
    g.set_expression(y, "b", "ch(\"s/mult/b\")").unwrap();
    let mut c = Cooker::new();
    let before = out(&mut c, &g, y, "out");
    assert_eq!(before, 3.0 * 6.0 + 6.0);

    g.make_definition(s, "times", "0.1").unwrap();
    assert!(g.is_instance(s));
    let def = g.definition("times").unwrap();
    assert_eq!(def.meta.name, "s");
    // The outside-reading expression became an override, not a default.
    assert!(def.node(id(GAIN)).unwrap().expressions.is_empty());
    assert_eq!(
        g.expression(iface(s, GAIN), INTERFACE_PORT).unwrap(),
        Some("ch(\"../x/a\") * 2")
    );
    assert!(!g.contains(mult));
    assert_eq!(out(&mut c, &g, y, "out"), before);
    let mut fresh = Cooker::new();
    assert_eq!(out(&mut fresh, &g, y, "out"), before);
    // A second instance of the new definition.
    let t = g.instantiate(None, id(20), "t", "times").unwrap();
    g.set_input(iface(t, IN), INTERFACE_PORT, float(2.0))
        .unwrap();
    g.set_input(iface(t, GAIN), INTERFACE_PORT, float(4.0))
        .unwrap();
    assert_eq!(out(&mut c, &g, iface(t, OUT), INTERFACE_PORT), 8.0);

    // Not a plain subnet.
    assert!(matches!(
        g.make_definition(s, "again", "1"),
        Err(GraphError::NotASubnet(_))
    ));
}

#[test]
fn rewired_moves_keep_the_data_flow() {
    let mut g = graph();
    let x = g.add_node(ADD, "x").unwrap();
    g.set_input(x, "a", float(2.0)).unwrap();
    let y = g.add_node(MUL, "y").unwrap();
    g.set_input(y, "b", float(3.0)).unwrap();
    let z = g.add_node(ADD, "z").unwrap();
    g.link(x, "out", y, "a").unwrap();
    g.link(y, "out", z, "a").unwrap();
    g.link(y, "out", z, "b").unwrap();
    let s = g
        .add_node_in(None, NodeId::new(), SUBNET_TYPE, "s", None)
        .unwrap();
    let t = g
        .add_node_in(Some(s), NodeId::new(), SUBNET_TYPE, "t", None)
        .unwrap();
    let mut c = Cooker::new();
    assert_eq!(out(&mut c, &g, z, "out"), 12.0);

    // Refused by default.
    let before = g.snapshot();
    assert!(matches!(
        g.move_node(y, Some(t)),
        Err(GraphError::CrossNetwork { .. })
    ));
    assert_eq!(g.snapshot(), before);

    let seed = id(77);
    let plan = g.plan_rewired_move(y, Some(t), seed).unwrap();
    assert_eq!(plan.unlink.len(), 3);
    // x into s and t (2 inputs); y out of t and s, shared by both links.
    assert_eq!(plan.ports.len(), 4);
    let (applied, _) = g.move_node_rewired(y, Some(t), seed).unwrap();
    assert_eq!(applied, plan);
    assert_eq!(g.parent(y), Some(t));
    assert_eq!(g.path(y).unwrap(), "/s/t/y");
    assert_eq!(out(&mut c, &g, z, "out"), 12.0);
    let mut fresh = Cooker::new();
    assert_eq!(out(&mut fresh, &g, z, "out"), 12.0);
    let names: Vec<&str> = plan.ports.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["x_out", "x_out", "y_out", "y_out"]);
    // Moving back rewires through the existing ports.
    g.move_node_rewired(y, None, id(78)).unwrap();
    assert_eq!(g.parent(y), None);
    assert_eq!(out(&mut c, &g, z, "out"), 12.0);

    // Event links are not rewired.
    let every = g.add_node(EVERY, "every").unwrap();
    let counter = g.add_node(COUNTER, "counter").unwrap();
    let ev_in = g.node_spec(counter).unwrap().inputs[0].name;
    let ev_out = g.node_spec(every).unwrap().outputs[0].name;
    if g.link(every, ev_out, counter, ev_in).is_ok() {
        assert!(matches!(
            g.move_node_rewired(counter, Some(s), id(79)),
            Err(GraphError::CrossNetwork { .. })
        ));
    }
    // A move that needs no rewiring is a plain move.
    let lone = g.add_node(CLOCK, "lone").unwrap();
    let (plan, _) = g.move_node_rewired(lone, Some(s), id(80)).unwrap();
    assert!(plan.is_empty());
    assert_eq!(g.parent(lone), Some(s));
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

/// Property: with Charm definitions, instances, per-instance overrides,
/// definition edits (versions with renamed, added and removed nodes,
/// changed defaults), detaching, rewired moves and outside readers, a
/// long-lived cooker always agrees with a fresh one.
#[test]
fn definitions_cached_cooks_equal_fresh_cooks() {
    let variants = [
        scale(2.0, false),
        scale(4.0, true),
        scale(3.0, false),
        scale(2.0, true),
    ];
    let mut ok = [0usize; 12];
    for seed in 1..=60u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut g = graph();
        g.add_definition(variants[0].clone()).unwrap();
        let clock = g.add_node(CLOCK, "clock").unwrap();
        let x = g.add_node(ADD, "x").unwrap();
        g.link(clock, "time", x, "a").unwrap();
        let s = g
            .add_node_in(None, NodeId::new(), SUBNET_TYPE, "s", None)
            .unwrap();
        let mut plain = vec![clock, x];
        let mut instances: Vec<NodeId> = Vec::new();
        let mut sinks: Vec<NodeId> = Vec::new();
        let mut cooker = Cooker::new();
        for step in 0..40 {
            let pick = |rng: &mut Rng, v: &[NodeId]| v.get(rng.below(v.len().max(1))).copied();
            let op = rng.below(12);
            let result = match op {
                0 | 1 => {
                    let n = NodeId::new();
                    let net = if rng.below(3) == 0 { Some(s) } else { None };
                    g.instantiate(net, n, &format!("i{step}"), "scale")
                        .map(|_| instances.push(n))
                }
                2 => {
                    let v = variants[rng.below(variants.len())].clone();
                    g.update_definition(v).map(|_| ())
                }
                3 | 4 => match (pick(&mut rng, &instances), pick(&mut rng, &plain)) {
                    (Some(i), Some(p)) => {
                        let port = [IN, GAIN][rng.below(2)];
                        g.link(p, "out", iface(i, port), INTERFACE_PORT)
                    }
                    _ => Ok(()),
                },
                5 => match pick(&mut rng, &instances) {
                    Some(i) => {
                        let port = [IN, GAIN][rng.below(2)];
                        let value = float(rng.below(9) as f64);
                        if rng.below(2) == 0 {
                            g.set_input(iface(i, port), INTERFACE_PORT, value)
                        } else {
                            g.clear_input(iface(i, port), INTERFACE_PORT)
                        }
                    }
                    None => Ok(()),
                },
                6 => match (pick(&mut rng, &instances), pick(&mut rng, &instances)) {
                    (Some(i), Some(j)) if i != j => {
                        let path = g.path(j).unwrap_or_default();
                        let source = format!("ch(\"{path}/gain/value\") + $T");
                        g.set_expression(iface(i, GAIN), INTERFACE_PORT, &source)
                    }
                    _ => Ok(()),
                },
                7 => match pick(&mut rng, &instances) {
                    Some(i) => {
                        let sink = NodeId::new();
                        g.add_node_with_id(sink, ADD, &format!("sink{step}"))
                            .and_then(|_| {
                                sinks.push(sink);
                                plain.push(sink);
                                let path = g.path(i).unwrap_or_default();
                                g.set_expression(sink, "b", &format!("ch(\"{path}/mult/b\")"))
                                    .ok();
                                g.link(iface(i, OUT), INTERFACE_PORT, sink, "a")
                            })
                    }
                    None => Ok(()),
                },
                8 => match pick(&mut rng, &instances) {
                    Some(i) => g.detach(i).map(|_| instances.retain(|&n| n != i)),
                    None => Ok(()),
                },
                9 => match pick(&mut rng, &plain) {
                    Some(p) => {
                        let net = if g.parent(p).is_some() { None } else { Some(s) };
                        g.move_node_rewired(p, net, NodeId::new()).map(|_| ())
                    }
                    None => Ok(()),
                },
                10 => match pick(&mut rng, &instances) {
                    Some(i) => g.remove_node(i).map(|()| instances.retain(|&n| n != i)),
                    None => Ok(()),
                },
                _ => match pick(&mut rng, &plain) {
                    Some(p) => g.set_input(p, "b", float(rng.below(5) as f64)),
                    None => Ok(()),
                },
            };
            if result.is_ok() {
                ok[op] += 1;
            }
            if rng.below(2) == 0 {
                let frame = rng.below(4) as u64;
                cooker.set_clock(CookContext {
                    frame,
                    time: frame as f64 * 0.5,
                    dt: 0.5,
                });
            }
            let mut targets: Vec<NodeId> = g.node_ids().collect();
            targets.retain(|&n| g.node_spec(n).is_some_and(|s| !s.outputs.is_empty()));
            for &n in &targets {
                let cached = cooker.cook(&g, n);
                let mut fresh = Cooker::new();
                fresh.set_clock(cooker.clock());
                let expected = fresh.cook(&g, n);
                assert_eq!(cached, expected, "seed {seed} step {step}");
                if let Ok(values) = &cached {
                    assert!(values.iter().all(|v| !matches!(v, Value::Bool(_))));
                }
            }
        }
    }
    // Every kind of edit happened often.
    eprintln!("successful edits by kind: {ok:?}");
    assert!(ok.iter().all(|&n| n >= 20), "{ok:?}");
}
