// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Subnets, Charm interfaces, `ch()` paths across networks, promotion,
//! moves, and a randomized differential test over nested graphs.

mod common;

use common::{float, graph};
use livspeil_graph::{
    CharmMeta, CookError, CookStats, Cooker, Graph, GraphError, INPUT_TYPE, INTERFACE_PORT, NodeId,
    OUTPUT_TYPE, PortDirection, SUBNET_TYPE,
};
use livspeil_node_api::{CookContext, PortType, Value};

const ADD: &str = "core.math.add@1";
const CLOCK: &str = "core.time.clock@1";
const MUL: &str = "core.math.mul@1";

fn sub(g: &mut Graph, parent: Option<NodeId>, name: &str) -> NodeId {
    g.add_node_in(parent, NodeId::new(), SUBNET_TYPE, name, None)
        .unwrap()
}

fn node(g: &mut Graph, parent: Option<NodeId>, ty: &str, name: &str) -> NodeId {
    g.add_node_in(parent, NodeId::new(), ty, name, None)
        .unwrap()
}

fn port(g: &mut Graph, subnet: NodeId, ty: &str, name: &str, pt: PortType) -> NodeId {
    g.add_node_in(Some(subnet), NodeId::new(), ty, name, Some(pt))
        .unwrap()
}

fn out(c: &mut Cooker, g: &Graph, id: NodeId, port: &str) -> f64 {
    c.cook_output(g, id, port).unwrap().as_float().unwrap()
}

/// `/x -> /s/in -> /s/double (in * 2) -> /s/out -> /y (out + 1)`.
struct Doubler {
    g: Graph,
    x: NodeId,
    s: NodeId,
    input: NodeId,
    double: NodeId,
    output: NodeId,
    y: NodeId,
}

fn doubler() -> Doubler {
    let mut g = graph();
    let x = node(&mut g, None, ADD, "x");
    let s = sub(&mut g, None, "s");
    let input = port(&mut g, s, INPUT_TYPE, "in", PortType::Float);
    let output = port(&mut g, s, OUTPUT_TYPE, "out", PortType::Float);
    let double = node(&mut g, Some(s), MUL, "double");
    g.set_input(double, "b", float(2.0)).unwrap();
    let y = node(&mut g, None, ADD, "y");
    g.set_input(y, "b", float(1.0)).unwrap();
    g.link(x, "out", input, INTERFACE_PORT).unwrap();
    g.link(input, INTERFACE_PORT, double, "a").unwrap();
    g.link(double, "out", output, INTERFACE_PORT).unwrap();
    g.link(output, INTERFACE_PORT, y, "a").unwrap();
    Doubler {
        g,
        x,
        s,
        input,
        double,
        output,
        y,
    }
}

#[test]
fn names_are_unique_per_network_and_paths_address_nodes() {
    let mut g = graph();
    let lamp = sub(&mut g, None, "lamp");
    let rig = sub(&mut g, Some(lamp), "rig");
    let bulb = node(&mut g, Some(rig), ADD, "bulb");
    // The same name in another network is fine.
    let other = node(&mut g, None, ADD, "bulb");
    assert_eq!(
        g.add_node_in(Some(rig), NodeId::new(), ADD, "bulb", None),
        Err(GraphError::DuplicateName("bulb".into()))
    );
    assert_eq!(g.path(bulb).as_deref(), Some("/lamp/rig/bulb"));
    assert_eq!(g.path(other).as_deref(), Some("/bulb"));
    assert_eq!(g.find_path("/lamp/rig/bulb"), Some(bulb));
    assert_eq!(g.find_path("lamp/rig/bulb"), Some(bulb));
    assert_eq!(g.find_path("/bulb"), Some(other));
    assert_eq!(g.find_path("/lamp/bulb"), None);
    assert_eq!(g.find("bulb"), Some(other));
    assert_eq!(g.find_in(Some(rig), "bulb"), Some(bulb));
    assert_eq!(g.parent(bulb), Some(rig));
    assert_eq!(g.parent(lamp), None);
    assert_eq!(g.children(Some(lamp)), [rig]);
    assert_eq!(g.children(None).len(), 2);
    assert!(g.is_subnet(rig) && !g.is_subnet(bulb));
}

#[test]
fn structural_rules() {
    let mut g = graph();
    let s = sub(&mut g, None, "s");
    let a = node(&mut g, None, ADD, "a");
    for bad in ["a/b", ".", ".."] {
        assert_eq!(
            g.add_node_in(Some(s), NodeId::new(), ADD, bad, None),
            Err(GraphError::InvalidName(bad.into()))
        );
        assert_eq!(
            g.add_node_in(None, NodeId::new(), SUBNET_TYPE, bad, None),
            Err(GraphError::InvalidName(bad.into()))
        );
    }
    // Root nodes keep the old freedom.
    node(&mut g, None, ADD, "a/b");
    assert_eq!(
        g.add_node_in(Some(a), NodeId::new(), ADD, "x", None),
        Err(GraphError::NotASubnet(a))
    );
    assert!(matches!(
        g.add_node_in(None, NodeId::new(), INPUT_TYPE, "i", None),
        Err(GraphError::NotInSubnet(_))
    ));
    assert_eq!(
        g.add_node_in(None, NodeId::new(), ADD, "y", Some(PortType::Color)),
        Err(GraphError::NotPolymorphic(ADD.into()))
    );
    // Root outputs are fine and typed.
    let o = g
        .add_node_in(None, NodeId::new(), OUTPUT_TYPE, "o", Some(PortType::Color))
        .unwrap();
    assert_eq!(g.port_type(o), Some(PortType::Color));
    assert_eq!(g.node_spec(o).unwrap().outputs[0].ty, PortType::Color);
    let plain = node(&mut g, None, OUTPUT_TYPE, "plain");
    assert_eq!(g.port_type(plain), Some(PortType::Float));
    assert_eq!(g.port_type(a), None);
    assert_eq!(
        g.link(a, "out", o, INTERFACE_PORT),
        Err(GraphError::TypeMismatch {
            expected: PortType::Color,
            found: PortType::Float
        })
    );
}

#[test]
fn links_cross_networks_only_through_the_interface() {
    let Doubler {
        mut g,
        x,
        s,
        input,
        double,
        output,
        y,
    } = doubler();
    // Directly into a node inside the subnet: refused.
    g.unlink(double, "a").unwrap();
    assert_eq!(
        g.link(x, "out", double, "a"),
        Err(GraphError::CrossNetwork {
            from: x,
            to: double
        })
    );
    // Out of an inner non-interface node: refused.
    assert_eq!(
        g.link(double, "out", y, "b"),
        Err(GraphError::CrossNetwork {
            from: double,
            to: y
        })
    );
    // An interface input is fed from outside only.
    g.unlink(input, INTERFACE_PORT).unwrap();
    assert_eq!(
        g.link(double, "out", input, INTERFACE_PORT),
        Err(GraphError::CrossNetwork {
            from: double,
            to: input
        })
    );
    // An interface output can be read inside as well.
    let inner = node(&mut g, Some(s), ADD, "inner");
    g.link(output, INTERFACE_PORT, inner, "a").unwrap();
    // Two levels down is not one boundary.
    let deep = sub(&mut g, Some(s), "deep");
    let deep_in = port(&mut g, deep, INPUT_TYPE, "in", PortType::Float);
    assert!(matches!(
        g.link(x, "out", deep_in, INTERFACE_PORT),
        Err(GraphError::CrossNetwork { .. })
    ));
    // Sibling subnets connect output to input.
    let t = sub(&mut g, None, "t");
    let t_in = port(&mut g, t, INPUT_TYPE, "in", PortType::Float);
    g.link(output, INTERFACE_PORT, t_in, INTERFACE_PORT)
        .unwrap();
    assert_eq!(g.interface(s, PortDirection::Input), [input]);
    assert_eq!(g.interface(s, PortDirection::Output), [output]);
    assert_eq!(
        g.interface_port(s, PortDirection::Output, "out"),
        Some(output)
    );
    assert_eq!(g.interface_port(s, PortDirection::Input, "out"), None);
}

#[test]
fn cooking_crosses_the_boundary_with_caching_and_cutoff() {
    let d = doubler();
    let mut g = d.g;
    g.set_input(d.x, "a", float(3.0)).unwrap();
    let mut c = Cooker::new();
    assert_eq!(out(&mut c, &g, d.y, "out"), 7.0);
    // x, s/in, s/double, s/out, y.
    assert_eq!(
        c.stats(),
        CookStats {
            cooked: 5,
            reused: 0
        }
    );
    c.reset_stats();
    assert_eq!(out(&mut c, &g, d.y, "out"), 7.0);
    assert_eq!(
        c.stats(),
        CookStats {
            cooked: 0,
            reused: 5
        }
    );
    // An inner edit invalidates the outer reader.
    c.reset_stats();
    g.set_input(d.double, "b", float(3.0)).unwrap();
    assert_eq!(out(&mut c, &g, d.y, "out"), 10.0);
    assert_eq!(
        c.stats(),
        CookStats {
            cooked: 3,
            reused: 2
        }
    );
    // Early cutoff across the boundary: same value, nothing downstream.
    c.reset_stats();
    g.set_input(d.x, "a", float(1.0)).unwrap();
    g.set_input(d.x, "b", float(2.0)).unwrap();
    assert_eq!(out(&mut c, &g, d.y, "out"), 10.0);
    assert_eq!(
        c.stats(),
        CookStats {
            cooked: 1,
            reused: 4
        }
    );
    // The subnet node itself has no outputs.
    assert_eq!(c.cook(&g, d.s), Ok(Vec::new()));
    // The interface output cooks the inner nodes.
    let mut fresh = Cooker::new();
    assert_eq!(out(&mut fresh, &g, d.output, INTERFACE_PORT), 9.0);
    assert_eq!(fresh.stats().cooked, 4);
}

#[test]
fn time_dependence_and_errors_cross_the_boundary() {
    let d = doubler();
    let mut g = d.g;
    let clock = node(&mut g, None, CLOCK, "clock");
    g.link(clock, "frame", d.x, "a").unwrap();
    let mut c = Cooker::new();
    for frame in 0..3 {
        c.set_clock(CookContext {
            frame,
            time: 0.0,
            dt: 0.0,
        });
        assert_eq!(out(&mut c, &g, d.y, "out"), frame as f64 * 2.0 + 1.0);
    }
    // A failing node inside the subnet keeps its last good output and
    // carries the error; the outside reader keeps working.
    let fragile = node(&mut g, Some(d.s), "test.fragile@1", "fragile");
    g.unlink(d.output, INTERFACE_PORT).unwrap();
    g.link(d.double, "out", fragile, "x").unwrap();
    g.link(fragile, "out", d.output, INTERFACE_PORT).unwrap();
    assert_eq!(out(&mut c, &g, d.y, "out"), 9.0);
    g.set_input(d.double, "b", float(-1.0)).unwrap();
    assert_eq!(out(&mut c, &g, d.y, "out"), 9.0);
    assert!(matches!(c.error(fragile), Some(CookError::Node(_))));
    // A panic inside is contained too.
    g.set_input(d.double, "b", float(1e6)).unwrap();
    assert_eq!(out(&mut c, &g, d.y, "out"), 9.0);
    assert!(matches!(c.error(fragile), Some(CookError::Panicked(_))));
}

#[test]
fn ch_paths_are_relative_to_the_network() {
    let mut g = graph();
    let top = node(&mut g, None, ADD, "x");
    g.set_input(top, "a", float(10.0)).unwrap();
    let s = sub(&mut g, None, "s");
    let inner_x = node(&mut g, Some(s), ADD, "x");
    g.set_input(inner_x, "a", float(20.0)).unwrap();
    let t = sub(&mut g, Some(s), "t");
    let deep = node(&mut g, Some(t), ADD, "deep");
    g.set_input(deep, "a", float(30.0)).unwrap();
    let reader = node(&mut g, Some(s), ADD, "reader");
    let mut c = Cooker::new();
    for (source, expected) in [
        (r#"ch("x/a")"#, 20.0),
        (r#"ch("../x/a")"#, 10.0),
        (r#"ch("../../x/a")"#, 10.0),
        (r#"ch("/x/a")"#, 10.0),
        (r#"ch("/s/x/a")"#, 20.0),
        (r#"ch("t/deep/a")"#, 30.0),
        (r#"ch("../s/t/deep/a")"#, 30.0),
    ] {
        g.set_expression(reader, "a", source).unwrap();
        assert_eq!(out(&mut c, &g, reader, "out"), expected, "{source}");
    }
    // From the root: the pre-M0.7 reading still applies.
    let root_reader = node(&mut g, None, ADD, "root_reader");
    for (source, expected) in [
        (r#"ch("x/a")"#, 10.0),
        (r#"ch("../x/a")"#, 10.0),
        (r#"ch("s/x/a")"#, 20.0),
        (r#"ch("s/t/deep/a")"#, 30.0),
    ] {
        g.set_expression(root_reader, "a", source).unwrap();
        assert_eq!(out(&mut c, &g, root_reader, "out"), expected, "{source}");
    }
    for bad in [r#"ch("x/x/a")"#, r#"ch("t/a")"#, r#"ch("/s/../x/a")"#] {
        assert!(
            matches!(
                g.set_expression(reader, "b", bad),
                Err(GraphError::Expression(_))
            ),
            "{bad}"
        );
    }
    // Invalidation follows ch() across networks.
    g.set_expression(reader, "a", r#"ch("t/deep/a")"#).unwrap();
    g.set_input(deep, "a", float(31.0)).unwrap();
    assert_eq!(out(&mut c, &g, reader, "out"), 31.0);
    // Removing a read node is refused, removing the whole subnet that
    // contains reader and read too, while the root reads into it.
    assert!(matches!(
        g.remove_node(t),
        Err(GraphError::Referenced { by, .. }) if by == reader
    ));
    g.remove_node(s).unwrap_err();
    g.clear_expression(root_reader, "a").unwrap();
    g.remove_node(s).unwrap();
    assert_eq!(g.node_count(), 2);
    assert_eq!(g.find_path("/s/x"), None);
}

#[test]
fn renaming_a_subnet_rewrites_paths_through_it() {
    let mut g = graph();
    let s = sub(&mut g, None, "s");
    let inner = node(&mut g, Some(s), ADD, "inner");
    let root = node(&mut g, None, ADD, "root");
    let sibling = node(&mut g, Some(s), ADD, "sibling");
    g.set_expression(root, "a", r#"ch("s/inner/a") + ch("/s/inner/b")"#)
        .unwrap();
    g.set_expression(sibling, "a", r#"ch("inner/a") + ch("../root/b")"#)
        .unwrap();
    g.rename(s, "lamp").unwrap();
    assert_eq!(
        g.expression(root, "a").unwrap(),
        Some(r#"ch("lamp/inner/a") + ch("/lamp/inner/b")"#)
    );
    // Relative paths inside the subnet do not name it.
    assert_eq!(
        g.expression(sibling, "a").unwrap(),
        Some(r#"ch("inner/a") + ch("../root/b")"#)
    );
    g.rename(inner, "core").unwrap();
    g.rename(root, "top").unwrap();
    assert_eq!(
        g.expression(root, "a").unwrap(),
        Some(r#"ch("lamp/core/a") + ch("/lamp/core/b")"#)
    );
    assert_eq!(
        g.expression(sibling, "a").unwrap(),
        Some(r#"ch("core/a") + ch("../top/b")"#)
    );
    assert_eq!(
        g.rename(inner, "a/b"),
        Err(GraphError::InvalidName("a/b".into()))
    );
}

#[test]
fn promotion_exposes_an_inner_input() {
    let mut g = graph();
    let s = sub(&mut g, None, "s");
    let inner = node(&mut g, Some(s), MUL, "inner");
    g.set_input(inner, "a", float(4.0)).unwrap();
    g.set_input(inner, "b", float(2.0)).unwrap();
    let before = g.snapshot();
    let gain = g.promote(inner, "b", NodeId::new(), "gain").unwrap();
    assert_eq!(g.port_type(gain), Some(PortType::Float));
    assert_eq!(
        g.input_value(gain, INTERFACE_PORT).unwrap(),
        Some(&float(2.0))
    );
    let mut c = Cooker::new();
    assert_eq!(out(&mut c, &g, inner, "out"), 8.0);
    // Set the promoted parameter on the interface.
    g.set_input(gain, INTERFACE_PORT, float(3.0)).unwrap();
    assert_eq!(out(&mut c, &g, inner, "out"), 12.0);
    // Or drive it from outside.
    let outer = node(&mut g, None, ADD, "outer");
    g.set_input(outer, "a", float(5.0)).unwrap();
    g.link(outer, "out", gain, INTERFACE_PORT).unwrap();
    assert_eq!(out(&mut c, &g, inner, "out"), 20.0);
    g.remove_node(outer).unwrap();
    // Unpromote keeps the promoted value on the inner node.
    assert_eq!(g.unpromote(inner, "b"), Ok(Some(gain)));
    assert!(!g.contains(gain));
    assert_eq!(g.input_value(inner, "b").unwrap(), Some(&float(3.0)));
    assert_eq!(g.unpromote(inner, "b"), Ok(None));
    g.set_input(inner, "b", float(2.0)).unwrap();
    assert_eq!(g.snapshot(), before);
    // Root nodes have no subnet to promote to; linked ports are refused.
    let r = node(&mut g, None, ADD, "r");
    assert_eq!(
        g.promote(r, "a", NodeId::new(), "x"),
        Err(GraphError::NotInSubnet(r))
    );
    let other = node(&mut g, Some(s), ADD, "other");
    g.link(other, "out", inner, "a").unwrap();
    assert!(matches!(
        g.promote(inner, "a", NodeId::new(), "x"),
        Err(GraphError::InputOccupied { .. })
    ));
    assert_eq!(g.snapshot().nodes.len(), 4);
}

#[test]
fn charm_metadata() {
    let mut g = graph();
    let s = sub(&mut g, None, "s");
    let a = node(&mut g, None, ADD, "a");
    let mut meta = CharmMeta::new("Daylight");
    meta.version = Some("1.0.0".into());
    meta.min_tier = 1;
    assert_eq!(g.set_charm(s, Some(meta.clone())), Ok(None));
    assert_eq!(g.charm(s), Some(&meta));
    assert_eq!(g.set_charm(a, None), Err(GraphError::NotASubnet(a)));
    let snap = g.snapshot();
    assert_eq!(
        snap.nodes.iter().find(|n| n.id == s).unwrap().charm,
        Some(meta.clone())
    );
    assert_eq!(g.set_charm(s, None), Ok(Some(meta)));
    assert_eq!(g.charm(s), None);
}

#[test]
fn moving_nodes_keeps_what_expressions_read() {
    let mut g = graph();
    let s = sub(&mut g, None, "s");
    let a = node(&mut g, None, ADD, "a");
    g.set_input(a, "a", float(1.0)).unwrap();
    let b = node(&mut g, None, ADD, "b");
    g.set_expression(b, "a", r#"ch("a/a") * 2"#).unwrap();
    let c_node = node(&mut g, None, ADD, "c");
    g.set_expression(c_node, "a", r#"ch("b/a")"#).unwrap();
    let mut c = Cooker::new();
    assert_eq!(out(&mut c, &g, c_node, "out"), 2.0);
    c.reset_stats();
    let rewritten = g.move_node(b, Some(s)).unwrap();
    assert_eq!(g.path(b).as_deref(), Some("/s/b"));
    assert_eq!(g.expression(b, "a").unwrap(), Some(r#"ch("../a/a") * 2"#));
    assert_eq!(g.expression(c_node, "a").unwrap(), Some(r#"ch("s/b/a")"#));
    assert_eq!(rewritten.len(), 2);
    // Moving changes nothing that is computed.
    assert_eq!(out(&mut c, &g, c_node, "out"), 2.0);
    assert_eq!(c.stats().cooked, 0);
    // Links must stay valid.
    g.link(a, "out", c_node, "b").unwrap();
    assert!(matches!(
        g.move_node(a, Some(s)),
        Err(GraphError::CrossNetwork { .. })
    ));
    assert!(matches!(
        g.move_node(s, Some(s)),
        Err(GraphError::WouldNest(_))
    ));
    assert_eq!(g.move_node(a, Some(b)), Err(GraphError::NotASubnet(b)));
    // And back: `../a/a` still reads `/a` from the root, so only the
    // reader's path changes.
    let back = g.move_node(b, None).unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(g.expression(b, "a").unwrap(), Some(r#"ch("../a/a") * 2"#));
    assert_eq!(g.expression(c_node, "a").unwrap(), Some(r#"ch("b/a")"#));
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

/// Property: on graphs with nested subnets, interface ports and
/// cross-network `ch()` reads, a long-lived cooker always agrees with a
/// fresh one after random edits (values, links across boundaries,
/// expressions, renames, moves, promotion, subnet removal) and clock
/// changes.
#[test]
fn nested_graphs_cached_cooks_equal_fresh_cooks() {
    let exprs = [
        "$T * 0.5 + $F",
        "ch(\"{p}\") * 0.5",
        "ch(\"{p}\") + ch(\"{q}\")",
        "3",
        // out() (M0.11): outputs across networks, mixed with ch().
        "out(\"{o}\") * 0.5",
        "out(\"{o}\") + ch(\"{q}\")",
    ];
    for seed in 1..=200u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut g = graph();
        let s1 = sub(&mut g, None, "s1");
        let s2 = sub(&mut g, Some(s1), "s2");
        let nets = [None, Some(s1), Some(s2)];
        let mut ids: Vec<NodeId> = Vec::new();
        let mut counter = 0;
        let mut add = |g: &mut Graph, rng: &mut Rng, ids: &mut Vec<NodeId>| {
            let net = nets[rng.below(3)];
            let ty = match (net, rng.below(6)) {
                (Some(_), 0) => INPUT_TYPE,
                (_, 1) => OUTPUT_TYPE,
                (_, 2) => CLOCK,
                _ => ADD,
            };
            counter += 1;
            if let Ok(id) = g.add_node_in(net, NodeId::new(), ty, &format!("n{counter}"), None) {
                ids.push(id);
            }
        };
        for _ in 0..9 {
            add(&mut g, &mut rng, &mut ids);
        }
        let mut cooker = Cooker::new();
        for step in 0..40 {
            if ids.is_empty() {
                add(&mut g, &mut rng, &mut ids);
                continue;
            }
            let pick = |rng: &mut Rng| ids[rng.below(ids.len())];
            let target = pick(&mut rng);
            let inputs: Vec<&str> = g
                .node_spec(target)
                .map(|s| s.inputs.iter().map(|p| p.name).collect())
                .unwrap_or_default();
            if inputs.is_empty() {
                continue;
            }
            let port = inputs[rng.below(inputs.len())];
            let path = |rng: &mut Rng, g: &Graph| {
                let read = ids[rng.below(ids.len())];
                let first = g
                    .node_spec(read)
                    .and_then(|s| s.inputs.first().map(|p| p.name))
                    .unwrap_or("a");
                g.path(read)
                    .map(|p| format!("{p}/{first}"))
                    .unwrap_or_default()
            };
            let out_path = |rng: &mut Rng, g: &Graph| {
                let read = ids[rng.below(ids.len())];
                let first = g
                    .node_spec(read)
                    .and_then(|s| s.outputs.first().map(|p| p.name))
                    .unwrap_or("out");
                g.path(read)
                    .map(|p| format!("{p}/{first}"))
                    .unwrap_or_default()
            };
            let _ = match rng.below(12) {
                0 | 1 => g.set_input(target, port, float(rng.below(50) as f64 / 10.0)),
                2 | 3 => {
                    let from = pick(&mut rng);
                    let out = g
                        .node_spec(from)
                        .and_then(|s| {
                            let n = s.outputs.len();
                            (n > 0).then(|| s.outputs[rng.below(n)].name)
                        })
                        .unwrap_or("out");
                    g.link(from, out, target, port)
                }
                4 => g.unlink(target, port).map(|_| ()),
                5 | 6 => {
                    let source = exprs[rng.below(exprs.len())]
                        .replace("{p}", &path(&mut rng, &g))
                        .replace("{q}", &path(&mut rng, &g))
                        .replace("{o}", &out_path(&mut rng, &g));
                    g.set_expression(target, port, &source)
                }
                7 => g.clear_expression(target, port),
                8 => g.rename(target, &format!("r{step}")).map(|_| ()),
                9 => g.move_node(target, nets[rng.below(3)]).map(|_| ()),
                10 => g
                    .promote(target, port, NodeId::new(), &format!("p{step}"))
                    .map(|id| ids.push(id)),
                _ => {
                    if g.remove_node(target).is_ok() {
                        ids.retain(|&id| g.contains(id));
                    }
                    Ok(())
                }
            };
            if rng.below(2) == 0 {
                let frame = rng.below(4) as u64;
                cooker.set_clock(CookContext {
                    frame,
                    time: frame as f64 * 0.4,
                    dt: 0.4,
                });
            }
            for &id in &ids {
                let cached = cooker.cook(&g, id);
                let mut fresh = Cooker::new();
                fresh.set_clock(cooker.clock());
                let expected = fresh.cook(&g, id);
                assert_eq!(cached, expected, "seed {seed} step {step}");
                if let Ok(values) = &cached {
                    assert!(values.iter().all(|v| !matches!(v, Value::Bool(_))));
                }
            }
        }
    }
}
