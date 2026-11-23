// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Polymorphic ports (M0.17): resolution of type variables from links,
//! values, expressions and pins; conflicts; unresolving; invalidation;
//! undo-friendliness (order independence).

mod common;

use std::sync::Arc;

use common::{float, graph, rebuild, resolutions};
use livspeil_graph::{CookError, Cooker, Graph, GraphError, NodeId};
use livspeil_node_api::{Curve, PortType, Table, TypeClass, TypedCurveKey, Value};
use livspeil_params::{AnimChannel, Keyframe};

const SELECT: &str = "core.logic.select@2";
const LERP: &str = "core.math.lerp@2";
const RAMP: &str = "core.curve.ramp@2";
const GET: &str = "core.table.get_value@1";
const ADD: &str = "core.math.add@1";
const COMPOSE3: &str = "core.vector.compose3@1";
const LENGTH3: &str = "core.vector.length3@1";
const CLOCK: &str = "core.time.clock@1";

fn ty(g: &Graph, id: NodeId) -> PortType {
    g.resolved_type(id, "T").expect("node has T")
}

fn out(g: &Graph, id: NodeId, port: &str) -> Value {
    Cooker::new().cook_output(g, id, port).expect("cooks")
}

fn vec3(v: [f64; 3]) -> Value {
    Value::Vec3(v)
}

#[test]
fn unresolved_nodes_use_the_fallback() {
    let mut g = graph();
    let s = g.add_node(SELECT, "s").unwrap();
    assert_eq!(ty(&g, s), PortType::Float);
    assert_eq!(g.type_vars(s).len(), 1);
    assert_eq!(g.type_vars(s)[0].class, TypeClass::Any);
    assert_eq!(g.resolved_types(s), [PortType::Float]);
    assert_eq!(g.resolved_type(s, "U"), None);
    assert_eq!(out(&g, s, "out"), float(0.0));
    let add = g.add_node(ADD, "add").unwrap();
    assert!(g.type_vars(add).is_empty());
    assert!(g.resolved_types(add).is_empty());
    assert_eq!(g.resolved_type(add, "T"), None);
}

#[test]
fn values_resolve_and_defaults_follow() {
    let mut g = graph();
    let s = g.add_node(SELECT, "s").unwrap();
    g.set_input(s, "a", vec3([1.0, 2.0, 3.0])).unwrap();
    assert_eq!(ty(&g, s), PortType::Vec3);
    let spec = g.node_spec(s).unwrap();
    assert_eq!(spec.inputs[1].ty, PortType::Vec3);
    assert_eq!(spec.inputs[2].default, Some(vec3([0.0; 3])));
    assert_eq!(spec.outputs[0].ty, PortType::Vec3);
    assert_eq!(out(&g, s, "out"), vec3([0.0; 3]));
    g.set_input(s, "cond", Value::Bool(true)).unwrap();
    assert_eq!(out(&g, s, "out"), vec3([1.0, 2.0, 3.0]));
    // A conflicting value is a typed error and changes nothing.
    let before = g.snapshot();
    let err = g.set_input(s, "b", float(1.0)).unwrap_err();
    assert_eq!(
        err,
        GraphError::TypeConflict {
            node: s,
            var: "T".into(),
            expected: PortType::Vec3,
            found: PortType::Float
        }
    );
    assert!(err.to_string().contains("type conflict"));
    assert_eq!(g.snapshot(), before);
    // Replacing the only constraint is fine.
    g.set_input(s, "a", Value::String("x".into())).unwrap();
    assert_eq!(ty(&g, s), PortType::String);
    // Clearing unresolves.
    g.clear_input(s, "a").unwrap();
    assert_eq!(ty(&g, s), PortType::Float);
    assert_eq!(out(&g, s, "out"), float(1.0));
}

#[test]
fn class_violations_are_typed_errors() {
    let mut g = graph();
    let l = g.add_node(LERP, "l").unwrap();
    let err = g.set_input(l, "a", Value::Bool(true)).unwrap_err();
    assert_eq!(
        err,
        GraphError::NotInClass {
            node: l,
            var: "T".into(),
            class: TypeClass::Interpolable,
            found: PortType::Bool
        }
    );
    assert!(err.to_string().contains("Interpolable"));
    // Int is lifted to Float in a class without Int.
    g.set_input(l, "a", Value::Int(2)).unwrap();
    assert_eq!(ty(&g, l), PortType::Float);
    assert_eq!(g.input_value(l, "a").unwrap(), Some(&float(2.0)));
    // Event is in no class.
    let s = g.add_node(SELECT, "s").unwrap();
    assert!(g.set_input(s, "a", Value::Event).is_err());
}

#[test]
fn links_resolve_in_both_directions() {
    let mut g = graph();
    let c = g.add_node(COMPOSE3, "c").unwrap();
    let s = g.add_node(SELECT, "s").unwrap();
    let len = g.add_node(LENGTH3, "len").unwrap();
    // Output-driven: the target is a Vec3 input.
    g.link(s, "out", len, "v").unwrap();
    assert_eq!(ty(&g, s), PortType::Vec3);
    // A Float source now conflicts.
    let add = g.add_node(ADD, "add").unwrap();
    let err = g.link(add, "out", s, "a").unwrap_err();
    assert!(matches!(err, GraphError::TypeConflict { .. }), "{err:?}");
    // A Vec3 source fits.
    g.set_input(c, "x", float(3.0)).unwrap();
    g.set_input(c, "y", float(4.0)).unwrap();
    g.link(c, "out", s, "b").unwrap();
    assert_eq!(out(&g, len, "out"), float(5.0));
    // Removing the output link keeps the input constraint.
    g.unlink(len, "v").unwrap();
    assert_eq!(ty(&g, s), PortType::Vec3);
    g.unlink(s, "b").unwrap();
    assert_eq!(ty(&g, s), PortType::Float);
    // Now the Float source is fine, and the Vec3 target is not.
    g.link(add, "out", s, "a").unwrap();
    let err = g.link(s, "out", len, "v").unwrap_err();
    assert_eq!(
        err,
        GraphError::TypeConflict {
            node: s,
            var: "T".into(),
            expected: PortType::Vec3,
            found: PortType::Float
        }
    );
}

#[test]
fn mixed_int_and_float_join_to_float() {
    let mut g = graph();
    let clock = g.add_node(CLOCK, "clock").unwrap();
    let s = g.add_node(SELECT, "s").unwrap();
    g.link(clock, "frame", s, "a").unwrap();
    assert_eq!(ty(&g, s), PortType::Int);
    g.set_input(s, "b", Value::Int(7)).unwrap();
    assert_eq!(ty(&g, s), PortType::Int);
    assert_eq!(out(&g, s, "out"), Value::Int(7));
    g.link(clock, "time", s, "b").unwrap();
    assert_eq!(ty(&g, s), PortType::Float);
    // The stored Int value was converted.
    assert_eq!(g.input_value(s, "b").unwrap(), Some(&float(7.0)));
    // Order does not matter: the same state from scratch.
    assert_eq!(resolutions(&rebuild(&g)), resolutions(&g));
    // Unlinking the Float leaves the converted Float value.
    g.unlink(s, "b").unwrap();
    assert_eq!(ty(&g, s), PortType::Float);
    g.clear_input(s, "b").unwrap();
    assert_eq!(ty(&g, s), PortType::Int);
    // Upper bounds: Int and Float targets give Int.
    let mut g = graph();
    let s = g.add_node(SELECT, "s").unwrap();
    let add = g.add_node(ADD, "add").unwrap();
    g.link(s, "out", add, "a").unwrap();
    assert_eq!(ty(&g, s), PortType::Float);
    let sink = g.add_node("test.sink@1", "sink").unwrap();
    g.link(s, "out", sink, "count").unwrap();
    assert_eq!(ty(&g, s), PortType::Int);
    g.unlink(sink, "count").unwrap();
    assert_eq!(ty(&g, s), PortType::Float);
}

#[test]
fn chains_of_polymorphic_nodes_resolve_together() {
    let mut g = graph();
    let a = g.add_node(SELECT, "a").unwrap();
    let b = g.add_node(LERP, "b").unwrap();
    let c = g.add_node(SELECT, "c").unwrap();
    g.link(a, "out", b, "a").unwrap();
    g.link(b, "out", c, "b").unwrap();
    assert_eq!(
        (ty(&g, a), ty(&g, b), ty(&g, c)),
        (PortType::Float, PortType::Float, PortType::Float)
    );
    g.set_input(c, "a", Value::Color([0.5, 0.1, 30.0, 1.0]))
        .unwrap();
    assert_eq!(
        (ty(&g, a), ty(&g, b), ty(&g, c)),
        (PortType::Color, PortType::Color, PortType::Color)
    );
    // lerp is Interpolable: a String anywhere in the component conflicts.
    let err = g.set_input(a, "b", Value::String("x".into())).unwrap_err();
    assert!(
        matches!(
            err,
            GraphError::NotInClass { .. } | GraphError::TypeConflict { .. }
        ),
        "{err:?}"
    );
    // Unlinking splits the component; each part unresolves on its own.
    g.unlink(c, "b").unwrap();
    assert_eq!(
        (ty(&g, a), ty(&g, b), ty(&g, c)),
        (PortType::Float, PortType::Float, PortType::Color)
    );
    g.link(b, "out", c, "b").unwrap();
    assert_eq!(ty(&g, a), PortType::Color);
    // Removing the middle node unresolves the head.
    g.remove_node(b).unwrap();
    assert_eq!((ty(&g, a), ty(&g, c)), (PortType::Float, PortType::Color));
}

#[test]
fn pins_fix_the_type() {
    let mut g = graph();
    let s = g
        .add_node_in(None, NodeId::new(), SELECT, "s", Some(PortType::Vec2))
        .unwrap();
    assert_eq!(ty(&g, s), PortType::Vec2);
    assert_eq!(g.port_type(s), Some(PortType::Vec2));
    assert_eq!(out(&g, s, "out"), Value::Vec2([0.0; 2]));
    assert!(g.set_input(s, "a", float(1.0)).is_err());
    g.set_input(s, "a", Value::Vec2([1.0, 2.0])).unwrap();
    g.clear_input(s, "a").unwrap();
    assert_eq!(ty(&g, s), PortType::Vec2);
    let err = g
        .add_node_in(None, NodeId::new(), LERP, "l", Some(PortType::String))
        .unwrap_err();
    assert!(matches!(err, GraphError::NotInClass { .. }), "{err:?}");
    let err = g
        .add_node_in(None, NodeId::new(), ADD, "add", Some(PortType::Vec2))
        .unwrap_err();
    assert_eq!(err, GraphError::NotPolymorphic(ADD.into()));
    let snap = g.snapshot();
    let node = snap.nodes.iter().find(|n| n.id == s).unwrap();
    assert_eq!(node.port_type, Some(PortType::Vec2));
}

#[test]
fn expressions_and_animations_resolve() {
    let mut g = graph();
    let s = g.add_node(SELECT, "s").unwrap();
    g.set_expression(s, "a", "vec3(1, 2, 3)").unwrap();
    assert_eq!(ty(&g, s), PortType::Vec3);
    assert!(g.set_expression(s, "b", "1.5").is_err());
    g.set_expression(s, "a", "\"text\"").unwrap();
    assert_eq!(ty(&g, s), PortType::String);
    g.clear_expression(s, "a").unwrap();
    assert_eq!(ty(&g, s), PortType::Float);

    let channel = AnimChannel::new(vec![
        Keyframe::new(0.0, Value::Vec2([0.0, 0.0])),
        Keyframe::new(1.0, Value::Vec2([2.0, 4.0])),
    ])
    .unwrap();
    g.set_animation(s, "b", channel).unwrap();
    assert_eq!(ty(&g, s), PortType::Vec2);
    assert!(
        g.set_animation(s, "a", AnimChannel::constant(1.0).unwrap())
            .is_err()
    );
    g.clear_animation(s, "b").unwrap();
    assert_eq!(ty(&g, s), PortType::Float);
}

#[test]
fn expressions_read_polymorphic_ports_only_when_pinned() {
    let mut g = graph();
    let s = g.add_node(SELECT, "s").unwrap();
    let add = g.add_node(ADD, "add").unwrap();
    for source in [r#"ch("s/a")"#, r#"out("s/out") + 1"#] {
        let err = g.set_expression(add, "a", source).unwrap_err();
        assert!(
            matches!(err, GraphError::UnpinnedRead { node, .. } if node == s),
            "{err:?}"
        );
        assert!(err.to_string().contains("pinned"));
    }
    // Concrete ports of a polymorphic node are fine.
    g.set_expression(add, "a", r#"ch("s/cond") ? 1 : 0"#)
        .unwrap();
    // A pinned node's variable ports are readable with the pinned type.
    let p = g
        .add_node_in(None, NodeId::new(), SELECT, "p", Some(PortType::Vec3))
        .unwrap();
    g.set_input(p, "a", vec3([1.0, 2.0, 2.0])).unwrap();
    let len = g.add_node(LENGTH3, "len").unwrap();
    g.set_expression(len, "v", r#"ch("p/a") * 2"#).unwrap();
    assert_eq!(out(&g, len, "out"), float(6.0));
    g.set_expression(add, "b", r#"length(out("p/out"))"#)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(resolutions(&rebuild(&g)), resolutions(&g));
}

#[test]
fn resolution_changes_invalidate() {
    let mut g = graph();
    let c = g.add_node(COMPOSE3, "c").unwrap();
    let s = g.add_node(SELECT, "s").unwrap();
    let len = g.add_node(LENGTH3, "len").unwrap();
    let mut cooker = Cooker::new();
    assert_eq!(cooker.cook_output(&g, s, "out"), Ok(float(0.0)));
    g.link(s, "out", len, "v").unwrap();
    // Re-cooked as Vec3 (default b is now [0, 0, 0]).
    assert_eq!(cooker.cook_output(&g, s, "out"), Ok(vec3([0.0; 3])));
    assert_eq!(cooker.cook_output(&g, len, "out"), Ok(float(0.0)));
    g.set_input(c, "x", float(2.0)).unwrap();
    g.link(c, "out", s, "b").unwrap();
    assert_eq!(cooker.cook_output(&g, len, "out"), Ok(float(2.0)));
    g.unlink(len, "v").unwrap();
    g.unlink(s, "b").unwrap();
    assert_eq!(cooker.cook_output(&g, s, "out"), Ok(float(0.0)));
    // A downstream reader of the output follows along.
    let add = g.add_node(ADD, "add").unwrap();
    g.link(s, "out", add, "a").unwrap();
    g.set_input(s, "b", Value::Int(3)).unwrap();
    assert_eq!(ty(&g, s), PortType::Int);
    assert_eq!(cooker.cook_output(&g, add, "out"), Ok(float(3.0)));
    assert_eq!(cooker.cook_output(&g, s, "out"), Ok(Value::Int(3)));
}

#[test]
fn typed_ramps_and_table_getters() {
    let mut g = graph();
    let ramp = g.add_node(RAMP, "ramp").unwrap();
    let curve = Curve::typed(vec![
        TypedCurveKey::new(0.0, vec3([0.0; 3])),
        TypedCurveKey::new(1.0, vec3([2.0, 4.0, 6.0])),
    ])
    .unwrap();
    g.set_input(ramp, "curve", Value::Curve(Arc::new(curve)))
        .unwrap();
    g.set_input(ramp, "position", float(0.5)).unwrap();
    // The curve's key type is data: T is still Float, so the cook fails.
    assert_eq!(ty(&g, ramp), PortType::Float);
    assert!(matches!(
        Cooker::new().cook(&g, ramp),
        Err(CookError::Node(_))
    ));
    let len = g.add_node(LENGTH3, "len").unwrap();
    g.link(ramp, "out", len, "v").unwrap();
    assert_eq!(out(&g, ramp, "out"), vec3([1.0, 2.0, 3.0]));

    let get = g
        .add_node_in(None, NodeId::new(), GET, "get", Some(PortType::String))
        .unwrap();
    let table = Table::from_rows(
        &[("name", PortType::String), ("n", PortType::Int)],
        [vec![Value::String("Dune".into()), Value::Int(3)]],
    )
    .unwrap();
    g.set_input(get, "table", Value::Table(table)).unwrap();
    g.set_input(get, "column", Value::String("name".into()))
        .unwrap();
    assert_eq!(out(&g, get, "value"), Value::String("Dune".into()));
    g.set_input(get, "column", Value::String("n".into()))
        .unwrap();
    assert!(matches!(
        Cooker::new().cook(&g, get),
        Err(CookError::Node(_))
    ));
}

#[test]
fn rebuilding_in_any_order_resolves_the_same() {
    let mut g = graph();
    let clock = g.add_node(CLOCK, "clock").unwrap();
    let a = g.add_node(SELECT, "a").unwrap();
    let b = g.add_node(LERP, "b").unwrap();
    let len = g.add_node(LENGTH3, "len").unwrap();
    let add = g.add_node(ADD, "add").unwrap();
    g.link(clock, "frame", a, "a").unwrap();
    g.link(a, "out", add, "a").unwrap();
    g.link(b, "out", len, "v").unwrap();
    g.set_expression(b, "t", r#"out("clock/time") * 0.1"#)
        .unwrap();
    g.set_expression(b, "a", r#"vec3(ch("a/cond") ? 1 : 0, 0, 0)"#)
        .unwrap();
    let r = resolutions(&g);
    assert_eq!(ty(&g, a), PortType::Int);
    assert_eq!(ty(&g, b), PortType::Vec3);
    assert_eq!(resolutions(&rebuild(&g)), r);
}
